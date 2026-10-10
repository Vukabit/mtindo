# mtindo

`mtindo` is a dependency-free layout tool for Rust source files. Its command is
`mto`.

`rustfmt` handles spacing, indentation and line wrapping inside items, but it
leaves the vertical organization of a file alone. `mto` handles that part: it
reorders the top-level items of a file (modules, imports, macros, constants,
types with their impls, functions) into a predictable top-down layout. It never
changes the inside of an item.

No more scrolling past 800 lines of helper functions just to find the main
`struct` definition.

## Core Features

* **Parsed, Not Guessed:** Every file is parsed into a syntax tree that keeps
  every byte. A file with syntax errors is left untouched and reported as
  skipped.
* **Item Bodies Are Never Touched:** Each item is moved byte for byte. Only the
  order of items and the blank lines between them change, at the top level and
  inside inline `mod name { ... }` blocks, at every level. Line endings (LF or
  CRLF) and a leading byte-order mark are preserved.
* **Agrees With rustfmt:** Imports and `mod` declarations are sorted exactly as
  rustfmt sorts them for the style edition, and blank lines stay within
  rustfmt's bounds, so running `rustfmt` after `mto` leaves the layout alone.
  This is checked on 44 real-world repositories.
* **Sticky Impl Grouping:** `impl Target` and `impl Trait for Target` blocks
  stay directly beneath the type they implement: the type's own impls first,
  then trait impls by trait name (or a list of your choice).
* **Visibility Hierarchy:** Items of the same kind are ordered by visibility,
  widest first (`pub`, `pub(crate)`, `pub(in path)`, `pub(super)`, private),
  then by name with the Rust style guide's version sorting (`u8` before `u16`).
  Modules are the exception: like rustfmt, mto orders them by name alone.
* **Comments Travel With Their Item:** A comment directly above an item, or at
  the end of its last line, moves with it.
* **Order-Sensitive Items Stay Put:** See [What mto never
  moves](#what-mto-never-moves).
* **Atomic Writes:** Files are replaced by an OS-level rename of a fully
  written, synced sibling temp file. Permissions are kept, and a symlinked file
  stays a symlink.
* **History:** Every run that writes files is recorded, so
  `mto restore last` undoes it. See [History and
  restore](#history-and-restore).
* **Safe Directory Walks:** Symlinked directories are skipped unless
  `--follow-links` is given, and every real path is visited at most once, so
  symlink cycles cannot cause repeated work.

## Installation

`mtindo` relies entirely on the Rust standard library. Clone this repository,
navigate to the root directory, and run:

```bash
cargo install --path .
```

This builds the `mto` and `cargo-mto` binaries in release mode and places them
in Cargo's binary directory (`~/.cargo/bin`), which should already be on your
`PATH`. The second makes `cargo mto` work.

## Turning layout on

Layout is opt-in. Until an `mto.toml` with `layout = true` applies to a file,
`mto` leaves it alone and says so, so running it on a project that has not
opted in is always safe. To opt in, run this in the project root:

```bash
mto init
```

It writes an `mto.toml` with `layout = true` and every other setting at its
default.

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

`mto` takes rustfmt's flags, so it can stand in for `rustfmt` in scripts and
editors. Several paths may be given. A file brings the modules it declares
(`mod name;`, with `#[path]`), as rustfmt follows them; a directory is walked.

* `--check`: Print a diff for every file that would change, in rustfmt's
  format (`Diff in FILE:LINE:`), and exit with code `1` if there are any.
* `--emit files|stdout|json`: Write the files (the default), print them, or
  print rustfmt's JSON report of what would change.
* `-l, --files-with-diff`: Name the files that change, or would with
  `--check`, instead of showing how.
* `--skip-children`: A file argument is that file alone.
* `--config <KEY=VALUE,...>`: Set settings for this run, as rustfmt's
  `--config` does: `--config max_width=80,hard_tabs=true`.
* `--config-path <FILE>`: Use only this configuration file.
* `--edition <YEAR>`, `--style-edition <YEAR>`: Set the edition and the style
  edition, which decides the defaults.
* `--print-config default|minimal|current [PATH]`: Print rustfmt's settings
  as rustfmt does (to PATH for `default` and `minimal`).
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

rustfmt's `--backup` is refused with an explanation: every run is recorded in
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
| 4 | The verifier stopped a write: a bug in mto; the file is unchanged |

## Every write is checked

Before `mto` writes a file, it parses the old and the new text and checks that
only the order of items and the blank lines between them changed: every item
keeps its exact text and its comments, every comment survives, the file header
stays first, `macro_rules!` keep their order, and the byte-order mark and line
endings are kept. If anything else would change, the file is left alone, `mto`
prints a bug report with the smallest part of the file that shows the problem,
and it exits with code `4`.

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

## Configuration

`mto` reads `rustfmt.toml` as rustfmt does, so an existing one works unchanged,
and its own settings from `mto.toml`. For each folder it formats, the settings
come from, lowest precedence first:

1. The defaults, for the style edition in effect.
2. The nearest `.rustfmt.toml` or `rustfmt.toml`, found as rustfmt finds it:
   the folder, each parent folder, the home folder, then the user's `rustfmt`
   configuration folder. Only that one file is used.
3. Every `.mto.toml` or `mto.toml` from the outermost folder down; a nearer
   file wins key by key, and a file with `root = true` stops the search
   upwards. An `mto.toml` may also set rustfmt's settings, and wins over
   `rustfmt.toml` with a warning when they disagree.
4. The package edition in the nearest `Cargo.toml`, as `cargo fmt` passes it.
5. The command line: `--config`, `--edition`, `--style-edition`.

`--config-path FILE` replaces steps 2 and 3 with that one file.
`mto config path` lists the files in effect and `mto config print` shows every
value with where it came from (`--minimal` shows only what differs from the
defaults).

Unknown keys are warnings. A value of the wrong type or outside the legal
values stops the run with exit code `2`, naming the file, line and column.
`mto --man` documents every setting, rustfmt's and mto's, with its legal values
and defaults. Until `mto` formats code, rustfmt's settings change nothing except
the style edition.

`mto init` writes an `mto.toml` with layout on and mto's other settings at
their defaults, each with a comment; without the comments:

```toml
layout = true
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

rustfmt's `reorder_impl_items` is honored inside impl blocks, exactly as
rustfmt applies it. `tab_width` was renamed `tab_spaces`, rustfmt's name.

## Safety and Recovery

1. **Refusal:** If a file has syntax errors, or cannot be split into items with
   certainty, it is skipped and left byte-for-byte unchanged.
2. **Verification:** The new text must hold exactly the original items, byte
   for byte with their comments; anything else stops the write.
3. **Record:** The file's contents before and after are stored, and synced, in
   history.
4. **Atomic Write:** The new content is written to a synced temp file next to
   the target, then renamed over it in one step.

Unchanged files are never rewritten.

### History and restore

Every run that writes files is recorded under an id, shown by a short handle
(the end of the id). Contents are stored once however many runs share them.

```bash
mto history                  # the runs, newest first
mto history show last        # one run's files, and where each stands now
mto restore last             # undo the last run
mto restore last~1           # the one before it
mto restore 3f9a src/lib.rs  # one file of run 3f9a
mto history prune --keep 10  # also --older-than 30 (days; or 2w, 12h)
```

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

[safety]
allow_dirty = false # with history off, overwrite uncommitted changes
```

`mto config print --user` shows each value and where it came from.

## Where it's headed

`mto` is growing into a complete Rust formatter: output identical to `rustfmt`
by default, plus per-team control over file layout, still with no dependencies
and nothing else to install. Until then, run `mto` together with `rustfmt`;
running `rustfmt` after `mto` is expected to leave the layout alone.

## Development

`cargo xtask test` runs every gate and test suite and reports each as passed,
failed or skipped (with the reason). `cargo xtask test -h` lists every group,
task, flag and environment variable. Each run is logged, with every task's full
output, to `logs/test.log` (not tracked by git). While it runs, the machine is
kept awake (`caffeinate` on macOS, `systemd-inhibit` on Linux), and a fuzz task
fails if the machine slept anyway, since sleep would cut its fuzzing short.

```bash
# Gates and all tests (the default)
cargo xtask test
# Also the real-crate check (network), fuzzing (nightly) and benchmarks
cargo xtask test --full
# Show the plan without running it
cargo xtask test --list
# Regenerate tests/fixtures expected outputs; review the diff before committing
cargo xtask test --bless
```

The other development commands are also in `xtask`, a development-only crate in
this workspace written in Rust like the rest, with no dependencies:

```bash
# No runtime dependencies, layering, docs width, capability statuses
cargo xtask gates
# Regenerate src/text/unicode tables from the Unicode Character Database
cargo xtask unicode
# Compare mto's lexer with rustc's own on .rs files
cargo xtask lexdiff [PATH...]
# Parse .rs files with their crate's edition and report syntax errors
cargo xtask parsecheck [PATH...]
# Re-read rustfmt's options and flags (local binaries; latest stable and
# nightly when online) into compat/rustfmt-surface.txt
cargo xtask capabilities refresh
# Add new items to compat/capabilities.txt as missing; drop stale ones
cargo xtask capabilities sync
# How much of rustfmt mto supports, and how rustfmt's own tests fare
cargo xtask capabilities report [--markdown]
# Check out the pinned real-world corpus (compat/corpus.txt), then check
# that every file round-trips, parses and lexes like rustc; --baseline
# records parsing speed in compat/baseline.txt
cargo xtask corpus check
# Time rustfmt stable and nightly (as cargo fmt runs them) and mto on the
# corpus, by wall clock and CPU time, as a table; fails if mto is below
# twice rustfmt's throughput
cargo xtask corpus bench [--markdown]
# Move every corpus pin to its repository's current HEAD
cargo xtask corpus update
# Time rustfmt, rustfmt nightly and mto feature by feature on rustfmt's
# own test inputs, and mto's layout settings one by one, drawn live;
# --markdown for a shareable table
cargo xtask race [--rounds N] [--filter TEXT]
# Regenerate src/options/rustfmt.rs, rustfmt's settings table, from the
# releases pinned in compat/rustfmt-surface.txt; --check only compares
cargo xtask settings [--check]
# Check the TOML parser against the pinned toml-test conformance suite
# (compat/toml-test.txt): every valid file decodes to the expected
# values and every invalid one is refused
cargo xtask toml-test
# Every command, gate and exit code
cargo xtask help
```
