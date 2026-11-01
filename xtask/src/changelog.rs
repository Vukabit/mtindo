//! `cargo xtask changelog`: CHANGELOG.md's newest section rendered by git-cliff from the
//! commits after the last release tag, each listed under its kind's heading
//! (`flow.toml`'s `[kinds]`), wrapped to the 80 columns
//! public Markdown keeps to, in place of the `## [Unreleased]` section. With `--tag` the
//! section is named for that release, dated today. The first release's section is
//! written by hand: with no tag yet there is nothing to render from.
//!
//! `--preview` prints the section a release made now would write, named for the next
//! version (from the kinds of the commits since the last tag, or `--tag`), and writes
//! nothing.
//!
//! git-cliff is a development tool, never part of the mto binary: on `PATH`, or named by
//! `GIT_CLIFF`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use crate::cli;
use crate::common::log::Logger;
use crate::common::process;
use crate::papers::wrap;

/// git-cliff's settings: Keep a Changelog headings, one per changelog heading
/// `[kinds]` names in the order first named, each kind's commits under its own; kinds
/// without one, and anything else, left out.
/// The scopes of the development tooling, which mto's users never see: a commit naming
/// only these (with `docs`, which tooling commits name for the generated command
/// reference) is not in the changelog. `docs` alone is the user documentation, and stays.
const TOOLING: &[&str] = &["xtask", "records", "flow", "ci", "tests", "fuzz"];

const UNRELEASED: &str = "## [Unreleased]";

/// The widest public Markdown line (the gates' `DOC_LINE_LIMIT`).
const WIDTH: usize = 80;

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "changelog");
  log.info(&format!("cargo xtask {}", cli::line()));
  let check = cli::on(m, "check");
  let tag = cli::text(m, "tag");
  if cli::on(m, "preview") {
    return preview(root, tag);
  }
  match render(root, tag.as_deref()) {
    Ok(section) => {
      let path = root.join("CHANGELOG.md");
      let written = fs::read_to_string(&path).unwrap_or_default();
      let fresh = match replace_unreleased(&written, &section) {
        Ok(fresh) => fresh,
        Err(e) => {
          println!("{}", e);
          return ExitCode::from(1);
        }
      };
      if fresh == written {
        println!("CHANGELOG.md is up to date");
        ExitCode::SUCCESS
      } else if check {
        println!("CHANGELOG.md is out of date: run `cargo xtask changelog`");
        ExitCode::from(1)
      } else if let Err(e) = fs::write(&path, fresh) {
        println!("could not write CHANGELOG.md: {}", e);
        ExitCode::from(1)
      } else {
        log.info("wrote CHANGELOG.md");
        println!("wrote CHANGELOG.md");
        ExitCode::SUCCESS
      }
    }
    Err(e) => {
      log.error(&e);
      println!("{}", e);
      ExitCode::from(1)
    }
  }
}

/// `text` with each line past [`WIDTH`] wrapped: a list item's later lines two columns in,
/// under its words; other lines from their own indentation. Headings are left whole.
pub fn wrapped(text: &str) -> String {
  let mut out = String::new();
  for line in text.lines() {
    if line.chars().count() <= WIDTH || line.starts_with('#') {
      out.push_str(line);
      out.push('\n');
      continue;
    }
    let indent = &line[..line.len() - line.trim_start().len()];
    let rest = line.trim_start();
    match rest.strip_prefix("- ") {
      Some(item) => {
        let first = format!("{}- ", indent);
        let later = format!("{}  ", indent);
        out.push_str(&wrap(item, WIDTH, &first, &later));
      }
      None => out.push_str(&wrap(rest, WIDTH, indent, indent)),
    }
  }
  out
}

/// CHANGELOG.md with its newest section rendered (named for `tag` when given), as
/// `flow release` writes it.
pub fn write(root: &Path, tag: Option<&str>) -> Result<(), String> {
  let section = render(root, tag)?;
  let path = root.join("CHANGELOG.md");
  let written = fs::read_to_string(&path).unwrap_or_default();
  let fresh = replace_unreleased(&written, &section)?;
  fs::write(&path, fresh).map_err(|e| format!("CHANGELOG.md: {}", e))
}

fn cliff_binary() -> Option<PathBuf> {
  if let Some(named) = std::env::var_os("GIT_CLIFF") {
    return Some(PathBuf::from(named));
  }
  process::on_path("git-cliff").then(|| PathBuf::from("git-cliff"))
}

fn cliff_settings(kinds: &[(String, Option<String>)]) -> String {
  let mut headings: Vec<&str> = Vec::new();
  // First, as git-cliff takes the first parser that matches.
  let mut parsers = format!(
    "  {{ message = \"{}\", skip = true }},\n",
    tooling_pattern()
  );
  for (kind, heading) in kinds {
    let Some(heading) = heading else {
      continue;
    };
    if !headings.contains(&heading.as_str()) {
      headings.push(heading);
    }
    let order = headings.iter().position(|h| h == heading).unwrap_or(0);
    // The comment orders the groups and is stripped when rendered.
    parsers.push_str(&format!(
      "  {{ message = \"^{}[(!:]\", group = \"<!-- {} -->{}\" }},\n",
      kind, order, heading
    ));
  }
  parsers.push_str("  { message = \".*\", skip = true },\n");
  format!(
    r#"# Written by `cargo xtask changelog` from flow.toml.
[changelog]
header = ""
body = """
{{% if version -%}}
## [{{{{ version | trim_start_matches(pat="v") }}}}] - {{{{ timestamp | date(format="%Y-%m-%d") }}}}
{{% else -%}}
## [Unreleased]
{{% endif -%}}
{{% for group, commits in commits | group_by(attribute="group") %}}
### {{{{ group | striptags | trim }}}}
{{% for commit in commits %}}
- {{% if commit.breaking %}}Breaking: {{% endif %}}{{{{ commit.message | split(pat="\n") | first | trim | upper_first }}}}
{{%- endfor %}}
{{% endfor %}}
"""
footer = ""
trim = true

[git]
conventional_commits = true
filter_unconventional = true
split_commits = false
protect_breaking_commits = true
filter_commits = true
tag_pattern = "v[0-9].*"
sort_commits = "oldest"
commit_parsers = [
{}]
"#,
    parsers
  )
}

/// Each commit's message since `last`, oldest first, subject then body.
fn commit_messages(root: &Path, last: &str) -> Vec<String> {
  let range = format!("{}..HEAD", last);
  let done = process::output(
    Command::new("git")
      .args(["log", "--reverse", "--format=%B%x1e", &range])
      .current_dir(root)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::QUICK,
  );
  match done {
    Ok(done) if done.code == 0 => String::from_utf8_lossy(&done.stdout)
      .split('\u{1e}')
      .map(|m| m.trim().to_string())
      .filter(|m| !m.is_empty())
      .collect(),
    _ => Vec::new(),
  }
}

/// A release's heading as git-cliff writes it, for one with nothing in it.
fn heading(tag: &str) -> String {
  format!(
    "## [{}] - {}",
    tag.trim_start_matches('v'),
    crate::common::log::timestamp(std::time::SystemTime::now())
      .get(..10)
      .unwrap_or_default()
  )
}

/// The newest `v` tag reachable from HEAD.
fn last_tag(root: &Path) -> Option<String> {
  let done = process::output(
    Command::new("git")
      .args(["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*"])
      .current_dir(root)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::QUICK,
  )
  .ok()?;
  let tag = String::from_utf8_lossy(&done.stdout).trim().to_string();
  (done.code == 0 && !tag.is_empty()).then_some(tag)
}

/// The version after `last` that `messages` call for, and why: a breaking change (`!`
/// after the kind, or a `BREAKING CHANGE` line) or a `feat` raises the minor version
/// below 1.0 and the major from it on (a `feat` the minor); anything else the patch.
fn next_version(last: &str, messages: &[String]) -> (String, String) {
  let numbers: Vec<u64> = last
    .trim_start_matches('v')
    .split('.')
    .map(|n| n.parse().unwrap_or(0))
    .collect();
  let (major, minor, patch) = (
    numbers.first().copied().unwrap_or(0),
    numbers.get(1).copied().unwrap_or(0),
    numbers.get(2).copied().unwrap_or(0),
  );
  let kind = |message: &String| -> String {
    let subject = message.lines().next().unwrap_or("");
    subject
      .split(':')
      .next()
      .unwrap_or("")
      .split('(')
      .next()
      .unwrap_or("")
      .to_string()
  };
  let breaking = messages.iter().any(|m| {
    let head = m
      .lines()
      .next()
      .unwrap_or("")
      .split(':')
      .next()
      .unwrap_or("");
    head.ends_with('!') || m.lines().any(|l| l.starts_with("BREAKING CHANGE"))
  });
  let features = messages.iter().filter(|m| kind(m) == "feat").count();
  let fixes = messages.iter().filter(|m| kind(m) == "fix").count();
  if breaking {
    let next = if major == 0 {
      format!("v0.{}.0", minor + 1)
    } else {
      format!("v{}.0.0", major + 1)
    };
    (next, "a breaking change".to_string())
  } else if features > 0 {
    (
      format!("v{}.{}.0", major, minor + 1),
      format!("{} feat, {} fix", features, fixes),
    )
  } else {
    (
      format!("v{}.{}.{}", major, minor, patch + 1),
      format!("{} fix, no feat", fixes),
    )
  }
}

/// The newest section as git-cliff renders it, wrapped.
/// `--preview`: the section a release made now would write, without writing it.
fn preview(root: &Path, tag: Option<String>) -> ExitCode {
  let Some(last) = last_tag(root) else {
    println!("no release tag yet: the first release's section is written by hand");
    return ExitCode::from(1);
  };
  // What the changelog shows: the commits mto's users see.
  let all = commit_messages(root, &last);
  let subjects: Vec<String> = all.iter().filter(|m| !tooling_only(m)).cloned().collect();
  let (tag, why) = match tag {
    Some(tag) => (tag, "as --tag names it".to_string()),
    None => {
      let (next, why) = next_version(&last, &subjects);
      (next, why)
    }
  };
  match render(root, Some(&tag)) {
    Ok(section) => {
      println!(
        "CHANGELOG.md's section if {} were released now ({}; {} of the {} commits since {} \
         change what users see):\n",
        tag,
        why,
        subjects.len(),
        all.len(),
        last
      );
      print!("{}", section);
      ExitCode::SUCCESS
    }
    Err(e) => {
      println!("{}", e);
      ExitCode::from(1)
    }
  }
}

fn render(root: &Path, tag: Option<&str>) -> Result<String, String> {
  if last_tag(root).is_none() {
    return Err(
      "no release tag yet (v0.1.0 and on): the first release's section is written by hand"
        .to_string(),
    );
  }
  // The pinned git-cliff, unless one is named.
  if std::env::var_os("GIT_CLIFF").is_none() {
    crate::flow::require(root, "changelog")?;
  }
  let cliff = cliff_binary()
    .ok_or("git-cliff is not installed: run `cargo xtask bootstrap`, or name it in GIT_CLIFF")?;
  let settings = root.join("target").join("xtask-cliff.toml");
  fs::create_dir_all(root.join("target")).map_err(|e| format!("target: {}", e))?;
  fs::write(&settings, cliff_settings(&crate::flow::kinds(root)?))
    .map_err(|e| format!("{}: {}", settings.display(), e))?;
  let mut command = Command::new(cliff);
  command
    .arg("--config")
    .arg(&settings)
    .args(["--unreleased", "--strip", "all"])
    .current_dir(root)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
  if let Some(tag) = tag {
    command.args(["--tag", tag]);
  }
  let done = process::output(&mut command, process::QUICK)
    .map_err(|e| format!("git-cliff could not be run: {}", e))?;
  if done.code != 0 {
    return Err(format!(
      "git-cliff failed:\n{}",
      String::from_utf8_lossy(&done.stderr)
    ));
  }
  let text = String::from_utf8_lossy(&done.stdout).into_owned();
  // Nothing after the tag: an empty section, its heading alone.
  let text = if text.trim().is_empty() {
    format!("{}\n", tag.map_or(UNRELEASED.to_string(), heading))
  } else {
    text
  };
  Ok(wrapped(&text))
}

/// `written` with its `## [Unreleased]` section, up to the next release's heading,
/// replaced by `section`; a file without one takes it after its title and introduction.
fn replace_unreleased(written: &str, section: &str) -> Result<String, String> {
  let section = format!("{}\n", section.trim_end());
  // The section's own release, named before (`--tag` run again), else Unreleased.
  let own = section
    .lines()
    .next()
    .and_then(|h| h.split_once("] "))
    .map(|(name, _)| format!("{}]", name));
  let heading = own
    .as_deref()
    .and_then(|h| written.find(&format!("\n{}", h)).map(|i| (i + 1, h.len())))
    .or_else(|| written.find(UNRELEASED).map(|i| (i, UNRELEASED.len())));
  if let Some((start, length)) = heading {
    let after = start + length;
    let end = written[after..]
      .find("\n## [")
      .map_or(written.len(), |i| after + i + 1);
    let mut out = written[..start].to_string();
    out.push_str(&section);
    if end < written.len() {
      out.push('\n');
      out.push_str(&written[end..]);
    }
    return Ok(out);
  }
  let Some(first) = written.find("\n## [") else {
    return Err("CHANGELOG.md has no `## [Unreleased]` or release section".to_string());
  };
  let mut out = written[..=first].to_string();
  out.push_str(&section);
  out.push('\n');
  out.push_str(&written[first + 1..]);
  Ok(out)
}

/// Whether a commit message's subject names only tooling scopes ([`tooling_pattern`]).
fn tooling_only(message: &str) -> bool {
  let subject = message.lines().next().unwrap_or("");
  let Some(scopes) = subject
    .split_once('(')
    .and_then(|(_, rest)| rest.split_once(')'))
    .map(|(scopes, _)| scopes)
  else {
    return false;
  };
  let scopes: Vec<&str> = scopes.split(',').map(str::trim).collect();
  scopes.iter().all(|s| TOOLING.contains(s) || *s == "docs")
    && scopes.iter().any(|s| TOOLING.contains(s))
}

/// The git-cliff pattern of a commit whose every scope is tooling, at least one of them
/// not `docs`: `feat(xtask,docs): ...`, never `feat(cli,xtask): ...` or `fix(docs): ...`.
fn tooling_pattern() -> String {
  let tooling = TOOLING.join("|");
  let any = format!("{}|docs", tooling);
  format!(
    "^[a-z]+\\\\((({}),)*({})(,({}))*\\\\)!?:",
    any, tooling, any
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_long_items_wrap_under_their_words() {
    let long = format!("- {}\n", "word ".repeat(30).trim_end());
    let out = wrapped(&long);
    assert!(out.lines().all(|l| l.chars().count() <= WIDTH), "{}", out);
    assert!(out.starts_with("- word"));
    assert!(
      out.lines().skip(1).all(|l| l.starts_with("  word")),
      "{}",
      out
    );
    assert_eq!(wrapped("### Fixed\n"), "### Fixed\n");
  }

  #[test]
  fn test_the_next_version_follows_the_commits() {
    let m = |list: &[&str]| -> Vec<String> { list.iter().map(|s| s.to_string()).collect() };
    assert_eq!(
      next_version("v0.1.0", &m(&["fix(cli): a", "docs: b"])).0,
      "v0.1.1"
    );
    assert_eq!(
      next_version("v0.1.3", &m(&["fix: a", "feat(cli): b"])).0,
      "v0.2.0"
    );
    assert_eq!(next_version("v0.1.0", &m(&["feat(cli)!: a"])).0, "v0.2.0");
    assert_eq!(
      next_version("v1.2.3", &m(&["fix: a\n\nBREAKING CHANGE: b"])).0,
      "v2.0.0"
    );
    assert_eq!(next_version("v1.2.3", &m(&["feat: a"])).0, "v1.3.0");
  }

  #[test]
  fn test_the_unreleased_section_is_replaced_up_to_the_next_release() {
    let written =
      "# Changelog\n\nIntro.\n\n## [Unreleased]\n\n- old\n\n## [0.1.0] - 2026-11-01\n\n- first\n";
    let out = replace_unreleased(written, "## [Unreleased]\n\n### Fixed\n\n- new\n").unwrap();
    assert_eq!(
      out,
      "# Changelog\n\nIntro.\n\n## [Unreleased]\n\n### Fixed\n\n- new\n\n## [0.1.0] - 2026-11-01\n\n- first\n"
    );
    // A release named by `--tag` takes the section's place, above the last release.
    let out = replace_unreleased(written, "## [0.2.0] - 2027-01-01\n\n- new\n").unwrap();
    assert!(
      out.contains("## [0.2.0] - 2027-01-01\n\n- new\n\n## [0.1.0]"),
      "{}",
      out
    );
    assert!(!out.contains("Unreleased"), "{}", out);
    // Named again, it replaces itself.
    let again = replace_unreleased(&out, "## [0.2.0] - 2027-01-02\n\n- newer\n").unwrap();
    assert_eq!(again.matches("## [0.2.0]").count(), 1, "{}", again);
    assert!(again.contains("- newer\n\n## [0.1.0]"), "{}", again);
  }

  #[test]
  fn test_tooling_only_commits_are_left_out() {
    assert!(tooling_only("feat(xtask,docs): a"));
    assert!(tooling_only("fix(tests): a"));
    assert!(tooling_only("refactor(records,xtask): a\n\nbody"));
    assert!(!tooling_only("feat(cli): a"));
    assert!(!tooling_only("feat(cli,xtask): a"));
    assert!(!tooling_only("fix(docs): a"));
    assert!(!tooling_only("feat: a"));
    assert!(tooling_pattern().contains("(xtask|records|flow|ci|tests|fuzz)"));
  }
}
