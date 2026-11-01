//! Pieces every xtask command shares.

pub mod awake;
// mto's own color code, compiled from its source file so xtask needs no dependency on
// mtindo; not every item is used here.
#[allow(dead_code)]
#[path = "../../../src/text/color.rs"]
pub mod color;
pub mod conditions;
pub mod manifest;
pub mod proof;
pub mod tree;

// Shared with xtask/tools.
pub use xtask_core::{edition, files, harness, log, process, sample, store};
