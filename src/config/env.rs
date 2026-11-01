//! The environment variables mto and cargo-mto pass or read, by name.

/// Every variable here: what a hermetic run of mto clears.
pub const ALL: &[&str] = &[CONFIG, COMMAND, PATHS_FILE];

/// The command as it was typed, which `cargo mto` gives mto for its history.
pub const COMMAND: &str = "MTINDO_COMMAND";

/// A configuration file read in place of the user's own.
pub const CONFIG: &str = "MTINDO_CONFIG";

/// What `cargo mto` sets for the mto it runs: not for any command mto runs in turn.
pub const FROM_CARGO_MTO: &[&str] = &[COMMAND, PATHS_FILE];

/// The file holding a list of targets too long for the command line, one path a line,
/// which `cargo mto` gives mto in place of arguments.
pub const PATHS_FILE: &str = "MTINDO_PATHS_FILE";
