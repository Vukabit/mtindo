mod common;

use common::TestEnvironment;

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
        bare.starts_with("usage: mto [OPTIONS] <PATH>\n"),
        "{}",
        bare
    );
    assert!(
        !bare.contains('\x1b'),
        "help piped to a file must not be colored"
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
    assert!(stdout.contains("-fn b() {}"), "{}", stdout);
    assert!(stdout.contains("+fn b() {}"), "{}", stdout);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("2 file(s) would be reformatted"),
        "{}",
        stderr
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
fn test_cli_init_success_and_failure() {
    let env = TestEnvironment::new();

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
fn test_cli_restore_contract() {
    let env = TestEnvironment::new();
    let backup_path = env.write_file("backup.bak", "fn original() {}");
    env.write_file("main.rs", "fn corrupted() {}");

    let mut cmd = env.mto_cmd();
    cmd.arg("--restore").arg(&backup_path).arg("main.rs");

    cmd.assert().success();

    let restored = env.read_file("main.rs");
    assert_eq!(restored, "fn original() {}");
}

#[test]
fn test_cli_skips_file_it_cannot_split() {
    let env = TestEnvironment::new();
    let broken = "fn b() {}\nfn a() {\n    if true {\n";
    env.write_file("main.rs", broken);

    let output = env.mto_cmd().arg("main.rs").output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Skipped"));
    assert_eq!(env.read_file("main.rs"), broken);
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
fn test_cli_version() {
    let env = TestEnvironment::new();
    let (code, out) = help_output(&env, &["-V"], &[]);
    assert_eq!(code, Some(0));
    assert_eq!(out, format!("mto {}\n", env!("CARGO_PKG_VERSION")));
}
