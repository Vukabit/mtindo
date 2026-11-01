//! `probe`: one of the standard formatter's test pairs against every answer key, outside
//! the repository, so that what a gap is measured against is seen rather than assumed.
//!
//! mto formats the pair's input with its header's settings, as `cargo xtask diff` scores
//! it. The keys are the expected file in the suite at stable and at nightly, and the
//! output of each toolchain's rustfmt run on the input in an empty folder with the header's
//! settings as its `rustfmt.toml` (and an empty home folder, so no setting leaks in).
//!
//! Where the keys disagree with the suite at stable (the default key), the disagreement is
//! looked up in `records/policy/disagreements.txt`. `--scan` checks every pair that way
//! and lists the disagreements not recorded and the recorded ones no longer seen;
//! `--record` adds the new ones, to triage.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::Mutex;

use mtindo::syntax::Edition;
use xtask_core::harness;

use crate::compare::{
  Answer, collect_rs, edition_year, mto_output, never_formatted, panic_message,
  reference_toolchain, run_rustfmt, test_pair,
};
use crate::process;

/// The answer keys, in the order they are shown; the first is the default.
pub const KEYS: [&str; 4] = ["tests@stable", "tests@nightly", "run@stable", "run@nightly"];

/// What a key makes of the case: a text, or why it gives none.
#[derive(Clone, Debug, PartialEq)]
enum Said {
  Text(String),
  Nothing(String),
}

/// One recorded disagreement: the case, the key that differs from the suite at stable,
/// and why.
#[derive(Clone, Debug, PartialEq)]
struct Disagreement {
  case: String,
  key: String,
  reason: String,
}

/// A key's answer, with a note on how it was made (a version), and why it is no answer for
/// the pair when it is not.
struct Keyed {
  key: &'static str,
  said: Said,
  note: String,
  /// Why the key's output is no answer for the pair: settings it ignored, being unstable,
  /// or a crate root it ran without.
  unfit: Option<String>,
}

struct Options {
  stable: PathBuf,
  nightly: Option<PathBuf>,
  surface: PathBuf,
  record: PathBuf,
  toolchain: String,
  show: Option<String>,
  scan: bool,
  write: bool,
  case: Option<String>,
}

struct Probe {
  name: String,
  edition: Edition,
  settings: Vec<(String, String)>,
  mto: String,
  /// What mto panicked with, when it did (its output is then empty).
  panicked: Option<String>,
  keys: Vec<Keyed>,
}

pub fn run(args: &[String]) -> ExitCode {
  let options = match parse_args(args) {
    Ok(options) => options,
    Err(e) => {
      eprintln!("{}", e);
      return ExitCode::from(2);
    }
  };
  let strings = harness::string_keys(&fs::read_to_string(&options.surface).unwrap_or_default());
  let versions = [
    ("run@stable", version(&reference_toolchain())),
    ("run@nightly", version(&options.toolchain)),
  ];
  let recorded = read_record(&options.record);
  if options.scan {
    return scan(&options, &strings, &versions, &recorded);
  }
  let Some(case) = &options.case else {
    eprintln!("probe CASE, or probe --scan");
    return ExitCode::from(2);
  };
  let Some(relative) = relative_of(case, &options.stable) else {
    eprintln!(
      "no test pair `{}` in the suite at stable ({}); name it as the diff does, \
       e.g. tests/configs/wrap_comments/true.rs",
      case,
      options.stable.display()
    );
    return ExitCode::from(1);
  };
  let home = scratch("home");
  let probe = probe(&options, &relative, &strings, &versions, &home, 0);
  print!("{}", text(&probe, &recorded));
  if let Some(key) = &options.show {
    match probe.keys.iter().find(|k| k.key == key) {
      Some(Keyed {
        said: Said::Text(theirs),
        ..
      }) => print!("\n{}", side_by_side(&probe.mto, theirs, key)),
      Some(_) => println!("\n{} gives no output to show", key),
      None => println!("\nno key `{}`; the keys are {}", key, KEYS.join(", ")),
    }
  }
  if options.write {
    let new: Vec<Disagreement> = disagreements(&probe)
      .into_iter()
      .filter(|d| !recorded.iter().any(|r| r.case == d.case && r.key == d.key))
      .collect();
    append_record(&options.record, &new);
    println!("\nrecorded {} disagreement(s)", new.len());
  }
  ExitCode::SUCCESS
}

fn append_record(path: &Path, new: &[Disagreement]) {
  if new.is_empty() {
    return;
  }
  let mut text = fs::read_to_string(path).unwrap_or_default();
  for d in new {
    text.push_str(&record_line(d));
    text.push('\n');
  }
  let _ = fs::write(path, text);
}

/// Where the keys differ from the suite at stable, as records: the suite at nightly
/// changed, or rustfmt does not produce the expected file. A stable rustfmt that ignored
/// unstable settings is no answer for the pair, so it is not compared.
fn disagreements(probe: &Probe) -> Vec<Disagreement> {
  let Said::Text(base) = &probe.keys[0].said else {
    return Vec::new();
  };
  probe.keys[1..]
    .iter()
    .filter(|k| k.unfit.is_none())
    .filter_map(|k| match &k.said {
      // Line by line, as the test runner compares: line endings aside.
      Said::Text(text) if !text.lines().eq(base.lines()) => Some(Disagreement {
        case: probe.name.clone(),
        key: k.key.to_string(),
        reason: if k.key.starts_with("tests@") {
          "auto-recorded, to triage: the suite's expected file changed".to_string()
        } else {
          "auto-recorded, to triage: rustfmt does not produce the expected file".to_string()
        },
      }),
      _ => None,
    })
    .collect()
}

/// The first line (0-based) where two texts differ; `None` when they are the same.
fn first_difference(a: &str, b: &str) -> Option<usize> {
  if a == b {
    return None;
  }
  let (a, b): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
  // Equal lines but for the endings: the first line, as the texts differ there.
  Some(
    (0..a.len().max(b.len()))
      .find(|&i| a.get(i) != b.get(i))
      .unwrap_or(0),
  )
}

/// The settings a stable rustfmt says it cannot set (an unstable option or value), from
/// its warnings.
fn ignored_settings(stderr: &str) -> Vec<String> {
  stderr
    .lines()
    .filter(|l| l.contains("are only available in nightly channel"))
    .filter_map(|l| l.split('`').nth(1))
    .filter_map(|setting| setting.split(" = ").next())
    .map(str::to_string)
    .collect()
}

fn parse_args(args: &[String]) -> Result<Options, String> {
  let mut options = Options {
    stable: PathBuf::new(),
    nightly: None,
    surface: PathBuf::new(),
    record: PathBuf::new(),
    toolchain: "nightly".to_string(),
    show: None,
    scan: false,
    write: false,
    case: None,
  };
  let mut i = 0;
  while i < args.len() {
    let value = |i: usize| {
      args
        .get(i + 1)
        .cloned()
        .ok_or(format!("{} needs a value", args[i]))
    };
    match args[i].as_str() {
      "--tests-stable" => options.stable = PathBuf::from(value(i)?),
      "--tests-nightly" => options.nightly = Some(PathBuf::from(value(i)?)),
      "--surface" => options.surface = PathBuf::from(value(i)?),
      "--disagreements" => options.record = PathBuf::from(value(i)?),
      "--toolchain" => options.toolchain = value(i)?,
      "--show" => options.show = Some(value(i)?),
      "--scan" => {
        options.scan = true;
        i += 1;
        continue;
      }
      "--record" => {
        options.write = true;
        i += 1;
        continue;
      }
      other if !other.starts_with("--") => {
        options.case = Some(other.to_string());
        i += 1;
        continue;
      }
      other => return Err(format!("probe: unknown argument {}", other)),
    }
    i += 2;
  }
  if options.stable.as_os_str().is_empty() || options.record.as_os_str().is_empty() {
    return Err("probe: --tests-stable and --disagreements are required".to_string());
  }
  Ok(options)
}

/// The pair `relative` against every key; `slot` keeps concurrent probes in folders of
/// their own.
fn probe(
  options: &Options,
  relative: &Path,
  strings: &BTreeSet<String>,
  versions: &[(&str, Option<String>)],
  home: &Path,
  slot: usize,
) -> Probe {
  let target = options.stable.join("target").join(relative);
  let expected = fs::read_to_string(&target).unwrap_or_default();
  let pair = test_pair(&options.stable, relative, expected);
  let (mto, panicked) = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
    mto_output(&pair.input, pair.edition, &pair.format)
  })) {
    Ok(text) => (text, None),
    Err(payload) => (String::new(), Some(panic_message(payload.as_ref()))),
  };
  let settings = harness::settings(&pair.input, &options.stable.join("config")).unwrap_or_default();
  let mut keys = vec![Keyed {
    key: KEYS[0],
    said: Said::Text(pair.expected.clone()),
    note: String::new(),
    unfit: None,
  }];
  let nightly = match &options.nightly {
    None => Said::Nothing("the suite at nightly is not cached".to_string()),
    Some(dir) => match fs::read_to_string(dir.join("target").join(relative)) {
      Ok(text) => Said::Text(test_pair(dir, relative, harness::older_headers(&text)).expected),
      Err(_) => Said::Nothing("not in the suite at nightly".to_string()),
    },
  };
  keys.push(Keyed {
    key: KEYS[1],
    said: nightly,
    note: String::new(),
    unfit: None,
  });
  let dir = scratch(&format!("case-{}", slot));
  let config = harness::config_text(&settings, strings);
  let _ = fs::remove_file(dir.join("rustfmt.toml"));
  if !config.is_empty() {
    let _ = fs::write(dir.join("rustfmt.toml"), &config);
  }
  let stable = reference_toolchain();
  for (key, toolchain) in [
    (KEYS[2], stable.as_str()),
    (KEYS[3], options.toolchain.as_str()),
  ] {
    let version = versions
      .iter()
      .find(|(k, _)| *k == key)
      .and_then(|(_, v)| v.clone());
    let Some(version) = version else {
      keys.push(Keyed {
        key,
        said: Said::Nothing(format!("no rustfmt for the {} toolchain", toolchain)),
        note: String::new(),
        unfit: None,
      });
      continue;
    };
    let (answer, stderr) = run_rustfmt(
      &pair.input,
      edition_year(pair.edition),
      &dir,
      home,
      toolchain,
    );
    let said = match answer {
      Answer::Text(text) => Said::Text(text),
      Answer::Refused(why) | Answer::NotRun(why) => {
        Said::Nothing(why.lines().take(3).collect::<Vec<_>>().join(" "))
      }
    };
    keys.push(Keyed {
      key,
      said,
      note: version,
      unfit: unfit(&ignored_settings(&stderr), &pair.format, &settings),
    });
  }
  Probe {
    name: format!("tests/{}", relative.display()),
    edition: pair.edition,
    settings,
    mto,
    panicked,
    keys,
  }
}

/// The recorded disagreements: `CASE  tests@stable != KEY  REASON` lines.
fn read_record(path: &Path) -> Vec<Disagreement> {
  fs::read_to_string(path)
    .unwrap_or_default()
    .lines()
    .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    .filter_map(|l| {
      let mut words = l.split_whitespace();
      let case = words.next()?.to_string();
      let base = words.next()?;
      let (_, key) = (
        words.next().filter(|w| *w == "!=")?,
        words.next()?.to_string(),
      );
      (base == KEYS[0]).then(|| Disagreement {
        case,
        key,
        reason: words.collect::<Vec<_>>().join(" "),
      })
    })
    .collect()
}

fn record_line(d: &Disagreement) -> String {
  format!("{}  tests@stable != {}  {}", d.case, d.key, d.reason)
}

/// The pair a case names: `tests/RELATIVE` as the diff names it, `RELATIVE`, or a path to
/// a file in a suite's `source/` or `target/`.
fn relative_of(case: &str, stable: &Path) -> Option<PathBuf> {
  let case = case.replace('\\', "/");
  let mut rest = case.as_str();
  for marker in ["/tests/target/", "/tests/source/"] {
    if let Some(at) = rest.find(marker) {
      rest = &rest[at + marker.len()..];
    }
  }
  let rest = rest.strip_prefix("tests/").unwrap_or(rest);
  let rest = rest
    .strip_prefix("target/")
    .or_else(|| rest.strip_prefix("source/"))
    .unwrap_or(rest);
  let relative = PathBuf::from(rest);
  stable
    .join("target")
    .join(&relative)
    .is_file()
    .then_some(relative)
}

/// Every pair of the suite at stable, probed: the disagreements not recorded, and the
/// recorded ones no longer seen. Fails when a disagreement is not recorded.
fn scan(
  options: &Options,
  strings: &BTreeSet<String>,
  versions: &[(&str, Option<String>)],
  recorded: &[Disagreement],
) -> ExitCode {
  let target_dir = options.stable.join("target");
  let mut targets = Vec::new();
  collect_rs(&target_dir, &mut targets);
  targets.sort();
  let relatives: Vec<PathBuf> = targets
    .iter()
    .filter_map(|t| t.strip_prefix(&target_dir).ok().map(Path::to_path_buf))
    .filter(|r| !never_formatted(r))
    .collect();
  let found = Mutex::new(Vec::new());
  let panics = Mutex::new(Vec::new());
  let next = Mutex::new(0usize);
  let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
  std::thread::scope(|s| {
    for worker in 0..threads {
      let (found, panics, next, relatives) = (&found, &panics, &next, &relatives);
      // mto's own stack: deep inputs recurse further than a thread's default allows.
      let builder = std::thread::Builder::new().stack_size(mtindo::STACK);
      let spawned = builder.spawn_scoped(s, move || {
        let home = scratch(&format!("home-{}", worker));
        loop {
          let i = {
            let mut next = next.lock().unwrap();
            *next += 1;
            *next - 1
          };
          let Some(relative) = relatives.get(i) else {
            break;
          };
          let probe = probe(options, relative, strings, versions, &home, worker + 1);
          if let Some(message) = &probe.panicked {
            panics
              .lock()
              .unwrap()
              .push(format!("{}: {}", probe.name, message));
          }
          found.lock().unwrap().extend(disagreements(&probe));
        }
      });
      if let Err(e) = spawned {
        eprintln!("cannot start a probe thread: {}", e);
      }
    }
  });
  let mut found = found.into_inner().unwrap();
  let mut panics = panics.into_inner().unwrap();
  panics.sort();
  found.sort_by(|a, b| (&a.case, &a.key).cmp(&(&b.case, &b.key)));
  let known = |d: &Disagreement, list: &[Disagreement]| {
    list.iter().any(|r| r.case == d.case && r.key == d.key)
  };
  let new: Vec<Disagreement> = found
    .iter()
    .filter(|d| !known(d, recorded))
    .cloned()
    .collect();
  let gone: Vec<&Disagreement> = recorded.iter().filter(|r| !known(r, &found)).collect();
  println!(
    "scanned {} pairs against {}: {} disagree with the suite at stable ({} recorded, {} new); \
     {} recorded no longer seen",
    relatives.len(),
    versions
      .iter()
      .map(|(key, v)| format!("{} ({})", key, v.as_deref().unwrap_or("not installed")))
      .collect::<Vec<_>>()
      .join(", "),
    found.len(),
    found.len() - new.len(),
    new.len(),
    gone.len()
  );
  if !new.is_empty() {
    println!("\nnot recorded:");
    for d in &new {
      println!("  {}", record_line(d));
    }
  }
  if !gone.is_empty() {
    println!("\nrecorded, no longer seen (remove them):");
    for d in &gone {
      println!("  {}", record_line(d));
    }
  }
  if !panics.is_empty() {
    println!("\nmto panicked on {} pair(s):", panics.len());
    for line in &panics {
      println!("  {}", line);
    }
    return ExitCode::from(1);
  }
  if options.write && !new.is_empty() {
    append_record(&options.record, &new);
    println!("\nrecorded {} disagreement(s)", new.len());
    return ExitCode::SUCCESS;
  }
  if new.is_empty() {
    ExitCode::SUCCESS
  } else {
    println!("\nrecord them with `cargo xtask probe --scan --record`, then triage the reasons");
    ExitCode::from(1)
  }
}

/// A folder of this run's own under the system's temporary folder, outside the repository.
fn scratch(name: &str) -> PathBuf {
  let dir = std::env::temp_dir()
    .join(format!("mtindo-probe-{}", std::process::id()))
    .join(name);
  let _ = fs::create_dir_all(&dir);
  dir
}

/// The lines around the first difference, mto's above the key's.
fn side_by_side(ours: &str, theirs: &str, key: &str) -> String {
  let Some(first) = first_difference(ours, theirs) else {
    return format!("mto and {} are the same\n", key);
  };
  let from = first.saturating_sub(3);
  let block = |text: &str| -> String {
    text
      .lines()
      .enumerate()
      .skip(from)
      .take(7)
      .map(|(i, l)| format!("{}{:5} {}\n", if i == first { ">" } else { " " }, i + 1, l))
      .collect()
  };
  format!("--- mto\n{}--- {}\n{}", block(ours), key, block(theirs))
}

/// The report of one probe.
fn text(probe: &Probe, recorded: &[Disagreement]) -> String {
  let mut out = format!("{} (edition {})\n", probe.name, edition_year(probe.edition));
  if probe.settings.is_empty() {
    out.push_str("  settings: the defaults\n");
  } else {
    let shown: Vec<String> = probe
      .settings
      .iter()
      .map(|(k, v)| format!("{} = {}", k, v))
      .collect();
    out.push_str(&format!("  settings: {}\n", shown.join(", ")));
  }
  if let Some(message) = &probe.panicked {
    out.push_str(&format!("\nmto panicked: {}\n", message));
  }
  out.push_str("\nmto against each key:\n");
  for k in &probe.keys {
    let verdict = match &k.said {
      Said::Text(theirs) => match first_difference(&probe.mto, theirs) {
        None => "matches".to_string(),
        Some(_) if probe.mto.lines().eq(theirs.lines()) => {
          "differs only in line endings or the last newline".to_string()
        }
        Some(line) => format!("differs from line {}", line + 1),
      },
      Said::Nothing(why) => format!("no answer: {}", why),
    };
    out.push_str(&format!("  {:<14} {}\n", k.key, verdict));
    if !k.note.is_empty() {
      out.push_str(&format!("  {:<14} {}\n", "", k.note));
    }
    if let Some(why) = &k.unfit {
      out.push_str(&format!("  {:<14} no answer for this pair: {}\n", "", why));
    }
  }
  let found = disagreements(probe);
  if found.is_empty() {
    out.push_str("\nThe keys agree with the suite at stable.\n");
  } else {
    out.push_str("\nWhere the keys disagree with the suite at stable:\n");
    for d in &found {
      match recorded.iter().find(|r| r.case == d.case && r.key == d.key) {
        Some(r) => out.push_str(&format!("  {}: recorded: {}\n", d.key, r.reason)),
        None => out.push_str(&format!("  {}: not recorded (`--record` adds it)\n", d.key)),
      }
    }
  }
  out
}

/// Why a run of rustfmt is no answer for a pair: it ignored settings the pair sets; the
/// pair is a module whose crate root's skip attributes the test runner applies, which a
/// run of the module alone cannot see; or the pair leaves generated files alone, which
/// rustfmt only recognizes in a file, not on stdin.
fn unfit(
  ignored: &[String],
  format: &mtindo::options::format::FormatConfig,
  settings: &[(String, String)],
) -> Option<String> {
  if !ignored.is_empty() {
    return Some(format!("it ignores {} (unstable)", ignored.join(", ")));
  }
  if settings
    .iter()
    .any(|(k, v)| k == "format_generated_files" && v == "false")
  {
    return Some("it formats stdin, which it never takes for a generated file".to_string());
  }
  if !format.crate_skipped_macros.is_empty() || !format.crate_skipped_attributes.is_empty() {
    return Some("it runs the module without its crate root's skip attributes".to_string());
  }
  None
}

/// The installed rustfmt of `toolchain`, by its version; `None` when it cannot run.
fn version(toolchain: &str) -> Option<String> {
  let out = process::output(
    Command::new("rustfmt")
      .arg("--version")
      .env("RUSTUP_TOOLCHAIN", toolchain)
      .env("RUSTUP_AUTO_INSTALL", "0")
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::QUICK,
  )
  .ok()?;
  (out.code == 0).then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn read_record_text(text: &str) -> Vec<Disagreement> {
    let path = std::env::temp_dir().join(format!("mtindo-probe-record-{}", std::process::id()));
    fs::write(&path, format!("# about\n{}\n", text)).unwrap();
    let read = read_record(&path);
    let _ = fs::remove_file(&path);
    read
  }

  #[test]
  fn test_a_case_is_named_as_the_diff_names_it_or_by_path() {
    let suite = std::env::temp_dir().join(format!("mtindo-probe-suite-{}", std::process::id()));
    let _ = fs::remove_dir_all(&suite);
    fs::create_dir_all(suite.join("target/configs/x")).unwrap();
    fs::write(suite.join("target/configs/x/true.rs"), "fn f() {}\n").unwrap();
    let want = Some(PathBuf::from("configs/x/true.rs"));
    for name in [
      "tests/configs/x/true.rs",
      "configs/x/true.rs",
      "tests/source/configs/x/true.rs",
      "/somewhere/rustfmt/tests/target/configs/x/true.rs",
    ] {
      assert_eq!(relative_of(name, &suite), want, "{}", name);
    }
    assert_eq!(relative_of("tests/none.rs", &suite), None);
    let _ = fs::remove_dir_all(&suite);
  }

  #[test]
  fn test_disagreements_leave_out_a_key_that_ignored_settings() {
    let keyed = |key, said: &str, ignored: &[&str]| Keyed {
      key,
      said: Said::Text(said.to_string()),
      note: String::new(),
      unfit: (!ignored.is_empty()).then(|| ignored.join(", ")),
    };
    let probe = Probe {
      name: "tests/a.rs".to_string(),
      edition: Edition::E2015,
      settings: Vec::new(),
      mto: "a\n".to_string(),
      panicked: None,
      keys: vec![
        keyed("tests@stable", "a\n", &[]),
        keyed("tests@nightly", "a\n", &[]),
        keyed("run@stable", "b\n", &["wrap_comments"]),
        keyed("run@nightly", "c\n", &[]),
      ],
    };
    let found = disagreements(&probe);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].key, "run@nightly");
    let line = record_line(&found[0]);
    assert_eq!(read_record_text(&line), found);
  }

  #[test]
  fn test_stable_names_the_unstable_settings_it_ignores() {
    let stderr = "Warning: can't set `wrap_comments = true`, unstable features are only \
                  available in nightly channel.\nWarning: can't set `comment_width = 20`, \
                  unstable features are only available in nightly channel.\nWarning: can't set \
                  `style_edition = Edition2027`, unstable variants are only available in \
                  nightly channel.\n";
    assert_eq!(
      ignored_settings(stderr),
      vec!["wrap_comments", "comment_width", "style_edition"]
    );
    assert!(ignored_settings("").is_empty());
  }

  #[test]
  fn test_the_first_difference_is_found_by_line() {
    assert_eq!(first_difference("a\nb\n", "a\nb\n"), None);
    assert_eq!(first_difference("a\nb\n", "a\nc\n"), Some(1));
    assert_eq!(first_difference("a\n", "a\r\n"), Some(0));
  }
}
