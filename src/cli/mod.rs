//! Layer 5: what the command line shows, built from the lower layers' tables.

pub mod history;
pub mod manual;
pub mod modules;

/// `cargo mto`'s options: (flags, value, help). Its parser, its help and the manual read
/// this table.
pub const CARGO_MTO_OPTIONS: &[(&str, &str, &str)] = &[
    ("-h, --help", "", "Show this help and exit"),
    ("--version", "", "Print mto's version and exit"),
    (
        "--check",
        "",
        "Print a diff for every file that would change; exit 1 if any",
    ),
    (
        "--all",
        "",
        "Every workspace member, and local path dependencies outside the workspace",
    ),
    (
        "-p, --package",
        "<NAME>...",
        "Only these members (repeatable)",
    ),
    (
        "--manifest-path",
        "<PATH>",
        "The Cargo.toml to start from (default: the nearest one)",
    ),
    (
        "--message-format",
        "<short|json|human>",
        "short: name the files (-l); json: rustfmt's JSON report (--emit json)",
    ),
    (
        "--no-history",
        "",
        "Do not record this run in history (it cannot be restored)",
    ),
    (
        "--allow-dirty",
        "",
        "With history off, rewrite files that have uncommitted changes",
    ),
    (
        "-q, --quiet",
        "",
        "Print only errors and what was asked for",
    ),
    (
        "-v, --verbose",
        "",
        "List the targets, and name each file as it is laid out",
    ),
];
