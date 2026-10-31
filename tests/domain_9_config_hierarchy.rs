mod common;

use std::fs;

use common::TestEnvironment;
use mtindo::config::{Environment, FileKind, Overrides, Resolver};
use mtindo::{RunOptions, walk_directory};

fn resolver() -> Resolver {
  Resolver::new(&Overrides::default(), Environment::default()).unwrap()
}

#[test]
fn test_cargo_workspace_root_inheritance() {
  let env = TestEnvironment::new();

  fs::write(
    env.dir.path().join("mto.toml"),
    "layout = true\nmax_width = 150\nignore_dirs = [\"custom_exclude\"]",
  )
  .unwrap();

  let member_dir = env.create_dir("crates/subcrate");
  let excluded_dir = env.create_dir("crates/subcrate/custom_exclude");

  let ignored_file = excluded_dir.join("lib.rs");
  let hostile_content = "fn b() {}\nfn a() {}";
  fs::write(&ignored_file, hostile_content).unwrap();

  let mut resolver = resolver();
  let config = resolver.config_for(&member_dir.join("src")).unwrap();
  let root_config = mtindo::real_path(env.dir.path().join("mto.toml")).unwrap();
  assert_eq!(config.files(), &[(FileKind::Mto, root_config)]);
  assert_eq!(config.layout().max_width, 150);

  walk_directory(env.dir.path(), &mut resolver, &RunOptions::default()).unwrap();

  let content = fs::read_to_string(&ignored_file).unwrap();
  assert_eq!(content, hostile_content);
}

#[test]
fn test_deep_hierarchical_config_inheritance() {
  let env = TestEnvironment::new();

  fs::write(
    env.dir.path().join("mto.toml"),
    "sort_alphabetically = false\nmax_width = 80",
  )
  .unwrap();

  let level1 = env.create_dir("level1");
  fs::write(level1.join("mto.toml"), "sort_alphabetically = true").unwrap();

  let level3 = env.create_dir("level1/level2/level3");
  let target_file = level3.join("main.rs");
  fs::write(&target_file, "fn b() {}\nfn a() {}").unwrap();

  let layout = resolver().config_for(&level3).unwrap().layout();
  assert!(layout.sort_alphabetically, "the nearest file wins");
  assert_eq!(layout.max_width, 80, "unset keys are inherited");

  // `root = true` stops the inheritance.
  fs::write(
    level1.join("mto.toml"),
    "root = true\nsort_alphabetically = true",
  )
  .unwrap();
  let layout = resolver().config_for(&level3).unwrap().layout();
  assert_eq!(layout.max_width, 100);
}

/// An `ignore_dirs` entry with a `/` is a path from the folder of the file that sets it:
/// only that folder is skipped, not every folder of its name; a bare name still is.
#[test]
fn test_ignore_dirs_paths_skip_only_that_folder() {
  let env = TestEnvironment::new();
  fs::write(
    env.dir.path().join("mto.toml"),
    "layout = true\nignore_dirs = [\"tests/fixtures\", \"./cache\", \"vendored\"]",
  )
  .unwrap();
  let messy = "fn b() {}\nfn a() {}\n";
  let files = [
    ("tests/fixtures", false),
    ("src/fixtures", true),
    ("cache", false),
    ("src/cache", true),
    ("src/vendored", false),
  ];
  for (dir, _) in files {
    fs::write(env.create_dir(dir).join("lib.rs"), messy).unwrap();
  }
  let mut resolver = resolver();
  walk_directory(env.dir.path(), &mut resolver, &RunOptions::default()).unwrap();
  for (dir, laid_out) in files {
    let content = fs::read_to_string(env.dir.path().join(dir).join("lib.rs")).unwrap();
    assert_eq!(content != messy, laid_out, "{}: {}", dir, content);
  }
}

#[test]
fn test_malformed_toml_handling() {
  let env = TestEnvironment::new();
  env.write_file(
    "mto.toml",
    "sort_alphabetically = [invalid_syntax_missing_quotes",
  );

  let target_file = env.write_file("main.rs", "fn b() {}\nfn a() {}");

  let mut cmd = env.mto_cmd();
  cmd.arg(&target_file);

  let output = cmd.output().unwrap();
  assert!(!output.status.success());
}

/// A standard setting mto does not honor yet, set to anything but its default, is said to
/// be ignored; one at its default, and one mto honors, say nothing.
#[test]
fn test_settings_mto_does_not_honor_are_reported() {
  let env = TestEnvironment::new();
  fs::write(
    env.dir.path().join("rustfmt.toml"),
    "binop_separator = \"Back\"\nwrap_comments = false\nmax_width = 90\n",
  )
  .unwrap();
  fs::write(env.dir.path().join("a.rs"), "fn f() {}\n").unwrap();
  let output = env.mto_cmd().args(["--check", "a.rs"]).output().unwrap();
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("`binop_separator` is not honored by mto yet"),
    "{}",
    stderr
  );
  assert!(
    !stderr.contains("wrap_comments") && !stderr.contains("max_width"),
    "{}",
    stderr
  );
}
