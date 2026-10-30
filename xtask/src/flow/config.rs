//! `flow.toml`, read into what the stages need.

use std::fs;
use std::path::Path;

use crate::toml::{self, Table, Value};

/// How a tool is installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
  /// A Rust toolchain from rustup.
  Toolchain,
  /// A crate from crates.io, at an exact version.
  Cargo,
  /// Installed by hand; the version is the oldest that works.
  System,
  /// A crate beside this repository; optional.
  Path,
}

/// When a commit may hold more than its first line.
#[derive(Clone, Debug)]
pub struct Body {
  pub refused_kinds: Vec<String>,
  pub min_changed_lines: usize,
  pub required_for_breaking: bool,
  pub max_width: usize,
  pub max_lines: usize,
  /// The names a last paragraph's `Name: value` lines may have, which are not a body.
  pub trailers: Vec<String>,
}

/// How a commit's message is checked.
#[derive(Clone, Debug)]
pub struct Commit {
  pub max_header: usize,
  pub scope_must_match: bool,
  /// The newest commit made before these rules: it and its history are not checked.
  pub since: Option<String>,
  pub body: Body,
}

/// `flow.toml`.
#[derive(Clone, Debug)]
pub struct Flow {
  pub commit: Commit,
  /// Each kind, with its changelog heading (`None`: left out), in the order written.
  pub kinds: Vec<(String, Option<String>)>,
  /// Each section, with the paths it covers.
  pub sections: Vec<(String, Vec<String>)>,
  /// Each stage's checks, in order.
  pub stages: Vec<(String, Vec<String>)>,
  pub release: Release,
  pub secrets: Secrets,
  /// The tools development needs, in the order written.
  pub tools: Vec<Tool>,
}

impl Flow {
  pub fn kind(&self, name: &str) -> Option<&Option<String>> {
    self.kinds.iter().find(|(k, _)| k == name).map(|(_, g)| g)
  }

  pub fn section(&self, name: &str) -> Option<&[String]> {
    self
      .sections
      .iter()
      .find(|(s, _)| s == name)
      .map(|(_, p)| p.as_slice())
  }

  pub fn stage(&self, name: &str) -> Option<&[String]> {
    self
      .stages
      .iter()
      .find(|(s, _)| s == name)
      .map(|(_, c)| c.as_slice())
  }

  pub fn tool(&self, name: &str) -> Option<&Tool> {
    self.tools.iter().find(|t| t.name == name)
  }

  /// The sections covering `path`, in the order written.
  pub fn sections_of(&self, path: &str) -> Vec<&str> {
    self
      .sections
      .iter()
      .filter(|(_, globs)| globs.iter().any(|g| super::glob::matches(g, path)))
      .map(|(s, _)| s.as_str())
      .collect()
  }
}

/// How releases are cut.
#[derive(Clone, Debug)]
pub struct Release {
  pub branch: String,
  /// Refs only a release may push.
  pub hold: Vec<String>,
  /// `owner/name` on GitHub.
  pub repository: String,
  /// The Homebrew tap, `owner/name`.
  pub tap: String,
  pub first_as_draft: bool,
  /// The targets each release carries binaries for.
  pub targets: Vec<String>,
  /// Each expiring credential CI uses, with its expiry (`YYYY-MM-DD`, empty when not
  /// yet made).
  pub tokens: Vec<(String, String)>,
}

/// What looks like a credential.
#[derive(Clone, Debug)]
pub struct Secrets {
  /// Extended regular expressions, as `git grep -E` reads them.
  pub patterns: Vec<String>,
  pub allow: Vec<String>,
}

/// A tool development needs.
#[derive(Clone, Debug)]
pub struct Tool {
  pub name: String,
  /// The command it puts on the path: its name, unless `bin` says otherwise.
  pub bin: String,
  pub kind: ToolKind,
  /// A toolchain's name, a crate's exact version, a system tool's oldest.
  pub version: String,
  pub components: Vec<String>,
  /// Where a system tool comes from, or a path crate's folder.
  pub source: String,
  /// The stages and commands that check for it before they run.
  pub needed_by: Vec<String>,
  /// Installed when it can be, never required (a tool not yet published).
  pub optional: bool,
}

/// `flow.toml` at `root`.
pub fn load(root: &Path) -> Result<Flow, String> {
  let path = root.join("flow.toml");
  let text = fs::read_to_string(&path).map_err(|e| format!("flow.toml: {}", e))?;
  parse(&text)
}

pub fn parse(text: &str) -> Result<Flow, String> {
  let doc = toml::parse(text).map_err(|e| format!("flow.toml: {}", e))?;
  let commit = table(&doc, "commit")?;
  let body = table(commit, "body")?;
  let kinds = table(&doc, "kinds")?
    .entries()
    .iter()
    .map(|e| match &e.value {
      Value::String(s) => Ok((e.key.clone(), Some(s.clone()))),
      Value::Boolean(false) => Ok((e.key.clone(), None)),
      _ => Err(format!(
        "flow.toml: kinds.{} is a changelog heading or false",
        e.key
      )),
    })
    .collect::<Result<Vec<_>, _>>()?;
  let sections = table(&doc, "sections")?
    .entries()
    .iter()
    .map(|e| {
      Ok((
        e.key.clone(),
        strings(&e.value, &format!("sections.{}", e.key))?,
      ))
    })
    .collect::<Result<Vec<_>, String>>()?;
  let stages = table(&doc, "stages")?
    .entries()
    .iter()
    .map(|e| {
      let checks = strings(&e.value, &format!("stages.{}", e.key))?;
      if let Some(bad) = checks
        .iter()
        .find(|c| !super::checks::NAMES.contains(&c.as_str()))
      {
        return Err(format!(
          "flow.toml: stages.{} lists `{}`, which is not a check ({})",
          e.key,
          bad,
          super::checks::NAMES.join(", ")
        ));
      }
      Ok((e.key.clone(), checks))
    })
    .collect::<Result<Vec<_>, String>>()?;
  let tools = table(&doc, "tools")?
    .entries()
    .iter()
    .map(|e| tool(&e.key, &e.value))
    .collect::<Result<Vec<_>, String>>()?;
  let release = table(&doc, "release")?;
  let secrets = table(&doc, "secrets")?;
  Ok(Flow {
    tools,
    stages,
    release: Release {
      branch: release
        .get("branch")
        .and_then(Value::as_str)
        .ok_or("flow.toml: release.branch is a string")?
        .to_string(),
      hold: list(release, "hold", "release")?,
      repository: release
        .get("repository")
        .and_then(Value::as_str)
        .ok_or("flow.toml: release.repository is `owner/name`")?
        .to_string(),
      tap: release
        .get("tap")
        .and_then(Value::as_str)
        .ok_or("flow.toml: release.tap is `owner/name`")?
        .to_string(),
      first_as_draft: boolean(release, "first_as_draft", "release")?,
      targets: list(release, "targets", "release")?,
      tokens: match release.get("tokens").and_then(Value::as_table) {
        Some(tokens) => tokens
          .entries()
          .iter()
          .map(|e| {
            e.value
              .as_str()
              .map(|d| (e.key.clone(), d.to_string()))
              .ok_or(format!(
                "flow.toml: release.tokens.{} is a date (YYYY-MM-DD) or \"\"",
                e.key
              ))
          })
          .collect::<Result<Vec<_>, String>>()?,
        None => Vec::new(),
      },
    },
    secrets: Secrets {
      patterns: list(secrets, "patterns", "secrets")?,
      allow: list(secrets, "allow", "secrets")?,
    },
    commit: Commit {
      max_header: number(commit, "max_header", "commit")?,
      scope_must_match: boolean(commit, "scope_must_match", "commit")?,
      since: match commit.get("since") {
        Some(v) => Some(
          v.as_str()
            .filter(|s| !s.is_empty())
            .ok_or("flow.toml: commit.since is a commit id")?
            .to_string(),
        ),
        None => None,
      },
      body: Body {
        refused_kinds: strings(
          body
            .get("refused_kinds")
            .ok_or("flow.toml: commit.body.refused_kinds is missing")?,
          "commit.body.refused_kinds",
        )?,
        min_changed_lines: number(body, "min_changed_lines", "commit.body")?,
        required_for_breaking: boolean(body, "required_for_breaking", "commit.body")?,
        max_width: number(body, "max_width", "commit.body")?,
        max_lines: number(body, "max_lines", "commit.body")?,
        trailers: strings(
          body
            .get("trailers")
            .ok_or("flow.toml: commit.body.trailers is missing")?,
          "commit.body.trailers",
        )?,
      },
    },
    kinds,
    sections,
  })
}

fn boolean(t: &Table, key: &str, at: &str) -> Result<bool, String> {
  t.get(key)
    .and_then(Value::as_bool)
    .ok_or(format!("flow.toml: {}.{} is true or false", at, key))
}

fn list(t: &Table, key: &str, at: &str) -> Result<Vec<String>, String> {
  strings(
    t.get(key)
      .ok_or(format!("flow.toml: {}.{} is missing", at, key))?,
    &format!("{}.{}", at, key),
  )
}

fn number(t: &Table, key: &str, at: &str) -> Result<usize, String> {
  t.get(key)
    .and_then(Value::as_integer)
    .and_then(|n| usize::try_from(n).ok())
    .ok_or(format!("flow.toml: {}.{} is a number", at, key))
}

fn strings(v: &Value, at: &str) -> Result<Vec<String>, String> {
  v.as_array()
    .ok_or(format!("flow.toml: {} is a list of strings", at))?
    .iter()
    .map(|s| {
      s.as_str()
        .map(String::from)
        .ok_or(format!("flow.toml: {} is a list of strings", at))
    })
    .collect()
}

fn table<'a>(parent: &'a Table, key: &str) -> Result<&'a Table, String> {
  parent
    .get(key)
    .and_then(Value::as_table)
    .ok_or(format!("flow.toml: [{}] is missing", key))
}

/// One `[tools.NAME]` table.
fn tool(name: &str, value: &Value) -> Result<Tool, String> {
  let at = format!("tools.{}", name);
  let t = value
    .as_table()
    .ok_or(format!("flow.toml: [{}] is a table", at))?;
  let text = |key: &str| t.get(key).and_then(Value::as_str).map(String::from);
  let kind = match text("kind").as_deref() {
    Some("toolchain") => ToolKind::Toolchain,
    Some("cargo") => ToolKind::Cargo,
    Some("system") => ToolKind::System,
    Some("path") => ToolKind::Path,
    _ => {
      return Err(format!(
        "flow.toml: {}.kind is toolchain, cargo, system or path",
        at
      ));
    }
  };
  let version = text("version").unwrap_or_default();
  if kind != ToolKind::Path && version.is_empty() {
    return Err(format!("flow.toml: {}.version is missing", at));
  }
  let optional = |key: &str| match t.get(key) {
    Some(v) => strings(v, &format!("{}.{}", at, key)),
    None => Ok(Vec::new()),
  };
  Ok(Tool {
    name: name.to_string(),
    bin: text("bin").unwrap_or_else(|| name.to_string()),
    kind,
    version,
    components: optional("components")?,
    source: text("url").or_else(|| text("path")).unwrap_or_default(),
    needed_by: optional("needed_by")?,
    optional: t.get("optional").and_then(Value::as_bool).unwrap_or(false),
  })
}
