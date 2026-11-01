//! The command line's tables: every flag, command and exit code. The parser, the help pages
//! and the manual all read them, so nothing is accepted without being documented.

/// `cargo mto`'s flags, each with what it becomes on mto's command line. Its parser, its
/// help and the manual read this table. Help lists the groups in the order they first
/// appear here, and each group's flags by name.
pub const CARGO_FLAGS: &[CargoFlagSpec] = &[
  CargoFlagSpec {
    flag: CargoFlag::All,
    group: "packages",
    short: None,
    long: "--all",
    value: "",
    help: "Every workspace member, and local path dependencies outside the workspace",
    forwards: Forward::Own,
  },
  CargoFlagSpec {
    flag: CargoFlag::ManifestPath,
    group: "packages",
    short: None,
    long: "--manifest-path",
    value: "<PATH>",
    help: "The Cargo.toml to start from (default: the nearest one)",
    forwards: Forward::Own,
  },
  CargoFlagSpec {
    flag: CargoFlag::Package,
    group: "packages",
    short: Some("-p"),
    long: "--package",
    value: "<NAME>...",
    help: "Only these members (repeatable)",
    forwards: Forward::Own,
  },
  CargoFlagSpec {
    flag: CargoFlag::Check,
    group: "changes",
    short: None,
    long: "--check",
    value: "",
    help: "Print a diff for every file that would change; exit 1 if any",
    forwards: Forward::Flag(Flag::Check),
  },
  CargoFlagSpec {
    flag: CargoFlag::MessageFormat,
    group: "changes",
    short: None,
    long: "--message-format",
    value: "",
    help: "short: name the files (-l); json: the JSON report (--emit json)",
    forwards: Forward::ByValue(&[
      ("short", &["-l"]),
      ("json", &["--emit", "json"]),
      ("human", &[]),
    ]),
  },
  CargoFlagSpec {
    flag: CargoFlag::AllowDirty,
    group: "history and safety",
    short: None,
    long: "--allow-dirty",
    value: "",
    help: "With history off, rewrite files that have uncommitted changes",
    forwards: Forward::Flag(Flag::AllowDirty),
  },
  CargoFlagSpec {
    flag: CargoFlag::NoHistory,
    group: "history and safety",
    short: None,
    long: "--no-history",
    value: "",
    help: "Do not record this run in history (it cannot be restored)",
    forwards: Forward::Flag(Flag::NoHistory),
  },
  CargoFlagSpec {
    flag: CargoFlag::Quiet,
    group: "output",
    short: Some("-q"),
    long: "--quiet",
    value: "",
    help: "Print only errors and what was asked for",
    forwards: Forward::Flag(Flag::Quiet),
  },
  CargoFlagSpec {
    flag: CargoFlag::Verbose,
    group: "output",
    short: Some("-v"),
    long: "--verbose",
    value: "",
    help: "List the targets, and name each file as it is formatted",
    forwards: Forward::Flag(Flag::Verbose),
  },
  CargoFlagSpec {
    flag: CargoFlag::Help,
    group: "general",
    short: Some("-h"),
    long: "--help",
    value: "",
    help: "Show this help and exit",
    forwards: Forward::Own,
  },
  CargoFlagSpec {
    flag: CargoFlag::Version,
    group: "general",
    short: None,
    long: "--version",
    value: "",
    help: "Print mto's version and exit",
    forwards: Forward::Own,
  },
];

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
    operand: Complete::Dirs,
    subcommands: &[],
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
    operand: Complete::Nothing,
    subcommands: &[
      Subcommand {
        word: "path",
        summary: "List the configuration files that apply, and the history folder",
        operand: Complete::Dirs,
      },
      Subcommand {
        word: "print",
        summary: "Show every setting in effect, with where it came from",
        operand: Complete::Dirs,
      },
      Subcommand {
        word: "options",
        summary: "List every setting a configuration file may hold",
        operand: Complete::Nothing,
      },
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
    operand: Complete::Nothing,
    subcommands: &[
      Subcommand {
        word: "show",
        summary: "List one run's files and where each stands now",
        operand: Complete::Nothing,
      },
      Subcommand {
        word: "prune",
        summary: "Remove old runs",
        operand: Complete::Nothing,
      },
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
    operand: Complete::Files,
    subcommands: &[],
    examples: &["mto restore last", "mto restore 3f9a src/lib.rs"],
  },
  CommandSpec {
    name: "doctor",
    args: "[on|off|holds|release|<RUN>]",
    summary: "Guard the build against mto's edits, or find the edit that broke it",
    about: "`on` turns the build guard on: each run that writes files then checks that the \
                project still builds, and holds back the edits that break it. It runs \
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
    operand: Complete::Nothing,
    subcommands: &[
      Subcommand {
        word: "on",
        summary: "Turn the build guard on",
        operand: Complete::Nothing,
      },
      Subcommand {
        word: "off",
        summary: "Turn the build guard off",
        operand: Complete::Nothing,
      },
      Subcommand {
        word: "holds",
        summary: "List the edits the guard holds back",
        operand: Complete::Nothing,
      },
      Subcommand {
        word: "release",
        summary: "Drop one held edit, or all",
        operand: Complete::Nothing,
      },
    ],
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
    operand: Complete::Nothing,
    subcommands: &[
      Subcommand {
        word: "show",
        summary: "Print one report",
        operand: Complete::Nothing,
      },
      Subcommand {
        word: "save",
        summary: "Write one report to a file",
        operand: Complete::Files,
      },
      Subcommand {
        word: "send",
        summary: "Open a new issue filled in with one report",
        operand: Complete::Nothing,
      },
      Subcommand {
        word: "prune",
        summary: "Remove every report",
        operand: Complete::Nothing,
      },
    ],
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
    operand: Complete::Nothing,
    subcommands: &[
      Subcommand {
        word: "tokens",
        summary: "Print the tokens of a file",
        operand: Complete::Files,
      },
      Subcommand {
        word: "tree",
        summary: "Print the syntax tree of a file",
        operand: Complete::Files,
      },
      Subcommand {
        word: "layout",
        summary: "Say where the formatter leaves code as written, and why",
        operand: Complete::Files,
      },
    ],
    examples: &["mto debug tree src/lib.rs", "mto debug layout src/lib.rs"],
  },
  CommandSpec {
    name: "completions",
    args: "[install|uninstall|status] [SHELL]",
    summary: "Print or install the completions for a shell",
    about: "Prints the completion script for SHELL: bash, zsh, fish or powershell. With no \
                SHELL, it is the shell mto runs in: the parent process when that is a shell, \
                else $SHELL. `install` writes the script where the shell loads completions \
                from (or into DIR), and names the line to add to the shell's startup file when \
                one is needed; mto never edits that file. Installing again changes nothing \
                while the script is current. `uninstall` removes a script mto wrote, and \
                nothing else. `status` says, for each shell, where its script is and whether \
                it is this mto's.",
    usage: &[
      "mto completions [SHELL]",
      "mto completions install [SHELL] [--dir DIR] [--dry-run]",
      "mto completions uninstall [SHELL] [--dir DIR] [--dry-run]",
      "mto completions status [SHELL]",
    ],
    options: &[(
      "options",
      "mto completions install|uninstall",
      &[Flag::Dir, Flag::DryRun],
    )],
    operand: Complete::Words(SHELLS),
    subcommands: &[
      Subcommand {
        word: "install",
        summary: "Write the script where the shell loads completions from",
        operand: Complete::Words(SHELLS),
      },
      Subcommand {
        word: "uninstall",
        summary: "Remove the script mto wrote",
        operand: Complete::Words(SHELLS),
      },
      Subcommand {
        word: "status",
        summary: "Say where each shell's script is, and whether it is current",
        operand: Complete::Words(SHELLS),
      },
    ],
    examples: &[
      "mto completions zsh",
      "mto completions install",
      "mto completions install fish --dry-run",
      "mto completions status",
    ],
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
    values: &[],
    help: "Print a diff for every file that would change; exit 1 if any",
  },
  FlagSpec {
    flag: Flag::DryRun,
    group: Group::Changes,
    short: Some("-d"),
    long: "--dry-run",
    value: "",
    values: &[],
    help: "List the files that would change, without writing",
  },
  FlagSpec {
    flag: Flag::Emit,
    group: Group::Changes,
    short: None,
    long: "--emit",
    value: "",
    values: &["files", "stdout", "json"],
    help: "Write the files (default), print them, or report changes as JSON",
  },
  FlagSpec {
    flag: Flag::FilesWithDiff,
    group: Group::Changes,
    short: Some("-l"),
    long: "--files-with-diff",
    value: "",
    values: &[],
    help: "Name the files that change (or would, with --check) instead of showing how",
  },
  FlagSpec {
    flag: Flag::FollowLinks,
    group: Group::Files,
    short: None,
    long: "--follow-links",
    value: "",
    values: &[],
    help: "Also follow symlinked files and directories",
  },
  FlagSpec {
    flag: Flag::Recursive,
    group: Group::Files,
    short: Some("-R"),
    long: "--recursive",
    value: "",
    values: &[],
    help: "Format every .rs file under the current directory",
  },
  FlagSpec {
    flag: Flag::SkipChildren,
    group: Group::Files,
    short: None,
    long: "--skip-children",
    value: "",
    values: &[],
    help: "A file argument is that file alone, not the modules it declares",
  },
  FlagSpec {
    flag: Flag::StdinFilepath,
    group: Group::Stdin,
    short: None,
    long: "--stdin-filepath",
    value: "<PATH>",
    values: &[],
    help: "With -: where the source belongs, for its settings and in messages",
  },
  FlagSpec {
    flag: Flag::Config,
    group: Group::Configuration,
    short: None,
    long: "--config",
    value: "<KEY=VALUE,...>",
    values: &[],
    help: "Set settings for this run, over every file",
  },
  FlagSpec {
    flag: Flag::ConfigPath,
    group: Group::Configuration,
    short: None,
    long: "--config-path",
    value: "<FILE>",
    values: &[],
    help: "Use only this configuration file; search for no other",
  },
  FlagSpec {
    flag: Flag::Edition,
    group: Group::Configuration,
    short: None,
    long: "--edition",
    value: "<YEAR>",
    values: &[],
    help: "The Rust edition, over Cargo.toml and configuration files",
  },
  FlagSpec {
    flag: Flag::PrintConfig,
    group: Group::Compatibility,
    short: None,
    long: "--print-config",
    value: "[PATH]",
    values: &["default", "minimal", "current"],
    help: "Print the formatting settings as the standard formatter prints them, and exit",
  },
  FlagSpec {
    flag: Flag::StyleEdition,
    group: Group::Configuration,
    short: None,
    long: "--style-edition",
    value: "<YEAR>",
    values: &[],
    help: "The style edition, which decides the defaults",
  },
  FlagSpec {
    flag: Flag::UnstableFeatures,
    group: Group::Configuration,
    short: None,
    long: "--unstable-features",
    value: "",
    values: &[],
    help: "Accepted for compatibility; mto applies every setting on every channel",
  },
  FlagSpec {
    flag: Flag::AllowDirty,
    group: Group::History,
    short: None,
    long: "--allow-dirty",
    value: "",
    values: &[],
    help: "With history off, rewrite files that have uncommitted changes",
  },
  FlagSpec {
    flag: Flag::Unguarded,
    group: Group::History,
    short: None,
    long: "--unguarded",
    value: "",
    values: &[],
    help: "Do not check this run with the build guard, nor hold back the edits it holds",
  },
  FlagSpec {
    flag: Flag::NoHistory,
    group: Group::History,
    short: None,
    long: "--no-history",
    value: "",
    values: &[],
    help: "Do not record this run in history (it cannot be restored)",
  },
  FlagSpec {
    flag: Flag::Color,
    group: Group::Output,
    short: None,
    long: "--color",
    value: "",
    values: &["always", "never", "auto"],
    help: "Color the output (default: auto, only on a terminal)",
  },
  FlagSpec {
    flag: Flag::Dbg,
    group: Group::Output,
    short: None,
    long: "--dbg",
    value: "",
    values: &[],
    help: "Print how each file was split into items",
  },
  FlagSpec {
    flag: Flag::Quiet,
    group: Group::Output,
    short: Some("-q"),
    long: "--quiet",
    value: "",
    values: &[],
    help: "Print only errors and what was asked for",
  },
  FlagSpec {
    flag: Flag::Verbose,
    group: Group::Output,
    short: Some("-v"),
    long: "--verbose",
    value: "",
    values: &[],
    help: "Name each file as it is formatted",
  },
  FlagSpec {
    flag: Flag::Help,
    group: Group::General,
    short: Some("-h"),
    long: "--help",
    value: "",
    values: &[],
    help: "Show this help and exit",
  },
  FlagSpec {
    flag: Flag::Man,
    group: Group::General,
    short: None,
    long: "--man",
    value: "",
    values: &[],
    help: "Print the full manual",
  },
  FlagSpec {
    flag: Flag::Version,
    group: Group::General,
    short: Some("-V"),
    long: "--version",
    value: "",
    values: &[],
    help: "Print the version and exit",
  },
  FlagSpec {
    flag: Flag::Current,
    group: Group::Command,
    short: None,
    long: "--current",
    value: "",
    values: &[],
    help: "Every setting in effect, with its source (the default)",
  },
  FlagSpec {
    flag: Flag::Default,
    group: Group::Command,
    short: None,
    long: "--default",
    value: "",
    values: &[],
    help: "Every setting at its default",
  },
  FlagSpec {
    flag: Flag::Minimal,
    group: Group::Command,
    short: None,
    long: "--minimal",
    value: "",
    values: &[],
    help: "Only the settings that differ from their defaults",
  },
  FlagSpec {
    flag: Flag::User,
    group: Group::Command,
    short: None,
    long: "--user",
    value: "",
    values: &[],
    help: "The user config's settings",
  },
  FlagSpec {
    flag: Flag::Force,
    group: Group::Command,
    short: None,
    long: "--force",
    value: "",
    values: &[],
    help: "Also restore files changed since the run, and missing ones",
  },
  FlagSpec {
    flag: Flag::Toml,
    group: Group::Command,
    short: None,
    long: "--toml",
    value: "",
    values: &[],
    help: "As a commented file, every setting at its default",
  },
  FlagSpec {
    flag: Flag::All,
    group: Group::Command,
    short: None,
    long: "--all",
    value: "",
    values: &[],
    help: "List every project's runs",
  },
  FlagSpec {
    flag: Flag::Keep,
    group: Group::Command,
    short: None,
    long: "--keep",
    value: "<N>",
    values: &[],
    help: "Keep only the newest N runs",
  },
  FlagSpec {
    flag: Flag::WithFile,
    group: Group::Command,
    short: None,
    long: "--with-file",
    value: "",
    values: &[],
    help: "Add the whole file the bug was found in",
  },
  FlagSpec {
    flag: Flag::Dir,
    group: Group::Command,
    short: None,
    long: "--dir",
    value: "<DIR>",
    values: &[],
    help: "Use this folder instead of the shell's own",
  },
  FlagSpec {
    flag: Flag::OlderThan,
    group: Group::Command,
    short: None,
    long: "--older-than",
    value: "<DAYS>",
    values: &[],
    help: "Remove runs this old: 30 (days), 2w, 12h",
  },
];

/// The shells `mto completions` writes scripts for.
pub const SHELLS: &[&str] = &["bash", "zsh", "fish", "powershell"];

pub const USAGE_LINES: &[&str] = &[
  "mto [OPTIONS] <PATH>...",
  "mto [OPTIONS] - [--stdin-filepath PATH]",
  "mto <COMMAND> [ARGS]",
  "cargo mto [OPTIONS] [-- <MTO OPTIONS>...]",
];

/// `cargo mto`'s flags.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CargoFlag {
  All,
  AllowDirty,
  Check,
  Help,
  ManifestPath,
  MessageFormat,
  NoHistory,
  Package,
  Quiet,
  Verbose,
  Version,
}

/// What a shell completes an argument to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Complete {
  /// Nothing it could know: a number, a run, an id.
  Nothing,
  /// Paths, Rust files first.
  Files,
  /// Folders.
  Dirs,
  /// One of these words.
  Words(&'static [&'static str]),
  /// One of a setting's legal values.
  Setting(&'static str),
  /// `KEY=VALUE` pairs: the settings `--config` takes, then after `=` the setting's values.
  Settings,
}

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
  Dir,
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

/// What a `cargo mto` flag becomes on mto's command line.
#[derive(Clone, Copy, Debug)]
pub enum Forward {
  /// Nothing: `cargo mto` acts on it itself.
  Own,
  /// This mto flag.
  Flag(Flag),
  /// For each value, the words mto is given instead.
  ByValue(&'static [(&'static str, &'static [&'static str])]),
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

/// One of `cargo mto`'s flags. Its parser, its help and the manual read [`CARGO_FLAGS`].
pub struct CargoFlagSpec {
  pub flag: CargoFlag,
  /// The heading help lists it under; groups are listed in the order they first appear.
  pub group: &'static str,
  pub short: Option<&'static str>,
  pub long: &'static str,
  /// The placeholder for an open value, as in [`FlagSpec::value`].
  pub value: &'static str,
  pub help: &'static str,
  /// What it becomes on mto's command line.
  pub forwards: Forward,
}

impl CargoFlagSpec {
  /// `-p, --package`, or `--check`.
  pub fn names(&self) -> String {
    match self.short {
      Some(short) => format!("{}, {}", short, self.long),
      None => self.long.to_string(),
    }
  }

  /// The values the flag takes, when they are a fixed set.
  pub fn values(&self) -> Vec<&'static str> {
    match self.forwards {
      Forward::ByValue(pairs) => pairs.iter().map(|(value, _)| *value).collect(),
      _ => Vec::new(),
    }
  }

  /// What follows the flag in help and the manual.
  pub fn value_text(&self) -> String {
    value_text(&self.values(), self.value)
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
  /// What its arguments complete to, after any subcommand word.
  pub operand: Complete,
  /// The words that may follow the name, each choosing what the command does. A command
  /// whose parser takes a word reads it from here, so an unlisted one is refused.
  pub subcommands: &'static [Subcommand],
  pub examples: &'static [&'static str],
}

impl CommandSpec {
  /// The flags that may follow `mto NAME`, or `mto NAME WORD` for a subcommand: those of
  /// each options section whose command names it (`mto config path|print` names both).
  pub fn flags_after(&self, word: Option<&str>) -> Vec<Flag> {
    let mut flags = Vec::new();
    for (_, after, section) in self.options {
      let rest = after
        .strip_prefix("mto ")
        .and_then(|rest| rest.strip_prefix(self.name))
        .unwrap_or("");
      let next = rest.split_whitespace().next().unwrap_or("");
      let words: Vec<&str> = next
        .split('|')
        .filter(|w| self.subcommand(Some(w)).is_some())
        .collect();
      // A section that names no subcommand belongs to the command itself.
      let applies = match word {
        Some(word) => words.contains(&word),
        None => words.is_empty(),
      };
      if applies {
        for flag in *section {
          if !flags.contains(flag) {
            flags.push(*flag);
          }
        }
      }
    }
    flags
  }

  /// The subcommand `word` names, when `word` is one of this command's.
  pub fn subcommand(&self, word: Option<&str>) -> Option<&'static str> {
    let word = word?;
    self
      .subcommands
      .iter()
      .find(|sub| sub.word == word)
      .map(|sub| sub.word)
  }
}

pub struct FlagSpec {
  pub flag: Flag,
  pub group: Group,
  pub short: Option<&'static str>,
  pub long: &'static str,
  /// The placeholder for an open value (`<PATH>`), or what follows the list of `values`
  /// (`--print-config`'s `[PATH]`).
  pub value: &'static str,
  /// Every value the flag takes, when they are a fixed set. The parser refuses any other,
  /// and help shows them as `<a|b|c>`.
  pub values: &'static [&'static str],
  pub help: &'static str,
}

impl FlagSpec {
  /// What a shell completes the flag's value to.
  pub fn completes(&self) -> Complete {
    match self.flag {
      _ if !self.values.is_empty() => Complete::Words(self.values),
      Flag::Config => Complete::Settings,
      Flag::ConfigPath | Flag::StdinFilepath => Complete::Files,
      Flag::Dir => Complete::Dirs,
      Flag::Edition => Complete::Setting("edition"),
      Flag::StyleEdition => Complete::Setting("style_edition"),
      _ => Complete::Nothing,
    }
  }

  /// Whether the flag is followed by a value.
  pub fn takes_value(&self) -> bool {
    !self.value.is_empty() || !self.values.is_empty()
  }

  /// What follows the flag in help and the manual: `<files|stdout|json>`, `<PATH>`, or
  /// nothing.
  pub fn value_text(&self) -> String {
    value_text(self.values, self.value)
  }
}

/// One of a command's subcommands: `show` in `mto history show <RUN>`.
pub struct Subcommand {
  pub word: &'static str,
  /// One line, for shells that describe what they complete.
  pub summary: &'static str,
  /// What its arguments complete to.
  pub operand: Complete,
}

/// The `cargo mto` flag spelled `arg`, long or short.
pub fn cargo_flag(arg: &str) -> Option<&'static CargoFlagSpec> {
  CARGO_FLAGS
    .iter()
    .find(|spec| spec.long == arg || spec.short == Some(arg))
}

pub fn cargo_spec_of(flag: CargoFlag) -> &'static CargoFlagSpec {
  CARGO_FLAGS
    .iter()
    .find(|spec| spec.flag == flag)
    .expect("every cargo mto flag has a spec")
}

/// The command named `name`.
pub fn command_named(name: &str) -> Option<&'static CommandSpec> {
  COMMANDS.iter().find(|command| command.name == name)
}

/// A flag as the manual names it: `-q, --quiet`, `--emit <files|stdout|json>`.
pub fn flag_name(spec: &FlagSpec) -> String {
  let mut name = match spec.short {
    Some(short) => format!("{}, {}", short, spec.long),
    None => spec.long.to_string(),
  };
  if spec.takes_value() {
    name.push(' ');
    name.push_str(&spec.value_text());
  }
  name
}

/// A group's flags, by name.
pub fn flags_of(group: Group) -> Vec<&'static FlagSpec> {
  let mut flags: Vec<&FlagSpec> = FLAGS.iter().filter(|spec| spec.group == group).collect();
  flags.sort_by_key(|spec| spec.long);
  flags
}

/// `a, b or c`: a flag's values, for messages.
pub fn listed(values: &[&str]) -> String {
  match values.split_last() {
    Some((last, rest)) if !rest.is_empty() => format!("{} or {}", rest.join(", "), last),
    Some((last, _)) => last.to_string(),
    None => String::new(),
  }
}

pub fn spec_of(flag: Flag) -> &'static FlagSpec {
  FLAGS
    .iter()
    .find(|spec| spec.flag == flag)
    .expect("every flag has a spec")
}

/// `<a|b|c>`, then `placeholder` after a space when there is one.
fn value_text(values: &[&str], placeholder: &str) -> String {
  if values.is_empty() {
    return placeholder.to_string();
  }
  let list = format!("<{}>", values.join("|"));
  if placeholder.is_empty() {
    list
  } else {
    format!("{} {}", list, placeholder)
  }
}
