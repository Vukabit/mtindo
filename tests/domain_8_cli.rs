mod common;

use common::TestEnvironment;

const SORTED: &str = "fn a() {}\n\nfn b() {}\n";

/// Debug builds of mto add a stray comment to their output when `MTINDO_INJECT_FAULT` is
/// set, so the verifier must stop the write.
const SOURCE: &str = "fn b() {}\n\nfn a() {}\n";

const UNSORTED: &str = "fn b() {}\n\nfn a() {}\n";

fn help_output(
    env: &TestEnvironment,
    args: &[&str],
    vars: &[(&str, &str)],
) -> (Option<i32>, String) {
    let mut cmd = env.mto_cmd();
    cmd.args(args)
        .env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR");
    for (key, value) in vars {
        cmd.env(key, value);
    }
    let output = cmd.output().unwrap();
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

/// Runs mto in `env` with `args`; returns (exit code, stdout, stderr).
fn mto(env: &TestEnvironment, args: &[&str]) -> (i32, String, String) {
    let output = env.mto_cmd().args(args).output().unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
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
        .env("CI", "1")
        .args(["mto", "--check", "--message-format", "short"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
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
        .env("CI", "1")
        .env_remove("XDG_STATE_HOME")
        .args(["mto", "-q"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let history = env.mto_cmd().arg("history").output().unwrap();
    let history = String::from_utf8(history.stdout).unwrap();
    assert!(history.contains("4 files  cargo mto -q\n"), "{}", history);
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
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(141), "{}", stderr);
    assert!(!stderr.contains("panicked"), "{}", stderr);
}

#[test]
fn test_cli_a_file_with_syntax_errors_exits_three() {
    let env = TestEnvironment::new();
    let broken = "fn b() {}\nfn a() {\n    if true {\n";
    env.write_file("main.rs", broken);

    let output = env.mto_cmd().arg("main.rs").output().unwrap();
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
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
    let stdout = String::from_utf8(output.stdout).unwrap();
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
    let stdout = String::from_utf8(output.stdout).unwrap();
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Diff in src/one.rs"), "{}", stdout);
    assert!(stdout.contains("Diff in src/two.rs"), "{}", stdout);
    assert!(!stdout.contains("three.rs"), "{}", stdout);
    // A minimal diff shows one of the two functions moving.
    let moved = |name: &str| {
        stdout.contains(&format!("-fn {}() {{}}", name))
            && stdout.contains(&format!("+fn {}() {{}}", name))
    };
    assert!(moved("a") || moved("b"), "{}", stdout);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("2 file(s) would be reformatted"),
        "{}",
        stderr
    );
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
    let stdout = String::from_utf8(output.stdout).unwrap();
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
    let stdout = String::from_utf8(output.stdout).unwrap();
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
    let stdout = String::from_utf8(output.stdout).unwrap();
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
    let stdout = String::from_utf8(output.stdout).unwrap();
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
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with("[history]\nenabled = true  # default\n"),
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
    let stdout = String::from_utf8(output.stdout).unwrap();
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
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", stdout);
    assert!(stdout.contains(" fn b() {}\n+\n fn a() {}"), "{}", stdout);

    // The standard formatter's meaning: `--config FILE` is a mistake for `--config-path
    // FILE` (S2-12).
    let output = env
        .mto_cmd()
        .args(["--config", "custom.toml", "main.rs"])
        .output()
        .unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
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
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr.contains("--config: `newline_style` cannot be `dos`"),
        "{}",
        stderr
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
    let all = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(all.contains("Extracted 2 blocks"), "{}", all);
    assert!(
        all.contains("[Sorted #0] Kind: FnDecl, Identifier: 'a'"),
        "{}",
        all
    );
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
    let dump = String::from_utf8(tree.stdout).unwrap();
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

    env.mto_cmd()
        .args(["debug", "tree"])
        .assert()
        .failure()
        .code(2);
    env.mto_cmd()
        .args(["debug", "tree", "missing.rs"])
        .assert()
        .failure()
        .code(3);
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
        String::from_utf8(output.stdout).unwrap(),
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
    let stdout = String::from_utf8(output.stdout).unwrap();
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
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout, "src/lib.rs\nsrc/b.rs\nsrc/a.rs\nother.rs\n");

    let output = env
        .mto_cmd()
        .args(["--check", "-l", "--skip-children", "src/lib.rs"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "src/lib.rs\n");
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
    let (code, stdout, _) = mto(&env, &["history", "prune", "--keep", "1"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("Removed 2 run(s)"), "{}", stdout);
    let (_, stdout, _) = mto(&env, &["history"]);
    assert_eq!(stdout.lines().count(), 2, "{}", stdout);
    assert_eq!(
        mto(&env, &["history", "prune", "--older-than", "soon"]).0,
        2
    );
    let (code, _, stderr) = mto(&env, &["--restore", "a", "b"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("mto restore last"), "{}", stderr);
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
fn test_cli_layout_is_off_until_opted_in() {
    let env = TestEnvironment::new();
    std::fs::remove_file(env.dir.path().join("mto.toml")).unwrap();
    env.write_file("main.rs", SOURCE);
    env.write_file("rustfmt.toml", "max_width = 80\n");

    // No mto.toml: nothing changes, and the hint points at `mto init`.
    let output = env.mto_cmd().arg("main.rs").output().unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(
        stderr.contains("1 file(s) left alone: layout is off. Run `mto init`"),
        "{}",
        stderr
    );
    assert_eq!(
        std::fs::read_to_string(env.dir.path().join("main.rs")).unwrap(),
        SOURCE
    );
    let output = env.mto_cmd().arg("-").write_stdin(SOURCE).output().unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap(), SOURCE);

    // An mto.toml without `layout = true` still changes nothing, and says why.
    env.write_file("mto.toml", "sort_alphabetically = false\n");
    let output = env.mto_cmd().args(["--check", "main.rs"]).output().unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(
        stderr.contains(
            "`sort_alphabetically` has no effect while layout is off; add `layout = true`"
        ),
        "{}",
        stderr
    );

    // `mto init` turns it on.
    std::fs::remove_file(env.dir.path().join("mto.toml")).unwrap();
    env.mto_cmd().arg("init").assert().success();
    let init = std::fs::read_to_string(env.dir.path().join("mto.toml")).unwrap();
    assert!(init.contains("\nlayout = true\n"), "{}", init);
    env.mto_cmd().args(["--check", "main.rs"]).assert().code(1);
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
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "max_width = 80\n"
    );
    let output = env
        .mto_cmd()
        .args(["--print-config", "default"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("\nmax_width = 100\n"), "{}", stdout);
    assert!(
        !stdout.contains("type_order"),
        "only the standard formatter's settings: {}",
        stdout
    );
    env.mto_cmd()
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
    let stdout = String::from_utf8(output.stdout).unwrap();
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
    assert!(String::from_utf8(output.stdout).unwrap().contains('\x1b'));

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
fn test_cli_stdin_filepath_and_backup() {
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
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.starts_with("Diff in sub/x.rs:1:\n fn b() {}\n+\n fn a() {}"),
        "{}",
        stdout
    );

    let output = env.mto_cmd().args(["--backup", "x.rs"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--backup is not needed"));
}

#[test]
fn test_cli_stdin_pipeline() {
    let env = TestEnvironment::new();

    let mut cmd = env.mto_cmd();
    cmd.arg("-").write_stdin("fn b() {}\nfn a() {}");

    let output = cmd.output().unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout, "fn a() {}\n\nfn b() {}\n");
}

#[test]
fn test_cli_unchanged_file_is_not_rewritten() {
    let env = TestEnvironment::new();
    env.write_file("main.rs", "fn a() {}\n\nfn b() {}\n");

    let output = env.mto_cmd().arg("main.rs").output().unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Formatted:"));
}

#[test]
fn test_cli_unknown_argument_exits_two() {
    let env = TestEnvironment::new();
    env.mto_cmd()
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
    assert_eq!(String::from_utf8(output.stdout).unwrap(), SOURCE);
}

#[test]
fn test_cli_verifier_stops_an_unsafe_write() {
    let env = TestEnvironment::new();
    env.write_file("main.rs", SOURCE);
    let output = env
        .mto_cmd()
        .arg("main.rs")
        .env("MTINDO_INJECT_FAULT", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(env.read_file("main.rs"), SOURCE);
    let stderr = String::from_utf8_lossy(&output.stderr);
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
        assert_eq!(out, format!("mto {}\n", env!("CARGO_PKG_VERSION")));
    }
}
