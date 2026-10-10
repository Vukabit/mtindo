mod common;

use std::fs;

use common::TestEnvironment;
use mtindo::write_atomic;

#[test]
fn test_write_atomic_keeps_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let env = TestEnvironment::new();
    let file_path = env.write_file("script.rs", "fn old() {}");
    fs::set_permissions(&file_path, fs::Permissions::from_mode(0o755)).unwrap();
    write_atomic(&file_path, "fn new() {}").unwrap();
    let mode = fs::metadata(&file_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o755);
}

#[test]
fn test_write_atomic_keeps_symlink() {
    let env = TestEnvironment::new();
    let real_path = env.write_file("real.rs", "fn old() {}");
    env.create_symlink("real.rs", "link.rs");
    let link_path = env.dir.path().join("link.rs");
    write_atomic(&link_path, "fn new() {}").unwrap();
    assert!(
        fs::symlink_metadata(&link_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&real_path).unwrap(), "fn new() {}");
}

#[test]
fn test_write_atomic_success() {
    let env = TestEnvironment::new();
    let file_path = env.write_file("target.rs", "fn old() {}");
    write_atomic(&file_path, "fn new() {}").unwrap();
    let updated_content = env.read_file("target.rs");
    assert_eq!(updated_content, "fn new() {}");
    let leftovers: Vec<_> = fs::read_dir(env.dir.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().contains("tmp_mtindo"))
        .collect();
    assert!(leftovers.is_empty());
}
