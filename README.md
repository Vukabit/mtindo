# mtindo

`mtindo` lays out Rust source files. It reorders a file's items (modules,
imports, macros, constants, types with their impls, functions) into a
predictable top-down order: the vertical organization of a file, which the
standard Rust formatter (see [Credits](#credits)) leaves alone. No more
scrolling past 800 lines of helper functions just to find the main `struct`
definition.

Its command is `mto`, and it has no dependencies. Layout moves items whole:
it changes nothing within them but their blank lines, the order of impl
items it was asked to sort, and the bodies of inline modules, which it lays
out the same way. Every write is checked first, so the code still means
what it meant.

`mto` also formats, and does by default: spacing, indentation and line
breaks in the Rust style, as the standard formatter writes them. See
[Formatting](#formatting).

## Core Features

* **Parsed, Not Guessed:** Every file is parsed into a syntax tree that keeps
  every byte. A file with syntax errors is left untouched and reported as
  skipped.
* **Items Move Whole:** Layout moves each item byte for byte. Only the order
  of items and the blank lines between them change, at the top level and
  inside inline `mod name { ... }` blocks, at every level. Line endings (LF or
  CRLF) and a leading byte-order mark are preserved.
* **Keeps to the Rust Style:** Imports and `mod` declarations are sorted
  exactly as the standard formatter sorts them for the style edition, and blank
  lines stay within its bounds, so formatting after `mto` leaves the layout
  alone. This is checked on 44 real-world repositories.
* **Sticky Impl Grouping:** `impl Target` and `impl Trait for Target` blocks
  stay directly beneath the type they implement: the type's own impls first,
  then trait impls by trait name (or a list of your choice).
* **Visibility Hierarchy:** Items of the same kind are ordered by visibility,
  widest first (`pub`, `pub(crate)`, `pub(in path)`, `pub(super)`, private),
  then by name with the Rust style guide's version sorting (`u8` before `u16`).
  Modules are the exception: like the standard formatter, mto orders them by
  name alone.
* **Comments Travel With Their Item:** A comment directly above an item, or at
  the end of its last line, moves with it.
* **Order-Sensitive Items Stay Put:** See [What mto never
  moves](#what-mto-never-moves).
* **Atomic Writes:** Files are replaced by an OS-level rename of a fully
  written sibling temp file, on disk before the rename. Permissions are kept,
  and a symlinked file stays a symlink.
* **History:** Every run that writes files is recorded, so
  `mto restore last` undoes it. See [History and
  restore](#history-and-restore).
* **Safe Directory Walks:** Symlinked directories are skipped unless
  `--follow-links` is given, and every real path is visited at most once, so
  symlink cycles cannot cause repeated work. A folder below the starting one
  that holds a repository of its own (`.git`) is another project: the walk
  passes over it and says so; name it to lay it out.

## Installation

`mtindo` relies entirely on the Rust standard library and needs Rust 1.88 or
later to build:

```bash
cargo install mtindo
```

This builds the `mto` and `cargo-mto` binaries in release mode and places them
in Cargo's binary directory (`~/.cargo/bin`), which should already be on your
`PATH`. The second makes `cargo mto` work. To build from a clone of the
repository instead, run `cargo install --path .` in its root.

## Turning layout on

Layout is opt-in. Until an `mto.toml` with `layout = true` applies to a file,
`mto` moves none of its items; it only formats it, as the standard formatter
would. To opt in, run this in the project root:

```bash
mto init
```

It writes an `mto.toml` with `layout = true` and every other setting at its
default.

## Formatting

Formatting is on by default: with no configuration, `mto` formats a file in
the Rust style exactly as the standard formatter does with the same
settings, so it can take the standard formatter's place in scripts, CI and
editors. It reads `rustfmt.toml` and honors its settings; `format = false`
in an `mto.toml` turns formatting off and leaves only layout.

Like the standard formatter, it leaves as written what it would give up on:
`macro_rules!` definitions, macro calls whose arguments are not code,
anything marked `#[rustfmt::skip]`, `// mto: off` regions and a statement or
item that cannot fit within `max_width`. `mto config options` shows how far
each of the standard formatter's settings is honored.

Formatting changes whitespace and the few tokens the Rust style rewrites (a
trailing comma, the order of imports), and a meaning check runs on every
formatted file before anything is written: the tokens must match the
input's, apart from those rewrites. A second run changes nothing.

## Command Line Interface

You can pass a single file or a whole directory. Directories are traversed
recursively, skipping the folders listed in `ignore_dirs`.

```bash
# Format a single file
mto src/main.rs

# Format an entire project
mto src/

# List the files that would change, without writing
mto --dry-run src/

# Show a diff of every file that would change, and fail if there are any
mto --check src/

# Read from stdin, write to stdout
cat src/main.rs | mto -
```

Running `mto` with no arguments prints the short help, the same as `mto -h`,
whether or not anything is piped in. Reading stdin always needs the explicit
`-`.

### Options

`mto` takes the standard formatter's flags, so it can stand in for it in
scripts and editors. Several paths may be given. A file brings the modules it
declares (`mod name;`, with `#[path]`), as the formatter follows them; a
directory is walked.

* `--check`: Print a diff for every file that would change, in the standard
  formatter's format (`Diff in FILE:LINE:`), and exit with code `1` if there
  are any.
* `--emit files|stdout|json`: Write the files (the default), print them, or
  print the JSON report of what would change.
* `-l, --files-with-diff`: Name the files that change, or would with
  `--check`, instead of showing how.
* `--skip-children`: A file argument is that file alone.
* `--config <KEY=VALUE,...>`: Set settings for this run:
  `--config max_width=80,hard_tabs=true`.
* `--config-path <FILE>`: Use only this configuration file.
* `--edition <YEAR>`, `--style-edition <YEAR>`: Set the edition and the style
  edition, which decides the defaults.
* `--print-config default|minimal|current [PATH]`: Print the formatting
  settings as the standard formatter does (to PATH for `default` and
  `minimal`).
* `-q, --quiet`, `-v, --verbose`: Print less, or name each file.
* `--color always|never|auto`: Color the output.
* `--unstable-features`: Accepted; mto applies every setting on every channel.
* `--stdin-filepath <PATH>`: With `-`, where the source belongs, for its
  settings and in messages.
* `-d, --dry-run`: List the files that would change without writing to disk.
* `-R, --recursive`: Lay out every `.rs` file under the current directory.
* `--follow-links`: Also follow symlinked files and directories.
* `--no-history`: Do not record this run, so it cannot be restored.
* `--allow-dirty`: With history off, rewrite files that have uncommitted
  changes.
* `--dbg`: Print how each file was split into items.
* `--man`: Print the manual, which documents every setting and its values.
* `-h, --help`: Print the short help menu.
* `-V, --version`: Print the version.

`--backup` is refused with an explanation: every run is recorded in
history instead. The old `--restore BACKUP TARGET` is gone too; `mto restore`
replaces it.

Output is colored only when it goes to a terminal, unless `--color` says
otherwise. Set `NO_COLOR` to turn color off, or `FORCE_COLOR` to turn it on;
`NO_COLOR` wins if both are set.

### Cargo

`cargo mto` lays out a package or workspace the way `cargo fmt` formats it:
every member when run from the workspace root, otherwise the current package.
Each target (library, binaries, examples, tests, benches, build script, as Cargo
discovers them) brings the modules it declares. `mto` reads the manifests
itself; Cargo is not run.

```bash
cargo mto --check          # every target; exit 1 if any file would change
cargo mto -p mycrate       # one member (repeatable)
cargo mto --all            # also local path dependencies outside the workspace
cargo mto -- --config max_width=80   # options after -- go to mto
```

It also takes `--manifest-path`, `--message-format short|json|human`, `-q` and
`-v`, as `cargo fmt` does, and `--no-history` and `--allow-dirty`.

### Exit Codes
| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | `--check` found files that would change; `mto restore` skipped files |
| 2 | Bad arguments or configuration |
| 3 | A file could not be read, written or parsed, or the git guard kept it |
| 4 | A bug in mto, kept as a report; the file is unchanged |
| 5 | The build guard's command failed with every edit held back |

## Every write is checked

Before `mto` writes a file, it parses the old and the new text and checks
that the code means what it did. Layout may change only the order of items
and the blank lines between them: every item keeps its exact text and its
comments, every comment survives, the file header stays first,
`macro_rules!` keep their order, and the byte-order mark and line endings
are kept. Formatting may change only whitespace and the tokens the Rust
style rewrites. If anything else would change, the file is left alone,
`mto` keeps a bug report and names it in one line, and it exits with code
`4`; laying out a file that fails (a panic) is reported the same way, and
the rest of the run goes on.

### Bug reports

A report holds what failed, the smallest part of the file that still fails
(never the whole file), the command, the settings that differ from the
defaults, and the build. The same bug seen again is counted, not kept twice.
Reports are kept beside history; one not seen for 90 days goes by itself.

```bash
mto report                     # every report, the latest first
mto report show 7f3a2c         # one report, as it would be filed
mto report save 7f3a2c bug.md  # --with-file adds the whole file
mto report send 7f3a2c         # a new issue, filled in, in the browser
mto report prune               # remove them all
```

`mto report send` opens the issue for you to read over and submit: `mto`
itself sends nothing. `save = false` under `[report]` in the user config
prints the whole report instead and keeps none.

## What mto never moves

Some items depend on their position in the file. `mto` keeps them in place:

* **The file header:** a shebang line, inner doc comments (`//!`, `/*! */`),
  inner attributes (`#![...]`), and a leading comment block separated from the
  first item by a blank line (such as a license header) always stay first.
* **`macro_rules!` definitions** keep their original order relative to each
  other, and never end up below an item that originally came after them, since a
  macro is only visible to code that follows it. When the configured order would
  break that, macros are placed first.
* **A file that defines the same macro name twice** is skipped, because there
  the order of definitions changes behavior.
* **`#[macro_use]` modules** move ahead of the other modules, in their original
  order, so every module that used their macros still can; like `macro_rules!`,
  nothing that came after one moves in front of it.
* **`extern crate` declarations and item-position macro calls** (`thread_local!
  { .. }`, `include!(..)`, your own `make_items!();`) stay at their original
  position in the file.
* **Items marked to stay:** `#[rustfmt::skip]` items, and items under a
  `// mto: pin` comment, keep their position. A file whose header has
  `#![rustfmt::skip]` is left alone.
* **`#[cfg]` variants** of one item (`#[cfg(unix)] fn f` and
  `#[cfg(windows)] fn f`, written next to each other) move as one.
* **A `#[cfg(test)]` module** goes last (or stays where it is, with
  `test_module = "in_place"`).
* **Anything between `// mto: off` and `// mto: on`** is left exactly as
  written, in its original position. An item directly under `// mto: on`
  (no blank line between) stays inside the region, and a region with no
  `// mto: on` runs to the end of the file.
* **A file with syntax errors** is left unchanged, and the run exits with code
  `3`.
* **A file nesting more than 1024 levels deep** (generated code; hand-written
  code stays far below) is skipped and left unchanged, as mto does not follow
  it that far.

## Configuration

`mto` reads `rustfmt.toml` as the standard formatter does, so an existing one
works unchanged, and its own settings from `mto.toml`. For each folder it
formats, the settings come from, lowest precedence first:

1. The defaults, for the style edition in effect.
2. The nearest `.rustfmt.toml` or `rustfmt.toml`, found as the standard
   formatter finds it: the folder, each parent folder, the home folder, then
   the formatter's folder in the user's configuration folder. Only that one
   file is used.
3. Every `.mto.toml` or `mto.toml` from the outermost folder down; a nearer
   file wins key by key, and a file with `root = true` stops the search
   upwards. An `mto.toml` may also set the formatting settings, and wins over
   `rustfmt.toml` with a warning when they disagree.
4. The package edition in the nearest `Cargo.toml`, as `cargo fmt` passes it.
5. The command line: `--config`, `--edition`, `--style-edition`.

`--config-path FILE` replaces steps 2 and 3 with that one file.
`mto config path` lists the files in effect and `mto config print` shows every
value with where it came from (`--minimal` shows only what differs from the
defaults). `mto config options` lists every setting a file may hold, by file,
with its values and default, and for the standard formatter's settings
whether mto honors it yet; `--toml` prints them as a commented file.

Unknown keys are warnings. A value of the wrong type or outside the legal
values stops the run with exit code `2`, naming the file, line and column.
`mto --man` documents every setting, the formatting settings and mto's, with
its legal values and defaults; so does [docs/config/](docs/config/README.md),
with how far mto honors each one and a JSON Schema of `mto.toml` for editors.
With `format = false`, the formatting settings change nothing except the
style edition.

`mto init` writes an `mto.toml` with layout on and mto's other settings at
their defaults, each with a comment; without the comments:

```toml
layout = true
format = true
blank_lines_between_blocks = 1
group_impls = true
ignore_dirs = [".git", "target", "node_modules", "vendor", "dist", ".venv"]
impl_method_order = "written"
import_grouping = true
import_sort = "style_edition"
name_sort = "version"
sort_alphabetically = true
sort_imports = true
test_module = "last"
trait_impl_order = []
trait_impl_sort = "name"
type_order = ["ModDecl", "UseDecl", "MacroDecl", "MacroDef", "ConstDecl",
    "StaticDecl", "TypeAlias", "TraitDecl", "EnumDecl", "StructDecl", "Union",
    "ImplBlock", "ExternBlock", "FnDecl", "Unknown"]
visibility_order = ["Pub", "PubCrate", "PubIn", "PubSuper", "Private"]
```

`reorder_impl_items` is honored inside impl blocks, exactly as the standard
formatter applies it. `tab_width` was renamed `tab_spaces`, the standard name.

## Safety and Recovery

1. **Refusal:** If a file has syntax errors, or cannot be split into items with
   certainty, it is skipped and left byte-for-byte unchanged.
2. **Verification:** The new text must mean what the old one did (see
   [Every write is checked](#every-write-is-checked)); anything else stops
   the write.
3. **Record:** The file's contents before the run are stored in history, and
   the file is added to the run's record, before the file changes.
4. **Atomic Write:** The new content is written to a temp file next to the
   target, then renamed over it in one step.

Each step reaches the disk before the next, so a run cut short anywhere, even
by a crash or a power cut, leaves every file as it was or as written, and
`mto restore` can undo whatever it changed. Unchanged files are never
rewritten.

By default (`sync = "batch"` under `[safety]` in the user config) the steps are
kept in order as they are written, and everything a run wrote is flushed to
disk once, when it ends, as git's `core.fsyncMethod = batch` does: on macOS,
where a full flush costs milliseconds, a run writing hundreds of files takes a
fraction of the time. `sync = "full"` flushes each file before the next.

### History and restore

Every run that writes files is recorded under an id, shown by a short handle
(the end of the id). Contents are stored once however many runs share them.

```bash
mto history                  # this project's runs, newest first
mto history --all            # every project's
mto history show last        # one run's files, and where each stands now
mto restore last             # undo this project's last run
mto restore last~1           # the one before it
mto restore 3f9a src/lib.rs  # one file of run 3f9a
mto history prune --keep 10  # also --older-than 30 (days; or 2w, 12h)
```

History is shared by every project. A run belongs to the project it wrote
files in: the nearest folder above holding `.git`, else `Cargo.toml`. Bare
`mto history` lists the current project's runs, with a `PROJECT` column, or
every run with a note when the project has none. `last` and `last~N` count
the current project's runs too, so a restore never reaches into another
project; a handle names its run from anywhere.

A restore puts back only the files that still hold what the run wrote; one
edited or deleted since is skipped (exit code `1`) unless `--force`. A restore
is a run of its own, so it can be undone too.

History lives in `~/.local/state/mtindo/history` (`$XDG_STATE_HOME` is
honored; `%LOCALAPPDATA%\mtindo\history` on Windows); `mto config path` shows
it. After each run it is pruned to the user config's limits.

With history off there is nothing to undo a run with, so `mto` leaves alone any
file that git sees uncommitted changes in, and says so (exit code `3`).
`--allow-dirty` lets it through. git is used only if it is installed;
without it, the guard is skipped with a note.

### The build guard

The verifier proves each file holds what it held, not that the project
still builds. `mto doctor on` turns on a guard that checks: each run that
writes files runs a command before and after, `cargo check --workspace
--all-targets --keep-going` unless you give another, and holds back the
fewest edits that break the build.

```bash
mto doctor on                  # or: mto doctor on -- cargo test -q
mto .                          # checked before and after
mto doctor holds               # edits held back, and how to keep them
mto doctor release all
mto doctor last -- cargo build # narrow down a run already made
mto doctor off
```

When a run breaks the build, mto narrows its edits down, running the
command on each candidate: to the fewest files, then to the fewest moved
items and changed lines in them. It writes every other edit, checks once
more, and writes a report naming each held edit, the errors it caused and
how to keep it (`// mto: pin`, `// mto: off`); reports are pruned with
the runs they belong to. Held edits stay held on later runs, without the
command, until mto's build or the text changes.

A tree whose command already fails before the run cannot be guarded: a
crate that does not build hides what an edit would break after it. mto
writes nothing, says why, and exits with code `5`. Fix the build, give the
guard a command that passes (`mto doctor on -- COMMAND`), or pass
`--unguarded` for one run.

mto runs the command only with the guard on, and only that command. The
setting lives in the user config, never in a project's files, so no
repository can make mto run anything. A run naming a single file (how
editors call mto) or reading stdin is not guarded; neither is one given
`--unguarded`.

### The user config

How `mto` behaves for you, on every project, is set in
`~/.config/mtindo/config.toml` (`$XDG_CONFIG_HOME` is honored;
`%APPDATA%\mtindo\config.toml` on Windows; `MTINDO_CONFIG` names another
file). It is created on the first run, with every setting commented out at its
default, except when `CI` is set. It holds no layout or formatting settings,
and project files cannot hold its settings.

```toml
[history]
enabled = true      # record every run that writes files
keep_days = 30      # 0: no limit; the newest run is always kept
keep_runs = 100
max_size = "256MB"  # bytes, or KB, MB, GB

[output]
color = "auto"      # always, never; --color, NO_COLOR, FORCE_COLOR win

[report]
save = true         # keep bug reports for `mto report`; false: print them

[safety]
allow_dirty = false # with history off, overwrite uncommitted changes
sync = "batch"      # or "full": flush each file to disk as it is written

[doctor]
enabled = false     # mto doctor on/off; the build guard
command = ["cargo", "check", "--workspace", "--all-targets", "--keep-going"]
timeout = 600       # seconds a check may take; 0: no limit
max_checks = 64     # checks one narrowing may run
```

`mto config print --user` shows each value and where it came from.

## Where it's headed

`mto` is growing into a complete Rust formatter: output in the Rust style,
identical to the standard formatter's by default, plus per-team control over
file layout, still with no dependencies and nothing else to install. Until
then, run `mto` together with the standard formatter; formatting after `mto`
is expected to leave the layout alone.

## Development

`cargo xtask test` runs every gate and test suite. How to build, test, fuzz,
benchmark and watch the project, and every development command, are in
[CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
* MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. The identifier and width tables in `src/text/unicode` are
derived from the Unicode Character Database, under the terms in
[LICENSE-UNICODE](LICENSE-UNICODE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.

## Credits

`mto` takes after [rustfmt](https://github.com/rust-lang/rustfmt) in its
formatting rules, its settings and its command line, and checks itself
against rustfmt's own tests. Thanks to its authors and maintainers.
