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

* **Structure-Aware Splitting:** A standard-library scanner tracks braces,
  parentheses, brackets, strings, raw strings (`r#"..."#`), char literals and
  nested block comments to find exactly where each top-level item starts and
  ends.
* **Refuses When Unsure:** If a file cannot be split cleanly (unbalanced
  delimiters, an unterminated string or comment, mixed line endings), it is left
  untouched and reported as skipped.
* **Item Bodies Are Never Touched:** Each item is moved byte-for-byte. Only the
  order of items and the blank lines between them change. Line endings (LF or
  CRLF) and a leading byte-order mark are preserved.
* **Sticky Impl Grouping:** `impl Target` and `impl Trait for Target` blocks
  stay directly beneath the type they implement.
* **Visibility Hierarchy:** Items of the same kind are ordered by visibility
  (`pub` -> `pub(super)` -> `pub(crate)` -> private). Modules are the exception:
  like rustfmt, mto orders them by name alone.
* **Comments Travel With Their Item:** A comment directly above an item, or at
  the end of its last line, moves with it.
* **Order-Sensitive Items Stay Put:** See [What mto never
  moves](#what-mto-never-moves).
* **Atomic Writes:** Files are replaced by an OS-level rename of a fully
  written, synced sibling temp file. Permissions are kept, and a symlinked file
  stays a symlink.
* **Backups:** Before a file is changed, an exact copy is saved to the system
  temporary directory and the undo command is printed.
* **Safe Directory Walks:** Symlinked directories are skipped unless
  `--follow-links` is given, and every real path is visited at most once, so
  symlink cycles cannot cause repeated work.

## Installation

`mtindo` relies entirely on the Rust standard library. Clone this repository,
navigate to the root directory, and run:

```bash
cargo install --path .
```

This builds the `mto` binary in release mode and places it in Cargo's binary
directory (`~/.cargo/bin`), which should already be on your `PATH`.

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
* `-c, --config <FILE>`: Use a specific `.toml` config file.
* `-d, --dry-run`: List the files that would change without writing to disk.
* `--check`: Print a diff for every file that would change, and exit with code
  `1` if there are any (useful in CI).
* `-R, --recursive`: Format every `.rs` file under the current directory.
* `--follow-links`: Also follow symlinked files and directories.
* `-r, --restore <BACKUP> <TARGET>`: Atomically replace the target file with a
  backup.
* `--dbg`: Print how each file was split into items.
* `--man`: Print the embedded manual.
* `-h, --help`: Print the short help menu.
* `-V, --version`: Print the version.

Output is colored only when it goes to a terminal. Set `NO_COLOR` to turn color
off, or `FORCE_COLOR` to turn it on; `NO_COLOR` wins if both are set.

### Exit Codes
| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | `--check` found files that would change |
| 2 | Bad arguments or configuration |
| 3 | A file could not be read or written |
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
  order, so every module that used their macros still can.
* **`extern crate` declarations and item-position macro calls** (`thread_local!
  { .. }`, `include!(..)`, your own `make_items!();`) stay at their original
  position in the file.
* **A `#[cfg(test)]` module** always goes last.
* **Anything between `// mto: off` and `// mto: on`** is left exactly as
  written, in its original position.

## Configuration (`mto.toml`)

`mto` uses the first configuration file it finds, in this order:

1. The path passed via `--config`.
2. `mto.toml` or `.mto.toml` in the target's directory, then each parent
   directory in turn.
3. `~/.config/mtindo/mto.toml` or `~/.config/mtindo/.mto.toml`.
4. `~/mto.toml` or `~/.mto.toml`.
5. The internal defaults below.

Only one file is used; settings are not merged across files. The parser is
strict: an invalid value stops the run with exit code `2` instead of silently
falling back.

Here is the default configuration:

```toml
# Blank lines between items.
blank_lines_between_blocks = 1

# Sort items of the same kind and visibility alphabetically.
sort_alphabetically = true

# Keep `impl` blocks directly below the struct/enum/trait they implement.
group_impls = true

# Sort use declarations alphabetically.
sort_imports = true

# Separate std, external and local (crate/self/super) imports with a blank line.
import_grouping = true

# The top-down order of item kinds. Items of an unlisted kind go last.
type_order = [
    "ModDecl",
    "UseDecl",
    "MacroDecl",
    "ConstDecl",
    "StaticDecl",
    "TraitDecl",
    "EnumDecl",
    "StructDecl",
    "ImplBlock",
    "FnDecl",
    "Unknown"
]

# Directories that are skipped during recursive traversal.
ignore_dirs = [
    ".git",
    "target",
    "node_modules",
    "vendor",
    "dist",
    ".venv"
]
```

`max_width` and `tab_width` are accepted for forward compatibility but currently
have no effect; they are reserved for the formatter.

Type aliases, `extern` blocks and other items without their own kind are
classified as `Unknown`.

## Safety and Recovery

1. **Refusal:** If a file cannot be split into items with certainty, it is
   skipped and left byte-for-byte unchanged.
2. **Snapshot:** Before a file is changed, a complete copy is saved to
   `/tmp/mtindo_backups/` (or your OS equivalent).
3. **Atomic Write:** The new content is written to a synced temp file next to
   the target, then renamed over it in one step.

Unchanged files are never rewritten. To revert a file, run the undo command
printed after it was formatted:

```bash
mto --restore /tmp/mtindo_backups/1700000000000_123_0_main.rs.bak main.rs
```

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
# Move every corpus pin to its repository's current HEAD
cargo xtask corpus update
# Time rustfmt, rustfmt nightly and mto feature by feature on rustfmt's
# own test inputs, drawn live; --markdown for a shareable table
cargo xtask race [--rounds N] [--filter TEXT]
# Every command, gate and exit code
cargo xtask help
```
