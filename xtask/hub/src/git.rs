//! The `git` source's rows: the repository's state read from git's machine-readable
//! output (`of`, SPEC.md section 3). Times are UTC, as the `ago`, `time` and `date`
//! formats read them.

use std::path::Path;
use std::time::Duration;

use crate::expr::{Row, Value};
use crate::source::run;
use crate::template::utc;

/// What a `git` source can read, for the config check.
pub const OF: &[&str] = &[
  "head",
  "status",
  "tree",
  "log",
  "reflog",
  "branches",
  "tags",
  "stashes",
  "remotes",
  "worktrees",
  "bisect",
  "changes",
  "split",
  "words",
  "commit",
  "blame",
  "conflicts",
];

/// A time as the formats read it: UTC, to the second.
const DATE: &str = "format-local:%Y-%m-%dT%H:%M:%SZ";

/// The longest a git read may take.
const GIT_LIMIT: Duration = Duration::from_secs(30);

/// A bisection under way, as `git bisect log` tells it: one row per step, `verb`
/// (`good`, `bad`, `skip`, `first bad`), `hash`, `short`, `subject`. None when no
/// bisection is under way.
pub fn bisect(base: &Path) -> Result<Vec<Row>, String> {
  let Ok(text) = git(base, &["bisect", "log"], &[0]) else {
    return Ok(Vec::new());
  };
  let mut rows = Vec::new();
  for line in text.lines() {
    // `# good: [hash] subject`, `# first bad commit: [hash] subject`
    let Some(rest) = line.strip_prefix("# ") else {
      continue;
    };
    let Some((verb, rest)) = rest.split_once(": [") else {
      continue;
    };
    let Some((hash, subject)) = rest.split_once("] ") else {
      continue;
    };
    let verb = verb.trim_end_matches(" commit");
    rows.push(Row::from([
      ("verb".to_string(), s(verb)),
      ("hash".to_string(), s(hash)),
      ("short".to_string(), s(hash.get(..7).unwrap_or(hash))),
      ("subject".to_string(), s(subject)),
    ]));
  }
  Ok(rows)
}

/// A file's lines with who changed each last: `line`, `text`, `hash`, `short`,
/// `author`, `when`, `summary`, `first` (the first of a run of lines from the same
/// commit, so a table can show the commit once) and `first_when` (`when` on such a line,
/// else empty).
pub fn blame(base: &Path, path: &str) -> Result<Vec<Row>, String> {
  let text = git(base, &["blame", "--porcelain", "--", path], &[0])?;
  let mut rows = Vec::new();
  let mut commits: std::collections::HashMap<String, Row> = std::collections::HashMap::new();
  let mut current: Option<(String, u64)> = None;
  let mut last_hash = String::new();
  for line in text.lines() {
    if let Some(content) = line.strip_prefix('\t') {
      let Some((hash, number)) = current.take() else {
        continue;
      };
      let info = commits.get(&hash).cloned().unwrap_or_default();
      let mut row = Row::new();
      row.insert("line".into(), n(number as f64));
      row.insert("text".into(), s(content));
      row.insert("hash".into(), s(&hash));
      row.insert("short".into(), s(hash.get(..7).unwrap_or(&hash)));
      for field in ["author", "when", "summary"] {
        row.insert(field.into(), info.get(field).cloned().unwrap_or(s("")));
      }
      let first = hash != last_hash;
      row.insert("first".into(), b(first));
      // The time again on the first line of a run only, for a column showing it once.
      let when = row["when"].clone();
      row.insert("first_when".into(), if first { when } else { s("") });
      last_hash = hash;
      rows.push(row);
      continue;
    }
    let mut parts = line.split_whitespace();
    let first = parts.next().unwrap_or("");
    if first.len() == 40 && first.chars().all(|c| c.is_ascii_hexdigit()) {
      let number = parts.nth(1).and_then(|x| x.parse().ok()).unwrap_or(0);
      current = Some((first.to_string(), number));
      commits.entry(first.to_string()).or_default();
    } else if let Some((hash, _)) = &current {
      let info = commits.entry(hash.clone()).or_default();
      if let Some(author) = line.strip_prefix("author ") {
        info.insert("author".into(), s(author));
      } else if let Some(time) = line.strip_prefix("author-time ") {
        let secs: i64 = time.parse().unwrap_or(0);
        info.insert("when".into(), s(&utc(secs)));
      } else if let Some(summary) = line.strip_prefix("summary ") {
        info.insert("summary".into(), s(summary));
      }
    }
  }
  Ok(rows)
}

/// Branches, local then remote: `name`, `remote` (whether it is one), `head` (checked
/// out), `upstream`, `ahead`, `behind`, `gone` (its upstream was deleted), `short`,
/// `when`, `subject`.
pub fn branches(base: &Path) -> Result<Vec<Row>, String> {
  let format = format!(
    "--format=%(HEAD)%1f%(refname)%1f%(refname:short)%1f%(upstream:short)%1f%(upstream:track,nobracket)%1f%(objectname:short)%1f%(committerdate:{})%1f%(subject)",
    DATE
  );
  let text = git(
    base,
    &[
      "for-each-ref",
      "--sort=-committerdate",
      &format,
      "refs/heads",
      "refs/remotes",
    ],
    &[0],
  )?;
  let mut rows = Vec::new();
  for line in text.lines() {
    let f: Vec<&str> = line.split('\u{1f}').collect();
    if f.len() < 8 || f[1].ends_with("/HEAD") {
      continue;
    }
    let (mut ahead, mut behind) = (0.0, 0.0);
    for part in f[4].split(", ") {
      if let Some(x) = part.strip_prefix("ahead ") {
        ahead = x.parse().unwrap_or(0.0);
      } else if let Some(x) = part.strip_prefix("behind ") {
        behind = x.parse().unwrap_or(0.0);
      }
    }
    let mut row = Row::new();
    row.insert("head".into(), b(f[0] == "*"));
    row.insert("remote".into(), b(f[1].starts_with("refs/remotes/")));
    row.insert("name".into(), s(f[2]));
    row.insert("upstream".into(), s(f[3]));
    row.insert("ahead".into(), n(ahead));
    row.insert("behind".into(), n(behind));
    row.insert("gone".into(), b(f[4] == "gone"));
    row.insert("short".into(), s(f[5]));
    row.insert("when".into(), s(f[6]));
    row.insert("subject".into(), s(f[7]));
    rows.push(row);
  }
  rows.sort_by_key(|r| r["remote"].truthy());
  Ok(rows)
}

/// A file's changes, as `diff_rows` gives them: those not staged first (`staged` false),
/// then those staged (true); an untracked file's as wholly added. Each side starts with
/// a row of kind `section` naming it.
pub fn changes(base: &Path, path: &str) -> Result<Vec<Row>, String> {
  let tracked = git(base, &["ls-files", "--error-unmatch", "--", path], &[0]).is_ok();
  let mut sides = Vec::new();
  if tracked {
    let text = git(base, &["diff", "--no-ext-diff", "-M", "--", path], &[0])?;
    sides.push(("unstaged", "Not staged", diff_rows(&text, "unstaged")));
    let text = git(
      base,
      &["diff", "--cached", "--no-ext-diff", "-M", "--", path],
      &[0],
    )?;
    sides.push(("staged", "Staged", diff_rows(&text, "staged")));
  } else {
    let text = git(
      base,
      &[
        "diff",
        "--no-ext-diff",
        "--no-index",
        "--",
        "/dev/null",
        path,
      ],
      &[0, 1],
    )?;
    sides.push(("untracked", "Untracked", diff_rows(&text, "untracked")));
  }
  let mut rows = Vec::new();
  for (side, title, lines) in sides {
    if lines.is_empty() {
      continue;
    }
    rows.push(Row::from([
      ("side".to_string(), s(side)),
      ("staged".to_string(), b(side == "staged")),
      ("file".to_string(), s(path)),
      ("kind".to_string(), s("section")),
      ("old".to_string(), s("")),
      ("new".to_string(), s("")),
      ("text".to_string(), s(&format!("── {} ──", title))),
      ("hunk".to_string(), n(0.0)),
    ]));
    rows.extend(lines);
  }
  Ok(rows)
}

/// A conflicted file's conflicts, one row per line of each, and each row carrying what
/// resolves its conflict: `hunk` (its number), `kind` (`ours`, `base`, `theirs`, a
/// marker as `marker`, or `context` around), `text`, `line`, and for the whole hunk
/// `ours_file`, `theirs_file` and `both_file`: the file with that conflict taken as ours,
/// as theirs, or as both (ours, then theirs), the others left as they are, for an action
/// to write back.
pub fn conflicts(base: &Path, path: &str) -> Result<Vec<Row>, String> {
  let text = std::fs::read_to_string(base.join(path)).map_err(|e| e.to_string())?;
  let lines: Vec<&str> = text.split_inclusive('\n').collect();
  // Each conflict: (start, middle base marker, separator, end) line indices.
  let mut found: Vec<(usize, Option<usize>, usize, usize)> = Vec::new();
  let mut at = 0;
  while at < lines.len() {
    if lines[at].starts_with("<<<<<<<") {
      let start = at;
      let mut base_marker = None;
      let mut separator = None;
      at += 1;
      while at < lines.len() && !lines[at].starts_with(">>>>>>>") {
        if lines[at].starts_with("|||||||") {
          base_marker = Some(at);
        } else if lines[at].starts_with("=======") {
          separator = Some(at);
        }
        at += 1;
      }
      if let (Some(sep), true) = (separator, at < lines.len()) {
        found.push((start, base_marker, sep, at));
      }
    }
    at += 1;
  }
  let resolved = |which: usize, take: &str| -> String {
    let (start, base_marker, sep, end) = found[which];
    let ours_end = base_marker.unwrap_or(sep);
    let mut out = String::new();
    out.extend(lines[..start].iter().copied());
    let ours = &lines[start + 1..ours_end];
    let theirs = &lines[sep + 1..end];
    match take {
      "ours" => out.extend(ours.iter().copied()),
      "theirs" => out.extend(theirs.iter().copied()),
      _ => {
        out.extend(ours.iter().copied());
        out.extend(theirs.iter().copied());
      }
    }
    out.extend(lines[end + 1..].iter().copied());
    out
  };
  let mut rows = Vec::new();
  for (k, &(start, base_marker, sep, end)) in found.iter().enumerate() {
    let files = [
      ("ours_file", resolved(k, "ours")),
      ("theirs_file", resolved(k, "theirs")),
      ("both_file", resolved(k, "both")),
    ];
    let from = start.saturating_sub(2);
    let to = (end + 3).min(lines.len());
    for (i, line) in lines.iter().enumerate().take(to).skip(from) {
      let kind = if i == start || i == sep || i == end || Some(i) == base_marker {
        "marker"
      } else if i < start || i > end {
        "context"
      } else if base_marker.is_some_and(|b| i > b && i < sep) {
        "base"
      } else if i < base_marker.unwrap_or(sep) {
        "ours"
      } else {
        "theirs"
      };
      let mut row = Row::from([
        ("hunk".to_string(), n((k + 1) as f64)),
        ("kind".to_string(), s(kind)),
        ("text".to_string(), s(line.trim_end_matches('\n'))),
        ("line".to_string(), n((i + 1) as f64)),
        ("path".to_string(), s(path)),
      ]);
      for (name, file) in &files {
        row.insert(name.to_string(), s(file));
      }
      rows.push(row);
    }
  }
  Ok(rows)
}

/// A unified diff as rows, one per line: `side` (as given: `unstaged`, `staged`,
/// `untracked`, `commit`), `staged`, `file`, `kind` (`file`, `hunk`, `context`, `add`,
/// `remove`, `note`), `old` and `new` (line numbers, or empty), `text` (the line, its
/// `+`, `-` or space kept), `hunk` (its number in the file, from 1; 0 on the file's
/// header), and on every line of a hunk `patch`: that hunk alone with its file's header,
/// as `git apply` takes it. An added or removed line also has `line_patch`: the patch
/// for that line alone.
pub fn diff_rows(text: &str, side: &str) -> Vec<Row> {
  let mut rows = Vec::new();
  // The lines of the file's header, and of the hunk being read: (row index, text).
  let mut header: Vec<String> = Vec::new();
  let mut file = String::new();
  let mut hunk: Vec<(usize, String)> = Vec::new();
  let mut number = 0;
  let (mut old, mut new) = (0u64, 0u64);
  let row = |kind: &str, file: &str, text: &str, number: usize| {
    Row::from([
      ("side".to_string(), s(side)),
      ("staged".to_string(), b(side == "staged")),
      ("file".to_string(), s(file)),
      ("kind".to_string(), s(kind)),
      ("old".to_string(), s("")),
      ("new".to_string(), s("")),
      ("text".to_string(), s(text)),
      ("hunk".to_string(), n(number as f64)),
    ])
  };
  let finish = |rows: &mut Vec<Row>, header: &[String], hunk: &mut Vec<(usize, String)>| {
    if hunk.is_empty() {
      return;
    }
    let lines: Vec<&str> = hunk.iter().map(|(_, l)| l.as_str()).collect();
    let patch = format!("{}\n{}\n", header.join("\n"), lines.join("\n"));
    for (i, (at, line)) in hunk.iter().enumerate() {
      rows[*at].insert("patch".into(), s(&patch));
      if line.starts_with('+') || line.starts_with('-') {
        rows[*at].insert(
          "line_patch".into(),
          s(&single_line_patch(header, &lines, i)),
        );
      }
    }
    hunk.clear();
  };
  for line in text.lines() {
    if line.starts_with("diff --git ") {
      finish(&mut rows, &header, &mut hunk);
      header = vec![line.to_string()];
      number = 0;
      file = line
        .rsplit_once(" b/")
        .map(|(_, p)| p.to_string())
        .unwrap_or_default();
      rows.push(row("file", &file, line, 0));
    } else if line.starts_with("@@") {
      finish(&mut rows, &header, &mut hunk);
      number += 1;
      let (o, n2) = hunk_starts(line);
      old = o;
      new = n2;
      rows.push(row("hunk", &file, line, number));
      hunk.push((rows.len() - 1, line.to_string()));
    } else if number == 0 {
      // The file's header: index, modes, ---, +++.
      if let Some(p) = line.strip_prefix("+++ b/") {
        file = p.to_string();
        for r in rows.iter_mut().rev() {
          if r["kind"].text() != "file" {
            break;
          }
          r.insert("file".into(), s(&file));
        }
      }
      header.push(line.to_string());
      rows.push(row("note", &file, line, 0));
    } else {
      let (kind, o, n2) = match line.chars().next() {
        Some('+') => ("add", None, Some(new)),
        Some('-') => ("remove", Some(old), None),
        Some('\\') => ("note", None, None),
        _ => ("context", Some(old), Some(new)),
      };
      let mut r = row(kind, &file, line, number);
      if let Some(o) = o {
        r.insert("old".into(), n(o as f64));
        old += 1;
      }
      if let Some(x) = n2 {
        r.insert("new".into(), n(x as f64));
        new += 1;
      }
      rows.push(r);
      hunk.push((rows.len() - 1, line.to_string()));
    }
  }
  finish(&mut rows, &header, &mut hunk);
  rows
}

/// The branch checked out and how it stands: `branch`, `short` (the commit), `upstream`,
/// `ahead`, `behind`, `dirty`, `subject`, `when`.
pub fn head(base: &Path) -> Result<Row, String> {
  let text = git(
    base,
    &[
      "status",
      "--porcelain=v2",
      "--branch",
      "--untracked-files=no",
    ],
    &[0],
  )?;
  let mut row = Row::new();
  let mut dirty = false;
  for line in text.lines() {
    if let Some(rest) = line.strip_prefix("# branch.oid ") {
      row.insert("commit".into(), s(rest));
      row.insert("short".into(), s(rest.get(..7).unwrap_or(rest)));
    } else if let Some(rest) = line.strip_prefix("# branch.head ") {
      row.insert("branch".into(), s(rest));
    } else if let Some(rest) = line.strip_prefix("# branch.upstream ") {
      row.insert("upstream".into(), s(rest));
    } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
      let mut parts = rest.split_whitespace();
      let ahead = parts
        .next()
        .and_then(|a| a.trim_start_matches('+').parse().ok());
      let behind = parts
        .next()
        .and_then(|a| a.trim_start_matches('-').parse().ok());
      row.insert("ahead".into(), n(ahead.unwrap_or(0.0)));
      row.insert("behind".into(), n(behind.unwrap_or(0.0)));
    } else if !line.starts_with('#') {
      dirty = true;
    }
  }
  row.entry("upstream".into()).or_insert(s(""));
  row.entry("ahead".into()).or_insert(n(0.0));
  row.entry("behind".into()).or_insert(n(0.0));
  row.insert("dirty".into(), b(dirty));
  if let Ok(text) = git(
    base,
    &[
      "log",
      "-1",
      &format!("--date={}", DATE),
      "--format=%s%x1f%ad",
    ],
    &[0],
  ) {
    let mut parts = text.trim_end().split('\u{1f}');
    row.insert("subject".into(), s(parts.next().unwrap_or("")));
    row.insert("when".into(), s(parts.next().unwrap_or("")));
  }
  Ok(row)
}

/// The newest `limit` commits (of `args`, extra arguments to `git log` such as `--all` or
/// a path), each with `hash`, `short`, `parents` (a list), `author`, `email`, `when`,
/// `refs`, `subject`, `merge`, and `graph`: the lines of history beside it.
pub fn log(base: &Path, args: &[String], limit: usize) -> Result<Vec<Row>, String> {
  let mut words: Vec<String> = vec![
    "log".into(),
    format!("-n{}", limit),
    format!("--date={}", DATE),
    "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%ad%x1f%D%x1f%s%x1e".into(),
  ];
  words.extend(args.iter().cloned());
  let refs: Vec<&str> = words.iter().map(String::as_str).collect();
  // A repository with no commits yet has no log.
  let text = match git(base, &refs, &[0]) {
    Ok(t) => t,
    Err(e) if e.contains("does not have any commits") => return Ok(Vec::new()),
    Err(e) => return Err(e),
  };
  let mut rows = Vec::new();
  let mut lanes: Vec<Option<String>> = Vec::new();
  for record in text.split('\u{1e}') {
    let record = record.trim_start_matches('\n');
    if record.is_empty() {
      continue;
    }
    let f: Vec<&str> = record.split('\u{1f}').collect();
    if f.len() < 7 {
      continue;
    }
    let hash = f[0].to_string();
    let parents: Vec<String> = f[1].split_whitespace().map(str::to_string).collect();
    let graph = lane_line(&mut lanes, &hash, &parents);
    let mut row = Row::new();
    row.insert("hash".into(), s(&hash));
    row.insert("short".into(), s(hash.get(..7).unwrap_or(&hash)));
    row.insert(
      "parents".into(),
      Value::List(parents.iter().map(|p| s(p)).collect()),
    );
    row.insert("merge".into(), b(parents.len() > 1));
    row.insert("author".into(), s(f[2]));
    row.insert("email".into(), s(f[3]));
    row.insert("when".into(), s(f[4]));
    row.insert("refs".into(), s(f[5]));
    row.insert("subject".into(), s(f[6]));
    row.insert("graph".into(), s(&graph));
    rows.push(row);
  }
  Ok(rows)
}

/// Stashes, newest first: `name` (`stash@{0}`), `index`, `hash`, `when`, `subject`.
/// Where HEAD has been, newest first: `hash`, `short`, `name` (`HEAD@{3}`), `subject`
/// (what moved it: `commit: …`, `checkout: moving from a to b`), `action` (the word
/// before its colon), `when`.
pub fn reflog(base: &Path, limit: usize) -> Result<Vec<Row>, String> {
  let count = format!("--max-count={}", limit);
  let text = git(
    base,
    &["reflog", &count, "--format=%H%x1f%h%x1f%gd%x1f%gs%x1f%ct"],
    &[0, 128],
  )?;
  Ok(
    text
      .lines()
      .filter_map(|line| {
        let f: Vec<&str> = line.split('\u{1f}').collect();
        (f.len() >= 5).then(|| {
          let action = f[3].split(':').next().unwrap_or("").trim();
          Row::from([
            ("hash".to_string(), s(f[0])),
            ("short".to_string(), s(f[1])),
            ("name".to_string(), s(f[2])),
            ("subject".to_string(), s(f[3])),
            ("action".to_string(), s(action)),
            ("when".to_string(), s(&utc(f[4].parse().unwrap_or(0)))),
          ])
        })
      })
      .collect(),
  )
}

/// Remotes: `name`, `url` (fetch), `push` (its push URL).
pub fn remotes(base: &Path) -> Result<Vec<Row>, String> {
  let text = git(base, &["remote", "-v"], &[0])?;
  let mut rows: Vec<Row> = Vec::new();
  for line in text.lines() {
    let mut parts = line.split_whitespace();
    let (Some(name), Some(url), Some(kind)) = (parts.next(), parts.next(), parts.next()) else {
      continue;
    };
    let at = match rows.iter().position(|r| r["name"].text() == name) {
      Some(i) => i,
      None => {
        rows.push(Row::from([
          ("name".to_string(), s(name)),
          ("url".to_string(), s("")),
          ("push".to_string(), s("")),
        ]));
        rows.len() - 1
      }
    };
    let field = if kind == "(push)" { "push" } else { "url" };
    rows[at].insert(field.into(), s(url));
  }
  Ok(rows)
}

/// The rows `of` gives, for `args` (a path, a commit, extra arguments to `git log`).
pub fn rows(base: &Path, of: &str, args: &[String], limit: usize) -> Result<Vec<Row>, String> {
  let first = args.first().map(String::as_str).unwrap_or("");
  match of {
    "head" | "" => head(base).map(|r| vec![r]),
    "status" => status(base),
    "log" => log(base, args, limit),
    "branches" => branches(base),
    "tags" => tags(base),
    "stashes" => stashes(base),
    "remotes" => remotes(base),
    "changes" if first.is_empty() => Ok(Vec::new()),
    "changes" => changes(base, first),
    "commit" if first.is_empty() => Ok(Vec::new()),
    "commit" => {
      let text = git(
        base,
        &["show", "--format=", "--no-ext-diff", "--patch", "-M", first],
        &[0],
      )?;
      Ok(diff_rows(&text, "commit"))
    }
    "blame" if first.is_empty() => Ok(Vec::new()),
    "blame" => blame(base, first),
    "tree" => status(base).map(|files| tree(&files)),
    "reflog" => reflog(base, limit),
    "worktrees" => worktrees(base),
    "bisect" => bisect(base),
    "split" if first.is_empty() => Ok(Vec::new()),
    "split" => changes(base, first).map(|rows| split(&rows)),
    "words" if first.is_empty() => Ok(Vec::new()),
    "words" => words(base, first),
    "conflicts" if first.is_empty() => Ok(Vec::new()),
    "conflicts" => conflicts(base, first),
    other => Err(format!("git has no `{}` (one of {})", other, OF.join(", "))),
  }
}

/// The lines of [`changes`] side by side: `left` (as it was), `right` (as it is), `old`
/// and `new` (their numbers), and `kind`: `context`, `changed` (a line removed and one
/// added in its place), `removed`, `added`, or the `section`, `file`, `hunk` and `note`
/// rows as they were. Removed lines pair with the added lines that follow them.
pub fn split(rows: &[Row]) -> Vec<Row> {
  let field = |r: &Row, k: &str| r.get(k).map(Value::text).unwrap_or_default();
  let mut out = Vec::new();
  let mut at = 0;
  while at < rows.len() {
    let kind = field(&rows[at], "kind");
    if kind == "remove" || kind == "add" {
      let removed: Vec<&Row> = rows[at..]
        .iter()
        .take_while(|r| field(r, "kind") == "remove")
        .collect();
      let added: Vec<&Row> = rows[at + removed.len()..]
        .iter()
        .take_while(|r| field(r, "kind") == "add")
        .collect();
      for i in 0..removed.len().max(added.len()) {
        let (left, right) = (removed.get(i), added.get(i));
        let base = left.or(right).copied().cloned().unwrap_or_default();
        let mut row = base;
        let strip = |r: Option<&&Row>| {
          r.map(|r| field(r, "text").chars().skip(1).collect::<String>())
            .unwrap_or_default()
        };
        row.insert("left".into(), s(&strip(left)));
        row.insert("right".into(), s(&strip(right)));
        row.insert(
          "old".into(),
          left.map_or(s(""), |r| r.get("old").cloned().unwrap_or(s(""))),
        );
        row.insert(
          "new".into(),
          right.map_or(s(""), |r| r.get("new").cloned().unwrap_or(s(""))),
        );
        let kind = match (left, right) {
          (Some(_), Some(_)) => "changed",
          (Some(_), None) => "removed",
          _ => "added",
        };
        row.insert("kind".into(), s(kind));
        out.push(row);
      }
      at += removed.len() + added.len();
      continue;
    }
    let mut row = rows[at].clone();
    let text = field(&rows[at], "text");
    if kind == "context" {
      let text: String = text.chars().skip(1).collect();
      row.insert("left".into(), s(&text));
      row.insert("right".into(), s(&text));
    } else {
      row.insert("left".into(), s(&text));
      row.insert("right".into(), s(""));
    }
    out.push(row);
    at += 1;
  }
  out
}

pub fn stashes(base: &Path) -> Result<Vec<Row>, String> {
  let text = git(
    base,
    // Without --date, which would name stashes by their time.
    &["stash", "list", "--format=%gd%x1f%H%x1f%ct%x1f%gs"],
    &[0],
  )?;
  Ok(
    text
      .lines()
      .enumerate()
      .filter_map(|(i, line)| {
        let f: Vec<&str> = line.split('\u{1f}').collect();
        (f.len() >= 4).then(|| {
          Row::from([
            ("name".to_string(), s(f[0])),
            ("index".to_string(), n(i as f64)),
            ("hash".to_string(), s(f[1])),
            ("when".to_string(), s(&utc(f[2].parse().unwrap_or(0)))),
            ("subject".to_string(), s(f[3])),
          ])
        })
      })
      .collect(),
  )
}

/// One row per changed file: `path`, `from` (a rename's old path), `index` and
/// `worktree` (git's two status letters, `.` for none), `staged`, `unstaged`,
/// `untracked`, `conflict`, `what` (in words), and `order` (conflicts, then staged, then
/// unstaged, then untracked).
pub fn status(base: &Path) -> Result<Vec<Row>, String> {
  let text = git(
    base,
    &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
    &[0],
  )?;
  let mut rows = Vec::new();
  let mut fields = text.split('\0').peekable();
  while let Some(entry) = fields.next() {
    if entry.is_empty() {
      continue;
    }
    let (kind, rest) = entry.split_at(1);
    let rest = rest.trim_start();
    let mut row = Row::new();
    let (xy, path, from) = match kind {
      "1" => {
        let parts: Vec<&str> = rest.splitn(8, ' ').collect();
        (
          parts[0].to_string(),
          parts.get(7).copied().unwrap_or(""),
          "",
        )
      }
      "2" => {
        let parts: Vec<&str> = rest.splitn(9, ' ').collect();
        let from = fields.next().unwrap_or("");
        (
          parts[0].to_string(),
          parts.get(8).copied().unwrap_or(""),
          from,
        )
      }
      "u" => {
        let parts: Vec<&str> = rest.splitn(10, ' ').collect();
        (
          parts[0].to_string(),
          parts.get(9).copied().unwrap_or(""),
          "",
        )
      }
      "?" => ("??".to_string(), rest, ""),
      _ => continue,
    };
    let mut letters = xy.chars();
    let (x, y) = (letters.next().unwrap_or('.'), letters.next().unwrap_or('.'));
    let untracked = kind == "?";
    let conflict = kind == "u";
    let staged = !untracked && !conflict && x != '.';
    let unstaged = !untracked && !conflict && y != '.';
    let what = if untracked {
      "untracked".to_string()
    } else if conflict {
      "conflict".to_string()
    } else {
      [change(x), change(y)]
        .iter()
        .filter(|w| !w.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
    };
    let order = if conflict {
      0
    } else if staged && !unstaged {
      1
    } else if staged || unstaged {
      2
    } else {
      3
    };
    row.insert("path".into(), s(path));
    row.insert("from".into(), s(from));
    row.insert(
      "index".into(),
      s(&if untracked { '?' } else { x }.to_string()),
    );
    row.insert(
      "worktree".into(),
      s(&if untracked { '?' } else { y }.to_string()),
    );
    row.insert("staged".into(), b(staged));
    row.insert("unstaged".into(), b(unstaged));
    row.insert("untracked".into(), b(untracked));
    row.insert("conflict".into(), b(conflict));
    row.insert("what".into(), s(&what));
    row.insert("order".into(), n(order as f64));
    rows.push(row);
  }
  rows.sort_by(|a, b| {
    let key = |r: &Row| (r["order"].num().unwrap_or(0.0) as i64, r["path"].text());
    key(a).cmp(&key(b))
  });
  Ok(rows)
}

/// Tags, newest first: `name`, `short`, `when`, `subject`.
pub fn tags(base: &Path) -> Result<Vec<Row>, String> {
  let format = format!(
    "--format=%(refname:short)%1f%(objectname:short)%1f%(creatordate:{})%1f%(subject)",
    DATE
  );
  let text = git(
    base,
    &["for-each-ref", "--sort=-creatordate", &format, "refs/tags"],
    &[0],
  )?;
  Ok(
    text
      .lines()
      .filter_map(|line| {
        let f: Vec<&str> = line.split('\u{1f}').collect();
        (f.len() >= 4).then(|| {
          Row::from([
            ("name".to_string(), s(f[0])),
            ("short".to_string(), s(f[1])),
            ("when".to_string(), s(f[2])),
            ("subject".to_string(), s(f[3])),
          ])
        })
      })
      .collect(),
  )
}

/// The changed files of [`status`] under the folders that hold them, folders first, each
/// row with `depth`, `name` (its last part), `label` (indented: `▾ src/` for a folder)
/// and `folder`. A folder's row has `path` (`src/`, which git takes for everything in it)
/// and whether any file in it is `staged`, `unstaged`, `untracked` or a `conflict`; a
/// file's row has every field [`status`] gives it.
pub fn tree(files: &[Row]) -> Vec<Row> {
  let mut sorted: Vec<&Row> = files.iter().collect();
  sorted.sort_by_key(|r| r.get("path").map(Value::text).unwrap_or_default());
  let mut rows: Vec<Row> = Vec::new();
  let mut open: Vec<String> = Vec::new();
  for file in sorted {
    let path = file.get("path").map(Value::text).unwrap_or_default();
    let parts: Vec<&str> = path.split('/').collect();
    let folders = &parts[..parts.len() - 1];
    // Folders this file is not in are closed; those it is in and are not open, opened.
    let shared = open.iter().zip(folders).take_while(|(a, b)| a == b).count();
    open.truncate(shared);
    for (depth, name) in folders.iter().enumerate().skip(shared) {
      open.push(name.to_string());
      let folder_path = format!("{}/", folders[..=depth].join("/"));
      let mut row = Row::from([
        ("path".to_string(), s(&folder_path)),
        ("name".to_string(), s(name)),
        ("depth".to_string(), n(depth as f64)),
        ("folder".to_string(), Value::Bool(true)),
        (
          "label".to_string(),
          s(&format!("{}▾ {}/", "  ".repeat(depth), name)),
        ),
      ]);
      for flag in ["staged", "unstaged", "untracked", "conflict"] {
        let any = files.iter().any(|f| {
          f.get("path")
            .is_some_and(|p| p.text().starts_with(&folder_path))
            && f.get(flag).is_some_and(Value::truthy)
        });
        row.insert(flag.into(), Value::Bool(any));
      }
      let count = files
        .iter()
        .filter(|f| {
          f.get("path")
            .is_some_and(|p| p.text().starts_with(&folder_path))
        })
        .count();
      row.insert("what".into(), s(&format!("{} file(s)", count)));
      rows.push(row);
    }
    let mut row = file.clone();
    let depth = folders.len();
    let name = parts.last().copied().unwrap_or("");
    row.insert("name".into(), s(name));
    row.insert("depth".into(), n(depth as f64));
    row.insert("folder".into(), Value::Bool(false));
    row.insert(
      "label".into(),
      s(&format!("{}{}", "  ".repeat(depth), name)),
    );
    rows.push(row);
  }
  rows
}

/// A file's changes word by word, as `git diff --word-diff` marks them in each line
/// (`[-old-]{+new+}`), not staged then staged: `text`, `kind` (`changed` for a line with
/// a mark, `context`, `hunk`, `section`), `old`, `new`, `side`.
pub fn words(base: &Path, path: &str) -> Result<Vec<Row>, String> {
  let mut rows = Vec::new();
  for (side, title, cached) in [
    ("unstaged", "Not staged", false),
    ("staged", "Staged", true),
  ] {
    let mut args = vec!["diff", "--no-ext-diff", "--word-diff=plain", "-M"];
    if cached {
      args.push("--cached");
    }
    args.extend(["--", path]);
    let text = git(base, &args, &[0])?;
    let mut lines = Vec::new();
    let (mut old, mut new) = (0u64, 0u64);
    for line in text.lines() {
      if line.starts_with("@@") {
        let (o, n_) = hunk_starts(line);
        (old, new) = (o, n_);
        lines.push(Row::from([
          ("kind".to_string(), s("hunk")),
          ("text".to_string(), s(line)),
          ("side".to_string(), s(side)),
          ("old".to_string(), s("")),
          ("new".to_string(), s("")),
        ]));
        continue;
      }
      if old == 0 && new == 0 {
        // The file's header, before its first hunk.
        continue;
      }
      let marked = line.contains("[-") || line.contains("{+");
      let only_removed = line.starts_with("[-") && line.ends_with("-]") && !line.contains("{+");
      let only_added = line.starts_with("{+") && line.ends_with("+}") && !line.contains("[-");
      lines.push(Row::from([
        (
          "kind".to_string(),
          s(if marked { "changed" } else { "context" }),
        ),
        ("text".to_string(), s(line)),
        ("side".to_string(), s(side)),
        (
          "old".to_string(),
          if only_added { s("") } else { n(old as f64) },
        ),
        (
          "new".to_string(),
          if only_removed { s("") } else { n(new as f64) },
        ),
      ]));
      if !only_added {
        old += 1;
      }
      if !only_removed {
        new += 1;
      }
    }
    if !lines.is_empty() {
      rows.push(Row::from([
        ("kind".to_string(), s("section")),
        ("text".to_string(), s(&format!("── {} ──", title))),
        ("side".to_string(), s(side)),
        ("old".to_string(), s("")),
        ("new".to_string(), s("")),
      ]));
      rows.extend(lines);
    }
  }
  Ok(rows)
}

/// Every working tree of the repository: `path`, `hash`, `short`, `branch` (empty when
/// detached), `detached`, `bare`, `locked`, `prunable`, `main` (the first).
pub fn worktrees(base: &Path) -> Result<Vec<Row>, String> {
  let text = git(base, &["worktree", "list", "--porcelain"], &[0])?;
  let mut rows = Vec::new();
  for block in text.split("\n\n").filter(|b| !b.trim().is_empty()) {
    let mut row = Row::new();
    for flag in ["detached", "bare", "locked", "prunable"] {
      row.insert(flag.into(), Value::Bool(false));
    }
    for line in block.lines() {
      let (key, value) = line.split_once(' ').unwrap_or((line, ""));
      match key {
        "worktree" => {
          row.insert("path".into(), s(value));
        }
        "HEAD" => {
          row.insert("hash".into(), s(value));
          row.insert("short".into(), s(value.get(..7).unwrap_or(value)));
        }
        "branch" => {
          row.insert(
            "branch".into(),
            s(value.strip_prefix("refs/heads/").unwrap_or(value)),
          );
        }
        "detached" | "bare" | "locked" | "prunable" => {
          row.insert(key.into(), Value::Bool(true));
        }
        _ => {}
      }
    }
    row.entry("branch".into()).or_insert_with(|| s(""));
    row.insert("main".into(), Value::Bool(rows.is_empty()));
    rows.push(row);
  }
  Ok(rows)
}

fn b(x: bool) -> Value {
  Value::Bool(x)
}

/// What a status letter means.
fn change(letter: char) -> &'static str {
  match letter {
    'M' => "modified",
    'T' => "type changed",
    'A' => "added",
    'D' => "deleted",
    'R' => "renamed",
    'C' => "copied",
    'U' => "conflict",
    _ => "",
  }
}

/// Runs git with `args` in `base`; its output. `ok` lists the exit codes that are not
/// failures (`git diff --no-index` exits 1 when there are differences).
fn git(base: &Path, args: &[&str], ok: &[i32]) -> Result<String, String> {
  let mut words: Vec<String> = vec![
    "git".into(),
    "-c".into(),
    "core.quotepath=off".into(),
    "-c".into(),
    "color.ui=never".into(),
  ];
  words.extend(args.iter().map(|a| a.to_string()));
  run(base, &words, None, &[("TZ", "UTC")], GIT_LIMIT, ok)
}

/// The first old and new line numbers of a hunk header `@@ -12,3 +12,4 @@`.
fn hunk_starts(header: &str) -> (u64, u64) {
  let mut parts = header.split_whitespace().skip(1);
  let start = |p: Option<&str>| {
    p.and_then(|p| p[1..].split(',').next())
      .and_then(|x| x.parse().ok())
      .unwrap_or(1)
  };
  (start(parts.next()), start(parts.next()))
}

/// The graph beside one commit: a column per line of history still open, `●` for the
/// commit (`◆` for a merge), `│` for the others, `┘` where a line ends in this commit;
/// then the lines are moved on to the commit's parents.
fn lane_line(lanes: &mut Vec<Option<String>>, hash: &str, parents: &[String]) -> String {
  let here = match lanes.iter().position(|l| l.as_deref() == Some(hash)) {
    Some(i) => i,
    None => match lanes.iter().position(Option::is_none) {
      Some(i) => i,
      None => {
        lanes.push(None);
        lanes.len() - 1
      }
    },
  };
  // Other lines waiting for this same commit end here.
  let joining: Vec<usize> = lanes
    .iter()
    .enumerate()
    .filter(|(i, l)| *i != here && l.as_deref() == Some(hash))
    .map(|(i, _)| i)
    .collect();
  let mut out = String::new();
  for (i, lane) in lanes.iter().enumerate() {
    out.push(if i == here {
      if parents.len() > 1 { '◆' } else { '●' }
    } else if joining.contains(&i) {
      '┘'
    } else if lane.is_some() {
      '│'
    } else {
      ' '
    });
    out.push(' ');
  }
  for i in joining {
    lanes[i] = None;
  }
  lanes[here] = parents.first().cloned();
  for parent in parents.iter().skip(1) {
    if !lanes.iter().any(|l| l.as_deref() == Some(parent)) {
      match lanes.iter().position(Option::is_none) {
        Some(i) => lanes[i] = Some(parent.clone()),
        None => lanes.push(Some(parent.clone())),
      }
    }
  }
  while lanes.last().is_some_and(Option::is_none) {
    lanes.pop();
  }
  out.trim_end().to_string()
}

fn n(x: f64) -> Value {
  Value::Num(x)
}

fn s(text: &str) -> Value {
  Value::Str(text.to_string())
}

/// The patch for line `at` of a hunk alone (`lines[0]` is its header): every other added
/// line left out, every other removed line kept as context, the header's counts redone.
fn single_line_patch(header: &[String], lines: &[&str], at: usize) -> String {
  let (old_start, new_start) = hunk_starts(lines[0]);
  let mut body = Vec::new();
  for (i, line) in lines.iter().enumerate().skip(1) {
    if i == at {
      body.push(line.to_string());
    } else if let Some(rest) = line.strip_prefix('-') {
      body.push(format!(" {}", rest));
    } else if line.starts_with('+') {
      continue;
    } else {
      body.push(line.to_string());
    }
  }
  let old_count = body
    .iter()
    .filter(|l| !l.starts_with('+') && !l.starts_with('\\'))
    .count();
  let new_count = body
    .iter()
    .filter(|l| !l.starts_with('-') && !l.starts_with('\\'))
    .count();
  format!(
    "{}\n@@ -{},{} +{},{} @@\n{}\n",
    header.join("\n"),
    old_start,
    old_count,
    new_start,
    new_count,
    body.join("\n")
  )
}

#[cfg(test)]
pub mod tests {
  use super::*;

  /// A repository made fresh for a test, its commits at fixed times by a fixed author.
  pub struct Repo(pub std::path::PathBuf);

  impl Repo {
    pub fn new(name: &str) -> Repo {
      let dir = std::env::temp_dir().join(format!("hub-git-{}-{}", name, std::process::id()));
      let _ = std::fs::remove_dir_all(&dir);
      std::fs::create_dir_all(&dir).unwrap();
      let repo = Repo(dir);
      repo.git(&["init", "-q", "-b", "main"]);
      repo
    }

    pub fn git(&self, args: &[&str]) -> String {
      let out = std::process::Command::new("git")
        .args(args)
        .current_dir(&self.0)
        .env("GIT_AUTHOR_NAME", "Ada")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .env("GIT_AUTHOR_DATE", "2026-01-02T03:04:05Z")
        .env("GIT_COMMITTER_DATE", "2026-01-02T03:04:05Z")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .unwrap();
      assert!(
        out.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
      );
      String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn write(&self, path: &str, text: &str) {
      std::fs::write(self.0.join(path), text).unwrap();
    }
  }

  impl Drop for Repo {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.0);
    }
  }

  #[test]
  fn test_blame_stashes_and_times() {
    let repo = Repo::new("blame");
    repo.write("a", "x\ny\n");
    repo.git(&["add", "."]);
    repo.git(&["commit", "-qm", "two lines"]);
    let rows = blame(&repo.0, "a").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["author"].text(), "Ada");
    assert_eq!(rows[0]["when"].text(), "2026-01-02T03:04:05Z");
    assert!(rows[0]["first"].truthy() && !rows[1]["first"].truthy());
    repo.write("a", "x\nz\n");
    repo.git(&["stash", "push", "-q", "-m", "wip"]);
    let stashes = stashes(&repo.0).unwrap();
    assert_eq!(stashes[0]["name"].text(), "stash@{0}");
    assert!(stashes[0]["subject"].text().contains("wip"));
    assert_eq!(utc(0), "1970-01-01T00:00:00Z");
  }

  /// A hunk's patch stages that hunk alone, and a line's patch that line alone.
  #[test]
  fn test_hunks_and_lines_stage_alone() {
    let repo = Repo::new("hunks");
    let lines: Vec<String> = (1..=30).map(|i| format!("line {}", i)).collect();
    repo.write("f.txt", &(lines.join("\n") + "\n"));
    repo.git(&["add", "."]);
    repo.git(&["commit", "-qm", "base"]);
    let mut changed = lines.clone();
    changed[1] = "line 2 changed".into();
    changed[25] = "line 26 changed".into();
    changed.insert(26, "inserted".into());
    repo.write("f.txt", &(changed.join("\n") + "\n"));
    let rows = changes(&repo.0, "f.txt").unwrap();
    let hunks: Vec<&Row> = rows.iter().filter(|r| r["kind"].text() == "hunk").collect();
    assert_eq!(hunks.len(), 2);
    let added = rows
      .iter()
      .find(|r| r["text"].text() == "+line 26 changed")
      .unwrap();
    assert_eq!(added["new"].num(), Some(26.0));
    // Stage the second hunk's "inserted" line only.
    let inserted = rows
      .iter()
      .find(|r| r["text"].text() == "+inserted")
      .unwrap();
    let patch = inserted["line_patch"].text();
    run(
      &repo.0,
      &["git".into(), "apply".into(), "--cached".into(), "-".into()],
      Some(&patch),
      &[],
      GIT_LIMIT,
      &[0],
    )
    .unwrap();
    let staged = repo.git(&["diff", "--cached"]);
    assert!(staged.contains("+inserted") && !staged.contains("line 26 changed"));
    // Then the first hunk whole.
    let first = rows
      .iter()
      .find(|r| r["kind"].text() == "hunk" && r["hunk"].num() == Some(1.0))
      .unwrap();
    run(
      &repo.0,
      &["git".into(), "apply".into(), "--cached".into(), "-".into()],
      Some(&first["patch"].text()),
      &[],
      GIT_LIMIT,
      &[0],
    )
    .unwrap();
    let rows = changes(&repo.0, "f.txt").unwrap();
    assert!(
      rows
        .iter()
        .any(|r| r["side"].text() == "staged" && r["text"].text() == "+line 2 changed")
    );
    assert!(
      rows
        .iter()
        .any(|r| r["side"].text() == "unstaged" && r["text"].text() == "+line 26 changed")
    );
  }

  #[test]
  fn test_log_draws_branches_and_merges() {
    let repo = Repo::new("log");
    repo.write("a", "1\n");
    repo.git(&["add", "."]);
    repo.git(&["commit", "-qm", "root"]);
    repo.git(&["checkout", "-qb", "side"]);
    repo.write("b", "1\n");
    repo.git(&["add", "."]);
    repo.git(&["commit", "-qm", "on side"]);
    repo.git(&["checkout", "-q", "main"]);
    repo.write("c", "1\n");
    repo.git(&["add", "."]);
    repo.git(&["commit", "-qm", "on main"]);
    repo.git(&["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    let rows = log(&repo.0, &[], 10).unwrap();
    let graph: Vec<String> = rows.iter().map(|r| r["graph"].text()).collect();
    let subjects: Vec<String> = rows.iter().map(|r| r["subject"].text()).collect();
    assert_eq!(subjects[0], "merge side");
    assert_eq!(graph[0], "◆");
    assert!(rows[0]["merge"].truthy());
    assert_eq!(graph.last().unwrap(), "● ┘");
    assert!(graph.iter().any(|g| g == "│ ●" || g == "● │"));
    let branches = branches(&repo.0).unwrap();
    assert_eq!(branches.len(), 2);
    assert!(
      branches
        .iter()
        .any(|r| r["name"].text() == "main" && r["head"].truthy())
    );
  }

  #[test]
  fn test_reflog_worktrees_split_words_bisect_and_conflicts() {
    let repo = Repo::new("tier3");
    repo.write("f.txt", "one\ntwo\nthree\n");
    repo.git(&["add", "."]);
    repo.git(&["commit", "-qm", "base"]);
    // The reflog says what moved HEAD.
    let reflog = reflog(&repo.0, 10).unwrap();
    assert_eq!(text(&reflog[0], "action"), "commit (initial)");
    assert_eq!(text(&reflog[0], "name"), "HEAD@{0}");
    // One working tree, the main one, on main.
    let trees = worktrees(&repo.0).unwrap();
    assert_eq!(trees.len(), 1);
    assert_eq!(text(&trees[0], "branch"), "main");
    assert_eq!(trees[0]["main"], Value::Bool(true));
    // Side by side: the changed line pairs with the one it replaced.
    repo.write("f.txt", "one\nTWO\nthree\nfour\n");
    let rows = split(&changes(&repo.0, "f.txt").unwrap());
    let changed = rows.iter().find(|r| text(r, "kind") == "changed").unwrap();
    assert_eq!(
      (text(changed, "left"), text(changed, "right")),
      ("two".into(), "TWO".into())
    );
    let added = rows.iter().find(|r| text(r, "kind") == "added").unwrap();
    assert_eq!(
      (text(added, "left"), text(added, "right")),
      ("".into(), "four".into())
    );
    // Word by word.
    repo.write("f.txt", "one\ntwo words here\nthree\n");
    repo.git(&["commit", "-qam", "words"]);
    repo.write("f.txt", "one\ntwo verbs here\nthree\n");
    let rows = words(&repo.0, "f.txt").unwrap();
    let changed = rows.iter().find(|r| text(r, "kind") == "changed").unwrap();
    assert_eq!(text(changed, "text"), "two [-words-]{+verbs+} here");
    assert_eq!(text(changed, "old"), "2");
    repo.git(&["checkout", "-q", "--", "f.txt"]);
    // No bisection, no rows; under way, its steps.
    assert!(bisect(&repo.0).unwrap().is_empty());
    let head = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["bisect", "start"]);
    repo.git(&["bisect", "bad", head.trim()]);
    let steps = bisect(&repo.0).unwrap();
    assert_eq!(text(&steps[0], "verb"), "bad");
    assert_eq!(text(&steps[0], "subject"), "words");
    repo.git(&["bisect", "reset"]);
    // A conflict, and the file with it taken each way.
    repo.git(&["switch", "-qc", "other"]);
    repo.write("f.txt", "one\nOTHER\nthree\n");
    repo.git(&["commit", "-qam", "other"]);
    repo.git(&["switch", "-q", "main"]);
    repo.write("f.txt", "one\nMAIN\nthree\n");
    repo.git(&["commit", "-qam", "main"]);
    let merged = std::process::Command::new("git")
      .args([
        "-c",
        "user.name=a",
        "-c",
        "user.email=a@b",
        "merge",
        "other",
      ])
      .current_dir(&repo.0)
      .output()
      .unwrap();
    assert!(!merged.status.success());
    let rows = conflicts(&repo.0, "f.txt").unwrap();
    let ours = rows.iter().find(|r| text(r, "kind") == "ours").unwrap();
    assert_eq!(text(ours, "text"), "MAIN");
    assert_eq!(text(ours, "ours_file"), "one\nMAIN\nthree\n");
    assert_eq!(text(ours, "theirs_file"), "one\nOTHER\nthree\n");
    assert_eq!(text(ours, "both_file"), "one\nMAIN\nOTHER\nthree\n");
    assert!(rows.iter().any(|r| text(r, "kind") == "theirs"));
  }

  #[test]
  fn test_status_names_each_change() {
    let repo = Repo::new("status");
    repo.write("a.txt", "one\n");
    repo.write("b.txt", "two\n");
    repo.git(&["add", "."]);
    repo.git(&["commit", "-qm", "first"]);
    repo.write("a.txt", "one changed\n");
    repo.write("b.txt", "two staged\n");
    repo.git(&["add", "b.txt"]);
    repo.write("new file.txt", "new\n");
    let rows = status(&repo.0).unwrap();
    let paths: Vec<String> = rows.iter().map(|r| r["path"].text()).collect();
    assert_eq!(paths, ["b.txt", "a.txt", "new file.txt"]);
    assert!(rows[0]["staged"].truthy() && !rows[0]["unstaged"].truthy());
    assert_eq!(rows[1]["what"].text(), "modified");
    assert!(rows[2]["untracked"].truthy());
    let h = head(&repo.0).unwrap();
    assert_eq!(h["branch"].text(), "main");
    assert_eq!(h["subject"].text(), "first");
    assert_eq!(h["when"].text(), "2026-01-02T03:04:05Z");
    assert!(h["dirty"].truthy());
  }

  #[test]
  fn test_the_changed_files_as_a_tree() {
    let files: Vec<Row> = ["src/a.rs", "src/b/c.rs", "README.md", "src/b/d.rs"]
      .iter()
      .map(|p| {
        Row::from([
          ("path".to_string(), s(p)),
          ("unstaged".to_string(), Value::Bool(p.ends_with("c.rs"))),
        ])
      })
      .collect();
    let rows = tree(&files);
    let labels: Vec<String> = rows.iter().map(|r| text(r, "label")).collect();
    assert_eq!(
      labels,
      [
        "README.md",
        "▾ src/",
        "  a.rs",
        "  ▾ b/",
        "    c.rs",
        "    d.rs"
      ]
    );
    assert_eq!(text(&rows[3], "path"), "src/b/");
    assert_eq!(rows[3]["unstaged"], Value::Bool(true));
    assert_eq!(text(&rows[3], "what"), "2 file(s)");
  }

  #[test]
  fn test_two_conflicts_in_one_file() {
    let dir = std::env::temp_dir().join(format!("hub-two-conflicts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
      dir.join("g.txt"),
      "a\n<<<<<<< HEAD\nB-ours\n=======\nB-theirs\n>>>>>>> other\nc\nd\ne\nf\n\
       <<<<<<< HEAD\nG-ours\n=======\nG-theirs\n>>>>>>> other\nh\n",
    )
    .unwrap();
    let rows = conflicts(&dir, "g.txt").unwrap();
    let hunks: Vec<(String, String, String)> = rows
      .iter()
      .map(|r| (text(r, "hunk"), text(r, "line"), text(r, "text")))
      .collect();
    assert_eq!(rows.iter().filter(|r| text(r, "kind") == "ours").count(), 2);
    // Each conflict numbered, with its own lines and those around it.
    assert!(hunks.contains(&("2".into(), "12".into(), "G-ours".into())));
    let _ = std::fs::remove_dir_all(&dir);
  }

  fn text(row: &Row, field: &str) -> String {
    row.get(field).map(Value::text).unwrap_or_default()
  }
}
