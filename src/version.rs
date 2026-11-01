//! Layer 0: which mto this is, as the build script found it when compiling: the version,
//! and the build (the commit, with a hash of the source when it differed from the commit;
//! a hash of the source alone when there was no git). Shown by `mto --version`, kept in
//! bug reports, and what the doctor's holds lapse on.

/// The build: the commit, with a hash of the source when it differed (`-dirty.`), or the
/// source's hash alone with neither git nor a package's record of its commit (`src.`);
/// then `-rel` for a release (a published package, or a clean checkout of its version's
/// tag) or `-dev` for any other build: `7bb39cb1a2-rel`, `7bb39cb1a2-dev`,
/// `7bb39cb1a2-dirty.0ca61952-dev`, `src.4f1c09e2b7-dev`.
pub const BUILD: &str = env!("MTINDO_BUILD_ID");

/// `rel` for a release build, `dev` for any other.
pub const CHANNEL: &str = env!("MTINDO_BUILD_CHANNEL");

/// The commit built from, when there was git.
pub const COMMIT: &str = env!("MTINDO_BUILD_COMMIT");

/// The version and the build as one word, `0.1.0+7bb39cb1a2`: two builds with the same
/// version differ here.
pub const FULL: &str = concat!(env!("CARGO_PKG_VERSION"), "+", env!("MTINDO_BUILD_ID"));

/// `release` or `debug`.
pub const PROFILE: &str = env!("MTINDO_BUILD_PROFILE");

/// The compiler, as `rustc -V` says it.
pub const RUSTC: &str = env!("MTINDO_BUILD_RUSTC");

/// The target it was built for, such as `aarch64-apple-darwin`.
pub const TARGET: &str = env!("MTINDO_BUILD_TARGET");

/// The version, as `Cargo.toml` gives it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Every detail, one per line, for a bug report.
pub fn details() -> String {
  format!(
    "mto {} ({})\ntarget: {}\nprofile: {}\ncompiler: {}\n",
    VERSION, BUILD, TARGET, PROFILE, RUSTC
  )
}

/// What `mto --version` prints after `mto `: `0.1.0 (7bb39cb1a2-rel)`.
pub fn short() -> String {
  format!("{} ({})", VERSION, BUILD)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_the_build_is_named() {
    assert!(!BUILD.is_empty());
    assert!(FULL.starts_with(VERSION) && FULL.ends_with(BUILD));
    assert!(!FULL.contains(' '));
    assert!(details().contains(TARGET));
    assert!(BUILD.ends_with(&format!("-{}", CHANNEL)));
    assert!(CHANNEL == "rel" || CHANNEL == "dev");
  }
}
