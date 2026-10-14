//! The fuzz targets' checks, one module per target in `fuzz/fuzz_targets/`, each a
//! `check(data)` that panics on a violation. The targets call them, and so does
//! `tests/fuzz_regressions.rs` for every input a fuzz run found and that was kept as a
//! test, so a fixed crash stays fixed under exactly the checks that found it.

pub mod comments;
pub mod format;
pub mod generate;
pub mod generated;
pub mod layout;
pub mod lexer;
pub mod parser;
pub mod toml;
pub mod verify;
