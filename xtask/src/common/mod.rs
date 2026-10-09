//! Pieces every xtask command shares.

// mto's own color code, compiled from its source file so xtask needs no dependency on
// mtindo; not every item is used here.
#[allow(dead_code)]
#[path = "../../../src/text/color.rs"]
pub mod color;
pub mod log;
pub mod process;
