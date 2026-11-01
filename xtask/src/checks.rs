//! `cargo xtask checks`: every check by its one id, what it checks, and where it runs. The
//! checks are the repository rules (`crate::gates`), the flow checks the stages of
//! `flow.toml` list (`crate::flow::checks`), and the tasks of `cargo xtask test`
//! (`crate::test::tasks`), whose gate tasks are the rules themselves. `flow.toml`'s
//! `[stages]` says what runs when; `cargo xtask check` and `prove` are two of its stages.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use crate::common::json::Json;

/// One check: its title, the `cargo xtask test` group it runs in, and the stages that list
/// it (a gate is also in every stage that lists `gates`).
#[derive(Debug, Default, PartialEq)]
pub struct Entry {
  pub title: String,
  pub group: Option<String>,
  pub stages: Vec<String>,
}

/// Every check as `mtindo.xtask.checks/1`: its id, title, test group and stages.
pub fn json(registry: &BTreeMap<String, Entry>) -> Json {
  Json::document(
    "mtindo.xtask.checks/1",
    vec![(
      "checks",
      Json::List(
        registry
          .iter()
          .map(|(id, e)| {
            Json::object(vec![
              ("id", id.into()),
              ("title", (&e.title).into()),
              ("group", e.group.clone().into()),
              ("stages", e.stages.clone().into()),
            ])
          })
          .collect(),
      ),
    )],
  )
}

/// Every check, by id.
pub fn registry(root: &Path) -> Result<BTreeMap<String, Entry>, String> {
  let flow = crate::flow::config::load(root)?;
  let mut out: BTreeMap<String, Entry> = BTreeMap::new();
  for (id, title) in crate::flow::checks::CHECKS {
    out.entry(id.to_string()).or_default().title = title.to_string();
  }
  for (id, title, _) in crate::gates::GATES {
    out.entry(id.to_string()).or_default().title = title.to_string();
  }
  let (tasks, _) = crate::test::tasks::build(root, false, crate::test::tasks::DEFAULT_FUZZ_TIME);
  for task in tasks {
    let entry = out.entry(task.id.clone()).or_default();
    if entry.title.is_empty() {
      entry.title = task.title.clone();
    }
    entry.group = Some(task.group.to_string());
  }
  for (stage, names) in &flow.stages {
    for name in names {
      let ids: Vec<String> = if name == "gates" {
        std::iter::once("gates".to_string())
          .chain(crate::gates::GATES.iter().map(|(n, _, _)| n.to_string()))
          .collect()
      } else {
        vec![name.clone()]
      };
      for id in ids {
        let stages = &mut out.entry(id).or_default().stages;
        if !stages.contains(stage) {
          stages.push(stage.clone());
        }
      }
    }
  }
  Ok(out)
}

pub fn run(root: &Path, _m: &clap::ArgMatches) -> ExitCode {
  let registry = match registry(root) {
    Ok(registry) => registry,
    Err(e) => {
      println!("{}", e);
      return ExitCode::from(1);
    }
  };
  if _m.get_flag("json") {
    println!("{}", json(&registry).render());
    return ExitCode::SUCCESS;
  }
  print!("{}", text(&registry));
  ExitCode::SUCCESS
}

pub fn text(registry: &BTreeMap<String, Entry>) -> String {
  let mut out = String::new();
  for (id, e) in registry {
    out.push_str(&format!("{:<20} {}\n", id, e.title));
    let mut places = Vec::new();
    if let Some(group) = &e.group {
      places.push(format!("cargo xtask test ({})", group));
    }
    if !e.stages.is_empty() {
      places.push(format!("stages: {}", e.stages.join(", ")));
    }
    if places.is_empty() {
      places.push("named by no stage or group".to_string());
    }
    out.push_str(&format!("{:<20} {}\n", "", places.join("; ")));
  }
  out.push_str(&format!(
    "\n{} checks. What runs when: flow.toml [stages]; run a stage with `cargo xtask flow \
     check STAGE`, or `cargo xtask check` and `cargo xtask prove`.\n",
    registry.len()
  ));
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  /// One id is one check: a flow check and a `cargo xtask test` task may share an id only
  /// where they run the same thing.
  #[test]
  fn test_a_flow_check_and_a_test_task_share_an_id_only_for_one_thing() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let (tasks, _) = crate::test::tasks::build(root, false, 1);
    let shared: Vec<&str> = crate::flow::checks::CHECKS
      .iter()
      .map(|(id, _)| *id)
      .filter(|id| tasks.iter().any(|t| t.id == *id))
      .collect();
    // Both run `cargo fmt --all --check` and `cargo clippy --workspace --all-targets`.
    assert_eq!(
      shared,
      vec!["fmt", "clippy"],
      "a shared id must name one check"
    );
  }

  #[test]
  fn test_the_registry_places_each_check() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let registry = registry(root).unwrap();
    let naming = &registry["naming"];
    assert_eq!(naming.group.as_deref(), Some("gates"));
    assert!(
      naming.stages.iter().any(|s| s == "pre-commit"),
      "{:?}",
      naming
    );
    assert!(registry["tests-near"].stages.iter().any(|s| s == "check"));
    let shown = text(&registry);
    assert!(
      shown.contains("cargo xtask test (gates); stages: "),
      "{}",
      shown
    );
  }
}
