mod common;

use common::TestEnvironment;

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
    cmd.write_stdin("fn b() {}\nfn a() {}");

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
