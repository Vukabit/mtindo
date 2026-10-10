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
        // Layout is opt-in (S2-10); the tests run with it on unless they say otherwise.
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

    pub fn mto_cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("mto").unwrap();
        // Hermetic: no rustfmt.toml or user config from the developer's home, and no user
        // config created.
        cmd.current_dir(self.dir.path())
            .env("HOME", self.dir.path().join(".home"))
            .env("CI", "1")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("MTINDO_CONFIG");
        cmd
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

        std::os::unix::fs::symlink(original_path, link_path).unwrap();
    }
}
