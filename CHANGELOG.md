# Changelog

Every release of mtindo, newest first. The layout follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Sections after 0.1.0 are written
from commit subjects by `cargo xtask changelog`.

## [0.1.0] - unreleased

First release.

### Added

- `mto` formats Rust source files in the Rust style, exactly as the standard
  formatter writes them with the same settings, reading the same
  `rustfmt.toml`; it matches over 99.9% of the lines of a pinned corpus of
  real crates.
- Layout, for projects that opt in with `mto init`: a file's items reordered
  into a predictable top-down order.
- Every write is checked first, so the code still means what it meant; every
  run is recorded, and `mto restore last` undoes it.
- `cargo mto` formats every target of a package or workspace; `--check` shows
  what would change, with JSON output for CI and editors.
- The build guard holds back edits that break the build.
- `mto report` writes a bug report with everything needed to reproduce a
  problem.
- No runtime dependencies: the Rust standard library alone, on Rust 1.88 or
  later.
