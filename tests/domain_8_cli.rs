mod common;

use common::TestEnvironment;

#[test]
fn test_cli_dry_run_contract() {
    let env = TestEnvironment::new();
    let initial_content = "fn b() {}\nfn a() {}";
    env.write_file("main.rs", initial_content);
    
    let mut cmd = env.lofmt_cmd();
    cmd.arg("--dry-run").arg("main.rs");
    
    cmd.assert().success();
    
    let untouched_content = env.read_file("main.rs");
    assert_eq!(untouched_content, initial_content);
}

#[test]
fn test_cli_check_drift_detected() {
    let env = TestEnvironment::new();
    env.write_file("main.rs", "fn b() {}\nfn a() {}");
    
    let mut cmd = env.lofmt_cmd();
    cmd.arg("--check").arg("main.rs");
    
    cmd.assert().failure().code(1);
}

#[test]
fn test_cli_check_no_drift() {
    let env = TestEnvironment::new();
    env.write_file("main.rs", "fn a() {}\n\nfn b() {}\n");
    
    let mut cmd = env.lofmt_cmd();
    cmd.arg("--check").arg("main.rs");
    
    cmd.assert().success();
}

#[test]
fn test_cli_init_success_and_failure() {
    let env = TestEnvironment::new();
    
    let mut cmd1 = env.lofmt_cmd();
    cmd1.arg("init");
    cmd1.assert().success();
    
    let config_exists = env.dir.path().join("lofmt.toml").exists();
    assert!(config_exists);
    
    let mut cmd2 = env.lofmt_cmd();
    cmd2.arg("init");
    cmd2.assert().failure().code(1);
}

#[test]
fn test_cli_restore_contract() {
    let env = TestEnvironment::new();
    let backup_path = env.write_file("backup.bak", "fn original() {}");
    env.write_file("main.rs", "fn corrupted() {}");
    
    let mut cmd = env.lofmt_cmd();
    cmd.arg("--restore").arg(&backup_path).arg("main.rs");
    
    cmd.assert().success();
    
    let restored = env.read_file("main.rs");
    assert_eq!(restored, "fn original() {}");
}

#[test]
fn test_cli_stdin_pipeline() {
    let env = TestEnvironment::new();
    
    let mut cmd = env.lofmt_cmd();
    cmd.write_stdin("fn b() {}\nfn a() {}");
    
    let output = cmd.output().unwrap();
    
    assert!(output.status.success());
    
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout, "fn a() {}\n\nfn b() {}\n");
}
