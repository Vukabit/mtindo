mod common;

use std::fs;

use common::TestEnvironment;
use mtindo::config::{Environment, Overrides, Resolver};
use mtindo::{RunOptions, walk_directory};

fn resolver() -> Resolver {
  Resolver::new(&Overrides::default(), Environment::default()).unwrap()
}

#[test]
fn test_config_resolution_hierarchy() {
  let env = TestEnvironment::new();
  fs::remove_file(env.dir.path().join("mto.toml")).unwrap();

  let root_config = env.write_file(
    ".mto.toml",
    "sort_alphabetically = false\ngroup_impls = false",
  );

  let nested_dir = env.create_dir("nested");
  let nested_config = nested_dir.join("mto.toml");
  fs::write(&nested_config, "sort_alphabetically = true").unwrap();

  let deep_dir = env.create_dir("nested/deep");
  let mut resolver = resolver();
  let config = resolver.config_for(&deep_dir).unwrap();
  let files: Vec<_> = config
    .files()
    .iter()
    .map(|(_, path)| path.clone())
    .collect();
  let canonical = |p: &std::path::Path| fs::canonicalize(p).unwrap();
  assert_eq!(
    files,
    vec![canonical(&root_config), canonical(&nested_config)]
  );
  // The nearer file wins key by key; the rest is inherited.
  let layout = config.layout();
  assert!(layout.sort_alphabetically);
  assert!(!layout.group_impls);
}

#[test]
fn test_directory_exclusion_matrix() {
  let env = TestEnvironment::new();

  let target_dir = env.create_dir("target");
  let git_dir = env.create_dir(".git");
  let node_dir = env.create_dir("node_modules");
  let src_dir = env.create_dir("src");

  let target_file = target_dir.join("ignore_me.rs");
  let git_file = git_dir.join("ignore_me.rs");
  let node_file = node_dir.join("ignore_me.rs");
  let src_file = src_dir.join("format_me.rs");

  let hostile_content = "fn b() {}\nfn a() {}";

  fs::write(&target_file, hostile_content).unwrap();
  fs::write(&git_file, hostile_content).unwrap();
  fs::write(&node_file, hostile_content).unwrap();
  fs::write(&src_file, hostile_content).unwrap();

  walk_directory(env.dir.path(), &mut resolver(), &RunOptions::default()).unwrap();

  let target_content = fs::read_to_string(&target_file).unwrap();
  assert_eq!(target_content, hostile_content);

  let git_content = fs::read_to_string(&git_file).unwrap();
  assert_eq!(git_content, hostile_content);

  let node_content = fs::read_to_string(&node_file).unwrap();
  assert_eq!(node_content, hostile_content);

  let src_content = fs::read_to_string(&src_file).unwrap();
  assert_eq!(src_content, "fn a() {}\n\nfn b() {}\n");
}
