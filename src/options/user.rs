//! The per-user settings: how mto behaves for one person on every project. Only the user
//! config may hold them, under their section (`[history]` holds `enabled` as
//! `history.enabled`).

use super::{Choice, File, Kind, Setting};

pub(super) static SETTINGS: &[Setting] = &[
  Setting {
    name: "doctor.command",
    file: File::User,
    kind: Kind::List,
    default: "[\"cargo\", \"check\", \"--workspace\", \"--all-targets\", \"--keep-going\"]",
    default_2024: None,
    summary: "The command the build guard runs, as its words",
    doc: "The build guard runs it in the project's folder before and after each run that \
              writes files, and while it narrows down a run that broke the build. It is run \
              as these words, not through a shell. `mto doctor on -- COMMAND` sets it.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "doctor.enabled",
    file: File::User,
    kind: Kind::Bool,
    default: "false",
    default_2024: None,
    summary: "Check that the project still builds after each run (it runs `command`)",
    doc: "With it on, mto runs `doctor.command` before and after each run that writes \
              files, and holds back the fewest edits that break the build. With it off, mto \
              runs no command. `mto doctor on` and `mto doctor off` set it.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "doctor.max_checks",
    file: File::User,
    kind: Kind::Integer,
    default: "64",
    default_2024: None,
    summary: "The most times one narrowing runs the command",
    doc: "Past this many runs of the command, a narrowing holds back every edit of the \
              files it has narrowed down to.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "doctor.timeout",
    file: File::User,
    kind: Kind::Integer,
    default: "600",
    default_2024: None,
    summary: "Seconds the command may run before it counts as failed",
    doc: "A command still running after this many seconds is stopped and counts as failed. \
              0: no limit.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "history.enabled",
    file: File::User,
    kind: Kind::Bool,
    default: "true",
    default_2024: None,
    summary: "Record every run that writes files, so `mto restore` can undo it",
    doc: "Every run that writes files is recorded: each file's contents before and after, \
              kept once however many runs share them. `mto history` lists the runs and \
              `mto restore` undoes one. `--no-history` skips the record for one run. With \
              history off, the git guard stops mto from overwriting uncommitted changes.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "history.keep_days",
    file: File::User,
    kind: Kind::Integer,
    default: "30",
    default_2024: None,
    summary: "Days a run is kept (0: no limit)",
    doc: "Runs older than this many days are removed after each recorded run and by \
              `mto history prune`. The newest run is always kept.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "history.keep_runs",
    file: File::User,
    kind: Kind::Integer,
    default: "100",
    default_2024: None,
    summary: "Runs kept (0: no limit)",
    doc: "Only this many of the newest runs are kept.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "history.max_size",
    file: File::User,
    kind: Kind::Size,
    default: "\"256MB\"",
    default_2024: None,
    summary: "The most room history may take (0: no limit)",
    doc: "The oldest runs are removed until the file contents the rest keep fit in this \
              much room. A number of bytes, or one with `KB`, `MB` or `GB` (powers of 1024).",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "output.color",
    file: File::User,
    kind: Kind::Choice(&[
      choice("auto", "Color only on a terminal."),
      choice("always", "Always color."),
      choice("never", "Never color."),
    ]),
    default: "\"auto\"",
    default_2024: None,
    summary: "Whether to color the output",
    doc: "`--color` decides over it, and so do the `NO_COLOR` and `FORCE_COLOR` \
              environment variables.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "report.save",
    file: File::User,
    kind: Kind::Bool,
    default: "true",
    default_2024: None,
    summary: "Keep a report of each bug mto finds in itself, for `mto report`",
    doc: "When the verifier stops a write, or laying out a file fails, the report is kept \
              beside history and the run names it in one line; `mto report` shows, saves, \
              sends and prunes reports. A report holds the smallest part of the file that \
              still fails, never the whole file unless asked. Off, the whole report is \
              printed instead, and nothing is kept.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "safety.allow_dirty",
    file: File::User,
    kind: Kind::Bool,
    default: "false",
    default_2024: None,
    summary: "With history off, let mto overwrite uncommitted changes",
    doc: "With history off there is no `mto restore`, so mto refuses to rewrite a file \
              that git sees uncommitted changes in. This turns that guard off, as \
              `--allow-dirty` does for one run. With history on it has no effect.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
  Setting {
    name: "safety.sync",
    file: File::User,
    kind: Kind::Choice(&[
      choice(
        "batch",
        "Each file reaches the disk in order, and everything a run wrote lasts once it \
         ends.",
      ),
      choice("full", "Each file lasts as soon as it is written."),
    ]),
    default: "\"batch\"",
    default_2024: None,
    summary: "How much each write makes last at once",
    doc: "Either way a crash leaves each file as it was or as mto wrote it, and history \
              holds its earlier contents before it changes. `batch` keeps writes in order \
              and makes them all last once the run ends, as git's `core.fsyncMethod = \
              batch` does: on macOS a run writing hundreds of files takes a fraction of \
              the time. `full` makes each file last before the next is written, so a power \
              cut mid-run loses nothing already written, at a full disk flush per file.",
    channels: None,
    hidden: false,
    replaced_by: None,
  },
];

/// Bytes in a size such as `256MB`; `None` when it is not one.
pub fn bytes(size: &str) -> Option<u64> {
  let size = size.trim();
  let digits = size
    .find(|c: char| !c.is_ascii_digit())
    .unwrap_or(size.len());
  let number: u64 = size[..digits].parse().ok()?;
  let scale: u64 = match size[digits..].trim().to_ascii_uppercase().as_str() {
    "" | "B" => 1,
    "KB" => 1 << 10,
    "MB" => 1 << 20,
    "GB" => 1 << 30,
    _ => return None,
  };
  number.checked_mul(scale)
}

const fn choice(name: &'static str, doc: &'static str) -> Choice {
  Choice {
    name,
    doc,
    channels: None,
  }
}
