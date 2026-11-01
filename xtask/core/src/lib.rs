//! What xtask and xtask/tools share: running and sampling child processes, the log, the
//! store under `.dev/`, and the repository's files and editions. std only and without mtindo, so that the test
//! runner builds when the library does not.

pub mod edition;
pub mod files;
pub mod harness;
pub mod log;
pub mod process;
pub mod sample;
pub mod store;
