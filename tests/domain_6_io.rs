// A test prints only to say what it did.
#![allow(clippy::disallowed_macros)]
mod common;

use std::fs;

use common::TestEnvironment;
use mtindo::write_atomic;

/// A run killed at any point leaves every file whole, as it was or as mto writes it, and
/// `mto restore last` puts back every file it changed.
#[test]
fn test_a_run_killed_anywhere_leaves_files_whole_and_restorable() {
  const FILES: usize = 300;
  let name = |i: usize| format!("src/f{}.rs", i);
  let original = |i: usize| format!("fn b{0}() {{}}\n\nfn a{0}() {{}}\n", i);
  let write_all = |env: &TestEnvironment| {
    for i in 0..FILES {
      env.write_file(&name(i), &original(i));
    }
  };
  // What a run left to finish writes.
  let finished = TestEnvironment::new();
  write_all(&finished);
  assert_eq!(finished.mto_spawn(&["src"]).wait().unwrap().code(), Some(0));
  let written: Vec<String> = (0..FILES).map(|i| finished.read_file(&name(i))).collect();
  assert!((0..FILES).all(|i| written[i] != original(i)));
  // Killed after longer and longer, from before it starts to after it is done.
  let mut cut_short = 0;
  for delay_ms in [0, 1, 2, 4, 8, 15, 25, 40, 60, 90, 130, 200] {
    let env = TestEnvironment::new();
    write_all(&env);
    let mut child = env.mto_spawn(&["src"]);
    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    let _ = child.kill();
    let status = child.wait().unwrap();
    let changed = (0..FILES)
      .filter(|&i| {
        let now = env.read_file(&name(i));
        assert!(
          now == original(i) || now == written[i],
          "killed after {} ms: {} is neither as it was nor as written:\n{}",
          delay_ms,
          name(i),
          now
        );
        now != original(i)
      })
      .count();
    if changed > 0 && changed < FILES {
      cut_short += 1;
    }
    if changed > 0 {
      let output = env.mto_cmd().args(["restore", "last"]).output().unwrap();
      assert_eq!(
        output.status.code(),
        Some(0),
        "killed after {} ms ({:?}), {} file(s) changed: {}",
        delay_ms,
        status,
        changed,
        String::from_utf8_lossy(&output.stderr)
      );
      for i in 0..FILES {
        assert_eq!(
          env.read_file(&name(i)),
          original(i),
          "killed after {} ms: {} not restored",
          delay_ms,
          name(i)
        );
      }
    }
  }
  // Not a guarantee on every machine, but a test that never cut a run short tests nothing.
  eprintln!("{} of 12 runs cut short mid-write", cut_short);
}

/// A temporary file a run cut short left behind goes with the next run that writes, once it
/// is too old to be another run's; a check leaves it.
#[test]
fn test_stale_temporary_files_are_removed_by_a_run_that_writes() {
  let env = TestEnvironment::new();
  env.write_file("src/a.rs", "fn b() {}\n\nfn a() {}\n");
  let stale = env.write_file("src/.a.rs.123.0.tmp_mtindo", "fn b");
  let young = env.write_file("src/.a.rs.124.0.tmp_mtindo", "fn b");
  let mine = env.write_file("src/notes.tmp_mtindo", "mine");
  let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
  for path in [&stale, &mine] {
    fs::OpenOptions::new()
      .write(true)
      .open(path)
      .unwrap()
      .set_modified(old)
      .unwrap();
  }
  assert_eq!(
    env
      .mto_cmd()
      .args(["--check", "src"])
      .output()
      .unwrap()
      .status
      .code(),
    Some(1)
  );
  assert!(stale.exists());
  let output = env.mto_cmd().arg("src").output().unwrap();
  assert_eq!(output.status.code(), Some(0));
  assert!(
    String::from_utf8_lossy(&output.stderr).contains("removed 1 temporary file(s)"),
    "{}",
    String::from_utf8_lossy(&output.stderr)
  );
  assert!(!stale.exists());
  // Another run's, still young; and a file not named as mto names them.
  assert!(young.exists() && mine.exists());
}

#[cfg(unix)]
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

/// A read-only file is left as it is, on every system.
#[test]
fn test_write_atomic_leaves_a_read_only_file() {
  let env = TestEnvironment::new();
  let file_path = env.write_file("generated.rs", "fn old() {}");
  let mut permissions = fs::metadata(&file_path).unwrap().permissions();
  permissions.set_readonly(true);
  fs::set_permissions(&file_path, permissions).unwrap();
  let e = write_atomic(&file_path, "fn new() {}").unwrap_err();
  assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
  assert_eq!(e.to_string(), "the file is read-only");
  assert_eq!(fs::read_to_string(&file_path).unwrap(), "fn old() {}");
  // No temporary file is left beside it.
  assert!(fs::read_dir(env.dir.path()).unwrap().all(|e| {
    !e.unwrap()
      .file_name()
      .to_string_lossy()
      .ends_with(".tmp_mtindo")
  }));
  let mut permissions = fs::metadata(&file_path).unwrap().permissions();
  #[allow(clippy::permissions_set_readonly_false)]
  permissions.set_readonly(false);
  fs::set_permissions(&file_path, permissions).unwrap();
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
