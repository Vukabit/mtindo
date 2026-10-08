mod common;

use std::fs;

use common::TestEnvironment;
use mtindo::{create_backup, restore_backup, write_atomic};

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
fn test_write_atomic_success() {
    let env = TestEnvironment::new();
    let file_path = env.write_file("target.rs", "fn old() {}");
    write_atomic(&file_path, "fn new() {}").unwrap();
    let updated_content = env.read_file("target.rs");
    assert_eq!(updated_content, "fn new() {}");
    let sidecar_path = file_path.with_extension("tmp_mtindo");
    assert!(!sidecar_path.exists());
}
