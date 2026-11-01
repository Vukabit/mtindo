//! `labels [--go]`: the repository's labels on GitHub made what `.github/labels.txt`
//! says: each missing one created, each with another color or description changed. A
//! plan unless `--go`. A label GitHub has and the file does not is reported, never
//! deleted: issues may wear it.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use super::config::Flow;
use crate::common::json::{self, Json};
use crate::common::process;

/// The file, from the repository root.
pub const FILE: &str = ".github/labels.txt";

/// One label: its name, color (six hex digits) and description.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
  pub name: String,
  pub color: String,
  pub description: String,
}

pub fn labels(root: &Path, flow: &Flow, go: bool) -> Result<(), String> {
  let wanted =
    parse(&fs::read_to_string(root.join(FILE)).map_err(|e| format!("{}: {}", FILE, e))?)?;
  let repo = &flow.release.repository;
  let listed = gh(
    root,
    &["api", &format!("repos/{}/labels?per_page=100", repo)],
  )?;
  let have = from_github(&listed)?;
  let changes = plan(&wanted, &have);
  for name in have
    .iter()
    .map(|l| &l.name)
    .filter(|n| !wanted.iter().any(|w| &w.name == *n))
  {
    println!("kept, not in {}: {}", FILE, name);
  }
  if changes.is_empty() {
    println!("labels: GitHub's are as {} says", FILE);
    return Ok(());
  }
  for (label, create) in &changes {
    println!(
      "{} {} ({}: {})",
      if *create { "create" } else { "update" },
      label.name,
      label.color,
      label.description
    );
    if !go {
      continue;
    }
    let path = if *create {
      format!("repos/{}/labels", repo)
    } else {
      format!("repos/{}/labels/{}", repo, label.name.replace(' ', "%20"))
    };
    gh(
      root,
      &[
        "api",
        "--method",
        if *create { "POST" } else { "PATCH" },
        &path,
        "-f",
        &format!("name={}", label.name),
        "-f",
        &format!("color={}", label.color),
        "-f",
        &format!("description={}", label.description),
      ],
    )?;
  }
  if !go {
    println!("a plan: run `cargo xtask flow labels --go` to make it so");
  }
  Ok(())
}

/// `COLOR NAME: DESCRIPTION` lines.
pub fn parse(text: &str) -> Result<Vec<Label>, String> {
  text
    .lines()
    .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    .map(|line| {
      let (color, rest) = line.split_once(' ').ok_or(format!(
        "{}: `{}` is not `COLOR NAME: DESCRIPTION`",
        FILE, line
      ))?;
      let (name, description) = rest.split_once(": ").ok_or(format!(
        "{}: `{}` is not `COLOR NAME: DESCRIPTION`",
        FILE, line
      ))?;
      if color.len() != 6 || !color.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("{}: `{}` is not a color", FILE, color));
      }
      Ok(Label {
        name: name.to_string(),
        color: color.to_lowercase(),
        description: description.to_string(),
      })
    })
    .collect()
}

/// The changes that make `have` hold `wanted`: each label to create (true) or update.
pub fn plan(wanted: &[Label], have: &[Label]) -> Vec<(Label, bool)> {
  wanted
    .iter()
    .filter_map(|w| match have.iter().find(|h| h.name == w.name) {
      None => Some((w.clone(), true)),
      Some(h) if !h.color.eq_ignore_ascii_case(&w.color) || h.description != w.description => {
        Some((w.clone(), false))
      }
      Some(_) => None,
    })
    .collect()
}

/// The labels in GitHub's answer.
fn from_github(text: &str) -> Result<Vec<Label>, String> {
  let Json::List(items) = json::parse(text)? else {
    return Err("GitHub's labels are not a list".to_string());
  };
  Ok(
    items
      .iter()
      .map(|item| Label {
        name: item
          .get("name")
          .and_then(Json::as_str)
          .unwrap_or("")
          .to_string(),
        color: item
          .get("color")
          .and_then(Json::as_str)
          .unwrap_or("")
          .to_string(),
        description: item
          .get("description")
          .and_then(Json::as_str)
          .unwrap_or("")
          .to_string(),
      })
      .collect(),
  )
}

fn gh(root: &Path, args: &[&str]) -> Result<String, String> {
  let done = process::output(
    Command::new("gh")
      .args(args)
      .current_dir(root)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::QUICK,
  )
  .map_err(|e| format!("cannot run gh: {}", e))?;
  if done.code != 0 {
    return Err(format!(
      "gh {}: {}",
      args.join(" "),
      String::from_utf8_lossy(&done.stderr).trim()
    ));
  }
  Ok(String::from_utf8_lossy(&done.stdout).into_owned())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_the_repository_file_reads_and_plans() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let wanted = parse(&fs::read_to_string(root.join(FILE)).unwrap()).unwrap();
    assert!(wanted.iter().any(|l| l.name == "good first issue"));
    let have = vec![
      Label {
        name: "bug".to_string(),
        color: "D73A4A".to_string(),
        description: "Something isn't working".to_string(),
      },
      Label {
        name: "compat".to_string(),
        color: "000000".to_string(),
        description: "old".to_string(),
      },
    ];
    let changes = plan(&wanted, &have);
    assert!(
      !changes.iter().any(|(l, _)| l.name == "bug"),
      "{:?}",
      changes
    );
    assert!(
      changes
        .iter()
        .any(|(l, create)| l.name == "compat" && !create)
    );
    assert!(
      changes
        .iter()
        .any(|(l, create)| l.name == "tooling" && *create)
    );
    assert!(parse("zz0000 x: y\n").is_err());
  }
}
