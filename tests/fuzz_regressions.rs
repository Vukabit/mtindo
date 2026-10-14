//! Every input a fuzz run found and that was kept as a test (`cargo xtask incidents
//! promote`), run through the checks of the target that found it: a crash fixed once stays
//! fixed. The inputs are `tests/fuzz_regressions/<target>/<signature>.bin`.

use std::fs;
use std::panic;
use std::path::Path;

/// Each fuzz target's checks, by the target's name.
fn checks(target: &str) -> Option<fn(&[u8])> {
  use mtindo::fuzzing;
  Some(match target {
    "comments" => fuzzing::comments::check,
    "format" => fuzzing::format::check,
    "generated" => fuzzing::generated::check,
    "layout" => fuzzing::layout::check,
    "lexer" => fuzzing::lexer::check,
    "parser" => fuzzing::parser::check,
    "toml" => fuzzing::toml::check,
    "verify" => fuzzing::verify::check,
    _ => return None,
  })
}

#[test]
fn test_every_kept_input_passes_its_targets_checks() {
  let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fuzz_regressions");
  let mut failures = Vec::new();
  let mut ran = 0;
  let mut targets: Vec<_> = fs::read_dir(&base)
    .map(|d| d.flatten().map(|e| e.path()).collect())
    .unwrap_or_default();
  targets.sort();
  for folder in targets.iter().filter(|p| p.is_dir()) {
    let target = folder.file_name().unwrap().to_string_lossy().into_owned();
    let Some(check) = checks(&target) else {
      failures.push(format!("{}: no such fuzz target", folder.display()));
      continue;
    };
    let mut inputs: Vec<_> = fs::read_dir(folder)
      .unwrap()
      .flatten()
      .map(|e| e.path())
      .filter(|p| p.extension().is_some_and(|e| e == "bin"))
      .collect();
    inputs.sort();
    for input in inputs {
      let bytes = fs::read(&input).unwrap();
      ran += 1;
      if let Err(payload) = panic::catch_unwind(|| check(&bytes)) {
        let message = payload
          .downcast_ref::<String>()
          .cloned()
          .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
          .unwrap_or_default();
        let first: String = message
          .lines()
          .next()
          .unwrap_or("")
          .chars()
          .take(200)
          .collect();
        failures.push(format!("{}: {}", input.display(), first));
      }
    }
  }
  assert!(
    failures.is_empty(),
    "{} of {} kept input(s) fail again:\n{}",
    failures.len(),
    ran,
    failures.join("\n")
  );
}
