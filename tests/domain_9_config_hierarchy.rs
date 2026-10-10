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
        "max_width = 150\nignore_dirs = [\"custom_exclude\"]",
    )
    .unwrap();

    let member_dir = env.create_dir("crates/subcrate");
    let excluded_dir = env.create_dir("crates/subcrate/custom_exclude");

    let ignored_file = excluded_dir.join("lib.rs");
    let hostile_content = "fn b() {}\nfn a() {}";
    fs::write(&ignored_file, hostile_content).unwrap();

    let mut resolver = resolver();
    let config = resolver.config_for(&member_dir.join("src")).unwrap();
    let root_config = fs::canonicalize(env.dir.path().join("mto.toml")).unwrap();
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
