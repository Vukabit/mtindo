use std::fs;
use std::io::Write;

use super::*;

/// Each mode on its own record of what is pending, as the process's would be.
#[test]
fn test_both_modes_write_and_settle() {
  let dir = std::env::temp_dir().join(format!("mtindo-durable-{}", std::process::id()));
  let _ = fs::remove_dir_all(&dir);
  fs::create_dir_all(&dir).unwrap();
  for mode in [Mode::Full, Mode::Batch] {
    let pending = Pending::new();
    let temp = dir.join("temp");
    let mut file = File::create(&temp).unwrap();
    file.write_all(b"written").unwrap();
    ordered_in(&pending, mode, &file, &dir).unwrap();
    drop(file);
    fs::rename(&temp, dir.join("target")).unwrap();
    renamed_in(&pending, mode, &dir).unwrap();
    // Batch leaves the folder for `settle`; full has nothing pending.
    assert_eq!(lock(&pending).dirs.contains(&dir), mode == Mode::Batch);
    settle_in(&pending).unwrap();
    assert!(lock(&pending).dirs.is_empty() && lock(&pending).touched.is_empty());
    assert_eq!(fs::read(dir.join("target")).unwrap(), b"written");
  }
  // A folder gone before the run settles is passed over.
  let pending = Pending::new();
  let gone = dir.join("gone");
  fs::create_dir_all(&gone).unwrap();
  renamed_in(&pending, Mode::Batch, &gone).unwrap();
  fs::remove_dir(&gone).unwrap();
  settle_in(&pending).unwrap();
  let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_the_mode_is_the_process_s() {
  assert_eq!(mode(), Mode::Batch);
}
