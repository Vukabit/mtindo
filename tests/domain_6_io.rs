mod common;

use std::fs;

use common::TestEnvironment;
use mtindo::{create_backup, restore_backup, write_atomic};

#[test]
fn test_backups_never_collide() {
    let env = TestEnvironment::new();
    let file_path = env.write_file("target.rs", "fn a() {}");
    let first = create_backup(&file_path, "first").unwrap();
    let second = create_backup(&file_path, "second").unwrap();
    assert_ne!(first, second);
    assert_eq!(fs::read_to_string(&first).unwrap(), "first");
    assert_eq!(fs::read_to_string(&second).unwrap(), "second");
}

#[test]
fn test_create_backup_timestamp_generation() {
    let env = TestEnvironment::new();
    let file_path = env.write_file("target.rs", "fn a() {}");
    let backup_path = create_backup(&file_path, "fn a() {}").unwrap();
    assert!(backup_path.exists());
    let content = fs::read_to_string(&backup_path).unwrap();
    assert_eq!(content, "fn a() {}");
}

#[test]
fn test_restore_backup_success() {
    let env = TestEnvironment::new();
    let backup_path = env.write_file("backup.bak", "fn original() {}");
    let target_path = env.write_file("target.rs", "fn corrupted() {}");
    restore_backup(&backup_path, &target_path).unwrap();
    let restored_content = env.read_file("target.rs");
    assert_eq!(restored_content, "fn original() {}");
}

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
