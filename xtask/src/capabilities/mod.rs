//! `cargo xtask capabilities`: tracks every rustfmt option, option value and command-line
//! flag, and mto's own features, against what mto supports.
//!
//! - `refresh` reads rustfmt's surface and writes `compat/rustfmt-surface.txt`. Sources: the
//!   local rustfmt binaries and, when online, the source of the latest stable and nightly
//!   releases (from Rust's channel manifests; beta is never read). Offline, the stable and
//!   nightly readings of the last online refresh are kept. When a local binary's own commit
//!   can be checked out, its source is read too and must agree with the binary, which
//!   catches a reader that has drifted from rustfmt.
//! - `check`, also run as `cargo xtask gates capabilities`, compares that file and mto's own
//!   surface with `compat/capabilities.txt`. It reads only tracked files, so it needs no
//!   network and gives the same answer everywhere.
//! - `sync` adds untracked items to the manifest as `missing`, drops stale ones and restores
//!   the canonical layout.
//! - `report` summarises the manifest, and runs rustfmt's own test cases through mto when
//!   the stable checkout is cached (with `xtask-tools cases`, which links mtindo).
//!
//! Every subcommand logs to `.dev/logs/capabilities.log`.

mod binary;
pub(crate) mod fetch;
pub(crate) mod manifest;
pub(crate) mod mto;
mod report;
pub(crate) mod source;
pub(crate) mod surface;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use manifest::{Line, Manifest, Status};
use report::Cases;
use surface::{Kind, Reading, Surface};

use crate::common::color::color_enabled;
use crate::common::log::Logger;

pub(crate) const MANIFEST_PATH: &str = "compat/capabilities.txt";

/// The statuses of the standard formatter's options, generated into the crate so that
/// `mto config options` can show them (the package ships `src/` only).
pub(crate) const SUPPORT_PATH: &str = "src/options/support.rs";

const SURFACE_PATH: &str = "compat/rustfmt-surface.txt";

/// The gate: every problem with the manifest, as one line each.
pub fn check(root: &Path) -> Vec<String> {
  let surface = match load_surface(root) {
    Ok(surface) => surface,
    Err(e) => return vec![e],
  };
  let mto_items = match mto::surface(root) {
    Ok(items) => items,
    Err(e) => return vec![format!("cannot read mto's own surface: {}", e)],
  };
  let text = match fs::read_to_string(root.join(MANIFEST_PATH)) {
    Ok(text) => text,
    Err(_) => {
      return vec![format!(
        "{} is missing; run `cargo xtask capabilities sync`",
        MANIFEST_PATH
      )];
    }
  };
  let mut problems = evaluate(&surface, &mto_items, &text, &|path| {
    root.join(path).exists()
  });
  let (manifest, _) = Manifest::parse(&text);
  match support_rs(&manifest) {
    Ok(support) if fs::read_to_string(root.join(SUPPORT_PATH)).ok().as_ref() == Some(&support) => {}
    Ok(_) => problems.push(format!(
      "{} does not match {}; run `cargo xtask capabilities sync`",
      SUPPORT_PATH, MANIFEST_PATH
    )),
    Err(e) => problems.push(format!("cannot generate {}: {}", SUPPORT_PATH, e)),
  }
  problems
}

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "capabilities");
  log.info(&format!("cargo xtask {}", crate::cli::line()));
  let code = match m.subcommand() {
    Some(("refresh", _)) => refresh(root, &log),
    Some(("check", _)) => {
      let problems = check(root);
      for problem in problems.iter() {
        println!("{}", problem);
        log.error(problem);
      }
      if problems.is_empty() {
        println!("capabilities: every item has a status");
        0
      } else {
        println!("capabilities: {} problem(s)", problems.len());
        1
      }
    }
    Some(("sync", _)) => sync(root, &log),
    Some(("report", sub)) => report(root, crate::cli::on(sub, "markdown"), &log),
    _ => unreachable!("clap requires a subcommand"),
  };
  log.info(&format!("exit code {}", code));
  ExitCode::from(code)
}

pub(crate) fn load_surface(root: &Path) -> Result<Surface, String> {
  let text = fs::read_to_string(root.join(SURFACE_PATH)).map_err(|_| {
    format!(
      "{} is missing; run `cargo xtask capabilities refresh`",
      SURFACE_PATH
    )
  })?;
  Surface::parse(&text).map_err(|e| format!("{}: {}", SURFACE_PATH, e))
}

/// Where a binary and the source it was built from disagree on names or stability. Values
/// are left out: they come from documentation, which may lag behind the code.
fn cross_check(binary: &Reading, own_source: &Reading) -> Vec<String> {
  let index = |reading: &Reading, kind: Kind| -> BTreeMap<String, (bool, bool)> {
    reading
      .items
      .iter()
      .filter(|f| f.kind == kind)
      .map(|f| (f.name.clone(), (f.stable, f.hidden)))
      .collect()
  };
  let stable_binary = binary
    .items
    .iter()
    .any(|f| f.kind == Kind::Flag && f.stable);
  let mut problems = Vec::new();
  let binary_options = index(binary, Kind::Option);
  let source_options = index(own_source, Kind::Option);
  for (name, (stable, _)) in binary_options.iter() {
    match source_options.get(name) {
      None => problems.push(format!("option {} is not in the source", name)),
      Some((source_stable, _)) if source_stable != stable => problems.push(format!(
        "option {} is {} in the binary but {} in the source",
        name,
        stability(*stable),
        stability(*source_stable)
      )),
      _ => {}
    }
  }
  for (name, (stable, hidden)) in source_options.iter() {
    if *stable && !*hidden && !binary_options.contains_key(name) {
      problems.push(format!("stable option {} is missing from the binary", name));
    }
  }
  let binary_flags = index(binary, Kind::Flag);
  let source_flags = index(own_source, Kind::Flag);
  for (name, (stable, _)) in source_flags.iter() {
    let visible = *stable || !stable_binary;
    if visible && !binary_flags.contains_key(name) {
      problems.push(format!("flag {} is missing from the binary", name));
    }
  }
  for name in binary_flags.keys() {
    match source_flags.get(name) {
      None => problems.push(format!("flag {} is not in the source", name)),
      Some((false, _)) if stable_binary => problems.push(format!(
        "flag {} is unstable but a stable binary shows it",
        name
      )),
      _ => {}
    }
  }
  problems
}

fn evaluate(
  surface: &Surface,
  mto_items: &[String],
  text: &str,
  exists: &dyn Fn(&str) -> bool,
) -> Vec<String> {
  let (manifest, mut problems) = Manifest::parse(text);
  let expected = expected_items(surface, mto_items);
  let mut seen = BTreeSet::new();
  for line in manifest.lines.iter() {
    let key = (line.kind, line.name.clone());
    let at = format!("{}:{}", MANIFEST_PATH, line.line_no);
    if !seen.insert(key.clone()) {
      problems.push(format!(
        "{}: {} {} is listed twice",
        at,
        line.kind.name(),
        line.name
      ));
    }
    if !expected.contains(&key) {
      problems.push(format!(
        "{}: {} {} is stale: no source has it any more; run `cargo xtask capabilities sync`",
        at,
        line.kind.name(),
        line.name
      ));
    }
    problems.extend(
      note_problems(line, exists)
        .into_iter()
        .map(|p| format!("{}: {}", at, p)),
    );
  }
  for (kind, name) in expected.iter() {
    if !seen.contains(&(*kind, name.clone())) {
      problems.push(format!(
        "{} {} has no status in {}; run `cargo xtask capabilities sync`",
        kind.name(),
        name,
        MANIFEST_PATH
      ));
    }
  }
  if problems.is_empty() && manifest.render() != text {
    problems.push(format!(
      "{} is not in canonical layout; run `cargo xtask capabilities sync`",
      MANIFEST_PATH
    ));
  }
  problems
}

/// Every item the manifest must list.
fn expected_items(surface: &Surface, mto_items: &[String]) -> BTreeSet<(Kind, String)> {
  let mut expected: BTreeSet<(Kind, String)> = surface.items.keys().cloned().collect();
  for item in mto_items {
    expected.insert((Kind::Mto, item.clone()));
  }
  expected
}

/// What a line's note must say for its status.
fn note_problems(line: &Line, exists: &dyn Fn(&str) -> bool) -> Vec<String> {
  let mut problems = Vec::new();
  let paths: Vec<&str> = line
    .note
    .split_whitespace()
    .map(|word| word.trim_matches(|c: char| ",;:()`'\"".contains(c)))
    .filter(|word| word.contains('/') && !word.contains("://"))
    .collect();
  for path in paths.iter() {
    if !exists(path) {
      problems.push(format!(
        "{} names {}, which does not exist",
        line.name, path
      ));
    }
  }
  match line.status {
    Status::Supported if paths.is_empty() => problems.push(format!(
      "{} is supported, so its note must name the test that shows it",
      line.name
    )),
    Status::Partial if line.note.is_empty() => problems.push(format!(
      "{} is partial, so its note must say which part works",
      line.name
    )),
    Status::NotPlanned if line.note.is_empty() => problems.push(format!(
      "{} is not-planned, so its note must say why",
      line.name
    )),
    _ => {}
  }
  problems
}

fn read_release(root: &Path, channel: &str, log: &Logger) -> Result<Reading, String> {
  let release = fetch::latest(root, channel, log)?;
  let dir = fetch::checkout(root, &release.commit, log)?;
  let description = format!(
    "rustfmt {}, rust-lang/rust {}, released {}",
    release.version, release.commit, release.date
  );
  source::read(channel, &description, &dir)
}

/// How rustfmt's own test pairs fare, from the numbers `cargo xtask diff` recorded in
/// compat/compatibility.txt (scored there with each pair's settings).
fn recorded_cases(root: &Path) -> Cases {
  let Ok(text) = fs::read_to_string(root.join("compat/compatibility.txt")) else {
    return Cases::Unavailable("compat/compatibility.txt cannot be read".to_string());
  };
  // `set NAME files MATCHED TOTAL lines ...`
  let set = |name: &str| {
    text.lines().find_map(|line| {
      let words: Vec<&str> = line.split_whitespace().collect();
      match words.as_slice() {
        ["set", set, "files", matched, total, ..] if *set == name => {
          Some((matched.parse().ok()?, total.parse().ok()?))
        }
        _ => None,
      }
    })
  };
  match (set("tests"), set("tests-with-settings")) {
    (Some((passed, total)), Some((with_settings_passed, with_settings_total))) => Cases::Recorded {
      passed,
      total,
      with_settings_passed,
      with_settings_total,
    },
    _ => Cases::Unavailable("no test pairs recorded; run `cargo xtask diff --update`".to_string()),
  }
}

fn refresh(root: &Path, log: &Logger) -> u8 {
  let previous = load_surface(root).ok();
  let mut readings: Vec<Reading> = binary::local_readings(log);
  for reading in readings.iter() {
    println!("{}: {}", reading.label, reading.description);
    log.info(&format!("{}: {}", reading.label, reading.description));
  }
  for channel in ["stable", "nightly"] {
    match read_release(root, channel, log) {
      Ok(reading) => {
        println!("{}: {}", channel, reading.description);
        log.info(&format!("{}: {}", channel, reading.description));
        readings.push(reading);
      }
      Err(e) => {
        log.warning(&format!("{}: {}", channel, e));
        match previous.as_ref().and_then(|s| s.reading(channel)) {
          Some(kept) => {
            println!(
              "{}: unavailable ({}); keeping {}",
              channel, e, kept.description
            );
            readings.push(kept);
          }
          None => println!(
            "{}: unavailable ({}); no earlier reading to keep",
            channel, e
          ),
        }
      }
    }
  }
  if readings.is_empty() {
    println!("no source could be read; {} left unchanged", SURFACE_PATH);
    log.error("no source could be read");
    return 1;
  }

  let mut disagreements = Vec::new();
  for reading in readings.iter().filter(|r| r.label.starts_with("local")) {
    let Some(commit) = binary::toolchain_commit(reading) else {
      println!(
        "{}: cross-check skipped (toolchain commit unknown)",
        reading.label
      );
      continue;
    };
    let checked = fetch::checkout(root, &commit, log)
      .and_then(|dir| source::read("cross-check", &commit, &dir));
    match checked {
      Ok(own_source) => {
        let found = cross_check(reading, &own_source);
        println!(
          "{}: cross-checked against its own source at {}: {}",
          reading.label,
          &commit[..12],
          if found.is_empty() {
            "agrees".to_string()
          } else {
            format!("{} disagreement(s)", found.len())
          }
        );
        disagreements.extend(
          found
            .into_iter()
            .map(|d| format!("{}: {}", reading.label, d)),
        );
      }
      Err(e) => {
        println!("{}: cross-check skipped ({})", reading.label, e);
        log.warning(&format!("{}: cross-check skipped: {}", reading.label, e));
      }
    }
  }
  if !disagreements.is_empty() {
    for d in disagreements.iter() {
      println!("  {}", d);
      log.error(d);
    }
    println!(
      "a binary and its own source disagree, so a reader is out of date; {} left unchanged",
      SURFACE_PATH
    );
    return 1;
  }

  let surface = Surface::from_readings(&readings);
  let text = surface.render();
  let path = root.join(SURFACE_PATH);
  if fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
    println!(
      "{} is up to date ({} items)",
      SURFACE_PATH,
      surface.items.len()
    );
    return 0;
  }
  if let Err(e) =
    fs::create_dir_all(path.parent().unwrap_or(root)).and_then(|_| fs::write(&path, &text))
  {
    println!("cannot write {}: {}", path.display(), e);
    log.error(&format!("cannot write {}: {}", path.display(), e));
    return 1;
  }
  let before: BTreeSet<(Kind, String)> = previous
    .map(|s| s.items.keys().cloned().collect())
    .unwrap_or_default();
  let after: BTreeSet<(Kind, String)> = surface.items.keys().cloned().collect();
  for (kind, name) in after.difference(&before) {
    println!("  + {} {}", kind.name(), name);
    log.info(&format!("new item: {} {}", kind.name(), name));
  }
  for (kind, name) in before.difference(&after) {
    println!("  - {} {}", kind.name(), name);
    log.info(&format!("item gone: {} {}", kind.name(), name));
  }
  println!("wrote {} ({} items)", SURFACE_PATH, surface.items.len());
  log.info(&format!(
    "wrote {} ({} items)",
    SURFACE_PATH,
    surface.items.len()
  ));
  0
}

fn report(root: &Path, markdown: bool, log: &Logger) -> u8 {
  let problems = check(root);
  if !problems.is_empty() {
    println!("the manifest has problems; run `cargo xtask capabilities check`");
    return 1;
  }
  let surface = match load_surface(root) {
    Ok(surface) => surface,
    Err(e) => {
      println!("{}", e);
      return 1;
    }
  };
  let text = fs::read_to_string(root.join(MANIFEST_PATH)).unwrap_or_default();
  let (manifest, _) = Manifest::parse(&text);
  let style = if markdown {
    report::Style::Markdown
  } else {
    report::Style::Terminal {
      color: color_enabled(false),
    }
  };
  let out = report::render(&surface, &manifest, &recorded_cases(root), style);
  print!("{}", out);
  log.info(&format!("report:\n{}", out));
  0
}

fn stability(stable: bool) -> &'static str {
  if stable { "stable" } else { "unstable" }
}

/// `src/options/support.rs` for a manifest: each option's status, sorted by name.
/// The support table's source, formatted as the repository's own code is.
fn support_rs(manifest: &Manifest) -> Result<String, String> {
  let mut options: Vec<(&str, &str)> = manifest
    .lines
    .iter()
    .filter(|line| line.kind == Kind::Option)
    .map(|line| (line.name.as_str(), line.status.name()))
    .collect();
  options.sort();
  // The values not honored yet of the options honored in part, which say so when set.
  let partial = |option: &str| {
    options
      .iter()
      .any(|&(name, status)| name == option && status == Status::Partial.name())
  };
  let mut values: Vec<(&str, &str)> = manifest
    .lines
    .iter()
    .filter(|line| line.kind == Kind::Value && line.status == Status::Missing)
    .filter_map(|line| line.name.split_once('='))
    .filter(|(option, _)| partial(option))
    .collect();
  values.sort();
  // In the order the repository's own layout keeps them.
  let mut out = String::from(
    "//! Generated by `cargo xtask capabilities sync` from compat/capabilities.txt; do not\n\
         //! edit. How far mto honors each of the standard formatter's options, for\n\
         //! `mto config options`.\n\n\
         /// The values not honored yet of the options honored in part (`partial`), as (option,\n\
         /// value), sorted.\n\
         pub const MISSING_VALUES: &[(&str, &str)] = &[\n",
  );
  for (option, value) in values {
    out.push_str(&format!("    (\"{}\", \"{}\"),\n", option, value));
  }
  out.push_str(
    "];\n\n\
         /// Each option's name and status (`supported`, `partial`, `missing`, `not-planned`),\n\
         /// sorted by name.\n\
         pub const OPTIONS: &[(&str, &str)] = &[\n",
  );
  for (name, status) in &options {
    out.push_str(&format!("    (\"{}\", \"{}\"),\n", name, status));
  }
  out.push_str("];\n");
  crate::unicode::rustfmt(&out)
}

fn sync(root: &Path, log: &Logger) -> u8 {
  let surface = match load_surface(root) {
    Ok(surface) => surface,
    Err(e) => {
      println!("{}", e);
      return 1;
    }
  };
  let mto_items = match mto::surface(root) {
    Ok(items) => items,
    Err(e) => {
      println!("cannot read mto's own surface: {}", e);
      return 1;
    }
  };
  let path = root.join(MANIFEST_PATH);
  let text = fs::read_to_string(&path).unwrap_or_default();
  let (mut manifest, errors) = Manifest::parse(&text);
  if !errors.is_empty() {
    for e in errors.iter() {
      println!("{}", e);
    }
    println!("fix these lines first; {} left unchanged", MANIFEST_PATH);
    return 1;
  }
  let expected = expected_items(&surface, &mto_items);
  let mut kept: Vec<Line> = Vec::new();
  let mut listed = BTreeSet::new();
  for line in manifest.lines.drain(..) {
    let key = (line.kind, line.name.clone());
    if !expected.contains(&key) {
      println!(
        "  - {} {} (stale, was {})",
        line.kind.name(),
        line.name,
        line.status.name()
      );
      log.info(&format!("dropped stale {} {}", line.kind.name(), line.name));
    } else if listed.insert(key) {
      kept.push(line);
    }
  }
  for (kind, name) in expected.iter() {
    if !listed.contains(&(*kind, name.clone())) {
      println!("  + {} {} (missing)", kind.name(), name);
      log.info(&format!("added {} {} as missing", kind.name(), name));
      kept.push(Line {
        kind: *kind,
        name: name.clone(),
        status: Status::Missing,
        note: String::new(),
        line_no: 0,
      });
    }
  }
  manifest.lines = kept;
  let support = match support_rs(&manifest) {
    Ok(support) => support,
    Err(e) => {
      println!("cannot generate {}: {}", SUPPORT_PATH, e);
      return 1;
    }
  };
  let support_path = root.join(SUPPORT_PATH);
  if fs::read_to_string(&support_path).ok() != Some(support.clone()) {
    if let Err(e) = fs::write(&support_path, &support) {
      println!("cannot write {}: {}", support_path.display(), e);
      return 1;
    }
    println!("wrote {}", SUPPORT_PATH);
  }
  let rendered = manifest.render();
  if rendered == text {
    println!("{} is up to date", MANIFEST_PATH);
    return 0;
  }
  if let Err(e) = fs::write(&path, &rendered) {
    println!("cannot write {}: {}", path.display(), e);
    return 1;
  }
  println!("wrote {} ({} items)", MANIFEST_PATH, manifest.lines.len());
  0
}

#[cfg(test)]
mod tests {
  use surface::Found;

  use super::*;

  fn manifest_text(lines: &[&str]) -> String {
    let mut manifest = Manifest::default();
    let (parsed, errors) = Manifest::parse(&lines.join("\n"));
    assert!(errors.is_empty(), "{:?}", errors);
    manifest.lines = parsed.lines;
    manifest.render()
  }

  fn sample_surface() -> Surface {
    let reading = |label: &str, items: Vec<Found>| Reading {
      label: label.to_string(),
      description: label.to_string(),
      items,
    };
    Surface::from_readings(&[
      reading(
        "stable",
        vec![
          Found::new(Kind::Option, "max_width", true),
          Found::new(Kind::Option, "group_imports", false),
          Found::new(Kind::Flag, "rustfmt:--check", true),
        ],
      ),
      reading(
        "nightly",
        vec![
          Found::new(Kind::Option, "max_width", true),
          Found::new(Kind::Option, "group_imports", true),
          Found::new(Kind::Option, "brand_new", false),
          Found::new(Kind::Flag, "rustfmt:--check", true),
        ],
      ),
    ])
  }

  #[test]
  fn test_a_complete_manifest_passes() {
    let text = manifest_text(&[
      "option max_width supported # tests/fixtures/x.rs",
      "option group_imports partial # One works",
      "option brand_new missing",
      "flag rustfmt:--check not-planned # covered by mto --check",
      "mto flag:--check supported # tests/domain_8_cli.rs",
    ]);
    let problems = evaluate(
      &sample_surface(),
      &["flag:--check".to_string()],
      &text,
      &|_| true,
    );
    assert!(problems.is_empty(), "{:?}", problems);
  }

  #[test]
  fn test_cross_check_finds_disagreements() {
    let binary = Reading {
      label: "local".to_string(),
      description: String::new(),
      items: vec![
        Found::new(Kind::Option, "max_width", true),
        Found::new(Kind::Option, "imports_granularity", true),
        Found::new(Kind::Flag, "rustfmt:--check", true),
      ],
    };
    let own_source = Reading {
      label: "cross-check".to_string(),
      description: String::new(),
      items: vec![
        Found::new(Kind::Option, "max_width", true),
        Found::new(Kind::Option, "imports_granularity", false),
        Found::new(Kind::Option, "tab_spaces", true),
        Found::new(Kind::Flag, "rustfmt:--check", true),
        Found::new(Kind::Flag, "rustfmt:--unstable-features", false),
      ],
    };
    let problems = cross_check(&binary, &own_source);
    assert_eq!(
      problems,
      vec![
        "option imports_granularity is stable in the binary but unstable in the source",
        "stable option tab_spaces is missing from the binary",
      ]
    );
  }

  #[test]
  fn test_layout_drift_is_reported_once_the_content_is_right() {
    let text = "option max_width missing\noption group_imports missing\noption brand_new missing\nflag rustfmt:--check missing\n";
    let problems = evaluate(&sample_surface(), &[], text, &|_| true);
    assert_eq!(problems.len(), 1, "{:?}", problems);
    assert!(problems[0].contains("not in canonical layout"));
  }

  #[test]
  fn test_notes_must_back_their_status() {
    let text = manifest_text(&[
      "option max_width supported # works",
      "option group_imports partial",
      "option brand_new not-planned",
      "flag rustfmt:--check supported # tests/missing.rs",
    ]);
    let problems = evaluate(&sample_surface(), &[], &text, &|path| {
      path != "tests/missing.rs"
    });
    let joined = problems.join("\n");
    assert!(
      joined.contains("max_width is supported, so its note must name the test"),
      "{}",
      joined
    );
    assert!(joined.contains("group_imports is partial"), "{}", joined);
    assert!(joined.contains("brand_new is not-planned"), "{}", joined);
    assert!(
      joined.contains("names tests/missing.rs, which does not exist"),
      "{}",
      joined
    );
  }

  #[test]
  fn test_untracked_stale_and_duplicate_items_fail() {
    let text = manifest_text(&[
      "option max_width missing",
      "option max_width missing",
      "option gone missing",
      "flag rustfmt:--check missing",
    ]);
    let problems = evaluate(&sample_surface(), &[], &text, &|_| true);
    let joined = problems.join("\n");
    assert!(joined.contains("max_width is listed twice"), "{}", joined);
    assert!(joined.contains("option gone is stale"), "{}", joined);
    assert!(
      joined.contains("option group_imports has no status"),
      "{}",
      joined
    );
    assert!(
      joined.contains("option brand_new has no status"),
      "{}",
      joined
    );
  }
}
