#![allow(dead_code)]

use assert_cmd::Command;
use std::fs;
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};

pub struct TestEnvironment {
    pub dir: TempDir,
}

impl TestEnvironment {
    pub fn new() -> Self {
        let dir = tempdir().unwrap();
        TestEnvironment { dir }
    }

    pub fn write_file(&self, name: &str, content: &str) -> PathBuf {
        let path = self.dir.path().join(name);
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

    pub fn lofmt_cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("lofmt").unwrap();
        cmd.current_dir(self.dir.path());
        cmd
    }
}
