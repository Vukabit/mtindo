//! The manual (`mto --man`, and `docs/generated/manual.md`): hand-written chapters, wrapped
//! here, and the command line and configuration generated from the tables the parser and
//! the settings checks read, so it cannot fall behind: every flag, command and exit code, and
//! every key with its file, type, legal values, defaults and description.

use super::spec::{COMMANDS, EXIT_CODES, Group, USAGE_LINES, flag_name, flags_of, spec_of};
use crate::options::{self, Channels, File, Kind, Setting, Stability};

/// How configuration is found and merged; kept beside the code in `config` that does it.
/// The settings reference in docs/generated/config/ reads it too.
pub const LOOKUP: &str = "\
mto reads the settings for each folder it formats from, lowest precedence first:

1. The defaults below, for the style edition in effect.
2. The nearest `.rustfmt.toml` or `rustfmt.toml`, found as the standard formatter \
finds it: in the folder, then each parent folder, then the home folder, then the \
standard formatter's folder in the user's configuration folder. Only that one \
file is used.
3. Every `.mto.toml` or `mto.toml` from the outermost folder down to the folder \
itself; a nearer file wins key by key. The search upwards stops at a file that \
sets `root = true`. An `mto.toml` may also set the formatting settings, and wins over \
`rustfmt.toml`, with a warning when the two disagree.
4. The edition of the package in the nearest `Cargo.toml` (inherited from the \
workspace when it says `edition.workspace = true`), as `cargo fmt` passes it.
5. The command line: `--config key=value`, `--edition` and `--style-edition`.

`--config-path FILE` replaces steps 2 and 3 with that one file, which may hold \
both mto's settings and the formatting settings. `mto config path` lists the files in effect \
and `mto config print` shows every value with where it came from.

The style edition is resolved as the standard formatter resolves it: `style_edition` if set, \
else `version` (`One` is 2015, `Two` is 2024), else `edition`, else 2015. It \
decides the defaults of the settings that change between style editions.

An unknown key, or a key in the wrong file, is a warning and is ignored. A value \
of the wrong type, or outside the legal values, is an error (exit code 2) naming \
the file, line and column. Values are matched without regard to case, as the \
standard formatter matches them.";

/// The per-user file; the settings reference reads it too.
pub const USER: &str = "\
The user configuration holds how mto behaves for you, never layout or \
formatting. It is `$MTINDO_CONFIG` when set (the file must exist), else \
`$XDG_CONFIG_HOME/mtindo/config.toml`, else `~/.config/mtindo/config.toml` on \
macOS and Linux and `%APPDATA%\\mtindo\\config.toml` on Windows. Project files \
cannot set user settings, and the user configuration cannot set project ones.";

/// The widest a manual line is wrapped to.
pub const WIDTH: usize = 80;

/// The manual's chapter on the build guard and `mto doctor`.
const DOCTOR: &str = r#"## The build guard: mto doctor

mto proves that each file it writes holds what it held, but not that the
project still builds: a reordered item can meet a macro or a build script
that depends on where it was. The build guard checks.

```bash
mto doctor on                    # guard with cargo check
mto doctor on -- cargo test -q   # or with any command
mto doctor                       # whether it is on, and what it holds
mto doctor off
```

It is off until you turn it on. Turned on, mto runs one command: the one
given to `mto doctor on`, else `cargo check` over the whole workspace and all
its targets. Turned off, it runs none. The setting lives in the user config,
under `[doctor]`, so no project can turn it on or change its command.

A run that writes files is guarded unless it names a single file, which is
how editors call mto, or reads stdin; `--unguarded` skips the guard for one
run. A guarded run:

1. Runs the command in the project's folder (the nearest above holding
   `.git`, else `Cargo.toml`) while it lays out and formats the files in
   memory. Nothing is written until it ends.
2. Writes the files, then runs the command again.
3. Compares the two. When the command passed before and fails now, the run
   broke the build. Errors are compared by their code and message
   (`error[E0425]: cannot find value in this scope`), not their lines, which
   formatting moves. A command that prints no errors the compiler's way is
   judged by its exit code alone.

A run that broke the build is taken apart. Its edits are the moves of items
(each item layout put somewhere else) and the changes formatting made (each
run of changed lines). mto narrows them down, running the command on each
candidate: first to the files that still break the build, then, within them,
one edit at a time until the build passes. It holds back as few edits as it
can find: the item stays where it was, the lines stay as they were written.
Every other edit is written. The run's record in history ends with what was
written, so `mto restore` undoes the run as usual.

```text
$ mto .
Formatted: ./src/lib.rs
Formatted: ./src/main.rs
doctor: before ✓  after ✗ error[E0433]: failed to resolve
doctor: 14 files, 23 edits: 9 checks
doctor: held back 1 edit:
  ./src/lib.rs: moved: mod generated (line 40, now before struct A)
doctor: the rest is written; check ✓
report: ~/.local/state/mtindo/doctor/reports/0192f3a1.md
```

The report says what each held edit was, the errors it caused, and how to
keep it for good without the guard: `// mto: pin` above a moved item,
`// mto: off` and `// mto: on` around changed lines. It goes when its run is
pruned from history, unless an edit it held is still held.

A narrowing runs the command at most `doctor.max_checks` times. Past that,
every changed file is held back whole, as no check can show what else is safe
to write: the files stay as they were before the run, and mto says so. When the build still fails with every edit held back, the
failure was not mto's (the command is flaky, or reads something else that
changed): the files are put back as they were, and mto says so (exit code
5).

A command that already fails before the run cannot guard it, as a crate that
does not build hides what an edit would break after it: mto writes nothing,
says why, and exits with code 5. Fix the build, give the guard a command that
passes, or pass `--unguarded`.

Held edits are remembered, in the doctor's folder next to history, so the
next run holds them back again without running the command, with the guard
off too; `--unguarded` writes them for one run. A hold lapses
when mto's build changes (the one `mto --version` names, which changes
between releases too) or the text it is about does; the guard then checks
again.

```bash
mto doctor holds        # every held edit, with how to keep it for good
mto doctor release 2    # drop one; release all drops every one
```

While a narrowing runs, the files hold its candidates, each recorded in the
run's history first. If it is cut short, the next run of mto puts the files
back as they were before the run, before anything else.

`mto doctor RUN` narrows down a run already made, guarded or not: RUN is
`last`, `last~N` or a handle from `mto history`. Its files must still hold
what it wrote. The command is the one after `--`, else the guard's; with the
guard off and no command given, it stops and says how to give one.
"#;

/// The manual's opening: what mto is and how it works. Hand-written Markdown, wrapped by
/// [`prose`].
const INTRO: &str = r#"# mtindo (mto)

`mtindo` is a formatter for Rust source files, with no dependencies. Its
command is `mto`.

Formatting does what the standard formatter does: spacing, indentation and
line breaks in the Rust style, changing only whitespace and the few tokens
the style rewrites. It is on by default, so with no configuration `mto` can
take the standard formatter's place; `format = false` in an `mto.toml` turns
it off.

Layout does what the standard formatter leaves alone: it reorders a file's
items (modules, imports, macros, constants, types with their impls,
functions) into a predictable top-down order, inside inline modules too, and
the items of impl blocks when asked. It moves items whole. Layout is opt-in,
with `layout = true` in an `mto.toml`; `mto init` writes one.

## How it works

- **Parsed, not guessed:** each file is parsed into a syntax tree that keeps
  every byte. A file with syntax errors is left untouched and reported as
  skipped, as is one nesting more than 1024 levels deep, which mto does not
  follow that far.
- **Checked before every write:** layout may change only the order of items
  and the blank lines between them, each item keeping its text and its
  comments; formatting may change only whitespace and the tokens the Rust
  style rewrites, and every comment must survive. Anything else stops the
  write (exit code 4), and mto keeps a bug report, named in one line:
  `mto report` shows and saves it (`report.save = false` prints it whole
  instead).
- **Settled in one run:** formatting mto's output again changes nothing.
- **Every level:** layout's rules apply inside inline `mod name { ... }`
  blocks, at every level; the items of impl blocks are ordered only when
  asked (`reorder_impl_items`, `impl_method_order`).
- **Keeps to the Rust style:** imports and `mod` declarations sort exactly as
  the standard formatter sorts them for the style edition, and blank lines
  stay within its bounds, so formatting never undoes the layout.
- **Impls stay with their type:** `impl Type` and `impl Trait for Type`
  blocks are kept directly beneath the type they implement, its own impls
  first, then trait impls by trait name.
- **Visibility, then name:** items of the same kind are ordered by visibility
  (`visibility_order`), then by name (`name_sort`); modules by name alone, as
  the standard formatter orders them.
- **Order-sensitive items stay put:** the file header (shebang, `//!`,
  `#![...]`), the order of `macro_rules!` definitions, `#[macro_use]`
  modules, `extern crate` and item-position macro calls are never moved past
  each other, and a macro never ends up below an item that followed it.
  `#[rustfmt::skip]` items and items under `// mto: pin` keep their place,
  and `#[cfg]` variants of one item move together.
- **Atomic writes:** a file is replaced by renaming a temp file beside it,
  on disk before the rename. Permissions are kept, and symlinked files stay
  symlinks.
- **History:** every run that writes files is recorded with each file's
  contents before it; `mto restore last` undoes the last one.
- **Check mode for CI:** `--check` prints a diff for every file that would
  change, and exits with code 1 if there are any.
- **Stdin and stdout:** `mto -` reads source from stdin and writes the
  result to stdout.

## Installation

From a clone of the repository:

```bash
cargo install --path .
```
"#;

/// The manual's chapter on how writes are kept safe and how to keep text in place.
const SAFETY: &str = r#"## Safety and history

A file is written in steps:

1. The new text is checked against the original before anything is written.
2. With history on (the default), the file's contents before are stored in
   the history folder (`mto config path` shows it), each content once however
   many runs share it, and the file is added to the run's record.
3. The new text is written to a temp file beside the original, and an atomic
   rename replaces the original.

Each step reaches the disk before the next, so a run cut short anywhere, even
by a power cut, leaves each file as it was or as written, and `mto restore`
can undo it. With `sync = "batch"` under `[safety]` (the default),
everything a run wrote is flushed to disk once, when it ends;
`sync = "full"` flushes each file before the next. Unchanged files are never
rewritten.

Each run that writes files is recorded under an id, shown by a short handle:

```bash
mto history            # this project's runs, newest first
mto history show last  # one run's files, and where each stands now
mto restore last       # undo this project's last run
mto restore 3f9a src/lib.rs
```

A restore puts back only files that still hold what the run wrote; `--force`
restores changed and missing files too. A restore is a run of its own, so it
can be undone. Runs are pruned after each run by the user config's
`[history]` settings, or with `mto history prune`, and the build guard's
reports of the runs pruned go with them.

With history off (`--no-history`, or `enabled = false` under `[history]`),
there is no undo, so mto leaves alone any file git sees uncommitted changes
in (exit code 3), unless `--allow-dirty` (or `allow_dirty = true` under
`[safety]`). git is used only if it is installed.

Text between `// mto: off` and `// mto: on` is never moved or changed. An
item directly under `// mto: on` stays inside the region; a region with no
`// mto: on` runs to the end of the file.
"#;

/// The configuration chapter of the manual, in the manual's Markdown.
pub fn configuration() -> String {
  let mut out = String::from("## Configuration\n");
  for paragraph in LOOKUP.split("\n\n") {
    out.push_str(&wrap_block(paragraph));
    out.push('\n');
  }
  out.push_str("### mto's settings\n");
  out.push_str(&wrap(
    "These go in `mto.toml` or `.mto.toml` only; the standard formatter would not \
         understand them.",
    "",
  ));
  for setting in options::all().filter(|s| s.file == File::Mto) {
    out.push_str(&entry(setting));
  }
  out.push_str("### Formatting settings\n");
  out.push_str(&wrap(
    &format!(
      "These go in `rustfmt.toml`, `.rustfmt.toml`, `mto.toml` or `.mto.toml`, \
             as the standard formatter {} documents them. mto reads and checks them all, \
             on every channel, so an existing `rustfmt.toml` works unchanged; the \
             standard formatter's own stability is noted for reference. \
             `mto config options` shows how far mto honors each.",
      options::RUSTFMT_VERSION
    ),
    "",
  ));
  for setting in options::all().filter(|s| s.file == File::Format && !s.hidden) {
    out.push_str(&entry(setting));
  }
  out.push_str("### Hidden formatting settings\n");
  out.push_str(&wrap(
    "The standard formatter leaves these out of its own listings: they are \
         deprecated, or set by the formatter itself. mto accepts them so that existing \
         files keep working.",
    "",
  ));
  for setting in options::all().filter(|s| s.file == File::Format && s.hidden) {
    out.push_str(&entry(setting));
  }
  out.push_str("### User settings\n");
  out.push_str(&wrap(USER, ""));
  let mut any = false;
  for setting in options::all().filter(|s| s.file == File::User) {
    out.push_str(&entry(setting));
    any = true;
  }
  if !any {
    out.push_str(&wrap(
      "There are no user settings yet, so mto does not create the file.",
      "",
    ));
  }
  out
}

/// The full manual (`mto --man`, and `docs/generated/manual.md`): the hand-written
/// chapters, and the command line, exit codes and configuration generated from the tables
/// the parser and the settings checks read.
pub fn render() -> String {
  let mut out = prose(INTRO);
  out.push_str("## Command line\n```text\n");
  for line in USAGE_LINES {
    out.push_str(line);
    out.push('\n');
  }
  out.push_str("```\n");
  out.push_str(&wrap(
    "`<PATH>`: a single `.rs` file or a directory. Directories are traversed \
     recursively, skipping `ignore_dirs`, symlinked entries, and folders that hold a \
     repository of their own (`.git`), which are named in a note; name one to format it. \
     Running `mto` with no arguments prints the short help.",
    "*   ",
  ));
  out.push_str(&wrap(
    "`mto -`: Read source from stdin and write the result to stdout; \
     `--stdin-filepath` says where it belongs.",
    "*   ",
  ));
  for group in Group::SHOWN {
    let heading = group.heading();
    out.push_str(&format!(
      "### {}{} (`{}`)\n",
      heading[..1].to_uppercase(),
      &heading[1..],
      group.command()
    ));
    for spec in flags_of(group) {
      out.push_str(&wrap(
        &format!("`{}`: {}.", flag_name(spec), spec.help),
        "*   ",
      ));
    }
  }
  out.push_str("### Commands\n");
  for command in COMMANDS {
    let usage = format!("mto {} {}", command.name, command.args);
    out.push_str(&wrap(
      &format!("`{}`: {}", usage.trim_end(), command.about),
      "*   ",
    ));
    for (_, after, flags) in command.options {
      for flag in *flags {
        let spec = spec_of(*flag);
        out.push_str(&wrap(
          &format!("`{}` (after `{}`): {}.", flag_name(spec), after, spec.help),
          "    *   ",
        ));
      }
    }
  }
  out.push_str("## Cargo\n");
  out.push_str(&wrap(
    "`cargo mto` formats the targets of the current package or workspace, as `cargo fmt` \
     does: every member when run from the workspace root, else the current package. Each \
     target's root file (library, binaries, examples, tests, benches, build script, as \
     Cargo discovers them) brings the modules it declares. The manifests are read \
     directly; Cargo is not run. Options after `--` go to `mto`.",
    "",
  ));
  for (_, flags, value, help) in super::CARGO_MTO_OPTIONS {
    let name = if value.is_empty() {
      flags.to_string()
    } else {
      format!("{} {}", flags, value)
    };
    out.push_str(&wrap(&format!("`cargo mto {}`: {}.", name, help), "*   "));
  }
  out.push_str("## Exit codes\n");
  for (code, meaning) in EXIT_CODES {
    out.push_str(&wrap(&format!("`{}`: {}.", code, meaning), "*   "));
  }
  out.push_str(&configuration());
  out.push_str(&prose(SAFETY));
  out.push_str(&prose(DOCTOR));
  spaced(&out)
}

/// `text` wrapped to [`WIDTH`], the first line after `lead` and the others indented to
/// match it.
pub fn wrap(text: &str, lead: &str) -> String {
  let indent = " ".repeat(lead.chars().count());
  let mut out = String::new();
  let mut line = lead.to_string();
  let mut empty = true;
  for word in unbreakable_words(text) {
    let word = word.as_str();
    if !empty && line.chars().count() + 1 + word.chars().count() > WIDTH {
      out.push_str(line.trim_end());
      out.push('\n');
      line = indent.clone();
      empty = true;
    }
    if !empty {
      line.push(' ');
    }
    line.push_str(word);
    empty = false;
  }
  out.push_str(line.trim_end());
  out.push('\n');
  out
}

/// A paragraph, or a numbered list item, wrapped and followed by its line break.
pub fn wrap_block(paragraph: &str) -> String {
  let mut out = String::new();
  for item in paragraph.split('\n') {
    match item.split_once(". ") {
      Some((number, rest)) if number.chars().all(|c| c.is_ascii_digit()) => {
        let lead = format!("{}. ", number);
        let indent = " ".repeat(lead.len());
        let wrapped = wrap(rest, &indent);
        out.push_str(&lead);
        out.push_str(&wrapped[indent.len()..]);
      }
      _ => match item.strip_prefix("- ") {
        Some(rest) => out.push_str(&wrap(rest, "*   ")),
        None => out.push_str(&wrap(item, "")),
      },
    }
  }
  out
}

fn default_line(setting: &Setting) -> String {
  if setting.default.is_empty() {
    return "Default: set by the standard formatter itself; mto ignores the value.".to_string();
  }
  match setting.default_2024 {
    Some(later) => format!(
      "Default: `{}`; `{}` from style edition 2024.",
      setting.default, later
    ),
    None => format!("Default: `{}`.", setting.default),
  }
}

/// One setting: heading, description, then its facts as a list.
fn entry(setting: &Setting) -> String {
  let mut out = format!("#### `{}`\n", setting.name);
  for paragraph in setting.doc.split("\n\n") {
    for line in paragraph.lines() {
      out.push_str(&wrap_block(line));
    }
    out.push('\n');
  }
  out.push_str(&wrap(&format!("Type: {}.", type_name(setting)), "*   "));
  out.push_str(&wrap(&default_line(setting), "*   "));
  if let Some(channels) = setting.channels {
    out.push_str(&wrap(
      &format!("In the standard formatter: {}.", stability(channels)),
      "*   ",
    ));
  }
  if let Some(replacement) = setting.replaced_by {
    out.push_str(&wrap(
      &format!("Deprecated: use `{}`.", replacement),
      "*   ",
    ));
  }
  let choices = setting.choices();
  if !choices.is_empty() {
    out.push_str("*   Values:\n");
    for choice in choices {
      let mut text = format!("`{}`", choice.name);
      if !choice.doc.is_empty() {
        text.push_str(&format!(": {}", choice.doc));
      }
      if let Some(channels) = choice.channels
        && setting.channels.is_some_and(|c| c != channels)
      {
        text.push_str(&format!(
          " (in the standard formatter: {})",
          stability(channels)
        ));
      }
      out.push_str(&wrap(&text, "    *   "));
    }
  }
  out
}

/// Hand-written Markdown wrapped to [`WIDTH`]: each paragraph, `- ` item and numbered item
/// re-flowed, so the source may break its lines anywhere; headings and code blocks as
/// written.
fn prose(text: &str) -> String {
  let mut out = String::new();
  let mut items: Vec<String> = Vec::new();
  let mut code = false;
  let flush = |items: &mut Vec<String>, out: &mut String| {
    if !items.is_empty() {
      out.push_str(&wrap_block(&items.join("\n")));
      items.clear();
    }
  };
  for line in text.lines() {
    let fence = line.starts_with("```");
    if fence || code || line.starts_with('#') {
      flush(&mut items, &mut out);
      code ^= fence;
      out.push_str(line);
      out.push('\n');
      continue;
    }
    let trimmed = line.trim();
    let starts_item = trimmed.starts_with("- ")
      || trimmed
        .split_once(". ")
        .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    if trimmed.is_empty() {
      flush(&mut items, &mut out);
      out.push('\n');
    } else if let Some(last) = items.last_mut().filter(|_| !starts_item) {
      last.push(' ');
      last.push_str(trimmed);
    } else {
      items.push(trimmed.to_string());
    }
  }
  flush(&mut items, &mut out);
  out
}

/// The manual with a blank line before every heading, so sections and entries stand apart
/// in a terminal. Code blocks are left as they are.
fn spaced(manual: &str) -> String {
  let mut out = String::new();
  let mut in_code = false;
  for line in manual.lines() {
    if line.starts_with("```") {
      in_code = !in_code;
    }
    let heading =
      !in_code && line.starts_with('#') && line.trim_start_matches('#').starts_with(' ');
    if heading && !out.is_empty() && !out.ends_with("\n\n") {
      out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
  }
  out
}

fn stability(channels: Channels) -> &'static str {
  match (channels.stable, channels.nightly) {
    (Stability::Stable, _) => "stable",
    (Stability::Unstable, _) => "unstable, nightly only",
    (Stability::Absent, Stability::Absent) => "not in the pinned releases",
    (Stability::Absent, _) => "nightly releases only",
  }
}

fn type_name(setting: &Setting) -> &'static str {
  match setting.kind {
    Kind::Bool => "`true` or `false`",
    Kind::Integer => "a whole number, zero or more",
    Kind::Text => "a string",
    Kind::Size => "a size such as `\"256MB\"`",
    Kind::Choice(_) => "one of the values below, as a string",
    Kind::List => "an array of strings",
    Kind::Sequence(_) => "an array of the values below, each at most once",
    Kind::Internal => "any value",
  }
}

/// The words of `text`, with a backticked span that holds spaces kept as one word. A
/// backtick with no partner is an ordinary character, and a span longer than half a line
/// may break.
fn unbreakable_words(text: &str) -> Vec<String> {
  let raw: Vec<&str> = text.split_whitespace().collect();
  let ticks = |word: &str| word.matches('`').count() % 2 == 1;
  let mut words = Vec::new();
  let mut idx = 0;
  while idx < raw.len() {
    let mut word = raw[idx].to_string();
    let close = (idx + 1..raw.len()).find(|&j| ticks(raw[j]));
    // A span too long for a line of its own still breaks.
    let fits =
      |close: usize| raw[idx..=close].iter().map(|w| w.len() + 1).sum::<usize>() <= WIDTH / 2;
    if ticks(raw[idx])
      && let Some(close) = close.filter(|&c| fits(c))
    {
      for part in &raw[idx + 1..=close] {
        word.push(' ');
        word.push_str(part);
      }
      idx = close;
    }
    words.push(word);
    idx += 1;
  }
  words
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_every_key_and_value_is_documented() {
    let manual = configuration();
    for setting in options::all() {
      let heading = format!("#### `{}`\n", setting.name);
      let at = manual
        .find(&heading)
        .unwrap_or_else(|| panic!("{} has no entry", setting.name));
      let rest = &manual[at + heading.len()..];
      let body = &rest[..rest.find("\n#").unwrap_or(rest.len())];
      for choice in setting.choices() {
        assert!(
          body.contains(&format!("`{}`", choice.name)),
          "{} = {} is not documented",
          setting.name,
          choice.name
        );
      }
      assert!(body.contains("Default: "), "{}", setting.name);
    }
  }

  #[test]
  fn test_lines_fit_the_width() {
    for line in configuration().lines() {
      // A single word longer than the width cannot be broken.
      let words = line.split_whitespace().count();
      assert!(line.chars().count() <= WIDTH || words <= 2, "{}", line);
    }
  }

  #[test]
  fn test_wrap() {
    let text = "one two three";
    assert_eq!(wrap(text, "*   "), "*   one two three\n");
    let long = "word ".repeat(30);
    let wrapped = wrap(&long, "*   ");
    let lines: Vec<&str> = wrapped.lines().collect();
    assert!(lines[0].starts_with("*   word"));
    assert!(lines[1].starts_with("    word"));
    assert!(lines.iter().all(|l| l.len() <= WIDTH));
    let code = format!("{} `version = \"One\"` end", "x ".repeat(36));
    assert!(wrap(&code, "").contains("\n`version = \"One\"` end"));
    assert_eq!(
      wrap_block("1. first item\n2. second"),
      "1. first item\n2. second\n"
    );
  }
}
