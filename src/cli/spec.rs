//! The command line's tables: every flag, command and exit code. The parser, the help pages
//! and the manual all read them, so nothing is accepted without being documented.

/// mto's commands, in the order help lists them. Each has its own help page (`mto history
/// -h`), built from this table and [`FLAGS`].
pub const COMMANDS: &[CommandSpec] = &[
  CommandSpec {
    name: "init",
    args: "[DIR]",
    summary: "Write a default mto.toml",
    about: "Writes an mto.toml with layout turned on and every mto setting at its default, \
                each with its summary, into DIR (default: the current directory). An existing \
                file is never overwritten.",
    usage: &["mto init [DIR]"],
    options: &[],
    examples: &["mto init", "mto init crates/core"],
  },
  CommandSpec {
    name: "config",
    args: "path|print|options [DIR]",
    summary: "Show the configuration that applies in a folder, or every setting",
    about: "`path` lists the configuration files that apply in DIR (default: the current \
                directory), and the history folder. `print` shows every setting in effect with \
                where it came from, in sections: layout, then formatting. `options` lists \
                every setting a configuration file may hold, by file, with its values and \
                default, and for the standard formatter's settings whether mto honors it yet.",
    usage: &[
      "mto config path [DIR]",
      "mto config print [OPTIONS] [DIR]",
      "mto config options [--toml]",
    ],
    options: &[
      ("views", "mto config options", &[Flag::Toml]),
      (
        "views",
        "mto config print",
        &[Flag::Current, Flag::Default, Flag::Minimal, Flag::User],
      ),
      (
        "configuration",
        "mto config path|print",
        &[
          Flag::Config,
          Flag::ConfigPath,
          Flag::Edition,
          Flag::StyleEdition,
        ],
      ),
    ],
    examples: &[
      "mto config path",
      "mto config print --minimal",
      "mto config print --user",
      "mto config options",
      "mto config options --toml",
    ],
  },
  CommandSpec {
    name: "history",
    args: "[--all | show <RUN> | prune]",
    summary: "List, show or prune the runs that wrote files",
    about: "Every run that writes files is recorded. With no argument, lists the runs that \
                wrote files in this folder's project (the nearest folder above holding .git, \
                else Cargo.toml), newest first, or every run when there are none; --all lists \
                every project's. `show` lists one run's files and where each stands now. `prune` \
                removes old runs, and the build guard's reports of them, by the user \
                config's limits unless told otherwise. RUN is \
                `last` or `last~N`, counted among this project's runs, or a handle from the \
                list, which names its run anywhere.",
    usage: &[
      "mto history [--all]",
      "mto history show <RUN>",
      "mto history prune [--older-than DAYS] [--keep N]",
    ],
    options: &[
      ("options", "mto history", &[Flag::All]),
      (
        "options",
        "mto history prune",
        &[Flag::Keep, Flag::OlderThan],
      ),
    ],
    examples: &[
      "mto history",
      "mto history --all",
      "mto history show last~1",
      "mto history prune --keep 10",
    ],
  },
  CommandSpec {
    name: "restore",
    args: "<RUN> [FILE...]",
    summary: "Undo a run",
    about: "Puts back what RUN replaced, in every file it wrote or only the FILEs named. A \
                file changed or deleted since is skipped (exit code 1) unless --force. The \
                restore is recorded as a run of its own, so it can be undone too. RUN is \
                `last` or `last~N`, counted among this project's runs, or a handle from \
                `mto history`, which names its run anywhere.",
    usage: &["mto restore <RUN> [FILE...] [--force]"],
    options: &[("options", "mto restore <RUN> [FILE...]", &[Flag::Force])],
    examples: &["mto restore last", "mto restore 3f9a src/lib.rs"],
  },
  CommandSpec {
    name: "doctor",
    args: "[on|off|holds|release|<RUN>]",
    summary: "Guard the build against mto's edits, or find the edit that broke it",
    about: "`on` turns the build guard on: each run that writes files then checks that the \
                project still builds, and holds back the fewest edits that break it. It runs \
                the command given after `--` (default: `cargo check` over the whole \
                workspace and all its targets) and no other; with the guard off, mto runs none. `off` turns \
                it off. With no argument, says whether it is on, its command and the edits it \
                holds. `holds` lists those edits; `release` drops one, or all. RUN narrows \
                down a run already made the same way: RUN is `last` or `last~N`, counted \
                among this project's runs, or a handle from `mto history`.",
    usage: &[
      "mto doctor",
      "mto doctor on [-- <COMMAND>...]",
      "mto doctor off",
      "mto doctor holds",
      "mto doctor release <N|all>",
      "mto doctor <RUN> [-- <COMMAND>...]",
    ],
    options: &[],
    examples: &[
      "mto doctor on",
      "mto doctor on -- cargo test -q",
      "mto doctor holds",
      "mto doctor release all",
      "mto doctor last -- cargo build",
    ],
  },
  CommandSpec {
    name: "report",
    args: "[show|save|send|prune]",
    summary: "The bug reports mto keeps when it finds a bug in itself",
    about: "When the verifier stops a write, or laying out a file fails, mto keeps a report \
                beside history and names it in one line (`report.save = false` in the user \
                config prints the whole report instead). A report holds what failed, the \
                smallest part of the file that still fails (never the whole file), the \
                command, the settings that differ from the defaults and the build; the same \
                bug seen again is counted. With no argument, lists the reports. `show` prints \
                one. `save` writes one to FILE; --with-file adds the whole file it was found \
                in. `send` opens a new issue on mto's repository in the browser, filled in \
                with the report, to read over and submit: mto itself sends nothing. `prune` \
                removes them all; a report not seen for 90 days goes by itself. ID is a \
                report's id from the list, or its start.",
    usage: &[
      "mto report",
      "mto report show <ID>",
      "mto report save <ID> <FILE> [--with-file]",
      "mto report send <ID>",
      "mto report prune",
    ],
    options: &[("options", "mto report save <ID> <FILE>", &[Flag::WithFile])],
    examples: &[
      "mto report",
      "mto report show 7f3a2c",
      "mto report save 7f3a2c bug.md --with-file",
      "mto report send 7f3a2c",
    ],
  },
  CommandSpec {
    name: "debug",
    args: "tokens|tree|layout <FILE>",
    summary: "Print what mto sees in a file, or where and why it leaves code as written",
    about: "Prints the tokens or the syntax tree of FILE as mto reads it, or (layout) each \
                place the formatter leaves code as written in it and why, with the settings \
                that apply to FILE and formatting on; nothing is written. The output is not stable; it is meant \
                for bug reports and for finding why code was not formatted.",
    usage: &["mto debug tokens|tree|layout <FILE> [--edition YEAR]"],
    options: &[(
      "options",
      "mto debug tokens|tree|layout <FILE>",
      &[Flag::Edition],
    )],
    examples: &["mto debug tree src/lib.rs", "mto debug layout src/lib.rs"],
  },
];

pub const EXAMPLES: &[&str] = &[
  "mto src/main.rs",
  "mto -R --check",
  "mto --check -l src/lib.rs src/main.rs",
  "mto --emit json --check src/",
  "cat src/main.rs | mto -",
  "mto --config max_width=80,hard_tabs=true src/",
  "mto config print --minimal",
  "mto restore last",
];

pub const EXIT_CODES: &[(&str, &str)] = &[
  ("0", "success"),
  (
    "1",
    "--check found files that would change; restore skipped files changed since",
  ),
  ("2", "bad arguments or configuration"),
  (
    "3",
    "a file could not be read, written, or parsed (it has syntax errors), or, with \
         history off, it has uncommitted changes",
  ),
  (
    "4",
    "a bug in mto: the meaning check stopped a write, or formatting a file failed; the \
         file is unchanged",
  ),
  (
    "5",
    "the build guard's command failed with every edit held back, or already failed \
         before the run; the files are as they were",
  ),
];

pub const FLAGS: &[FlagSpec] = &[
  FlagSpec {
    flag: Flag::Check,
    group: Group::Changes,
    short: None,
    long: "--check",
    value: "",
    help: "Print a diff for every file that would change; exit 1 if any",
  },
  FlagSpec {
    flag: Flag::DryRun,
    group: Group::Changes,
    short: Some("-d"),
    long: "--dry-run",
    value: "",
    help: "List the files that would change, without writing",
  },
  FlagSpec {
    flag: Flag::Emit,
    group: Group::Changes,
    short: None,
    long: "--emit",
    value: "<files|stdout|json>",
    help: "Write the files (default), print them, or report changes as JSON",
  },
  FlagSpec {
    flag: Flag::FilesWithDiff,
    group: Group::Changes,
    short: Some("-l"),
    long: "--files-with-diff",
    value: "",
    help: "Name the files that change (or would, with --check) instead of showing how",
  },
  FlagSpec {
    flag: Flag::FollowLinks,
    group: Group::Files,
    short: None,
    long: "--follow-links",
    value: "",
    help: "Also follow symlinked files and directories",
  },
  FlagSpec {
    flag: Flag::Recursive,
    group: Group::Files,
    short: Some("-R"),
    long: "--recursive",
    value: "",
    help: "Format every .rs file under the current directory",
  },
  FlagSpec {
    flag: Flag::SkipChildren,
    group: Group::Files,
    short: None,
    long: "--skip-children",
    value: "",
    help: "A file argument is that file alone, not the modules it declares",
  },
  FlagSpec {
    flag: Flag::StdinFilepath,
    group: Group::Stdin,
    short: None,
    long: "--stdin-filepath",
    value: "<PATH>",
    help: "With -: where the source belongs, for its settings and in messages",
  },
  FlagSpec {
    flag: Flag::Config,
    group: Group::Configuration,
    short: None,
    long: "--config",
    value: "<KEY=VALUE,...>",
    help: "Set settings for this run, over every file",
  },
  FlagSpec {
    flag: Flag::ConfigPath,
    group: Group::Configuration,
    short: None,
    long: "--config-path",
    value: "<FILE>",
    help: "Use only this configuration file; search for no other",
  },
  FlagSpec {
    flag: Flag::Edition,
    group: Group::Configuration,
    short: None,
    long: "--edition",
    value: "<YEAR>",
    help: "The Rust edition, over Cargo.toml and configuration files",
  },
  FlagSpec {
    flag: Flag::PrintConfig,
    group: Group::Compatibility,
    short: None,
    long: "--print-config",
    value: "<default|minimal|current> [PATH]",
    help: "Print the formatting settings as the standard formatter prints them, and exit",
  },
  FlagSpec {
    flag: Flag::StyleEdition,
    group: Group::Configuration,
    short: None,
    long: "--style-edition",
    value: "<YEAR>",
    help: "The style edition, which decides the defaults",
  },
  FlagSpec {
    flag: Flag::UnstableFeatures,
    group: Group::Configuration,
    short: None,
    long: "--unstable-features",
    value: "",
    help: "Accepted for compatibility; mto applies every setting on every channel",
  },
  FlagSpec {
    flag: Flag::AllowDirty,
    group: Group::History,
    short: None,
    long: "--allow-dirty",
    value: "",
    help: "With history off, rewrite files that have uncommitted changes",
  },
  FlagSpec {
    flag: Flag::Unguarded,
    group: Group::History,
    short: None,
    long: "--unguarded",
    value: "",
    help: "With the build guard on (`mto doctor on`), do not check this run",
  },
  FlagSpec {
    flag: Flag::NoHistory,
    group: Group::History,
    short: None,
    long: "--no-history",
    value: "",
    help: "Do not record this run in history (it cannot be restored)",
  },
  FlagSpec {
    flag: Flag::Color,
    group: Group::Output,
    short: None,
    long: "--color",
    value: "<always|never|auto>",
    help: "Color the output (default: auto, only on a terminal)",
  },
  FlagSpec {
    flag: Flag::Dbg,
    group: Group::Output,
    short: None,
    long: "--dbg",
    value: "",
    help: "Print how each file was split into items",
  },
  FlagSpec {
    flag: Flag::Quiet,
    group: Group::Output,
    short: Some("-q"),
    long: "--quiet",
    value: "",
    help: "Print only errors and what was asked for",
  },
  FlagSpec {
    flag: Flag::Verbose,
    group: Group::Output,
    short: Some("-v"),
    long: "--verbose",
    value: "",
    help: "Name each file as it is formatted",
  },
  FlagSpec {
    flag: Flag::Help,
    group: Group::General,
    short: Some("-h"),
    long: "--help",
    value: "",
    help: "Show this help and exit",
  },
  FlagSpec {
    flag: Flag::Man,
    group: Group::General,
    short: None,
    long: "--man",
    value: "",
    help: "Print the full manual",
  },
  FlagSpec {
    flag: Flag::Version,
    group: Group::General,
    short: Some("-V"),
    long: "--version",
    value: "",
    help: "Print the version and exit",
  },
  FlagSpec {
    flag: Flag::Current,
    group: Group::Command,
    short: None,
    long: "--current",
    value: "",
    help: "Every setting in effect, with its source (the default)",
  },
  FlagSpec {
    flag: Flag::Default,
    group: Group::Command,
    short: None,
    long: "--default",
    value: "",
    help: "Every setting at its default",
  },
  FlagSpec {
    flag: Flag::Minimal,
    group: Group::Command,
    short: None,
    long: "--minimal",
    value: "",
    help: "Only the settings that differ from their defaults",
  },
  FlagSpec {
    flag: Flag::User,
    group: Group::Command,
    short: None,
    long: "--user",
    value: "",
    help: "The user config's settings",
  },
  FlagSpec {
    flag: Flag::Force,
    group: Group::Command,
    short: None,
    long: "--force",
    value: "",
    help: "Also restore files changed since the run, and missing ones",
  },
  FlagSpec {
    flag: Flag::Toml,
    group: Group::Command,
    short: None,
    long: "--toml",
    value: "",
    help: "As a commented file, every setting at its default",
  },
  FlagSpec {
    flag: Flag::All,
    group: Group::Command,
    short: None,
    long: "--all",
    value: "",
    help: "List every project's runs",
  },
  FlagSpec {
    flag: Flag::Keep,
    group: Group::Command,
    short: None,
    long: "--keep",
    value: "<N>",
    help: "Keep only the newest N runs",
  },
  FlagSpec {
    flag: Flag::WithFile,
    group: Group::Command,
    short: None,
    long: "--with-file",
    value: "",
    help: "Add the whole file the bug was found in",
  },
  FlagSpec {
    flag: Flag::OlderThan,
    group: Group::Command,
    short: None,
    long: "--older-than",
    value: "<DAYS>",
    help: "Remove runs this old: 30 (days), 2w, 12h",
  },
];

pub const USAGE_LINES: &[&str] = &[
  "mto [OPTIONS] <PATH>...",
  "mto [OPTIONS] - [--stdin-filepath PATH]",
  "mto <COMMAND> [ARGS]",
  "cargo mto [OPTIONS] [-- <MTO OPTIONS>...]",
];

/// Every flag mto accepts. The parser and the help text both read this table, so a flag
/// cannot be accepted without being documented.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flag {
  Check,
  Color,
  Emit,
  FilesWithDiff,
  PrintConfig,
  Quiet,
  SkipChildren,
  StdinFilepath,
  UnstableFeatures,
  Verbose,
  Config,
  ConfigPath,
  Current,
  Dbg,
  Default,
  DryRun,
  Edition,
  FollowLinks,
  Help,
  Man,
  Minimal,
  Recursive,
  NoHistory,
  AllowDirty,
  Unguarded,
  Force,
  OlderThan,
  Keep,
  WithFile,
  All,
  Toml,
  StyleEdition,
  User,
  Version,
}

/// The groups `mto -h` lists flags in, in this order. Within a group, flags are listed by
/// name. `Command` flags belong to one command and are listed on its own help page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
  Changes,
  Files,
  Stdin,
  Configuration,
  History,
  Output,
  Compatibility,
  General,
  Command,
}

impl Group {
  pub const SHOWN: [Group; 8] = [
    Group::Changes,
    Group::Files,
    Group::Stdin,
    Group::Configuration,
    Group::History,
    Group::Output,
    Group::Compatibility,
    Group::General,
  ];

  /// The command the group's flags follow, shown beside its heading.
  pub fn command(self) -> &'static str {
    match self {
      Group::Changes | Group::Files | Group::History => "mto <PATH>...",
      Group::Stdin => "mto -",
      Group::Configuration | Group::Output => "mto <PATH>... | mto -",
      Group::Compatibility | Group::General | Group::Command => "mto",
    }
  }

  pub fn heading(self) -> &'static str {
    match self {
      Group::Changes => "changes",
      Group::Files => "files",
      Group::Stdin => "stdin",
      Group::Configuration => "configuration",
      Group::History => "history and safety",
      Group::Output => "output",
      Group::Compatibility => "compatibility",
      Group::General => "general",
      Group::Command => "command options",
    }
  }
}

/// One of mto's commands: its help page, and its line in `mto -h`.
pub struct CommandSpec {
  pub name: &'static str,
  /// What follows the name, for `mto -h` and the manual.
  pub args: &'static str,
  /// One line, for `mto -h`.
  pub summary: &'static str,
  /// The command's help page's description.
  pub about: &'static str,
  pub usage: &'static [&'static str],
  /// The flags it takes, in sections: (heading, the command they follow, flags in the
  /// order its help lists them).
  pub options: &'static [(&'static str, &'static str, &'static [Flag])],
  pub examples: &'static [&'static str],
}

pub struct FlagSpec {
  pub flag: Flag,
  pub group: Group,
  pub short: Option<&'static str>,
  pub long: &'static str,
  pub value: &'static str,
  pub help: &'static str,
}

/// A flag as the manual names it: `-q, --quiet`, `--emit <files|stdout|json>`.
pub fn flag_name(spec: &FlagSpec) -> String {
  let mut name = match spec.short {
    Some(short) => format!("{}, {}", short, spec.long),
    None => spec.long.to_string(),
  };
  if !spec.value.is_empty() {
    name.push(' ');
    name.push_str(spec.value);
  }
  name
}

/// A group's flags, by name.
pub fn flags_of(group: Group) -> Vec<&'static FlagSpec> {
  let mut flags: Vec<&FlagSpec> = FLAGS.iter().filter(|spec| spec.group == group).collect();
  flags.sort_by_key(|spec| spec.long);
  flags
}

pub fn spec_of(flag: Flag) -> &'static FlagSpec {
  FLAGS
    .iter()
    .find(|spec| spec.flag == flag)
    .expect("every flag has a spec")
}
