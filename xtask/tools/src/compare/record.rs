//! The record and the reports: each source's tally, the record it may only climb above
//! (`records/results/compatibility.txt`), the logs the reports are drawn from, and the
//! standard formatter's bugs found on the way.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use xtask_core::{harness, store};

use super::key::*;
use super::measure::*;
use super::source::*;
use super::*;

/// The mismatch groups the report lists.
pub(crate) const TOP_KINDS: usize = 50;

/// A flag on the standard formatter's output, not listed, where mto's own output is right.
pub(crate) struct Found<'a> {
  pub(crate) case: &'a Case,
  pub(crate) key: String,
  pub(crate) message: String,
  /// A default-settings pair mto does not match: excused too.
  pub(crate) excused: bool,
}

/// The numbers for one set: files matched of scored, lines matched of the answers' lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tally {
  pub(crate) files: usize,
  pub(crate) matched_files: usize,
  pub(crate) lines: usize,
  pub(crate) matched_lines: usize,
  pub(crate) skipped: usize,
}

/// Writes each file's matched lines to `log` and lists the files that matched fewer lines
/// than at the run before, which a total alone hides behind gains elsewhere; beside it,
/// `diff-first.txt` names each mismatching file's first differing line and its kind.
pub(crate) fn compare_scores(log: &Path, cases: &[Case], scores: &[Score]) {
  let before: HashMap<String, usize> = fs::read_to_string(log)
    .unwrap_or_default()
    .lines()
    .filter_map(|line| {
      let (key, matched) = line.rsplit_once(' ')?;
      Some((key.to_string(), matched.split('/').next()?.parse().ok()?))
    })
    .collect();
  let mut now = String::new();
  let mut first = String::new();
  let mut lost = Vec::new();
  for (case, score) in cases.iter().zip(scores).filter(|(_, s)| !s.skipped) {
    let key = format!("[{}] {}", case.source.key(), case.name);
    if let Some((line, kind)) = &score.first {
      first.push_str(&format!("{} {}:{}\n", kind, key, line));
    }
    if let Some(&old) = before.get(&key)
      && score.matched_lines < old
    {
      lost.push(format!("{} {} -> {}", key, old, score.matched_lines));
    }
    now.push_str(&format!(
      "{} {}/{}\n",
      key, score.matched_lines, score.lines
    ));
  }
  let _ = fs::write(log, now);
  // Each mismatching file's first differing line and its syntax kind, to sample a kind by.
  let _ = fs::write(log.with_file_name("diff-first.txt"), first);
  if !lost.is_empty() {
    println!(
      "files matching fewer lines than at the last run: {}",
      lost.len()
    );
    for line in lost.iter().take(20) {
      println!("  {}", line);
    }
  }
}

/// Every way `tallies` fall below the record: a lower share of files or lines matching, or
/// more inputs the standard formatter refuses.
pub(crate) fn drops(
  recorded: &BTreeMap<String, (Tally, bool)>,
  tallies: &BTreeMap<Source, Tally>,
) -> Vec<String> {
  let mut drops = Vec::new();
  for (set, now) in tallies {
    let Some((then, has_skipped)) = recorded.get(set.key()) else {
      continue;
    };
    if *has_skipped && now.skipped > then.skipped {
      drops.push(format!(
        "{}: {} inputs the standard formatter refuses, recorded {} (the files cannot be \
         scored; see the .failed markers in the cache)",
        set.title(),
        now.skipped,
        then.skipped
      ));
    }
    // a/b < c/d without division: a*d < c*b.
    let below = |a: usize, b: usize, c: usize, d: usize| a * d.max(1) < c * b.max(1);
    if below(now.matched_files, now.files, then.matched_files, then.files) {
      drops.push(format!(
        "{}: files {} of {}, recorded {} of {}",
        set.title(),
        now.matched_files,
        now.files,
        then.matched_files,
        then.files
      ));
    }
    if below(now.matched_lines, now.lines, then.matched_lines, then.lines) {
      drops.push(format!(
        "{}: lines {} of {}, recorded {} of {}",
        set.title(),
        now.matched_lines,
        now.lines,
        then.matched_lines,
        then.lines
      ));
    }
  }
  drops
}

/// The `PATH [SET]` keys listed in the exceptions file, the meaning check's (`excused`
/// false) or the test pairs' mto need not match (`excused` true, lines starting `excused `).
pub(crate) fn exceptions(path: &Path, excused: bool) -> Vec<String> {
  let Ok(text) = fs::read_to_string(path) else {
    return Vec::new();
  };
  text
    .lines()
    .map(str::trim)
    .filter(|l| !l.is_empty() && !l.starts_with('#'))
    .filter_map(|l| match l.strip_prefix("excused ") {
      Some(rest) if excused => Some(rest),
      None if !excused => Some(l),
      _ => None,
    })
    .filter_map(|l| l.find(']').map(|end| l[..=end].trim().to_string()))
    .collect()
}

/// The reference and each set's numbers; the flag says whether the record has the set's
/// skipped count.
pub(crate) fn parse_record(text: &str) -> (String, BTreeMap<String, (Tally, bool)>) {
  let mut reference = String::new();
  let mut sets = BTreeMap::new();
  for line in text.lines() {
    if let Some(r) = line.strip_prefix("reference ") {
      reference = r.to_string();
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    let n = |s: &str| s.parse::<usize>().unwrap_or(0);
    let (name, mf, f, ml, l, skipped) = match words.as_slice() {
      ["set", name, "files", mf, f, "lines", ml, l, "skipped", k] => {
        (name, mf, f, ml, l, Some(n(k)))
      }
      // Records from before the skipped count was kept.
      ["set", name, "files", mf, f, "lines", ml, l] => (name, mf, f, ml, l, None),
      _ => continue,
    };
    sets.insert(
      name.to_string(),
      (
        Tally {
          matched_files: n(mf),
          files: n(f),
          matched_lines: n(ml),
          lines: n(l),
          skipped: skipped.unwrap_or(0),
        },
        skipped.is_some(),
      ),
    );
  }
  (reference, sets)
}

/// Compares with the record, or writes it with `--update`.
pub(crate) fn ratchet(options: &Options, tallies: &BTreeMap<Source, Tally>) -> ExitCode {
  let current = record_text(&options.reference, tallies);
  if options.update {
    return match fs::write(&options.record, &current) {
      Ok(()) => {
        println!("recorded in {}", options.record.display());
        ExitCode::SUCCESS
      }
      Err(e) => {
        println!("cannot write {}: {}", options.record.display(), e);
        ExitCode::from(1)
      }
    };
  }
  let Ok(text) = fs::read_to_string(&options.record) else {
    println!(
      "no record at {}; run `cargo xtask diff --update` to start one",
      options.record.display()
    );
    return ExitCode::from(1);
  };
  let (reference, recorded) = parse_record(&text);
  let drops = drops(&recorded, tallies);
  if reference != options.reference {
    println!(
      "the record was made with {}, this run with {}; check the numbers, then run \
             `cargo xtask diff --update`",
      reference, options.reference
    );
    return ExitCode::from(1);
  }
  if drops.is_empty() {
    println!("compatibility: ok, at or above the record");
    ExitCode::SUCCESS
  } else {
    println!(
      "compatibility dropped below the record (S3-7):\n  {}",
      drops.join("\n  ")
    );
    ExitCode::from(1)
  }
}

/// Lists each of `found` in the exceptions file at `path` for triage (and excuses the test
/// pairs among them), and keeps its case in the repository's `.dev/upstream/rustfmt/` when
/// that folder is there. Returns each entry with where its case went.
pub(crate) fn record_found(path: &Path, found: &[Found<'_>]) -> Vec<(String, String)> {
  let reason = |message: &str| {
    let message: String = message.chars().filter(|c| *c != '\n').take(140).collect();
    format!(
      "auto-recorded, to triage (a bug of the formatter's, or a rewrite the check lacks): {}",
      message
    )
  };
  let text = fs::read_to_string(path).unwrap_or_default();
  let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
  // Listed among the others, before the excused pairs; excused pairs at the end.
  let first_excused = lines
    .iter()
    .position(|l| l.starts_with("excused "))
    .unwrap_or(lines.len());
  let listed: Vec<String> = found
    .iter()
    .map(|f| format!("{}  {}", f.key, reason(&f.message)))
    .collect();
  let at = lines[..first_excused]
    .iter()
    .rposition(|l| !l.trim().is_empty())
    .map_or(first_excused, |i| i + 1);
  lines.splice(at..at, listed);
  for f in found.iter().filter(|f| f.excused) {
    lines.push(format!(
      "excused {}  auto-recorded, to triage: the expected file fails the meaning check where \
       mto's output passes",
      f.key
    ));
  }
  let _ = fs::write(path, lines.join("\n") + "\n");
  // The repository: the folder holding the records/ the file is in.
  let upstream = path
    .ancestors()
    .find(|dir| dir.file_name().is_some_and(|name| name == "records"))
    .and_then(Path::parent)
    .map(|root| store::dir(root, store::UPSTREAM).join("rustfmt"));
  found
    .iter()
    .map(|f| {
      let place = upstream
        .as_deref()
        .filter(|dir| dir.is_dir())
        .and_then(|dir| upstream_case(dir, f))
        .unwrap_or_else(|| "no case kept".to_string());
      (f.key.clone(), place)
    })
    .collect()
}

/// The record file's text.
pub(crate) fn record_text(reference: &str, tallies: &BTreeMap<Source, Tally>) -> String {
  let mut out = String::from(
    "# How much of the Rust style mto reproduces: files and lines matching the standard\n\
         # formatter exactly, per input set (`cargo xtask diff`), and the inputs it refuses\n\
         # (skipped). Matches may only go up, skipped only down; `cargo xtask diff --update`\n\
         # records new numbers.\n",
  );
  out.push_str(&format!("reference {}\n", reference));
  for (set, t) in tallies {
    out.push_str(&format!(
      "set {} files {} {} lines {} {} skipped {}\n",
      set.key(),
      t.matched_files,
      t.files,
      t.matched_lines,
      t.lines,
      t.skipped
    ));
  }
  out
}

pub(crate) fn report(
  tallies: &BTreeMap<Source, Tally>,
  kinds: &BTreeMap<String, (usize, String)>,
  reference: &str,
  markdown: bool,
) -> String {
  let percent = |a: usize, b: usize| {
    if b == 0 {
      0.0
    } else {
      a as f64 * 100.0 / b as f64
    }
  };
  let mut out = String::new();
  if markdown {
    out.push_str(&format!(
      "# mto compatibility\n\nAgainst {}.\n\n| input set | files matching | lines \
             matching | left out |\n|---|---:|---:|---:|\n",
      reference
    ));
  } else {
    out.push_str(&format!(
      "against {}\n{:<30} {:>24} {:>28} {:>9}\n",
      reference, "input set", "files matching", "lines matching", "left out"
    ));
  }
  for set in Source::ALL {
    let Some(t) = tallies.get(&set) else {
      continue;
    };
    let files = format!(
      "{} of {} ({:.1}%)",
      t.matched_files,
      t.files,
      percent(t.matched_files, t.files)
    );
    let lines = format!(
      "{} of {} ({:.2}%)",
      t.matched_lines,
      t.lines,
      percent(t.matched_lines, t.lines)
    );
    if markdown {
      out.push_str(&format!(
        "| {} | {} | {} | {} |\n",
        set.title(),
        files,
        lines,
        t.skipped
      ));
    } else {
      out.push_str(&format!(
        "{:<30} {:>24} {:>28} {:>9}\n",
        set.title(),
        files,
        lines,
        t.skipped
      ));
    }
  }
  let mut ranked: Vec<(&String, &(usize, String))> = kinds.iter().collect();
  ranked.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
  if !ranked.is_empty() {
    if markdown {
      out.push_str(
        "\n## Mismatches by syntax kind at the first differing line\n\n| kind | files \
                 | for example |\n|---|---:|---|\n",
      );
    } else {
      out.push_str("\nmismatches by syntax kind at the first differing line:\n");
    }
    for (kind, (count, example)) in ranked.into_iter().take(TOP_KINDS) {
      if markdown {
        out.push_str(&format!("| `{}` | {} | `{}` |\n", kind, count, example));
      } else {
        out.push_str(&format!("  {:<28} {:>6}  e.g. {}\n", kind, count, example));
      }
    }
  }
  out
}

/// Keeps `f`'s case in `dir` as the cases there are kept (`input.rs`, `edition`,
/// `rustfmt.toml`, `notes.md`) and adds its row to the README's table; `None` when one for
/// its file is there already.
pub(crate) fn upstream_case(dir: &Path, f: &Found<'_>) -> Option<String> {
  let entries: Vec<String> = fs::read_dir(dir)
    .ok()?
    .filter_map(|e| e.ok())
    .filter(|e| e.path().is_dir())
    .map(|e| e.file_name().to_string_lossy().into_owned())
    .collect();
  let known = entries.iter().any(|name| {
    fs::read_to_string(dir.join(name).join("notes.md")).is_ok_and(|n| n.contains(&f.key))
  });
  if known {
    return None;
  }
  let number = entries
    .iter()
    .filter_map(|name| name.split('-').next()?.parse::<usize>().ok())
    .max()
    .unwrap_or(0)
    + 1;
  let slug: String = f
    .case
    .name
    .trim_end_matches(".rs")
    .chars()
    .map(|c| {
      if c.is_ascii_alphanumeric() {
        c.to_ascii_lowercase()
      } else {
        '-'
      }
    })
    .collect();
  let name = format!("{:02}-auto-{}", number, slug.trim_matches('-'));
  let case_dir = dir.join(&name);
  fs::create_dir_all(&case_dir).ok()?;
  let _ = fs::write(case_dir.join("input.rs"), &f.case.input);
  let _ = fs::write(
    case_dir.join("edition"),
    format!("{}\n", edition_year(f.case.edition)),
  );
  // The settings: a test pair's from its `// rustfmt-` lines, another's from its folder.
  let settings = if f.case.source == Source::Tests || f.case.source == Source::TestsWithSettings {
    // As the test runner reads them; the runner's own keys and a `config` file aside.
    let surface =
      Path::new(env!("CARGO_MANIFEST_DIR")).join("../../records/upstream/rustfmt-surface.txt");
    let strings = harness::string_keys(&fs::read_to_string(surface).unwrap_or_default());
    let pairs: Vec<(String, String)> = harness::headers(&f.case.input)
      .into_iter()
      .filter(|(k, _)| k != "config" && !harness::DIRECTIVES.contains(&k.as_str()))
      .collect();
    harness::config_text(&pairs, &strings)
  } else {
    nearest_config(&f.case.dir)
  };
  let _ = fs::write(case_dir.join("rustfmt.toml"), settings);
  let _ = fs::write(
    case_dir.join("notes.md"),
    format!(
      "# {} (auto-recorded)\n\n\
       - Found by: `cargo xtask diff`. The meaning check flags the standard formatter's\n  \
       output where mto's own output passes it.\n\
       - The check said: {}\n\
       - To triage: confirm a bug of the formatter's (then name it, search upstream and\n  \
       make `input.rs` the smallest that shows it), or a rewrite the meaning check lacks\n  \
       (then add it to src/verify/rewrites.rs and drop this case and its exception).\n\
       - Upstream: not searched yet.\n",
      f.key,
      f.message.replace('\n', " ")
    ),
  );
  let readme = dir.join("README.md");
  if let Ok(text) = fs::read_to_string(&readme) {
    let row = format!(
      "| {} | auto-recorded from `{}`, to triage | not searched |",
      name.replacen('-', " ", 1),
      f.key
    );
    let mut lines: Vec<&str> = text.lines().collect();
    let at = lines
      .iter()
      .rposition(|l| l.starts_with("| ") && !l.starts_with("|---"))
      .map_or(lines.len(), |i| i + 1);
    lines.insert(at, &row);
    let _ = fs::write(&readme, lines.join("\n") + "\n");
  }
  Some(format!(".dev/upstream/rustfmt/{}", name))
}

/// What `cargo xtask diff` files as incidents, beside the logs: `diff-findings.tsv`, one
/// line per finding (kind, set, file, message), and each finding's input under
/// `diff-inputs/` (cleared on every run). The first line says whether every input was
/// scored: only a complete run can say a finding is gone.
pub(crate) fn write_findings(dir: &Path, cases: &[Case], scores: &[Score]) {
  let inputs = dir.join("diff-inputs");
  let _ = fs::remove_dir_all(&inputs);
  let _ = fs::create_dir_all(&inputs);
  let complete = scores.iter().all(|s| s.unanswered.is_none());
  let mut out = format!("complete\t{}\n", complete);
  let clean = |text: &str| text.replace(['\t', '\n', '\r'], " ");
  for (case, score) in cases.iter().zip(scores) {
    let found = if let Some(message) = &score.panicked {
      Some(("panic", message.clone()))
    } else if let Some(message) = &score.own {
      let kind = if message == "a second run changes it" {
        "unstable"
      } else {
        "meaning"
      };
      Some((kind, message.clone()))
    } else if score.broke {
      let (line, kind) = score.first.clone().unwrap_or_default();
      Some((
        "compat-break",
        format!("mto changes line {} ({})", line, kind),
      ))
    } else {
      None
    };
    let Some((kind, message)) = found else {
      continue;
    };
    let file = format!(
      "{}-{}.input",
      case.source.key(),
      case.name.replace('/', "__")
    );
    let _ = fs::write(inputs.join(&file), &case.input);
    out.push_str(&format!(
      "{}\t{}\t{}\t{}\t{}\n",
      kind,
      case.source.key(),
      clean(&case.name),
      file,
      clean(&message)
    ));
  }
  let _ = fs::write(dir.join("diff-findings.tsv"), out);
}
