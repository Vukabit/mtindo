#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use tempfile::{TempDir, tempdir};

pub struct TestEnvironment {
  pub dir: TempDir,
}

impl TestEnvironment {
  pub fn new() -> Self {
    let dir = tempdir().unwrap();
    // Layout is opt-in; the tests run with it on unless they say otherwise.
    fs::write(dir.path().join("mto.toml"), "layout = true\n").unwrap();
    TestEnvironment { dir }
  }

  pub fn write_file(&self, name: &str, content: &str) -> PathBuf {
    let path = self.dir.path().join(name);
    if let Some(parent_dir) = path.parent() {
      fs::create_dir_all(parent_dir).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
  }

  pub fn read_file(&self, name: &str) -> String {
    let path = self.dir.path().join(name);
    fs::read_to_string(path).unwrap()
  }

  pub fn create_dir(&self, name: &str) -> PathBuf {
    let path = self.dir.path().join(name);
    fs::create_dir_all(&path).unwrap();
    path
  }

  /// `mto` with `args`, as [`TestEnvironment::mto_cmd`] runs it, started and not waited
  /// for.
  pub fn mto_spawn(&self, args: &[&str]) -> std::process::Child {
    let mut command = StdCommand::new(assert_cmd::cargo::cargo_bin("mto"));
    for name in mtindo::config::env::ALL {
      command.env_remove(name);
    }
    command
      .args(args)
      .current_dir(self.dir.path())
      .env("HOME", self.dir.path().join(".home"))
      .env("USERPROFILE", self.dir.path().join(".home"))
      .env("CI", "1")
      // Set rather than removed: they come before `%APPDATA%` and `%LOCALAPPDATA%`, so
      // every system reads and writes the same folders under the test's home.
      .env("XDG_CONFIG_HOME", self.dir.path().join(".home/.config"))
      .env("XDG_STATE_HOME", self.dir.path().join(".home/.local/state"))
      .stdout(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .spawn()
      .unwrap()
  }

  pub fn mto_cmd(&self) -> Command {
    let mut cmd = Command::cargo_bin("mto").unwrap();
    // Hermetic: no rustfmt.toml or user config from the developer's home, and no user
    // config created.
    cmd
      .current_dir(self.dir.path())
      .env("HOME", self.dir.path().join(".home"))
      .env("USERPROFILE", self.dir.path().join(".home"))
      .env("CI", "1")
      // Set rather than removed: they come before `%APPDATA%` and `%LOCALAPPDATA%`, so
      // every system reads and writes the same folders under the test's home.
      .env("XDG_CONFIG_HOME", self.dir.path().join(".home/.config"))
      .env("XDG_STATE_HOME", self.dir.path().join(".home/.local/state"));
    for name in mtindo::config::env::ALL {
      cmd.env_remove(name);
    }
    cmd
  }

  /// Runs git in `sub_dir`, quietly, as a fixed author; panics if it fails.
  pub fn git(&self, sub_dir: &str, args: &[&str]) {
    let status = StdCommand::new("git")
      .args([
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "-c",
        "commit.gpgsign=false",
      ])
      .args(args)
      .current_dir(self.dir.path().join(sub_dir))
      .stdout(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .status()
      .unwrap();
    assert!(status.success(), "git {:?}", args);
  }

  pub fn git_init(&self, sub_dir: &str) {
    let target_path = self.dir.path().join(sub_dir);
    fs::create_dir_all(&target_path).unwrap();

    let mut cmd = StdCommand::new("git");
    cmd.arg("init");
    cmd.current_dir(&target_path);

    let status = cmd.status().unwrap();
    assert!(status.success());
  }

  pub fn create_symlink(&self, original: &str, link: &str) {
    let original_path = self.dir.path().join(original);
    let link_path = self.dir.path().join(link);

    if let Some(parent_dir) = link_path.parent() {
      fs::create_dir_all(parent_dir).unwrap();
    }

    #[cfg(unix)]
    std::os::unix::fs::symlink(original_path, link_path).unwrap();
    // Windows links a folder and a file differently.
    #[cfg(windows)]
    if original_path.is_dir() {
      std::os::windows::fs::symlink_dir(original_path, link_path).unwrap();
    } else {
      std::os::windows::fs::symlink_file(original_path, link_path).unwrap();
    }
  }
}
