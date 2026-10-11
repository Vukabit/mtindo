//! Layer 5: what the command line shows, built from the lower layers' tables.

pub mod history;
pub mod manual;
pub mod modules;
pub mod options;

/// `cargo mto`'s options: (group, flags, value, help). Its parser, its help and the manual
/// read this table. Help lists the groups in the order they first appear here, and each
/// group's flags by name.
pub const CARGO_MTO_OPTIONS: &[(&str, &str, &str, &str)] = &[
  (
    "packages",
    "--all",
    "",
    "Every workspace member, and local path dependencies outside the workspace",
  ),
  (
    "packages",
    "--manifest-path",
    "<PATH>",
    "The Cargo.toml to start from (default: the nearest one)",
  ),
  (
    "packages",
    "-p, --package",
    "<NAME>...",
    "Only these members (repeatable)",
  ),
  (
    "layout",
    "--check",
    "",
    "Print a diff for every file that would change; exit 1 if any",
  ),
  (
    "layout",
    "--message-format",
    "<short|json|human>",
    "short: name the files (-l); json: the JSON report (--emit json)",
  ),
  (
    "history and safety",
    "--allow-dirty",
    "",
    "With history off, rewrite files that have uncommitted changes",
  ),
  (
    "history and safety",
    "--no-history",
    "",
    "Do not record this run in history (it cannot be restored)",
  ),
  (
    "output",
    "-q, --quiet",
    "",
    "Print only errors and what was asked for",
  ),
  (
    "output",
    "-v, --verbose",
    "",
    "List the targets, and name each file as it is laid out",
  ),
  ("general", "-h, --help", "", "Show this help and exit"),
  ("general", "--version", "", "Print mto's version and exit"),
];

/// The long form of a `flags` entry: `--package` for `-p, --package`.
pub fn long_flag(flags: &str) -> &str {
  flags.rsplit(", ").next().unwrap_or(flags)
}
