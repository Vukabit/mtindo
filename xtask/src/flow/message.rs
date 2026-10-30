//! A commit message against `flow.toml`'s rules: `kind(scope): subject`, a body only
//! where the change earns one.

use super::config::Flow;

/// What a commit changes, for the rules that depend on it.
pub struct Change<'a> {
  /// The files it touches, added, changed or deleted.
  pub files: &'a [String],
  /// Lines added and removed.
  pub changed_lines: usize,
  /// Whether a path names a file or folder in the tree (or one the commit deletes).
  pub known: &'a dyn Fn(&str) -> bool,
}

/// A first line read apart.
#[derive(Debug, PartialEq)]
pub struct Header<'a> {
  pub kind: &'a str,
  pub scopes: Vec<&'a str>,
  pub breaking: bool,
  pub subject: &'a str,
}

/// The message as git keeps it: comment lines and everything below the scissors line
/// dropped, trailing blank lines too.
pub fn cleaned(raw: &str) -> String {
  let mut lines = Vec::new();
  for line in raw.lines() {
    if line.starts_with("# ") && line.contains(">8") {
      break;
    }
    if line.starts_with('#') {
      continue;
    }
    lines.push(line.trim_end());
  }
  while lines.last().is_some_and(|l| l.is_empty()) {
    lines.pop();
  }
  while lines.first().is_some_and(|l| l.is_empty()) {
    lines.remove(0);
  }
  lines.join("\n")
}

/// `kind(scope, ...)!: subject`.
pub fn header(line: &str) -> Result<Header<'_>, String> {
  let shape = "the first line is `kind(scope): subject`";
  let (head, subject) = line.split_once(": ").ok_or(shape)?;
  let (head, breaking) = match head.strip_suffix('!') {
    Some(head) => (head, true),
    None => (head, false),
  };
  let (kind, scopes) = head.split_once('(').ok_or(shape)?;
  let scopes = scopes.strip_suffix(')').ok_or(shape)?;
  if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_lowercase()) {
    return Err(shape.to_string());
  }
  let scopes: Vec<&str> = scopes.split(',').map(str::trim).collect();
  if scopes.iter().any(|s| s.is_empty()) {
    return Err("a scope is empty".to_string());
  }
  Ok(Header {
    kind,
    scopes,
    breaking,
    subject,
  })
}

/// Whether git made the message for a commit to be squashed later.
pub fn is_autosquash(message: &str) -> bool {
  ["fixup! ", "squash! ", "amend! "]
    .iter()
    .any(|p| message.starts_with(p))
}

/// Everything wrong with `message` (already [`cleaned`]) for `change`.
pub fn problems(message: &str, flow: &Flow, change: &Change) -> Vec<String> {
  let mut found = Vec::new();
  let mut lines = message.lines();
  let first = lines.next().unwrap_or("");
  let rules = &flow.commit;
  let width = first.chars().count();
  if width > rules.max_header {
    found.push(format!(
      "the first line is {} columns (limit {}): say the rest in the body, or not at all",
      width, rules.max_header
    ));
  }
  let header = match header(first) {
    Ok(header) => header,
    Err(e) => {
      found.push(e);
      return found;
    }
  };
  if flow.kind(header.kind).is_none() {
    let kinds: Vec<&str> = flow.kinds.iter().map(|(k, _)| k.as_str()).collect();
    found.push(format!(
      "`{}` is not a kind (flow.toml: {})",
      header.kind,
      kinds.join(", ")
    ));
  }
  let subject = header.subject.trim();
  if subject.is_empty() {
    found.push("the subject is empty".to_string());
  } else if subject.ends_with('.') {
    found.push("the subject ends with a period".to_string());
  }
  // Each scope a section or a path; together they cover a file the commit changes.
  let mut covered = false;
  for scope in &header.scopes {
    let covers: Box<dyn Fn(&str) -> bool> = if let Some(globs) = flow.section(scope) {
      let globs = globs.to_vec();
      Box::new(move |f: &str| globs.iter().any(|g| super::glob::matches(g, f)))
    } else if (change.known)(scope) {
      let prefix = format!("{}/", scope.trim_end_matches('/'));
      let scope = scope.to_string();
      Box::new(move |f: &str| f == scope || f.starts_with(&prefix))
    } else {
      found.push(format!(
        "`{}` is neither a section (flow.toml) nor a path in the tree",
        scope
      ));
      continue;
    };
    covered |= change.files.iter().any(|f| covers(f));
  }
  if rules.scope_must_match && !covered && !change.files.is_empty() {
    let sections = suggested_scope(flow, change.files);
    found.push(format!(
      "the scope covers none of the commit's files{}",
      if sections.is_empty() {
        String::new()
      } else {
        format!(" (they are in: {})", sections)
      }
    ));
  }
  // The body: what follows the blank line, its trailers (`Name: value`) apart.
  let rest: Vec<&str> = lines.collect();
  if rest.first().is_some_and(|l| !l.is_empty()) {
    found.push("a blank line goes between the first line and the body".to_string());
  }
  let (body, trailers) = split_trailers(&rest, &rules.body.trailers);
  let breaking = header.breaking
    || trailers
      .iter()
      .any(|t| t.starts_with("BREAKING CHANGE: ") || t.starts_with("BREAKING-CHANGE: "));
  let rules = &rules.body;
  if body.is_empty() {
    if breaking && rules.required_for_breaking {
      found.push("a breaking change says in its body what breaks".to_string());
    }
    return found;
  }
  if !breaking {
    if rules.refused_kinds.iter().any(|k| k == header.kind) {
      found.push(format!(
        "a `{}` commit has no body: its first line says it all",
        header.kind
      ));
    } else if change.changed_lines < rules.min_changed_lines {
      found.push(format!(
        "a body is for changes of {} lines or more; this one changes {}",
        rules.min_changed_lines, change.changed_lines
      ));
    }
  }
  if body.len() > rules.max_lines {
    found.push(format!(
      "the body is {} lines (limit {})",
      body.len(),
      rules.max_lines
    ));
  }
  for (i, line) in body.iter().enumerate() {
    let width = line.chars().count();
    if width > rules.max_width {
      found.push(format!(
        "body line {} is {} columns (limit {})",
        i + 1,
        width,
        rules.max_width
      ));
    }
  }
  found
}

/// The sections a commit's files are in, as a scope would name them.
pub fn suggested_scope(flow: &Flow, files: &[String]) -> String {
  let mut sections: Vec<&str> = Vec::new();
  for file in files {
    for section in flow.sections_of(file) {
      if !sections.contains(&section) {
        sections.push(section);
      }
    }
  }
  sections.join(",")
}

/// The body's lines, its blank edges dropped, and the trailers of its last paragraph:
/// `Name: value` lines whose names `names` lists (any case).
fn split_trailers<'a>(rest: &[&'a str], names: &[String]) -> (Vec<&'a str>, Vec<&'a str>) {
  let mut lines: Vec<&str> = rest.to_vec();
  while lines.first().is_some_and(|l| l.is_empty()) {
    lines.remove(0);
  }
  let start = lines
    .iter()
    .rposition(|l| l.is_empty())
    .map_or(0, |i| i + 1);
  let trailer = |l: &&str| {
    l.split_once(": ").is_some_and(|(name, value)| {
      !value.is_empty() && names.iter().any(|n| n.eq_ignore_ascii_case(name))
    })
  };
  if start < lines.len() && lines[start..].iter().all(trailer) {
    let trailers = lines.split_off(start);
    while lines.last().is_some_and(|l| l.is_empty()) {
      lines.pop();
    }
    (lines, trailers)
  } else {
    (lines, Vec::new())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn check(message: &str, files: &[&str], changed_lines: usize) -> Vec<String> {
    let files: Vec<String> = files.iter().map(|f| f.to_string()).collect();
    let known = |p: &str| p == "src/format/expr.rs" || p == "src/format";
    problems(
      &cleaned(message),
      &flow(),
      &Change {
        files: &files,
        changed_lines,
        known: &known,
      },
    )
  }

  fn flow() -> Flow {
    super::super::config::parse(include_str!("../../../flow.toml")).expect("flow.toml")
  }

  #[test]
  fn test_a_body_only_where_the_change_earns_one() {
    let with_body = "fix(format): x\n\nWhy it was so.\n";
    assert!(check(with_body, &["src/format/a.rs"], 12)[0].contains("40 lines or more"));
    assert!(check(with_body, &["src/format/a.rs"], 80).is_empty());
    assert!(check("docs(docs): x\n\nMore.", &["README.md"], 80)[0].contains("has no body"));
    // Trailers are not a body; a line that only looks like one is.
    assert!(
      check(
        "fix(format): x\n\nCo-authored-by: A <a@b>",
        &["src/format/a.rs"],
        3
      )
      .is_empty()
    );
    assert!(
      check("fix(format): x\n\nWhy: bytes.", &["src/format/a.rs"], 3)[0].contains("40 lines")
    );
    // A breaking change says what breaks, whatever its size.
    assert!(check("feat(cli)!: x", &["src/main.rs"], 3)[0].contains("breaking"));
    assert!(check("feat(cli)!: x\n\nThe flag is gone.", &["src/main.rs"], 3).is_empty());
    let wide = format!("fix(format): x\n\n{}", "b ".repeat(40));
    assert!(check(&wide, &["src/format/a.rs"], 80)[0].contains("body line 1"));
    assert!(check("fix(format): x\nno blank", &["src/format/a.rs"], 80)[0].contains("blank line"));
  }

  #[test]
  fn test_a_plain_first_line_passes() {
    assert_eq!(
      check(
        "fix(format): keep a generic default's header as written",
        &["src/format/types.rs"],
        12
      ),
      Vec::<String>::new()
    );
    // Several scopes, a path for one.
    assert_eq!(
      check(
        "fix(format,cli): x",
        &["src/main.rs", "src/format/expr.rs"],
        3
      ),
      Vec::<String>::new()
    );
    assert_eq!(
      check("fix(src/format/expr.rs): x", &["src/format/expr.rs"], 3),
      Vec::<String>::new()
    );
  }

  #[test]
  fn test_comments_and_the_scissors_are_dropped() {
    assert_eq!(
      cleaned(
        "fix(format): x\n# Please enter\n\n# ------------------------ >8 ------------------------\ndiff\n"
      ),
      "fix(format): x"
    );
  }

  #[test]
  fn test_the_first_line_is_checked() {
    let long = format!("fix(format): {}", "a".repeat(70));
    assert!(check(&long, &["src/format/a.rs"], 1)[0].contains("columns (limit 72)"));
    assert!(check("fixed the thing", &["src/format/a.rs"], 1)[0].contains("kind(scope)"));
    assert!(check("feature(format): x", &["src/format/a.rs"], 1)[0].contains("not a kind"));
    assert!(check("fix(nowhere): x", &["src/format/a.rs"], 1)[0].contains("neither a section"));
    assert!(check("fix(format): x.", &["src/format/a.rs"], 1)[0].contains("period"));
    let wrong = check("fix(cli): x", &["src/format/types.rs"], 1);
    assert_eq!(
      wrong,
      vec!["the scope covers none of the commit's files (they are in: format)".to_string()]
    );
  }
}
