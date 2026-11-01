//! The answer keys: the standard formatter run on an input (each toolchain's, through
//! rustup, from an empty home folder, cached by input, edition, settings and version) and
//! what a finished run says about its input.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use mtindo::history::sha256;
use mtindo::syntax::Edition;

use super::source::*;
use crate::process;

/// The first line of a cached refusal; a marker without it predates the refusals being
/// told apart from failures to run, so it is run again.
pub(crate) const REFUSED_HEADER: &str = "refused: exit ";

/// The longest the standard formatter may take on one input.
pub(crate) const STANDARD_LIMIT: Duration = Duration::from_secs(120);

/// What the standard formatter made of an input.
#[derive(Debug, PartialEq)]
pub enum Answer {
  /// Its output.
  Text(String),
  /// It ran and refused the input (a non-zero exit): its exit code and what it said.
  Refused(String),
  /// It did not run on the input: why. Says nothing about the input, so never cached.
  NotRun(String),
}

/// What a case is held against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Key {
  /// The answer the case carries (a test pair's expected file).
  Expected,
  /// The standard formatter's output: run until it settles or once, cached by input or
  /// run fresh.
  Standard { settle: bool, cached: bool },
  /// rustc's own lexer, token for token.
  RustcLexer,
  /// None: the case is only parsed.
  Nothing,
}

/// Fences `downloads` (.dev/cache) off from this repository, whose own settings must not
/// reach the corpus and the test pairs: a `root = true` mto.toml ends mto's search upwards,
/// and an empty rustfmt.toml ends both tools' search for one (empty, so cached answers keep
/// their key).
pub fn fence(downloads: &Path) {
  let fences = [
    (
      "mto.toml",
      "# Written by `cargo xtask compare`: nothing here takes settings from above.\nroot = true\n",
    ),
    ("rustfmt.toml", ""),
  ];
  for (name, text) in fences {
    let fence = downloads.join(name);
    if !fence.exists() {
      let _ = fs::write(&fence, text);
    }
  }
}

/// The toolchain `rust-toolchain.toml` pins, the reference; `stable` when it names none.
pub fn reference_toolchain() -> String {
  let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rust-toolchain.toml");
  fs::read_to_string(file)
    .unwrap_or_default()
    .lines()
    .filter_map(|l| l.trim().strip_prefix("channel"))
    .filter_map(|rest| rest.trim().strip_prefix('='))
    .map(|v| v.trim().trim_matches('"').to_string())
    .next()
    .unwrap_or_else(|| "stable".to_string())
}

/// [`run_standard`] with the rustfmt of `toolchain`; with what it wrote on stderr (its
/// warnings, when it formatted).
pub fn run_rustfmt(
  text: &str,
  edition: &str,
  dir: &Path,
  home: &Path,
  toolchain: &str,
) -> (Answer, String) {
  let rustup_home = std::env::var_os("RUSTUP_HOME")
    .map(PathBuf::from)
    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".rustup")));
  let mut command = Command::new("rustfmt");
  if let Some(rustup_home) = rustup_home {
    command.env("RUSTUP_HOME", rustup_home);
  }
  let done = process::output_with_input(
    command
      .args(["--edition", edition, "--emit", "stdout"])
      .current_dir(dir)
      .env("HOME", home)
      .env_remove("XDG_CONFIG_HOME")
      .env("RUSTUP_TOOLCHAIN", toolchain)
      .env("RUSTUP_AUTO_INSTALL", "0")
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    Some(text.as_bytes()),
    STANDARD_LIMIT,
  );
  match done {
    Ok(done) => {
      let stderr = String::from_utf8_lossy(&done.stderr).into_owned();
      (classify(done), stderr)
    }
    Err(e) => (
      Answer::NotRun(format!("cannot run rustfmt: {}", e)),
      String::new(),
    ),
  }
}

/// The standard formatter on `text` from `dir`, with an empty home folder. rustup is told
/// where it lives and which toolchain to run: the empty home hides its own settings, and a
/// repository's `rust-toolchain` file would ask for another. The toolchain is the one this
/// repository pins ([`reference_toolchain`]), whose rustfmt the record names: rustup's
/// `stable` may be older or newer.
pub fn run_standard(text: &str, edition: &str, dir: &Path, home: &Path) -> Answer {
  run_rustfmt(text, edition, dir, home, &reference_toolchain()).0
}

/// The standard formatter's answer for `input` formatted from `dir`, from the cache or a
/// run.
pub fn standard_output(
  input: &str,
  edition: Edition,
  dir: &Path,
  reference: &str,
  cache: &Path,
  home: &Path,
) -> Answer {
  let config = nearest_config(dir);
  let edition = edition_year(edition);
  let mut key = String::new();
  key.push_str(reference);
  key.push('\0');
  key.push_str(edition);
  key.push('\0');
  key.push_str(&config);
  key.push('\0');
  key.push_str(input);
  let hash = sha256::hex(key.as_bytes());
  let cached = cache.join(&hash[..2]).join(&hash[2..]);
  let failed = cached.with_extension("failed");
  if let Ok(text) = fs::read_to_string(&cached) {
    return Answer::Text(text);
  }
  if let Ok(said) = fs::read_to_string(&failed) {
    if said.starts_with(REFUSED_HEADER) {
      return Answer::Refused(said);
    }
    let _ = fs::remove_file(&failed);
  }
  let answer = run_standard(input, edition, dir, home);
  let _ = fs::create_dir_all(cached.parent().unwrap_or(cache));
  match &answer {
    Answer::Text(text) => {
      let _ = fs::write(&cached, text);
    }
    Answer::Refused(said) => {
      let _ = fs::write(&failed, said);
    }
    Answer::NotRun(_) => {}
  }
  answer
}

/// What a finished run of the standard formatter says about its input.
pub(crate) fn classify(done: process::Separate) -> Answer {
  let stderr = String::from_utf8_lossy(&done.stderr);
  if done.timed_out {
    return Answer::NotRun(format!(
      "rustfmt passed its time limit of {}s",
      STANDARD_LIMIT.as_secs()
    ));
  }
  match done.code {
    0 => match String::from_utf8(done.stdout) {
      Ok(text) => Answer::Text(text),
      Err(_) => Answer::NotRun("rustfmt wrote invalid UTF-8".to_string()),
    },
    // A shell's 128 + signal: killed, not refusing.
    code if !(0..=128).contains(&code) => Answer::NotRun(format!(
      "rustfmt ended without an exit status (code {}): {}",
      code,
      stderr.trim()
    )),
    code => Answer::Refused(format!("{}{}\n{}", REFUSED_HEADER, code, stderr)),
  }
}

/// The text of the configuration file the standard formatter finds from `dir`, for the
/// cache key: the nearest `.rustfmt.toml` or `rustfmt.toml` in it or a parent.
pub(crate) fn nearest_config(dir: &Path) -> String {
  let mut current = Some(dir);
  while let Some(here) = current {
    for name in [".rustfmt.toml", "rustfmt.toml"] {
      if let Ok(text) = fs::read_to_string(here.join(name)) {
        return text;
      }
    }
    current = here.parent();
  }
  String::new()
}
