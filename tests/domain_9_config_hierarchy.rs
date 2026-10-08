mod common;

use std::fs;

use common::TestEnvironment;
use mtindo::{resolve_config_path, walk_directory, LayoutConfig};

#[test]
fn test_cargo_workspace_root_inheritance() {
    let env = TestEnvironment::new();

    fs::write(env.dir.path().join("mto.toml"), "max_width = 150\nignore_dirs = [\"custom_exclude\"]").unwrap();

    let member_dir = env.create_dir("crates/subcrate");
    let excluded_dir = env.create_dir("crates/subcrate/custom_exclude");

    let ignored_file = excluded_dir.join("lib.rs");
    let hostile_content = "fn b() {}\nfn a() {}";
    fs::write(&ignored_file, hostile_content).unwrap();

    let resolved = resolve_config_path(None, Some(&member_dir.join("src/lib.rs")));
    assert_eq!(resolved, Some(env.dir.path().join("mto.toml")));

    let mut config = LayoutConfig::default();
    config.ignore_dirs = vec!["custom_exclude".to_string()];

    walk_directory(env.dir.path(), &config, false, false, false);

    let content = fs::read_to_string(&ignored_file).unwrap();
    assert_eq!(content, hostile_content);
}

#[test]
fn test_deep_hierarchical_config_inheritance() {
    let env = TestEnvironment::new();

    fs::write(env.dir.path().join("mto.toml"), "sort_alphabetically = false\nmax_width = 80").unwrap();

    let level1 = env.create_dir("level1");
    fs::write(level1.join("mto.toml"), "sort_alphabetically = true").unwrap();

    let level2 = env.create_dir("level1/level2");
    let _ = level2;

    let level3 = env.create_dir("level1/level2/level3");
    let target_file = level3.join("main.rs");
    fs::write(&target_file, "fn b() {}\nfn a() {}").unwrap();

    let resolved_config_path = resolve_config_path(None, Some(&target_file));
    assert_eq!(resolved_config_path, Some(level1.join("mto.toml")));
}

#[test]
fn test_malformed_toml_handling() {
    let env = TestEnvironment::new();
    env.write_file("mto.toml", "sort_alphabetically = [invalid_syntax_missing_quotes");

    let target_file = env.write_file("main.rs", "fn b() {}\nfn a() {}");

    let mut cmd = env.mto_cmd();
    cmd.arg(&target_file);

    let output = cmd.output().unwrap();
    assert!(!output.status.success());
}
