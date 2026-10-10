mod common;

use std::fs;

use common::TestEnvironment;

#[test]
fn test_distributed_rollback() {
    let env = TestEnvironment::new();

    env.git_init("repo_alpha");
    env.git_init("repo_beta");

    let mut original_files = Vec::new();
    let mut idx = 0;

    while idx < 25 {
        let path_a = format!("repo_alpha/file_{}.rs", idx);
        let path_b = format!("repo_beta/file_{}.rs", idx);

        let content_a = format!("fn func_b_{}() {{}}\nfn func_a_{}() {{}}", idx, idx);
        let content_b = format!("fn func_d_{}() {{}}\nfn func_c_{}() {{}}", idx, idx);

        let full_a = env.write_file(&path_a, &content_a);
        let full_b = env.write_file(&path_b, &content_b);

        original_files.push((full_a, content_a));
        original_files.push((full_b, content_b));

        idx += 1;
    }

    // Each repository named: a walk from `.` would pass over them as other projects.
    let mut format_cmd = env.mto_cmd();
    format_cmd.args(["repo_alpha", "repo_beta"]);

    let output = format_cmd.output().unwrap();
    assert!(output.status.success());

    for file_data in &original_files {
        let path = &file_data.0;
        let original_content = &file_data.1;
        let formatted_content = fs::read_to_string(path).unwrap();
        assert_ne!(&formatted_content, original_content);
    }

    // One run wrote all 50 files across both repositories; one restore undoes it.
    let restore = env.mto_cmd().args(["restore", "last"]).output().unwrap();
    assert!(restore.status.success(), "{:?}", restore);
    let restored = String::from_utf8_lossy(&restore.stdout);
    assert_eq!(restored.matches("Restored").count(), 50, "{}", restored);

    for file_data in &original_files {
        let path = &file_data.0;
        let original_content = &file_data.1;
        let restored_content = fs::read_to_string(path).unwrap();
        assert_eq!(&restored_content, original_content);
    }
}

#[test]
fn test_follow_links_visits_each_real_file_once() {
    let env = TestEnvironment::new();

    env.create_dir("dir_a");
    env.create_dir("dir_b");
    env.create_symlink("dir_b", "dir_a/link_to_b");
    env.create_symlink("dir_a", "dir_b/link_to_a");
    let file_b = env.write_file("dir_b/lib.rs", "fn d() {}\nfn c() {}");

    let output = env
        .mto_cmd()
        .arg("--follow-links")
        .arg("dir_a")
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.matches("Formatted:").count(),
        1,
        "formatted more than once:\n{}",
        stdout
    );
    assert_eq!(
        fs::read_to_string(file_b).unwrap(),
        "fn c() {}\n\nfn d() {}\n"
    );
}

#[test]
fn test_polyrepo_traversal() {
    let env = TestEnvironment::new();

    env.git_init("repo_a");
    env.git_init("repo_b");
    env.git_init("repo_c");

    let file_a = env.write_file("repo_a/main.rs", "fn b() {}\nfn a() {}");
    let file_b = env.write_file("repo_b/main.rs", "fn d() {}\nfn c() {}");
    let file_c = env.write_file("repo_c/main.rs", "fn f() {}\nfn e() {}");

    // A walk from `.` passes over folders that hold a repository of their own, and says so.
    let output = env.mto_cmd().arg(".").output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("passed over 3 folder(s) holding a repository of their own"),
        "{}",
        stderr
    );
    assert_eq!(fs::read_to_string(&file_a).unwrap(), "fn b() {}\nfn a() {}");

    // Named, each is laid out.
    let mut cmd = env.mto_cmd();
    cmd.args(["repo_a", "repo_b", "repo_c"]);

    cmd.assert().success();

    let content_a = fs::read_to_string(file_a).unwrap();
    let content_b = fs::read_to_string(file_b).unwrap();
    let content_c = fs::read_to_string(file_c).unwrap();

    assert_eq!(content_a, "fn a() {}\n\nfn b() {}\n");
    assert_eq!(content_b, "fn c() {}\n\nfn d() {}\n");
    assert_eq!(content_c, "fn e() {}\n\nfn f() {}\n");
}

#[test]
fn test_symlink_cycles() {
    let env = TestEnvironment::new();

    env.create_dir("dir_a");
    env.create_dir("dir_b");

    env.create_symlink("dir_b", "dir_a/link_to_b");
    env.create_symlink("dir_a", "dir_b/link_to_a");

    let file_a = env.write_file("dir_a/main.rs", "fn b() {}\nfn a() {}");

    let output = env.mto_cmd().arg(".").output().unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.matches("Formatted:").count(),
        1,
        "formatted more than once:\n{}",
        stdout
    );

    let content_a = fs::read_to_string(file_a).unwrap();
    assert_eq!(content_a, "fn a() {}\n\nfn b() {}\n");
}

#[test]
fn test_symlinked_dirs_skipped_by_default() {
    let env = TestEnvironment::new();

    env.create_dir("project");
    let outside = env.write_file("outside/lib.rs", "fn b() {}\nfn a() {}");
    env.create_symlink("outside", "project/linked");

    env.mto_cmd().arg("project").assert().success();

    assert_eq!(fs::read_to_string(outside).unwrap(), "fn b() {}\nfn a() {}");
}
