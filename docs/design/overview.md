# Overview

This page follows one run of `mto` from the paths on the command line to the
files on disk, naming the code that does each step and why it comes where it
does. The sibling pages linked from each section cover each step in depth.

## Source layout

The library crate `mtindo` holds all of the logic; `src/main.rs` (the `mto`
binary) and `src/bin/cargo-mto.rs` parse arguments and call into it. Each
top-level module under `src/` owns one concern:

| Module | Owns |
| --- | --- |
| `text` | Text that knows nothing about Rust: BOM and line endings, colors |
| `durable` | Writes that survive a crash: ordering, folder syncs, settling |
| `version` | Which build this is, for `--version` and bug reports |
| `syntax` | Lexer, parser and the lossless syntax tree, with typed views |
| `options` | The settings table: every key, its type, default and docs |
| `history` | The record of each run that wrote files, for `mto restore` |
| `verify` | The meaning checks that every output must pass |
| `order` | The standard formatter's order for `use` items and `mod` lines |
| `layout` | Reordering the items of a file |
| `format` | Formatting the code inside and between items |
| `report` | Bug reports kept when mto finds a bug in itself |
| `config` | Finding and merging configuration files per folder |
| `doctor` | The build guard and the edits it holds back |
| `cli` | What the command line shows, module discovery, subcommands |
| `fuzzing` | The checks the fuzz targets run |

`src/lib.rs` itself holds the run: collecting files, the worker pool,
`run_file`, `shape`, the verifier calls and `write_atomic`. Outside the
layers, `src/cache_alloc.rs` is a per-thread cache of small blocks that the
binaries install as their allocator.

### The layering rule

The modules sit in layers, lowest first, and a module may only use modules of
a lower layer; modules that share a layer may not use each other. A gate in
`xtask/src/gates.rs` scans every `crate::` reference under `src/` and fails
on any that points sideways or up.

| Layer | Modules |
| --- | --- |
| 0 | `text`, `durable`, `version` |
| 1 | `syntax`, `options`, `history` |
| 2 | `verify`, `order` |
| 3 | `layout`, `format`, `report` |
| 4 | `config` |
| 5 | `doctor` |
| 6 | `cli`, `fuzzing` |

So `layout` and `format`, sharing a layer, never know of each other
(`src/lib.rs` chains them), and `verify`, below both, checks their output
without borrowing their code or knowing which engine produced it.

## 1. Finding the files

`collect_jobs` in `src/lib.rs` turns the paths into jobs: a file path paired
with the settings of its folder, or with none when neither layout nor
formatting is on there. Each file appears once, in the order found; files are
deduplicated by their canonical path.

- **A folder** is walked by `walk_inner`. Entries are listed once and sorted
  by name. A folder whose name is in `ignore_dirs` (by default `.git`,
  `target`, `node_modules`, `vendor`, `dist`, `.venv`) is passed over, as is
  any folder below the starting one that holds a `.git` of its own: another
  repository, laid out only when named. Symlinks are skipped unless
  `--follow-links`. Every `.rs` file becomes a job; leftover temporary files
  from an interrupted run are collected so a writing run can remove them.
- **A file** brings the modules it declares, found by `src/cli/modules.rs` the
  way the compiler and the standard formatter find them: `mod a;` as `a.rs` or
  `a/mod.rs`, `#[path = "..."]`, every `cfg_attr` path alternative, and items
  inside `cfg_if!`. A module marked `#[rustfmt::skip]` is not entered; a
  module whose file is missing is reported and skipped. `--skip-children` (or
  `skip_children = true`) takes the file alone.
- **`cargo mto`** (`src/bin/cargo-mto.rs`) reads the manifests through
  `src/config/workspace.rs`, with no call to Cargo, finds each package's
  targets as Cargo does, and runs the sibling `mto` on their root files.

## 2. Configuration per folder

`config::Resolver` (`src/config/mod.rs`) answers "what are the settings for
this folder", reading each file once and caching each folder's answer. From
lowest precedence to highest:

1. the defaults in the settings table, for the style edition in effect;
2. the nearest `.rustfmt.toml` or `rustfmt.toml`, searched as the standard
   formatter searches: the folder, its parents, the home folder, then the
   user's configuration folder;
3. every `.mto.toml` or `mto.toml` from the outermost folder down, a nearer
   file winning key by key, the upward search stopping at `root = true`;
4. the edition of the nearest `Cargo.toml` package;
5. `--config key=value`, `--edition` and `--style-edition`.

`--config-path FILE` replaces steps 2 and 3 with that one file. Unknown keys
are warnings; a value of the wrong type is an error that stops the run.
`Settings::of` in `src/lib.rs` reduces the result to the two switches that
matter for a file: layout (off by default) and formatting (on by default).

## 3. Reading and normalizing

`run_file` reads the file into a `String`. Each step that parses the text
removes the byte-order mark and normalizes line endings first, and puts them
back on its output:

- layout calls `normalize` in `src/text/source.rs`, which strips the BOM and
  turns `\r\n` into `\n`. It refuses mixed line endings, because restoring a
  single style would change lines inside string literals; the file is then
  skipped with that reason. `Normalized::restore` puts both back.
- formatting does the same in `src/format/file.rs` and chooses the output line
  ending from `newline_style` (by default, the one the first line uses). A
  lone `\r` left after normalizing leaves the file as written.

## 4. Lexing and the syntax tree

The lexer (`src/syntax/lexer/`) never fails: every byte belongs to exactly
one token, whitespace and comments included, and malformed input becomes
tokens that carry an error. The hand-written recursive-descent parser
(`src/syntax/parser/`) sees only significant tokens and emits events; the
tree builder (`src/syntax/tree/builder/`) puts trivia back, attaching each
comment to a node as [comment attachment](comment-attachment.md) specifies.
The result is a lossless tree: printing it reproduces the input exactly. See
[syntax](syntax.md).

The file is parsed with the first edition in which it has no errors (2021,
2018, 2015, then 2024), so code valid only in an older edition still parses.
A file with errors in every edition, or nested deeper than `MAX_DEPTH` (1024
levels), is left alone. Layout, formatting and both verifiers parse the same
few texts, so `ParseCache` keeps a thread's recent parses for the length of
one file.

## 5. Layout

When layout is on, `layout::layout_holding` (`src/layout/mod.rs`) cuts the
top level into a header (shebang, inner attributes, a licence comment) that
stays first, units (each item with the loose comments above it), pinned units
that keep their place (`extern crate`, item-position macro calls,
`#[rustfmt::skip]` items, `// mto: pin`, `// mto: off` regions) and a tail.
The units between pinned ones are sorted by the `LayoutConfig` and the file is
put back together with the configured blank lines. Inline modules are laid
out the same way at their own indentation. A file that defines one
`macro_rules!` name twice is refused. See [layout](layout.md).

## 6. Formatting

When formatting is on, `format::file::format_file` takes layout's output (or
the source, when layout is off) and changes only the whitespace between
tokens, plus a fixed set of token rewrites the meaning check knows about. It
leaves files that skip themselves, files with syntax errors and
`disable_all_formatting` alone by returning its input. See
[formatting](formatting.md).

`shape` in `src/lib.rs` chains the two steps and returns both texts: the
laid-out one and the formatted one. If the final text equals the source, the
file is unchanged and nothing more happens.

## 7. The meaning checks

Before anything is written, printed or diffed, `verified_all` runs two
checks, each against its own step's input (see
[meaning check](meaning-check.md)):

- `verify::verify_layout` compares the source with layout's output: every
  item keeps its exact bytes, every comment survives, the header and anything
  unparsed stay put, `macro_rules!` definitions keep their relative order, the
  output parses at least as well, and the BOM and line endings are kept.
- `verify::format::verify_format` compares layout's output with the
  formatted text: the token streams, with whitespace and comments dropped and
  allowed rewrites made canonical, must be equal, and the comments must match
  as a multiset.

A violation means a bug in mto, not in the user's code. The file is left as
it was, with the outcome `VerifierFailed`, and a bug report is saved (or
printed whole when reports cannot be kept) with the smallest part of the file
that still fails (`verify::report::minimize`).

## 8. Writing

`--check`, `--dry-run` and `--emit stdout|json` stop here and print. A run
that writes takes three more steps per file, in this order (see
[writes](writes.md)):

1. Without history, the git guard (`src/history/git.rs`) has already listed
   files with uncommitted changes; those are left alone unless
   `--allow-dirty`, since nothing could undo the change.
2. With history, the file's current contents are stored by SHA-256 in the
   history folder, and an entry (path, hash before, hash after) is appended
   to the run's record. If either fails, the file is not written.
3. `write_atomic` writes a sibling temporary file, copies the permissions,
   orders it to disk (`durable::ordered`), and renames it over the target; a
   symlink keeps its link and its pointee is replaced.

So a crash at any point leaves each file either as it was or as written, and
history already holds what it was. At the end of the process
`durable::settle` makes everything written last before mto exits.

## 9. The build guard

With the guard on (`mto doctor on`), a run that writes files and names more
than one file is guarded; a single file, as editors pass, is not. The guard's
command (by default `cargo check --workspace --all-targets --keep-going`)
runs on its own thread while the files are laid out in memory, and the new
texts are kept rather than written. `doctor::guard::finish` then writes
nothing if the check already failed before mto changed anything; otherwise it
writes every file and checks again. If the build broke, it narrows the edits
down (delta debugging over files, then one move or changed line at a time),
holds back what makes it pass, and remembers them by content for later runs. See
[build guard](build-guard.md).

## Threads and failures

`walking` in `src/lib.rs` starts one worker per available core (no more than
there are files, when the list is known). Workers take jobs from a shared queue
in order; each `run_file` builds its output into a `FileReport` instead of
printing, and the main thread prints reports in the order the files were found,
so the output matches a single-threaded run. A run that writes nothing lays
files out while the walk is still going. A run that writes walks first, so that
a bad configuration stops it before any file changes and the git guard sees
every file. Workers share one history recorder behind a mutex.

Every thread, the main one included (`with_stack`), gets a 16 MB stack,
because layout, formatting and the verifiers recurse over the tree and the
platform defaults differ (1 MB on Windows' main thread).

A panic while shaping or verifying one file is contained by `caught`:
`catch_unwind` around the work, with a panic hook that keeps the message and
backtrace instead of printing them. The file is left as it was, a bug report is
kept as in step 7, and the other workers carry on. Locks recover from poisoning,
and a drop guard marks the queue finished even if the walk itself panics, so no
worker waits forever.
