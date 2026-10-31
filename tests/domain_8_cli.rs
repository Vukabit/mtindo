mod common;

use common::TestEnvironment;

const SORTED: &str = "fn a() {}\n\nfn b() {}\n";

/// Debug builds of mto add a stray comment to their output when `MTINDO_INJECT_FAULT` is
/// set, so the verifier must stop the write.
const SOURCE: &str = "fn b() {}\n\nfn a() {}\n";

const UNSORTED: &str = "fn b() {}\n\nfn a() {}\n";

/// Not a test of its own: the build check of [`check_words`], idle without `doctor-check`
/// in the folder it runs in.
#[test]
fn check_helper() {
  let Ok(mode) = std::fs::read_to_string("doctor-check") else {
    return;
  };
  let lib = std::fs::read_to_string("src/lib.rs").unwrap_or_default();
  let line = |what: &str| lib.lines().position(|l| l.contains(what));
  let fails = match mode.trim() {
    "order" => line("mod generated") < line("struct A"),
    _ => true,
  };
  if fails {
    if mode.trim() == "order" {
      eprintln!("error[E0433]: failed to resolve: generated");
    } else {
      eprintln!("error[E0425]: cannot find value `x` in this scope");
    }
    std::process::exit(1);
  }
}

/// [`check_words`] as a TOML array, for a config file.
fn check_toml() -> String {
  let words: Vec<String> = check_words().iter().map(|w| format!("'{}'", w)).collect();
  format!("[{}]", words.join(", "))
}

/// The doctor tests' build check, the same on every system: this test binary, run as
/// [`check_helper`] in the project's folder. The `doctor-check` file there says how it
/// fails, as a compiler would: `order` when `mod generated` stands above `struct A` in
/// src/lib.rs, `always` whatever the files hold.
fn check_words() -> Vec<String> {
  let exe = std::env::current_exe().unwrap();
  [
    exe.display().to_string(),
    "--exact".into(),
    "check_helper".into(),
    "--quiet".into(),
    // What it says reaches the doctor, not the test harness.
    "--nocapture".into(),
  ]
  .into()
}

fn help_output(
  env: &TestEnvironment,
  args: &[&str],
  vars: &[(&str, &str)],
) -> (Option<i32>, String) {
  let mut cmd = env.mto_cmd();
  cmd
    .args(args)
    .env_remove("NO_COLOR")
    .env_remove("FORCE_COLOR");
  for (key, value) in vars {
    cmd.env(key, value);
  }
  let output = cmd.output().unwrap();
  (output.status.code(), printed(&output.stdout))
}

fn mto(env: &TestEnvironment, args: &[&str]) -> (i32, String, String) {
  let output = env.mto_cmd().args(args).output().unwrap();
  (
    output.status.code().unwrap_or(-1),
    printed(&output.stdout).to_string(),
    printed(&output.stderr).to_string(),
  )
}

/// Runs mto in `env` with `args`; returns (exit code, stdout, stderr).
/// What mto printed, with Windows' path separators as `/`, which the assertions use: mto
/// prints paths as the system writes them.
fn printed(bytes: &[u8]) -> String {
  let text = String::from_utf8_lossy(bytes).into_owned();
  if cfg!(windows) {
    text.replace('\\', "/")
  } else {
    text
  }
}

/// A check lays files out while the walk is still finding them; a folder's broken
/// configuration found late in the walk still stops it before any file's report.
#[test]
fn test_a_broken_configuration_found_late_stops_a_check_before_any_report() {
  let env = TestEnvironment::new();
  env.write_file("a.rs", UNSORTED);
  env.write_file("z/sub/mto.toml", "layout = [\n");
  env.write_file("z/sub/b.rs", UNSORTED);
  let output = env.mto_cmd().args(["--check", "."]).output().unwrap();
  assert_eq!(output.status.code(), Some(2));
  assert!(output.stdout.is_empty(), "{}", printed(&output.stdout));
  let stderr = printed(&output.stderr);
  assert!(stderr.contains("mto.toml:2:1"), "{}", stderr);
  assert!(!stderr.contains("a.rs"), "{}", stderr);
}

#[test]
fn test_a_bug_failing_on_one_file_leaves_it_and_the_run_goes_on() {
  let env = TestEnvironment::new();
  let broken = "fn inject_panic() {}\n\nfn a() {}\n";
  env.write_file("a.rs", UNSORTED);
  env.write_file("b.rs", broken);
  env.write_file("c.rs", UNSORTED);
  let output = env
    .mto_cmd()
    .args(["-l", "."])
    .env("MTINDO_INJECT_FAULT", "panic")
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(4));
  let stderr = printed(&output.stderr);
  // Kept as a report, named in one line, with no standard panic message besides.
  assert!(
    stderr.contains("mto hit a bug on ./b.rs; saved as report "),
    "{}",
    stderr
  );
  assert!(!stderr.contains("panicked at"), "{}", stderr);
  let id = stderr
    .split("saved as report ")
    .nth(1)
    .and_then(|rest| rest.split(':').next())
    .unwrap()
    .to_string();
  let (code, listed, _) = mto(&env, &["report"]);
  assert_eq!(code, 0);
  assert!(
    listed.contains(&id) && listed.contains("(./b.rs)"),
    "{}",
    listed
  );
  // The report: what failed, the smallest part that still fails, and the build.
  let (code, shown, _) = mto(&env, &["report", "show", &id]);
  assert_eq!(code, 0);
  assert!(
    shown.contains("## panic: injected by MTINDO_INJECT_FAULT"),
    "{}",
    shown
  );
  assert!(
    shown.contains("still fails</summary>\n\n```rust\nfn inject_panic() {}\n```"),
    "{}",
    shown
  );
  assert!(shown.contains("seen 1 time(s)"), "{}", shown);
  let (code, saved, _) = mto(&env, &["report", "save", &id, "bug.md", "--with-file"]);
  assert_eq!(code, 0, "{}", saved);
  let saved = printed(env.read_file("bug.md").as_bytes());
  assert!(saved.contains("The whole file, `./b.rs`:"), "{}", saved);
  assert_eq!(env.read_file("b.rs"), broken);
  assert_eq!(env.read_file("a.rs"), SORTED);
  assert_eq!(env.read_file("c.rs"), SORTED);
  let listed = printed(&output.stdout);
  assert!(
    listed.contains("a.rs") && listed.contains("c.rs"),
    "{}",
    listed
  );
  let output = env
    .mto_cmd()
    .arg("-")
    .env("MTINDO_INJECT_FAULT", "panic")
    .write_stdin(broken)
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(4));
  assert_eq!(printed(&output.stdout), broken);
  // The same bug again, from stdin: counted in the same report.
  let (_, shown, _) = mto(&env, &["report", "show", &id]);
  assert!(shown.contains("seen 2 time(s)"), "{}", shown);
  let (code, pruned, _) = mto(&env, &["report", "prune"]);
  assert_eq!(code, 0);
  assert_eq!(pruned, "removed 1 report(s)\n");
}

#[test]
fn test_a_file_nested_too_deeply_is_left_as_written() {
  let env = TestEnvironment::new();
  // Deeper than mto follows (`mtindo::syntax::MAX_DEPTH`): it said so, rather than running
  // out of stack.
  let deep = format!(
    "fn f() {{\n{}{}}}\n",
    "{\n".repeat(5_000),
    "}\n".repeat(5_000)
  );
  let path = env.write_file("deep.rs", &deep);
  env
    .mto_cmd()
    .arg("deep.rs")
    .assert()
    .success()
    .stderr(predicates::str::contains(
      "nests more than 1024 levels deep",
    ));
  assert_eq!(std::fs::read_to_string(&path).unwrap(), deep);
  let output = env
    .mto_cmd()
    .arg("-")
    .write_stdin(deep.clone())
    .output()
    .unwrap();
  assert!(output.status.success());
  assert_eq!(printed(&output.stdout), deep);
}

/// More targets than a Windows command line holds reach mto all the same: the list goes in
/// a file, removed after.
#[test]
fn test_cargo_mto_gives_mto_a_long_list_of_targets() {
  let env = TestEnvironment::new();
  env.write_file(
    "Cargo.toml",
    "[package]\nname = \"demo\"\nedition = \"2021\"\n",
  );
  env.write_file("src/lib.rs", "fn z() {}\nfn y() {}\n");
  let name = "an_example_with_a_name_long_enough_to_fill_a_command_line_quickly";
  for i in 0..300 {
    env.write_file(
      &format!("examples/{}_{}.rs", name, i),
      "fn main() {}\nfn b() {}\nfn a() {}\n",
    );
  }
  let listed_before = || {
    std::fs::read_dir(std::env::temp_dir())
      .unwrap()
      .filter(|e| {
        e.as_ref()
          .unwrap()
          .file_name()
          .to_string_lossy()
          .starts_with("mtindo-paths-")
      })
      .count()
  };
  let before = listed_before();
  let output = assert_cmd::Command::cargo_bin("cargo-mto")
    .unwrap()
    .current_dir(env.dir.path())
    .env("HOME", env.dir.path().join(".home"))
    .env("USERPROFILE", env.dir.path().join(".home"))
    .env("XDG_CONFIG_HOME", env.dir.path().join(".home/.config"))
    .env("XDG_STATE_HOME", env.dir.path().join(".home/.local/state"))
    .env("CI", "1")
    .args(["mto", "--check", "--message-format", "short"])
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(1), "{}", printed(&output.stderr));
  let stdout = printed(&output.stdout);
  assert_eq!(stdout.lines().count(), 301, "{}", stdout);
  assert!(stdout.contains(&format!("{}_299.rs", name)), "{}", stdout);
  assert!(listed_before() <= before);
}

#[test]
fn test_cargo_mto_lays_out_every_target_like_cargo_fmt() {
  let env = TestEnvironment::new();
  env.write_file(
    "Cargo.toml",
    "[package]\nname = \"demo\"\nedition = \"2021\"\n",
  );
  env.write_file("src/lib.rs", "mod a;\nfn z() {}\nfn y() {}\n");
  env.write_file("src/a.rs", "fn d() {}\nfn c() {}\n");
  env.write_file("src/main.rs", "fn main() {}\nfn helper() {}\n");
  env.write_file("tests/it.rs", "fn q() {}\nfn p() {}\n");
  env.write_file("notes/stray.rs", "fn n() {}\nfn m() {}\n");

  let mut cmd = assert_cmd::Command::cargo_bin("cargo-mto").unwrap();
  let output = cmd
    .current_dir(env.dir.path())
    .env("HOME", env.dir.path().join(".home"))
    .env("USERPROFILE", env.dir.path().join(".home"))
    .env("XDG_CONFIG_HOME", env.dir.path().join(".home/.config"))
    .env("XDG_STATE_HOME", env.dir.path().join(".home/.local/state"))
    .env("CI", "1")
    .args(["mto", "--check", "--message-format", "short"])
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(1));
  let stdout = printed(&output.stdout);
  let mut changed: Vec<&str> = stdout
    .lines()
    .map(|l| {
      l.rsplit_once(env.dir.path().file_name().unwrap().to_str().unwrap())
        .unwrap()
        .1
    })
    .collect();
  changed.sort();
  // Every target and its modules; not files no target reaches.
  assert_eq!(
    changed,
    ["/src/a.rs", "/src/lib.rs", "/src/main.rs", "/tests/it.rs"]
  );
  // A write is recorded under the command as typed, not mto's file list.
  let output = assert_cmd::Command::cargo_bin("cargo-mto")
    .unwrap()
    .current_dir(env.dir.path())
    .env("HOME", env.dir.path().join(".home"))
    .env("USERPROFILE", env.dir.path().join(".home"))
    .env("XDG_CONFIG_HOME", env.dir.path().join(".home/.config"))
    .env("XDG_STATE_HOME", env.dir.path().join(".home/.local/state"))
    .env("CI", "1")
    .args(["mto", "-q"])
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(0));
  let history = env.mto_cmd().arg("history").output().unwrap();
  let history = printed(&history.stdout);
  assert!(
    history.contains("4 files") && history.contains("  cargo mto -q\n"),
    "{}",
    history
  );
}

#[test]
fn test_cli_a_closed_pipe_ends_quietly() {
  use std::io::Read;
  use std::process::{Command, Stdio};
  let env = TestEnvironment::new();
  let source: String = (0..20_000).map(|i| format!("fn f{}() {{}}\n", i)).collect();
  let file = env.write_file("big.rs", &source);
  // Enough output to fill the pipe, read a little, then hang up, as `| head` does.
  let mut child = Command::new(assert_cmd::cargo::cargo_bin("mto"))
    .args(["debug", "tokens"])
    .arg(&file)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
  let mut first = [0u8; 16];
  child.stdout.take().unwrap().read_exact(&mut first).unwrap();
  let output = child.wait_with_output().unwrap();
  let stderr = printed(&output.stderr);
  assert_eq!(output.status.code(), Some(141), "{}", stderr);
  assert!(!stderr.contains("panicked"), "{}", stderr);
}

/// A file that cannot be read is an error (exit 3), named or reached through `mod`, and
/// checking does not pass over it.
#[cfg(unix)]
#[test]
fn test_cli_a_file_that_cannot_be_read_fails_the_run() {
  use std::os::unix::fs::PermissionsExt;
  let env = TestEnvironment::new();
  env.write_file("main.rs", "mod child;\nfn main() {}\n");
  env.write_file("child.rs", "fn g() {}\n");
  let child = env.dir.path().join("child.rs");
  std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o000)).unwrap();
  // Running as root reads it anyway: nothing to test.
  if std::fs::read_to_string(&child).is_ok() {
    return;
  }
  for args in [
    vec!["child.rs"],
    vec!["--check", "child.rs"],
    vec!["--check", "main.rs"],
  ] {
    let output = env.mto_cmd().args(&args).output().unwrap();
    assert_eq!(output.status.code(), Some(3), "{:?}: {:?}", args, output);
    assert!(printed(&output.stderr).contains("child.rs"), "{:?}", output);
  }
  std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn test_cli_a_file_with_syntax_errors_exits_three() {
  let env = TestEnvironment::new();
  let broken = "fn b() {}\nfn a() {\n    if true {\n";
  env.write_file("main.rs", broken);

  let output = env.mto_cmd().arg("main.rs").output().unwrap();
  assert_eq!(output.status.code(), Some(3));
  let stderr = printed(&output.stderr);
  assert!(
    stderr.contains("Error in main.rs: the file has syntax errors"),
    "{}",
    stderr
  );
  assert_eq!(env.read_file("main.rs"), broken);
}

#[test]
fn test_cli_bare_invocation_ignores_piped_stdin() {
  let env = TestEnvironment::new();
  let output = env
    .mto_cmd()
    .write_stdin("fn b() {}\nfn a() {}")
    .output()
    .unwrap();
  assert!(output.status.success());
  let stdout = printed(&output.stdout);
  assert!(stdout.starts_with("usage:"), "{}", stdout);
}

#[test]
fn test_cli_bare_invocation_prints_the_same_help_as_dash_h() {
  let env = TestEnvironment::new();
  let (bare_code, bare) = help_output(&env, &[], &[]);
  let (short_code, short) = help_output(&env, &["-h"], &[]);
  let (long_code, long) = help_output(&env, &["--help"], &[]);
  assert_eq!(
    (bare_code, short_code, long_code),
    (Some(0), Some(0), Some(0))
  );
  assert_eq!(bare, short);
  assert_eq!(short, long);
  assert!(
    bare.starts_with("usage: mto [OPTIONS] <PATH>...\n"),
    "{}",
    bare
  );
  assert!(
    !bare.contains('\x1b'),
    "help piped to a file must not be colored"
  );
}

/// The file that tripped the old text scanner (`{ 1 }` inside generics) now lays out fine.
#[test]
fn test_cli_braces_inside_generics_are_laid_out() {
  let env = TestEnvironment::new();
  env.write_file(
    "main.rs",
    "fn b() {}\n\nstruct A<const N: i32 = { 1 }>;\n\nfn a() {}\n",
  );
  let output = env.mto_cmd().arg("main.rs").output().unwrap();
  assert_eq!(output.status.code(), Some(0));
  assert_eq!(
    env.read_file("main.rs"),
    "struct A<const N: i32 = { 1 }>;\n\nfn a() {}\n\nfn b() {}\n"
  );
}

#[test]
fn test_cli_check_drift_detected() {
  let env = TestEnvironment::new();
  env.write_file("main.rs", "fn b() {}\nfn a() {}");

  let mut cmd = env.mto_cmd();
  cmd.arg("--check").arg("main.rs");

  cmd.assert().failure().code(1);
}

#[test]
fn test_cli_check_no_drift() {
  let env = TestEnvironment::new();
  env.write_file("main.rs", "fn a() {}\n\nfn b() {}\n");

  let mut cmd = env.mto_cmd();
  cmd.arg("--check").arg("main.rs");

  cmd.assert().success();
}

#[test]
fn test_cli_check_prints_the_standard_diff() {
  let env = TestEnvironment::new();
  env.write_file("main.rs", "fn b() {}\nfn a() {}\n");
  let output = env.mto_cmd().args(["--check", "main.rs"]).output().unwrap();
  let stdout = printed(&output.stdout);
  assert_eq!(output.status.code(), Some(1));
  assert!(stdout.starts_with("Diff in main.rs:1:\n"), "{}", stdout);
  assert!(
    stdout.contains("\n+\n") || stdout.contains("\n+fn"),
    "{}",
    stdout
  );
}

#[test]
fn test_cli_check_reports_every_drifted_file() {
  let env = TestEnvironment::new();
  env.write_file("src/one.rs", "fn b() {}\nfn a() {}");
  env.write_file("src/two.rs", "fn d() {}\nfn c() {}");
  env.write_file("src/three.rs", "fn e() {}\n");

  let output = env.mto_cmd().arg("--check").arg("src").output().unwrap();
  assert_eq!(output.status.code(), Some(1));

  let stdout = printed(&output.stdout);
  assert!(stdout.contains("Diff in src/one.rs"), "{}", stdout);
  assert!(stdout.contains("Diff in src/two.rs"), "{}", stdout);
  assert!(!stdout.contains("three.rs"), "{}", stdout);
  // A minimal diff shows one of the two functions moving.
  let moved = |name: &str| {
    stdout.contains(&format!("-fn {}() {{}}", name))
      && stdout.contains(&format!("+fn {}() {{}}", name))
  };
  assert!(moved("a") || moved("b"), "{}", stdout);

  let stderr = printed(&output.stderr);
  assert!(
    stderr.contains("2 file(s) would be reformatted"),
    "{}",
    stderr
  );
}

/// A configuration file named in another case (`Mto.toml`) counts when the file system says
/// it is the file asked for, as the standard formatter finds its own: a walk and a file
/// named alone agree, whichever the file system decides.
#[test]
fn test_cli_config_names_in_another_case_count_as_the_file_system_says() {
  let env = TestEnvironment::new();
  env.write_file("sub/Mto.toml", "layout = false\n");
  env.write_file("sub/a.rs", UNSORTED);
  let walked = mto(&env, &["--check", "."]);
  let named = mto(&env, &["--check", "sub/a.rs"]);
  assert_eq!(walked.0, named.0, "{}\n---\n{}", walked.2, named.2);
  // Layout, on by default, would put `a` first; the file turns it off.
  let found = env.dir.path().join("sub/mto.toml").is_file();
  assert_eq!(walked.0, if found { 0 } else { 1 }, "{}", walked.2);
}

#[test]
fn test_cli_config_options_lists_every_setting() {
  let env = TestEnvironment::new();
  let (code, stdout, _) = mto(&env, &["config", "options"]);
  assert_eq!(code, 0);
  assert!(stdout.starts_with("mto.toml ("), "{}", stdout);
  assert!(stdout.contains("\nrustfmt.toml ("), "{}", stdout);
  let (code, stdout, _) = mto(&env, &["config", "options", "--toml"]);
  assert_eq!(code, 0);
  assert!(stdout.contains("# format = true\n"), "{}", stdout);
  assert_eq!(mto(&env, &["config", "options", "extra"]).0, 2);
  assert_eq!(mto(&env, &["--toml", "."]).0, 2);
}

#[test]
fn test_cli_config_path_and_print() {
  let env = TestEnvironment::new();
  env.write_file("rustfmt.toml", "max_width = 80\n");
  env.write_file("mto.toml", "group_impls = false\n");
  env.write_file(
    "Cargo.toml",
    "[package]\nname = \"x\"\nedition = \"2024\"\n",
  );
  env.create_dir("src");

  let output = env
    .mto_cmd()
    .args(["config", "path", "src"])
    .output()
    .unwrap();
  let stdout = printed(&output.stdout);
  assert_eq!(output.status.code(), Some(0));
  let names: Vec<&str> = stdout
    .lines()
    .map(|line| line.split(": ").next().unwrap())
    .collect();
  assert_eq!(
    names,
    [
      "user config",
      "formatting config",
      "mto config",
      "Cargo.toml (edition)",
      "history"
    ],
    "{}",
    stdout
  );

  let output = env
    .mto_cmd()
    .args(["config", "print", "--minimal", "--config", "hard_tabs=true"])
    .output()
    .unwrap();
  let stdout = printed(&output.stdout);
  let lines: Vec<&str> = stdout.lines().collect();
  assert_eq!(lines[0], "# style edition 2024");
  // Layout first, then formatting, each sorted by name.
  assert_eq!(lines[2], "# layout (mto.toml)", "{}", stdout);
  assert!(
    lines[3].starts_with("group_impls = false  # "),
    "{}",
    stdout
  );
  assert_eq!(
    lines[5], "# formatting (mto.toml or rustfmt.toml)",
    "{}",
    stdout
  );
  assert!(lines[6].starts_with("edition = \"2024\"  # "), "{}", stdout);
  assert!(
    lines.contains(&"hard_tabs = true  # command line"),
    "{}",
    stdout
  );
  assert!(
    lines.iter().any(|l| l.starts_with("max_width = 80  # ")),
    "{}",
    stdout
  );
  assert!(
    lines
      .iter()
      .any(|l| l.starts_with("group_impls = false  # ")),
    "{}",
    stdout
  );
  assert_eq!(lines.len(), 9, "{}", stdout);

  let output = env
    .mto_cmd()
    .args(["config", "print", "--default", "--style-edition", "2024"])
    .output()
    .unwrap();
  let stdout = printed(&output.stdout);
  // The keys that pick the defaults show what is in effect, beside the defaults they pick.
  assert!(
    stdout.starts_with("# defaults for style edition 2024\n"),
    "{}",
    stdout
  );
  assert!(
    stdout.contains("\nversion = \"Two\"  # in effect\n"),
    "{}",
    stdout
  );
  assert!(stdout.contains("\nmax_width = 100\n"), "{}", stdout);

  // --edition beats Cargo.toml; --current shows every setting with its source.
  let output = env
    .mto_cmd()
    .args(["config", "print", "--current", "--edition", "2021"])
    .output()
    .unwrap();
  let stdout = printed(&output.stdout);
  assert!(stdout.starts_with("# style edition 2021\n"), "{}", stdout);
  assert!(
    stdout.contains("\nedition = \"2021\"  # command line\n"),
    "{}",
    stdout
  );
  assert!(
    stdout.contains("\nhard_tabs = false  # default\n"),
    "{}",
    stdout
  );

  let output = env
    .mto_cmd()
    .args(["config", "print", "--user"])
    .output()
    .unwrap();
  let stdout = printed(&output.stdout);
  assert!(
    stdout.starts_with("[doctor]\ncommand = [\"cargo\", \"check\""),
    "{}",
    stdout
  );
  assert!(
    stdout.contains("\ntimeout = 600  # default\n\n[history]\nenabled = true  # default\n"),
    "{}",
    stdout
  );
  assert!(
    stdout.contains("\n\n[safety]\nallow_dirty = false  # default\n"),
    "{}",
    stdout
  );
}

#[test]
fn test_cli_config_path_uses_the_given_file() {
  let env = TestEnvironment::new();
  env.write_file("main.rs", "fn b() {}\nfn a() {}\n");
  env.write_file(
    "custom.toml",
    "layout = true\nsort_alphabetically = false\n",
  );

  let output = env
    .mto_cmd()
    .args(["--config-path", "custom.toml", "--check", "main.rs"])
    .output()
    .unwrap();
  let stdout = printed(&output.stdout);
  assert_eq!(output.status.code(), Some(1));
  // Unsorted, so `b` stays first and only a blank line is added.
  assert!(stdout.contains(" fn b() {}\n+\n fn a() {}"), "{}", stdout);
}

#[test]
fn test_cli_config_sets_values_and_refuses_a_file() {
  let env = TestEnvironment::new();
  env.write_file("main.rs", "fn b() {}\nfn a() {}\n");

  let output = env
    .mto_cmd()
    .args([
      "--config",
      "sort_alphabetically=false,type_order=[\"FnDecl\",\"UseDecl\"]",
      "--check",
      "main.rs",
    ])
    .output()
    .unwrap();
  let stdout = printed(&output.stdout);
  assert_eq!(output.status.code(), Some(1), "{}", stdout);
  assert!(stdout.contains(" fn b() {}\n+\n fn a() {}"), "{}", stdout);

  // The standard formatter's meaning: `--config FILE` is a mistake for `--config-path
  // FILE`.
  let output = env
    .mto_cmd()
    .args(["--config", "custom.toml", "main.rs"])
    .output()
    .unwrap();
  let stderr = printed(&output.stderr);
  assert_eq!(output.status.code(), Some(2));
  assert!(
    stderr.contains("did you mean `--config-path custom.toml`?"),
    "{}",
    stderr
  );

  let output = env
    .mto_cmd()
    .args(["--config", "newline_style=dos", "main.rs"])
    .output()
    .unwrap();
  let stderr = printed(&output.stderr);
  assert_eq!(output.status.code(), Some(2));
  assert!(
    stderr.contains("--config: `newline_style` cannot be `dos`"),
    "{}",
    stderr
  );
}

#[test]
fn test_cli_crate_skip_attributes_reach_its_modules() {
  let env = TestEnvironment::new();
  env.write_file("mto.toml", "format = true\n");
  env.write_file(
    "src/lib.rs",
    "#![rustfmt::skip::macros(kept)]\n#![rustfmt::skip::attributes(odd)]\n\nmod a;\n",
  );
  let child = "#[odd(a,   b)]\nfn f() {\n    kept!(x   ,  y);\n    other!(x   ,  y);\n}\n";
  env.write_file("src/a.rs", child);

  let output = env.mto_cmd().arg("src/lib.rs").output().unwrap();
  assert_eq!(output.status.code(), Some(0), "{:?}", output);
  assert_eq!(
    env.read_file("src/a.rs"),
    "#[odd(a,   b)]\nfn f() {\n    kept!(x   ,  y);\n    other!(x, y);\n}\n"
  );
}

#[test]
fn test_cli_dbg_prints_the_split() {
  let env = TestEnvironment::new();
  env.write_file("main.rs", "fn b() {}\nfn a() {}\n");

  let output = env
    .mto_cmd()
    .args(["--dbg", "--check", "main.rs"])
    .output()
    .unwrap();
  let all = printed(&output.stdout).to_string() + &printed(&output.stderr);
  assert!(all.contains("Extracted 2 blocks"), "{}", all);
  assert!(
    all.contains("[Sorted #0] Kind: FnDecl, Identifier: 'a'"),
    "{}",
    all
  );
}

/// `mto debug layout`: each place left as written, where and why, with formatting on
/// though the folder has it off; nothing written.
#[test]
fn test_cli_debug_layout_says_why_code_is_left_as_written() {
  let env = TestEnvironment::new();
  let source = "fn f() {\n    #[rustfmt::skip]\n    let  a  =  1;\n    let b   = 2;\n}\n";
  env.write_file("main.rs", source);
  let out = env
    .mto_cmd()
    .args(["debug", "layout", "main.rs"])
    .output()
    .unwrap();
  assert!(out.status.success());
  let shown = printed(&out.stdout);
  assert!(
    shown.contains("main.rs:2:5  LET_STMT  #[rustfmt::skip]\n    #[rustfmt::skip] let a = 1;"),
    "{}",
    shown
  );
  assert!(!shown.contains("let b"), "{}", shown);
  assert_eq!(env.read_file("main.rs"), source);

  env.write_file("plain.rs", "fn g() {}\n");
  let out = env
    .mto_cmd()
    .args(["debug", "layout", "plain.rs"])
    .output()
    .unwrap();
  assert!(out.status.success());
  assert_eq!(printed(&out.stdout), "plain.rs: nothing left as written\n");
}

#[test]
fn test_cli_debug_tree_and_tokens() {
  let env = TestEnvironment::new();
  let source = "// about a\nfn a() {}\n";
  env.write_file("main.rs", source);

  let tree = env
    .mto_cmd()
    .args(["debug", "tree", "main.rs", "--edition", "2021"])
    .output()
    .unwrap();
  assert!(tree.status.success());
  let dump = printed(&tree.stdout);
  assert!(dump.starts_with("SOURCE_FILE@0..21\n"), "{}", dump);
  assert!(
    dump.contains("LINE_COMMENT@0..10 \"// about a\""),
    "{}",
    dump
  );

  let tokens = env
    .mto_cmd()
    .args(["debug", "tokens", "main.rs"])
    .output()
    .unwrap();
  assert!(tokens.status.success());
  assert!(
    String::from_utf8(tokens.stdout)
      .unwrap()
      .contains("IDENT@11..13 \"fn\"")
  );

  env
    .mto_cmd()
    .args(["debug", "tree"])
    .assert()
    .failure()
    .code(2);
  env
    .mto_cmd()
    .args(["debug", "tree", "missing.rs"])
    .assert()
    .failure()
    .code(3);
}

/// The build guard: off, mto runs nothing; on, it checks before and after a run, holds
/// back the one move that breaks a pretend build (a script that reads the order of
/// `src/lib.rs`), writes the rest and remembers the hold; `--unguarded` skips the checks.
#[test]
fn test_cli_doctor_holds_back_what_breaks_the_build() {
  const LIB: &str = "struct A;\nfn b() {}\nmod generated {}\n";
  let env = TestEnvironment::new();
  env.write_file(
    ".home/.config/mtindo/config.toml",
    "[history]\nkeep_runs = 50\n",
  );
  env.write_file("src/lib.rs", LIB);
  env.write_file("src/other.rs", "fn z() {}\nstruct Y;\n");
  env.write_file("doctor-check", "order");
  let (code, stdout, _) = mto(&env, &["doctor"]);
  assert_eq!(code, 0);
  assert!(stdout.starts_with("build guard: off\n"), "{}", stdout);
  let (code, _, stderr) = mto(&env, &["doctor", "last"]);
  assert_eq!(code, 2);
  assert!(stderr.contains("give it after `--`"), "{}", stderr);
  let words = check_words();
  let mut on = vec!["doctor", "on", "--"];
  on.extend(words.iter().map(String::as_str));
  let (code, stdout, stderr) = mto(&env, &on);
  assert_eq!(code, 0, "{}", stderr);
  assert!(
    stdout.contains("check_helper --quiet --nocapture\n"),
    "{}",
    stdout
  );
  assert!(
    env
      .read_file(".home/.config/mtindo/config.toml")
      .contains("[doctor]\nenabled = true\ncommand = [")
  );
  let (code, _, stderr) = mto(&env, &["."]);
  assert_eq!(code, 0, "{}", stderr);
  assert!(
    stderr.contains("doctor: before ✓  after ✗ error[E0433]"),
    "{}",
    stderr
  );
  assert!(
    stderr.contains("doctor: held back 1 edit:\n  ./src/lib.rs: moved: mod generated (line 3, now before struct A)\n"),
    "{}",
    stderr
  );
  assert!(stderr.contains("report: "), "{}", stderr);
  assert_eq!(
    env.read_file("src/lib.rs"),
    "struct A;\n\nfn b() {}\n\nmod generated {}\n"
  );
  assert_eq!(env.read_file("src/other.rs"), "struct Y;\n\nfn z() {}\n");
  let (_, stdout, _) = mto(&env, &["doctor", "holds"]);
  assert!(
    stdout.contains(" 1. ") && stdout.contains("moved: mod generated"),
    "{}",
    stdout
  );
  // The hold is remembered: the next run holds the move back before checking.
  env.write_file("src/lib.rs", LIB);
  let (code, _, stderr) = mto(&env, &["."]);
  assert_eq!(code, 0, "{}", stderr);
  assert!(stderr.contains("doctor: before ✓  after ✓"), "{}", stderr);
  // Unguarded, no check runs and the hold does not apply: every edit is written.
  env.write_file("src/lib.rs", LIB);
  let (code, _, stderr) = mto(&env, &[".", "--unguarded"]);
  assert_eq!(code, 0, "{}", stderr);
  assert!(!stderr.contains("doctor:"), "{}", stderr);
  assert!(env.read_file("src/lib.rs").starts_with("mod generated"));
  // Narrowing down that run holds the move back again, which is all it takes.
  let (code, _, stderr) = mto(&env, &["doctor", "last"]);
  assert_eq!(code, 0, "{}", stderr);
  assert!(
    stderr.contains(
      "doctor: 1 file, 0 edits: 1 check\ndoctor: the edits it already holds back were enough"
    ),
    "{}",
    stderr
  );
  assert!(env.read_file("src/lib.rs").starts_with("struct A;"));
  // With the guard off no command runs, and the hold still applies.
  let (code, stdout, _) = mto(&env, &["doctor", "off"]);
  assert_eq!(code, 0);
  assert!(stdout.contains("mto runs no command"), "{}", stdout);
  env.write_file("src/lib.rs", LIB);
  let (code, _, stderr) = mto(&env, &["."]);
  assert_eq!(code, 0, "{}", stderr);
  assert!(!stderr.contains("doctor:"), "{}", stderr);
  assert!(env.read_file("src/lib.rs").starts_with("struct A;"));
  // Released, the move is made again.
  let (code, stdout, _) = mto(&env, &["doctor", "release", "all"]);
  assert_eq!(code, 0);
  assert!(stdout.contains("Released 1 hold"), "{}", stdout);
  env.write_file("src/lib.rs", LIB);
  assert_eq!(mto(&env, &["."]).0, 0);
  assert!(env.read_file("src/lib.rs").starts_with("mod generated"));
}

/// Out of checks (`doctor.max_checks`), every changed file is held back whole: the files are
/// as before the run, which the check before passed, and the run ends without an error.
#[test]
fn test_cli_doctor_out_of_checks_holds_back_whole_files() {
  const LIB: &str = "struct A;\nfn b() {}\nmod generated {}\n";
  let env = TestEnvironment::new();
  env.write_file("src/lib.rs", LIB);
  env.write_file("src/other.rs", "fn z() {}\nstruct Y;\n");
  env.write_file("doctor-check", "order");
  env.write_file(
    ".home/.config/mtindo/config.toml",
    &format!(
      "[doctor]\nenabled = true\ncommand = {}\nmax_checks = 1\n",
      check_toml()
    ),
  );
  let (code, _, stderr) = mto(&env, &["."]);
  assert_eq!(code, 0, "{}", stderr);
  assert!(
    stderr.contains(
      "doctor: out of checks (doctor.max_checks = 1); every file is as it was, and nothing \
       is written"
    ),
    "{}",
    stderr
  );
  assert_eq!(env.read_file("src/lib.rs"), LIB);
  assert_eq!(env.read_file("src/other.rs"), "fn z() {}\nstruct Y;\n");
}

/// A build guard whose command already fails before the run has no baseline: it cannot tell
/// what the run would break, so nothing is written and mto says why (exit code 5);
/// `--unguarded` writes as usual.
#[test]
fn test_cli_doctor_writes_nothing_without_a_baseline() {
  let env = TestEnvironment::new();
  env.write_file("src/lib.rs", UNSORTED);
  env.write_file("doctor-check", "always");
  let words = check_words();
  let mut on = vec!["doctor", "on", "--"];
  on.extend(words.iter().map(String::as_str));
  let (code, _, stderr) = mto(&env, &on);
  assert_eq!(code, 0, "{}", stderr);
  let (code, _, stderr) = mto(&env, &["."]);
  assert_eq!(code, 5, "{}", stderr);
  assert!(
    stderr.contains("already fails before mto changes anything (error[E0425]"),
    "{}",
    stderr
  );
  assert!(stderr.contains("nothing was written"), "{}", stderr);
  assert_eq!(env.read_file("src/lib.rs"), UNSORTED);
  let (code, _, stderr) = mto(&env, &["--unguarded", "."]);
  assert_eq!(code, 0, "{}", stderr);
  assert_eq!(env.read_file("src/lib.rs"), SORTED);
}

#[test]
fn test_cli_dry_run_contract() {
  let env = TestEnvironment::new();
  let initial_content = "fn b() {}\nfn a() {}";
  env.write_file("main.rs", initial_content);

  let mut cmd = env.mto_cmd();
  cmd.arg("--dry-run").arg("main.rs");

  cmd.assert().success();

  let untouched_content = env.read_file("main.rs");
  assert_eq!(untouched_content, initial_content);
}

#[test]
fn test_cli_emit_stdout_and_json() {
  let env = TestEnvironment::new();
  let source = "fn b() {}\nfn a() {}\n";
  env.write_file("main.rs", source);

  let output = env
    .mto_cmd()
    .args(["--emit", "stdout", "main.rs"])
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(0));
  assert_eq!(
    printed(&output.stdout),
    "main.rs:\n\nfn a() {}\n\nfn b() {}\n"
  );
  assert_eq!(
    env.read_file("main.rs"),
    source,
    "--emit stdout writes nothing"
  );

  let output = env
    .mto_cmd()
    .args(["--emit=json", "main.rs"])
    .output()
    .unwrap();
  let stdout = printed(&output.stdout);
  assert!(
    stdout.starts_with("[{\"name\":\"main.rs\",\"mismatches\":[{\"original_begin_line\":"),
    "{}",
    stdout
  );
  assert!(stdout.ends_with("}]}]\n"), "{}", stdout);
  assert_eq!(env.read_file("main.rs"), source);
}

#[test]
fn test_cli_files_bring_their_modules() {
  let env = TestEnvironment::new();
  env.write_file("src/lib.rs", "mod b;\nmod a;\n");
  env.write_file("src/a.rs", "fn z() {}\nfn y() {}\n");
  env.write_file("src/b.rs", "fn d() {}\nfn c() {}\n");
  env.write_file("other.rs", "fn q() {}\nfn p() {}\n");

  // Several paths; lib.rs brings a.rs and b.rs; -l names the files that would change.
  let output = env
    .mto_cmd()
    .args(["--check", "-l", "src/lib.rs", "other.rs"])
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(1));
  let stdout = printed(&output.stdout);
  assert_eq!(stdout, "src/lib.rs\nsrc/b.rs\nsrc/a.rs\nother.rs\n");

  let output = env
    .mto_cmd()
    .args(["--check", "-l", "--skip-children", "src/lib.rs"])
    .output()
    .unwrap();
  assert_eq!(printed(&output.stdout), "src/lib.rs\n");
}

#[test]
fn test_cli_format_works_with_and_without_layout() {
  let env = TestEnvironment::new();
  let messy = "fn b() {\n        let x = 1;   \n\n\n\n  x;\n}\n\n\n\nfn a() {}\n";
  let formatted = "fn b() {\n    let x = 1;\n\n    x;\n}\n\nfn a() {}\n";

  // Formatting alone: indentation and blank lines change, the order does not.
  env.write_file("mto.toml", "format = true\n");
  env.write_file("main.rs", messy);
  let output = env.mto_cmd().arg("main.rs").output().unwrap();
  assert_eq!(output.status.code(), Some(0), "{:?}", output);
  assert_eq!(
    std::fs::read_to_string(env.dir.path().join("main.rs")).unwrap(),
    formatted
  );
  let output = env.mto_cmd().arg("-").write_stdin(messy).output().unwrap();
  assert_eq!(printed(&output.stdout), formatted);

  // With layout too: reordered, then formatted.
  env.write_file("mto.toml", "layout = true\nformat = true\n");
  env.write_file("main.rs", messy);
  let output = env.mto_cmd().arg("main.rs").output().unwrap();
  assert_eq!(output.status.code(), Some(0), "{:?}", output);
  assert_eq!(
    std::fs::read_to_string(env.dir.path().join("main.rs")).unwrap(),
    "fn a() {}\n\nfn b() {\n    let x = 1;\n\n    x;\n}\n"
  );

  // A body written on one line is laid out once formatting has put it on lines, in the
  // same run: a second run changes nothing.
  env.write_file("mto.toml", "layout = true\nimpl_method_order = \"trait\"\n");
  env.write_file(
    "main.rs",
    "trait T { fn a(&self); fn b(&self); }\nstruct S;\nimpl T for S { fn b(&self) {} fn a(&self) {} }\n",
  );
  let output = env.mto_cmd().arg("main.rs").output().unwrap();
  assert_eq!(output.status.code(), Some(0), "{:?}", output);
  let once = std::fs::read_to_string(env.dir.path().join("main.rs")).unwrap();
  assert!(
    once.contains("    fn a(&self) {}\n    fn b(&self) {}\n"),
    "{}",
    once
  );
  let output = env.mto_cmd().arg("main.rs").output().unwrap();
  assert_eq!(output.status.code(), Some(0), "{:?}", output);
  assert_eq!(
    std::fs::read_to_string(env.dir.path().join("main.rs")).unwrap(),
    once
  );

  // A file with syntax errors is reported, not formatted.
  env.write_file("mto.toml", "format = true\n");
  env.write_file("main.rs", "fn b( {\n");
  let output = env.mto_cmd().arg("main.rs").output().unwrap();
  assert_eq!(output.status.code(), Some(3), "{:?}", output);
  assert_eq!(
    std::fs::read_to_string(env.dir.path().join("main.rs")).unwrap(),
    "fn b( {\n"
  );
}

#[test]
fn test_cli_formats_by_default_and_lays_out_once_opted_in() {
  let env = TestEnvironment::new();
  std::fs::remove_file(env.dir.path().join("mto.toml")).unwrap();
  let messy = "fn b( ) {}\n\nfn a() {  }\n";
  env.write_file("main.rs", messy);
  env.write_file("rustfmt.toml", "max_width = 80\n");

  // No mto.toml: formatted as the standard formatter would, its items left in place.
  let output = env.mto_cmd().arg("main.rs").output().unwrap();
  let stderr = printed(&output.stderr);
  assert_eq!(output.status.code(), Some(0), "{}", stderr);
  assert!(!stderr.contains("left alone"), "{}", stderr);
  assert_eq!(
    std::fs::read_to_string(env.dir.path().join("main.rs")).unwrap(),
    SOURCE
  );
  let output = env.mto_cmd().arg("-").write_stdin(messy).output().unwrap();
  assert_eq!(printed(&output.stdout), SOURCE);

  // `format = false` and no layout: nothing changes, and the hint says why.
  env.write_file("main.rs", messy);
  env.write_file("mto.toml", "format = false\n");
  let output = env.mto_cmd().arg("main.rs").output().unwrap();
  let stderr = printed(&output.stderr);
  assert_eq!(output.status.code(), Some(0));
  assert!(
    stderr.contains("1 file(s) left alone: layout and formatting are off (`format = false`)"),
    "{}",
    stderr
  );
  assert_eq!(
    std::fs::read_to_string(env.dir.path().join("main.rs")).unwrap(),
    messy
  );

  // An mto.toml without `layout = true` moves no items, and says why a layout setting
  // does nothing.
  env.write_file("main.rs", SOURCE);
  env.write_file("mto.toml", "sort_alphabetically = false\n");
  let output = env.mto_cmd().args(["--check", "main.rs"]).output().unwrap();
  let stderr = printed(&output.stderr);
  assert_eq!(output.status.code(), Some(0));
  assert!(
    stderr.contains("`sort_alphabetically` has no effect while layout is off; add `layout = true`"),
    "{}",
    stderr
  );

  // `mto init` turns layout on.
  std::fs::remove_file(env.dir.path().join("mto.toml")).unwrap();
  env.mto_cmd().arg("init").assert().success();
  let init = std::fs::read_to_string(env.dir.path().join("mto.toml")).unwrap();
  assert!(init.contains("\nlayout = true\n"), "{}", init);
  env.mto_cmd().args(["--check", "main.rs"]).assert().code(1);
}

/// A generated file is formatted as the standard formatter would (`format_generated_files`)
/// but not laid out, unless `layout_generated_files`; `generated_markers` says what marks
/// one.
#[test]
fn test_cli_generated_files_are_not_laid_out() {
  let env = TestEnvironment::new();
  let source = "// @generated by a tool\nfn  z() {}\nfn a() {}\n";
  let run = |mto_toml: &str, style_toml: &str, file: &str| {
    env.write_file("mto.toml", mto_toml);
    env.write_file("rustfmt.toml", style_toml);
    env.write_file("gen.rs", file);
    let output = env.mto_cmd().arg("gen.rs").output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{:?}", output);
    std::fs::read_to_string(env.dir.path().join("gen.rs")).unwrap()
  };
  assert_eq!(
    run("layout = true\n", "", source),
    "// @generated by a tool\nfn z() {}\nfn a() {}\n"
  );
  assert_eq!(
    run(
      "layout = true\n",
      "format_generated_files = false\n",
      source
    ),
    source
  );
  assert_eq!(
    run("layout = true\nlayout_generated_files = true\n", "", source),
    "fn a() {}\n\n// @generated by a tool\nfn z() {}\n"
  );
  let marked = "// DO NOT EDIT\nfn  z() {}\nfn a() {}\n";
  assert_eq!(
    run(
      "layout = true\ngenerated_markers = [\"DO NOT EDIT\"]\n",
      "",
      marked
    ),
    "// DO NOT EDIT\nfn z() {}\nfn a() {}\n"
  );
}

#[test]
fn test_cli_help_color_follows_no_color_and_force_color() {
  let env = TestEnvironment::new();
  let (_, forced) = help_output(&env, &["-h"], &[("FORCE_COLOR", "1")]);
  assert!(
    forced.starts_with("\x1b[1;34musage:\x1b[0m \x1b[1;35mmto\x1b[0m"),
    "{:?}",
    &forced[..40]
  );
  let (_, both) = help_output(&env, &["-h"], &[("FORCE_COLOR", "1"), ("NO_COLOR", "1")]);
  assert!(!both.contains('\x1b'), "NO_COLOR must win over FORCE_COLOR");
  let (_, plain) = help_output(&env, &["-h"], &[]);
  let stripped = forced
    .split('\x1b')
    .enumerate()
    .map(|(idx, part)| {
      if idx == 0 {
        part
      } else {
        part.split_once('m').map(|(_, rest)| rest).unwrap_or(part)
      }
    })
    .collect::<String>();
  assert_eq!(stripped, plain, "colors must be the only difference");
}

#[test]
fn test_cli_history_lists_this_project_unless_all() {
  let env = TestEnvironment::new();
  for project in ["alpha", "beta", "gamma"] {
    env.write_file(&format!("{}/Cargo.toml", project), "[package]\n");
  }
  env.write_file("alpha/main.rs", UNSORTED);
  env.write_file("beta/main.rs", UNSORTED);
  assert_eq!(mto(&env, &["alpha/main.rs"]).0, 0);
  assert_eq!(mto(&env, &["beta/main.rs"]).0, 0);
  let history = |dir: &str, args: &[&str]| {
    let output = env
      .mto_cmd()
      .current_dir(env.dir.path().join(dir))
      .arg("history")
      .args(args)
      .output()
      .unwrap();
    assert_eq!(output.status.code(), Some(0), "{:?}", output);
    printed(&output.stdout)
  };
  // In a project: its runs only, with the project named.
  let alpha = history("alpha", &[]);
  assert!(alpha.contains("PROJECT"), "{}", alpha);
  assert!(
    alpha.contains("alpha") && !alpha.contains("beta"),
    "{}",
    alpha
  );
  // --all: every project's.
  let all = history("alpha", &["--all"]);
  assert!(all.contains("alpha") && all.contains("beta"), "{}", all);
  assert!(!all.contains("No runs wrote files"), "{}", all);
  // A project with no runs: every run, saying so.
  let gamma = history("gamma", &[]);
  assert!(gamma.contains("No runs wrote files in "), "{}", gamma);
  assert!(
    gamma.contains("alpha") && gamma.contains("beta"),
    "{}",
    gamma
  );
  // --all goes with `mto history` only.
  assert_eq!(mto(&env, &["--all", "."]).0, 2);

  // `last` counts this project's runs: in alpha, beta's later run is not last.
  let in_dir = |dir: &str, args: &[&str]| {
    env
      .mto_cmd()
      .current_dir(env.dir.path().join(dir))
      .args(args)
      .output()
      .unwrap()
  };
  let shown = in_dir("alpha", &["history", "show", "last"]);
  let shown = printed(&shown.stdout);
  assert!(shown.contains("alpha/main.rs"), "{}", shown);
  let output = in_dir("alpha", &["history", "show", "last~1"]);
  assert_eq!(output.status.code(), Some(2), "{:?}", output);
  assert!(
    printed(&output.stderr).contains("goes back too far"),
    "{:?}",
    output
  );
  let output = in_dir("alpha", &["restore", "last"]);
  assert_eq!(output.status.code(), Some(0), "{:?}", output);
  assert_eq!(env.read_file("alpha/main.rs"), UNSORTED);
  assert_eq!(env.read_file("beta/main.rs"), SORTED);
  // A project with no runs refuses `last`; a handle works anywhere.
  let output = in_dir("gamma", &["restore", "last"]);
  assert_eq!(output.status.code(), Some(2), "{:?}", output);
  assert!(
    printed(&output.stderr).contains("no run wrote files in"),
    "{:?}",
    output
  );
  let handle = all
    .lines()
    .find(|line| line.contains("beta"))
    .and_then(|line| line.split_whitespace().next())
    .unwrap()
    .to_string();
  let output = in_dir("gamma", &["restore", &handle]);
  assert_eq!(output.status.code(), Some(0), "{:?}", output);
  assert_eq!(env.read_file("beta/main.rs"), UNSORTED);
}

#[test]
fn test_cli_history_off_and_the_git_guard() {
  let env = TestEnvironment::new();
  env.git_init("repo");
  env.write_file("repo/mto.toml", "layout = true\n");
  env.write_file("repo/clean.rs", UNSORTED);
  env.write_file("repo/dirty.rs", UNSORTED);
  env.git("repo", &["add", "."]);
  env.git("repo", &["commit", "-q", "-m", "x"]);
  env.write_file("repo/dirty.rs", &format!("{}// edited\n", UNSORTED));
  // With history on, the guard stays out of the way.
  let (code, _, stderr) = mto(&env, &["--dry-run", "repo"]);
  assert_eq!(code, 0, "{}", stderr);
  // With history off, the uncommitted file is left alone; the run is not recorded.
  let (code, _, stderr) = mto(&env, &["--no-history", "repo"]);
  assert_eq!(code, 3, "{}", stderr);
  assert!(stderr.contains("uncommitted changes"), "{}", stderr);
  assert!(!stderr.contains("Recorded"), "{}", stderr);
  assert_eq!(env.read_file("repo/clean.rs"), SORTED);
  assert!(env.read_file("repo/dirty.rs").starts_with(UNSORTED));
  let (_, stdout, _) = mto(&env, &["history"]);
  assert!(stdout.contains("No runs"), "{}", stdout);
  // --allow-dirty, or the user config, lets it through.
  let (code, _, stderr) = mto(&env, &["--no-history", "--allow-dirty", "repo"]);
  assert_eq!(code, 0, "{}", stderr);
  assert!(env.read_file("repo/dirty.rs").starts_with(SORTED));
}

#[test]
fn test_cli_history_prune_and_removed_flags() {
  let env = TestEnvironment::new();
  for i in 0..3 {
    env.write_file("main.rs", &format!("fn b{}() {{}}\n\nfn a() {{}}\n", i));
    assert_eq!(mto(&env, &["main.rs"]).0, 0);
  }
  // A build guard report for each run, written before pruning's grace period: those of
  // the runs pruned go with them.
  let state = env.dir.path().join(".home/.local/state/mtindo");
  let reports = state.join("doctor/reports");
  std::fs::create_dir_all(&reports).unwrap();
  let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
  for run in std::fs::read_dir(state.join("history/runs")).unwrap() {
    let name = run
      .unwrap()
      .file_name()
      .to_string_lossy()
      .replace(".run", ".md");
    std::fs::write(reports.join(&name), "# mto doctor\n").unwrap();
    let file = std::fs::OpenOptions::new()
      .write(true)
      .open(reports.join(&name))
      .unwrap();
    file.set_modified(old).unwrap();
  }
  let (code, stdout, _) = mto(&env, &["history", "prune", "--keep", "1"]);
  assert_eq!(code, 0);
  assert!(stdout.contains("Removed 2 run(s)"), "{}", stdout);
  assert!(stdout.contains("and 2 build guard report(s)"), "{}", stdout);
  assert_eq!(std::fs::read_dir(&reports).unwrap().count(), 1);
  let (_, stdout, _) = mto(&env, &["history"]);
  assert_eq!(stdout.lines().count(), 2, "{}", stdout);
  assert_eq!(
    mto(&env, &["history", "prune", "--older-than", "soon"]).0,
    2
  );
  assert_eq!(mto(&env, &["--force", "main.rs"]).0, 2);
}

#[test]
fn test_cli_init_success_and_failure() {
  let env = TestEnvironment::new();
  std::fs::remove_file(env.dir.path().join("mto.toml")).unwrap();

  let mut cmd1 = env.mto_cmd();
  cmd1.arg("init");
  cmd1.assert().success();

  let config_exists = env.dir.path().join("mto.toml").exists();
  assert!(config_exists);

  let mut cmd2 = env.mto_cmd();
  cmd2.arg("init");
  cmd2.assert().failure().code(2);
}

#[test]
fn test_cli_invalid_config_exits_two() {
  let env = TestEnvironment::new();
  env.write_file("mto.toml", "max_width = wide");
  env.write_file("main.rs", "fn a() {}\n");

  env.mto_cmd().arg("main.rs").assert().failure().code(2);
}

#[test]
fn test_cli_man_prints_the_manual() {
  let env = TestEnvironment::new();
  let (code, out) = help_output(&env, &["--man"], &[]);
  assert_eq!(code, Some(0));
  assert!(out.starts_with("# mtindo (mto)\n"), "{}", out);
}

#[test]
fn test_cli_options_without_a_path_are_an_error() {
  let env = TestEnvironment::new();
  let output = env
    .mto_cmd()
    .arg("--check")
    .write_stdin("fn a() {}")
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(2));
  assert!(
    String::from_utf8(output.stderr)
      .unwrap()
      .contains("no path given")
  );
}

#[test]
fn test_cli_print_config_like_the_standard_formatter() {
  let env = TestEnvironment::new();
  env.write_file("rustfmt.toml", "max_width = 80\n");
  let output = env
    .mto_cmd()
    .args(["--print-config", "minimal"])
    .output()
    .unwrap();
  assert_eq!(printed(&output.stdout), "max_width = 80\n");
  let output = env
    .mto_cmd()
    .args(["--print-config", "default"])
    .output()
    .unwrap();
  let stdout = printed(&output.stdout);
  assert!(stdout.contains("\nmax_width = 100\n"), "{}", stdout);
  assert!(
    !stdout.contains("type_order"),
    "only the standard formatter's settings: {}",
    stdout
  );
  env
    .mto_cmd()
    .args(["--print-config", "default", "out.toml"])
    .assert()
    .success();
  assert!(env.read_file("out.toml").contains("hard_tabs = false"));
}

#[test]
fn test_cli_quiet_verbose_color_and_unstable_features() {
  let env = TestEnvironment::new();
  env.write_file("main.rs", "fn b() {}\nfn a() {}\n");

  let output = env
    .mto_cmd()
    .args(["--check", "-v", "--unstable-features", "main.rs"])
    .output()
    .unwrap();
  let stdout = printed(&output.stdout);
  assert!(
    stdout.starts_with("Formatting main.rs\nDiff in main.rs:1:\n"),
    "{}",
    stdout
  );
  assert!(stdout.contains("{}⏎\n"), "-v marks line ends: {}", stdout);

  let output = env
    .mto_cmd()
    .args(["--check", "--color", "always", "main.rs"])
    .output()
    .unwrap();
  assert!(printed(&output.stdout).contains('\x1b'));

  let output = env.mto_cmd().args(["-q", "main.rs"]).output().unwrap();
  assert_eq!(output.status.code(), Some(0));
  assert!(output.stdout.is_empty() && output.stderr.is_empty());
  assert_eq!(env.read_file("main.rs"), "fn a() {}\n\nfn b() {}\n");
}

#[test]
fn test_cli_restore_skips_files_edited_since_unless_forced() {
  let env = TestEnvironment::new();
  env.write_file("a.rs", UNSORTED);
  env.write_file("b.rs", UNSORTED);
  assert_eq!(mto(&env, &["a.rs", "b.rs"]).0, 0);
  env.write_file("a.rs", "fn edited() {}\n");
  std::fs::remove_file(env.dir.path().join("b.rs")).unwrap();
  let (code, _, stderr) = mto(&env, &["restore", "last"]);
  assert_eq!(code, 1, "{}", stderr);
  assert!(stderr.contains("changed since run"), "{}", stderr);
  assert!(stderr.contains("no longer exists"), "{}", stderr);
  assert_eq!(env.read_file("a.rs"), "fn edited() {}\n");
  // Only the files named, and every file named must be the run's.
  assert_eq!(mto(&env, &["restore", "last", "c.rs"]).0, 2);
  let (code, _, stderr) = mto(&env, &["restore", "last", "a.rs", "--force"]);
  assert_eq!(code, 0, "{}", stderr);
  assert_eq!(env.read_file("a.rs"), UNSORTED);
  assert!(!env.dir.path().join("b.rs").exists());
  // The forced restore recorded the edit, so it can be had back.
  assert_eq!(mto(&env, &["restore", "last"]).0, 0);
  assert_eq!(env.read_file("a.rs"), "fn edited() {}\n");
}

#[test]
fn test_cli_restore_undoes_the_last_run() {
  let env = TestEnvironment::new();
  env.write_file("main.rs", UNSORTED);
  let (code, _, stderr) = mto(&env, &["main.rs"]);
  assert_eq!(code, 0, "{}", stderr);
  assert!(stderr.contains("Recorded as run "), "{}", stderr);
  assert_eq!(env.read_file("main.rs"), SORTED);
  let (code, stdout, _) = mto(&env, &["history"]);
  assert_eq!(code, 0);
  assert!(
    stdout.contains("1 file") && stdout.contains("mto main.rs"),
    "{}",
    stdout
  );
  let (code, stdout, stderr) = mto(&env, &["restore", "last"]);
  assert_eq!(code, 0, "{}", stderr);
  assert!(stdout.contains("Restored"), "{}", stdout);
  assert_eq!(env.read_file("main.rs"), UNSORTED);
  // The restore is a run too: undoing it lays the file out again.
  let (code, stdout, _) = mto(&env, &["history"]);
  assert_eq!(code, 0);
  assert!(stdout.contains("(restores "), "{}", stdout);
  assert_eq!(mto(&env, &["restore", "last"]).0, 0);
  assert_eq!(env.read_file("main.rs"), SORTED);
  // last~2 is the first run again; the file already holds what that run wrote.
  let (code, stdout, _) = mto(&env, &["history", "show", "last~2"]);
  assert_eq!(code, 0);
  assert!(stdout.contains("as written"), "{}", stdout);
}

#[test]
fn test_cli_skip_children_setting_leaves_modules_out() {
  let env = TestEnvironment::new();
  env.write_file("rustfmt.toml", "skip_children = true\n");
  env.write_file("mto.toml", "layout = true\n");
  env.write_file("src/lib.rs", "mod b;\nmod a;\n");
  env.write_file("src/a.rs", "fn z() {}\nfn y() {}\n");
  env.write_file("src/b.rs", "fn d() {}\nfn c() {}\n");

  let output = env
    .mto_cmd()
    .args(["--check", "-l", "src/lib.rs"])
    .output()
    .unwrap();
  assert_eq!(printed(&output.stdout), "src/lib.rs\n");
}

#[test]
fn test_cli_stdin_filepath() {
  let env = TestEnvironment::new();
  env.write_file(
    "sub/mto.toml",
    "layout = true\nsort_alphabetically = false\n",
  );
  let output = env
    .mto_cmd()
    .args(["-", "--stdin-filepath", "sub/x.rs", "--check"])
    .write_stdin("fn b() {}\nfn a() {}\n")
    .output()
    .unwrap();
  // sub/mto.toml keeps the order, so only a blank line is added.
  let stdout = printed(&output.stdout);
  assert!(
    stdout.starts_with("Diff in sub/x.rs:1:\n fn b() {}\n+\n fn a() {}"),
    "{}",
    stdout
  );
}

#[test]
fn test_cli_stdin_pipeline() {
  let env = TestEnvironment::new();

  let mut cmd = env.mto_cmd();
  cmd.arg("-").write_stdin("fn b() {}\nfn a() {}");

  let output = cmd.output().unwrap();

  assert!(output.status.success());

  let stdout = printed(&output.stdout);
  assert_eq!(stdout, "fn a() {}\n\nfn b() {}\n");
}

#[test]
fn test_cli_unchanged_file_is_not_rewritten() {
  let env = TestEnvironment::new();
  env.write_file("main.rs", "fn a() {}\n\nfn b() {}\n");

  let output = env.mto_cmd().arg("main.rs").output().unwrap();
  assert!(output.status.success());
  assert!(!printed(&output.stdout).contains("Formatted:"));
}

#[test]
fn test_cli_unknown_argument_exits_two() {
  let env = TestEnvironment::new();
  env
    .mto_cmd()
    .arg("--no-such-flag")
    .assert()
    .failure()
    .code(2);
}

#[test]
fn test_cli_user_config_turns_history_off() {
  let env = TestEnvironment::new();
  env.write_file(
    ".home/.config/mtindo/config.toml",
    "[history]\nenabled = false\n[safety]\nallow_dirty = true\n",
  );
  env.write_file("main.rs", UNSORTED);
  let (code, _, stderr) = mto(&env, &["main.rs"]);
  assert_eq!(code, 0, "{}", stderr);
  assert!(!stderr.contains("Recorded"), "{}", stderr);
  let (code, _, stderr) = mto(&env, &["restore", "last"]);
  assert_eq!(code, 2);
  assert!(stderr.contains("history is empty"), "{}", stderr);
  env.write_file(
    ".home/.config/mtindo/config.toml",
    "[history]\nmax_size = \"lots\"\n",
  );
  let (code, _, stderr) = mto(&env, &["main.rs"]);
  assert_eq!(code, 2);
  assert!(stderr.contains("max_size"), "{}", stderr);
}

#[test]
fn test_cli_verifier_on_stdin_passes_the_input_through() {
  let env = TestEnvironment::new();
  let output = env
    .mto_cmd()
    .arg("-")
    .env("MTINDO_INJECT_FAULT", "1")
    .write_stdin(SOURCE)
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(4));
  assert_eq!(printed(&output.stdout), SOURCE);
}

#[test]
fn test_cli_verifier_stops_an_unsafe_write() {
  let env = TestEnvironment::new();
  // Reports not kept: the whole report is printed.
  env.write_file(
    ".home/.config/mtindo/config.toml",
    "[report]\nsave = false\n",
  );
  env.write_file("main.rs", SOURCE);
  let output = env
    .mto_cmd()
    .arg("main.rs")
    .env("MTINDO_INJECT_FAULT", "1")
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(4));
  assert_eq!(env.read_file("main.rs"), SOURCE);
  let stderr = printed(&output.stderr);
  assert!(
    stderr.contains("the verifier stopped the write"),
    "{}",
    stderr
  );
  assert!(stderr.contains("comment-added"), "{}", stderr);
  assert!(
    stderr.contains("The smallest part of the file that still shows it"),
    "{}",
    stderr
  );
}

#[test]
fn test_cli_version() {
  let env = TestEnvironment::new();
  for flag in ["-V", "--version"] {
    let (code, out) = help_output(&env, &[flag], &[]);
    assert_eq!(code, Some(0));
    // The version, and the build it is (the commit, or a hash of the source).
    let version = format!("mto {} (", env!("CARGO_PKG_VERSION"));
    assert!(out.starts_with(&version) && out.ends_with(")\n"), "{}", out);
  }
}
