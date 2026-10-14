mod common;

use std::process::Command;

use common::TestEnvironment;

/// The layout settings each crate is laid out with, from a clean checkout each time: the
/// defaults, then two mixes that between them set every layout setting away from its
/// default.
const LAYOUT_CONFIGS: &[(&str, &str)] = &[
  ("defaults", ""),
  (
    "mix one",
    "blank_lines_between_blocks = 0\ngroup_impls = false\nimpl_method_order = \"trait\"\n\
         import_sort = \"2024\"\nname_sort = \"bytes\"\nreorder_impl_items = true\n\
         test_module = \"in_place\"\ntrait_impl_order = [\"Default\", \"Clone\", \"Debug\"]\n\
         type_order = [\"FnDecl\", \"ImplBlock\", \"StructDecl\", \"EnumDecl\", \"TraitDecl\", \
         \"UseDecl\", \"ModDecl\"]\nvisibility_order = [\"Private\", \"PubSuper\", \"PubIn\", \
         \"PubCrate\", \"Pub\"]\n",
  ),
  (
    "mix two",
    "import_grouping = false\nimport_sort = \"2015\"\nname_sort = \"case_insensitive\"\n\
         sort_alphabetically = false\nsort_imports = false\ntrait_impl_sort = \"original\"\n",
  ),
];

/// The crates, smallest first. Each is checked out at the commit `compat/corpus.txt` pins
/// it to, so the result changes only when mto or a pin does.
const REAL_WORLD_TARGETS: &[&str] = &[
  // Small
  "lazy_static",
  "itoa",
  "termcolor",
  "logos",
  "once_cell",
  // Medium
  "bitflags",
  "bytes",
  "crossbeam",
  "uuid",
  "anyhow",
  // Large
  "clap",
  "serde",
  "tokio",
  "rayon",
  "rand",
];

/// What `mto doctor` finds to hold back in the run that broke the build, in one line.
fn culprit(env: &TestEnvironment, repo_dir: &std::path::Path) -> String {
  let output = env
    .mto_cmd()
    .args(["doctor", "last", "--", "cargo", "check", "--lib", "--quiet"])
    .current_dir(repo_dir)
    .output()
    .unwrap();
  let said = String::from_utf8_lossy(&output.stderr);
  let held: Vec<&str> = said
    .lines()
    .skip_while(|l| !l.starts_with("doctor: held back"))
    .skip(1)
    .take_while(|l| l.starts_with("  "))
    .map(str::trim)
    .collect();
  match output.status.code() {
    Some(0) if !held.is_empty() => format!("mto doctor holds back: {}", held.join("; ")),
    Some(5) => "mto doctor: the build fails with every edit held back".into(),
    code => format!(
      "mto doctor exited with {:?}: {}",
      code,
      said.lines().last().unwrap_or("")
    ),
  }
}

fn git(dir: &std::path::Path, args: &[&str]) -> bool {
  Command::new("git")
    .args(args)
    .current_dir(dir)
    .status()
    .is_ok_and(|s| s.success())
}

/// The URL and commit `compat/corpus.txt` pins `name` to.
fn pin(name: &str) -> (String, String) {
  let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("compat/corpus.txt");
  let pins = std::fs::read_to_string(&path).expect("compat/corpus.txt is readable");
  pins
    .lines()
    .map(|line| line.split_whitespace().collect::<Vec<_>>())
    .find(|words| words.len() >= 3 && words[0] == name)
    .map(|words| (words[1].to_string(), words[2].to_string()))
    .unwrap_or_else(|| panic!("compat/corpus.txt pins no commit for {}", name))
}

#[test]
#[ignore = "network: clones 15 crates and builds each four times; \
            run with `cargo xtask test --target domain_13_rw`"]
fn test_realworld_compilation_invariance() {
  let env = TestEnvironment::new();
  let mut failures = Vec::new();

  for name in REAL_WORLD_TARGETS {
    let (url, commit) = pin(name);
    println!(
      ">>> Evaluating Ecosystem Target: {} at {}",
      name,
      &commit[..12]
    );
    let repo_dir = env.dir.path().join(name);
    std::fs::create_dir_all(&repo_dir).unwrap();
    // Only the pinned commit, fetched by its hash.
    let fetched = git(&repo_dir, &["init", "--quiet"])
      && git(
        &repo_dir,
        &["fetch", "--quiet", "--depth", "1", &url, &commit],
      )
      && git(&repo_dir, &["checkout", "--quiet", "FETCH_HEAD"]);
    if !fetched {
      failures.push(format!("{}: cannot fetch {} at {}", name, url, commit));
      continue;
    }

    // Explicitly target --lib to prevent workspace feature unification traps
    let mut check_pre = Command::new("cargo");
    check_pre.args(["check", "--lib"]).current_dir(&repo_dir);
    if !check_pre.status().unwrap().success() {
      failures.push(format!(
        "{}: failed pre-format compilation (not mto's fault)",
        name
      ));
      continue;
    }

    // Each configuration from a clean checkout: the defaults, then two mixes that
    // between them turn every layout setting away from its default.
    for (label, settings) in LAYOUT_CONFIGS {
      assert!(
        git(&repo_dir, &["checkout", "--quiet", "--", "."]),
        "{}: git checkout failed",
        name
      );
      std::fs::write(
        repo_dir.join("mto.toml"),
        format!("root = true\nlayout = true\n{}", settings),
      )
      .unwrap();
      let format_output = env
        .mto_cmd()
        // Recorded in the test's own history, for `mto doctor` to narrow down a break.
        .args(["-q", "-R", "."])
        .current_dir(&repo_dir)
        .output()
        .unwrap();
      // Exit 3 marks files mto leaves alone for syntax errors (test fixtures); the
      // rest were written.
      if !matches!(format_output.status.code(), Some(0) | Some(3)) {
        failures.push(format!(
          "{} ({}): mto exited with {:?}: {}",
          name,
          label,
          format_output.status.code(),
          String::from_utf8_lossy(&format_output.stderr)
        ));
        continue;
      }
      let check_post = Command::new("cargo")
        .args(["check", "--lib", "--quiet"])
        .current_dir(&repo_dir)
        .status()
        .unwrap();
      if !check_post.success() {
        failures.push(format!(
          "{} ({}): failed post-format compilation; {}",
          name,
          label,
          culprit(&env, &repo_dir)
        ));
      }
    }
    println!(">>> Target {} verified: 100% compilation invariance.", name);
  }

  assert!(
    failures.is_empty(),
    "{} target(s) failed:\n{}",
    failures.len(),
    failures.join("\n")
  );
}
