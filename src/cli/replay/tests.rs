use std::fs;
use std::path::PathBuf;

use super::*;
use crate::history::now_millis;
use crate::report::{Bug, Failure, Replay, Store};

fn bug(said: &str, minimal: &str, file: &str) -> Bug {
  Bug {
    failure: Failure::Panicked { said: said.into() },
    file: file.into(),
    path: Some(PathBuf::from("/work").join(file)),
    minimal: Some(minimal.into()),
    backtrace: None,
    source_hash: "ab".repeat(32),
    settings: "layout: on\n".into(),
    command: "mto .".into(),
    replay: None,
  }
}

fn scratch(name: &str) -> PathBuf {
  let dir = std::env::temp_dir().join(format!("mtindo-replay-{}-{}", name, std::process::id()));
  let _ = fs::remove_dir_all(&dir);
  dir
}

/// A report keeps what runs its bug again; one whose bug no longer shows is found fixed,
/// and only `recheck` without `--dry-run` removes it.
#[test]
fn test_a_report_that_passes_is_found_fixed() {
  let dir = scratch("fixed");
  let store = Store::new(dir.clone());
  let settings = crate::Settings {
    layout: None,
    format: Some(crate::options::format::FormatConfig::default()),
    edition: crate::syntax::Edition::E2021,
  };
  let fixed = Bug {
    replay: Some(Replay {
      input: "fn a() {}\n".into(),
      settings: settings_file(&settings).unwrap(),
    }),
    ..bug("boom (at src/a.rs:1)", "fn a() {}\n", "./a.rs")
  };
  let id = store.save(&fixed).unwrap();
  let entry = store.find(&id).unwrap();
  assert_eq!(entry.replay(), fixed.replay);
  assert_eq!(recheck(&entry).verdict, Verdict::Fixed);
  let mut rows = String::new();
  let said =
    crate::cli::report::recheck(&store, &[], true, &mut |row| rows.push_str(&row)).unwrap();
  assert!(rows.contains(&format!("{}   fixed", &id[..6])), "{}", rows);
  assert!(
    said.contains("1 fixed (--dry-run: none removed)"),
    "{}",
    said
  );
  assert_eq!(store.list().len(), 1);
  let said = crate::cli::report::recheck(&store, &[], false, &mut |_| {}).unwrap();
  assert!(said.contains("1 fixed and removed"), "{}", said);
  assert!(store.list().is_empty());
  let _ = fs::remove_dir_all(&dir);
}

/// A report kept before replays were is run again from what it shows: its smallest part and
/// settings, under every edition when it does not say which. One whose settings do not
/// all map, or that kept no smallest part of a file since changed, is left as it is.
#[test]
fn test_an_older_report_is_replayed_only_when_certain() {
  let dir = scratch("older");
  let store = Store::new(dir.clone());
  let older = |id: &str, minimal: &str, settings: &str| {
    let report = format!(
      "## panic: boom (at src/a.rs:1)\n\nmto failed (panicked) while laying out a file:\n\n\
       ```text\nboom (at src/a.rs:1)\n```\n\n{}Command:\n\n```text\nmto --config \
       format=true --emit stdout --stdin-filepath x.rs -\n```\n\nSettings that differ from \
       the defaults:\n\n```text\n{}\n```\n\nBuild:\n\n```text\nmto 0.1.0\n```\n",
      minimal, settings
    );
    let at = dir.join(id);
    fs::create_dir_all(&at).unwrap();
    fs::write(at.join("report.md"), report).unwrap();
    fs::write(
      at.join("meta"),
      format!(
        "title panic: boom\ncount 1\nfirst 1\nlast {}\nfile {}\t/work/gone.rs\tgone.rs\n",
        now_millis(),
        "ab".repeat(32)
      ),
    )
    .unwrap();
    store.find(id).unwrap()
  };
  let smallest = "The smallest part of the file that still fails:\n\n```rust\nfn a() {}\n```\n\n";
  let plain = older(
    "aaaaaaaaaaaa",
    smallest,
    "layout: off\nformat: style_edition: 2021\nformat: max_width: 79",
  );
  let found = recheck(&plain);
  assert_eq!(found.verdict, Verdict::Fixed);
  let unmapped = older(
    "bbbbbbbbbbbb",
    smallest,
    "layout: off\nformat: crate_skipped_macros: [",
  );
  assert!(matches!(
    recheck(&unmapped).verdict,
    Verdict::CannotReplay(why) if why.contains("crate_skipped_macros")
  ));
  let unknown = older(
    "cccccccccccc",
    smallest,
    "layout: off\nformat: no_such_field: 3",
  );
  assert!(matches!(
    recheck(&unknown).verdict,
    Verdict::CannotReplay(_)
  ));
  let nothing_kept = older(
    "dddddddddddd",
    "No smaller part of the file was found that still fails.\n\n",
    "layout: off",
  );
  assert!(matches!(
    recheck(&nothing_kept).verdict,
    Verdict::CannotReplay(why) if why.contains("no smallest part")
  ));
  // Left as they are: a real recheck removes only the one it is sure of.
  crate::cli::report::recheck(&store, &[], false, &mut |_| {}).unwrap();
  let left: Vec<String> = store.list().into_iter().map(|e| e.id).collect();
  assert_eq!(left.len(), 3, "{:?}", left);
  assert!(!left.contains(&"aaaaaaaaaaaa".to_string()));
  assert!(!dir.join("bbbbbbbbbbbb").join("replay.toml").exists());
  let _ = fs::remove_dir_all(&dir);
}

/// Each setting, at each value it may take (a flag flipped, a number raised, every choice),
/// with layout on and with it off, is written as a replay file that reads back the same.
#[test]
fn test_every_setting_reads_back_from_a_replay_file() {
  use crate::options::{self, File, Kind};
  let mut failed = Vec::new();
  for setting in options::all().filter(|s| s.file != File::User) {
    let values: Vec<String> = match &setting.kind {
      Kind::Bool => vec!["true".into(), "false".into()],
      Kind::Integer => vec!["0".into(), "3".into(), "120".into()],
      Kind::Choice(choices) => choices.iter().map(|c| format!("{:?}", c.name)).collect(),
      Kind::List => vec!["[\"a\", \"b c\"]".into()],
      Kind::Sequence(choices) => vec![format!("[{:?}]", choices[0].name)],
      _ => continue,
    };
    for value in values {
      for layout in ["true", "false"] {
        let text = format!("layout = {}\n{} = {}\n", layout, setting.name, value);
        let Ok(config) = crate::config::replayed(&text) else {
          continue;
        };
        let Some(settings) = crate::Settings::of(&config) else {
          continue;
        };
        if settings_file(&settings).is_none() {
          failed.push(text.replace('\n', "; "));
        }
      }
    }
  }
  assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// `prune --under` removes only the reports seen in files under the folder, never one seen
/// on stdin; `remove` removes those named, once every id names one.
#[test]
fn test_reports_are_removed_by_folder_and_by_id() {
  let dir = scratch("remove");
  let store = Store::new(dir.clone());
  let root = std::path::absolute("/work").unwrap();
  let under = Bug {
    path: Some(root.join("deep").join("a.rs")),
    ..bug("one", "fn a() {}", "deep/a.rs")
  };
  let under = store.save(&under).unwrap();
  let elsewhere = Bug {
    path: Some(PathBuf::from("/elsewhere/b.rs")),
    ..bug("two", "fn a() {}", "b.rs")
  };
  let elsewhere = store.save(&elsewhere).unwrap();
  let stdin = Bug {
    path: None,
    ..bug("three", "fn a() {}", "<stdin>")
  };
  let stdin = store.save(&stdin).unwrap();
  let said = crate::cli::report::prune_under(&store, &root).unwrap();
  assert_eq!(
    said,
    format!("removed 1 report(s) seen only under {}\n", root.display())
  );
  assert!(store.find(&under).is_err());
  assert!(crate::cli::report::remove(&store, &[elsewhere.clone(), "zz".into()]).is_err());
  assert_eq!(store.list().len(), 2);
  let said = crate::cli::report::remove(&store, &[elsewhere[..6].to_string()]).unwrap();
  assert_eq!(said, format!("removed report {}\n", &elsewhere[..6]));
  assert_eq!(
    store.list().into_iter().map(|e| e.id).collect::<Vec<_>>(),
    [stdin]
  );
  let _ = fs::remove_dir_all(&dir);
}

/// Settings of every kind of field, laid out and formatted, written as a replay file read
/// back the same; a setting no key gives has none.
#[test]
fn test_settings_read_back_from_their_replay_file() {
  use crate::options::format::{FormatConfig, ImportsGranularity, NewlineStyle};
  use crate::options::layout::{ItemKind, LayoutConfig, NameSort, Sections, Visibility};
  let layout = LayoutConfig {
    keep_paragraphs: true,
    keep_helpers_near: true,
    sections: Sections::Comments,
    name_sort: NameSort::CaseInsensitive,
    import_sort: Some(2024),
    type_order: vec![ItemKind::FnDecl, ItemKind::UseDecl],
    visibility_order: vec![Visibility::Private, Visibility::Pub],
    trait_impl_order: vec!["Debug".into(), "a \"b\", c".into()],
    // What the formatting settings below give layout too.
    max_width: 79,
    style_edition: 2024,
    edition: 2021,
    ..LayoutConfig::default()
  };
  let mut format = FormatConfig {
    max_width: 79,
    hard_tabs: true,
    newline_style: NewlineStyle::Windows,
    brace_next_line: true,
    imports_granularity: ImportsGranularity::Crate,
    skip_macro_invocations: vec!["m".into()],
    style_edition: 2024,
    // `import_sort` reaches formatting too, which sorts imports by it.
    import_sort: Some(2024),
    ..FormatConfig::default()
  };
  format.set_heuristics(crate::options::format::SmallHeuristics::Max);
  let settings = crate::Settings {
    layout: Some(layout),
    format: Some(format),
    edition: crate::syntax::Edition::E2021,
  };
  let file = settings_file(&settings).expect("a replay file");
  for line in [
    "layout = true\n",
    "edition = \"2021\"\n",
    "keep_paragraphs = true\n",
    "sections = \"comments\"\n",
    "name_sort = \"case_insensitive\"\n",
    "type_order = [\"FnDecl\", \"UseDecl\"]\n",
    "brace_style = \"AlwaysNextLine\"\n",
    "fn_call_width = 79\n",
  ] {
    assert!(file.contains(line), "{}\n{}", line, file);
  }
  // Formatting alone, off by default: the edition and style edition are given all the same.
  let format_only = crate::Settings {
    layout: None,
    format: Some(FormatConfig::default()),
    edition: crate::syntax::Edition::E2024,
  };
  let file = settings_file(&format_only).expect("a replay file");
  assert!(
    file.ends_with("layout = false\nformat = true\nedition = \"2024\"\nstyle_edition = \"2015\"\n"),
    "{}",
    file
  );
  let skipping = crate::Settings {
    format: Some(FormatConfig {
      crate_skipped_macros: vec!["m".into()],
      ..FormatConfig::default()
    }),
    ..format_only
  };
  assert_eq!(settings_file(&skipping), None);
}
