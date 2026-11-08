# Changelog

Every release of mtindo, newest first. The layout follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Each section is written by hand
from the draft `cargo xtask changelog --preview` makes of the commits.

## [0.2.0] - unreleased

### Changed

- `--dbg` is gone: `mto debug units` shows how layout cuts a file into
  items, and `mto debug layout` is now `mto debug kept`. The debug views
  read stdin too, and `--json` gives each one with a versioned shape
- `group_imports = "StdExternalCrate"` and `"One"` now group imports as the
  standard formatter does, where mto warned it did not honor them yet, and
  `hard_tabs` lays files out in tabs as it does in spaces

### Formatting

- Match the standard formatter in 17 more of its test pairs that change
  settings (446 of 559), and in 8 more scrambled corpus files (9275 of
  9348); the corpus itself still matches in 9358 of 9361 files
- `// mto: ignore` in a file's header leaves the file as written;
  `ignore=layout` keeps its items in place and still formats it, and
  `ignore=format` keeps its lines and still lays it out
- Format closures, macro arguments, struct literals, kept impls and
  comments in lists, fields and patterns as the standard formatter does,
  where fuzzing and a bug hunt found mto differed, or needed a second run
  to settle
- Refuse code the compiler rejects, such as `a..b..c`, as a syntax error
  instead of formatting it

### Layout

- Keep your own structure when items are reordered: `keep_paragraphs`
  moves items written together as one, `sections` keeps items inside the
  part of the file a comment heads, `keep_helpers_near` keeps a private
  helper next to its one user (`helper_kinds` names which kinds move), and
  `cfg_groups` keeps items under one `#[cfg]` together
- Choose what decides the order: `order_keys` (with `entry`, `deprecated`
  and `hidden` to put `fn main` first or sink deprecated and hidden items),
  `flow` to order items by their uses, top down or bottom up, and
  `inherent_method_order` for the methods of an impl
- `reexports` sets a module's `pub use` apart from its imports, and
  `test_helpers = "last"` puts `#[cfg(test)]` items beside the tests
- `// mto: first` and `// mto: last` put an item at the top or bottom of
  its section, and `// mto: hold` keeps an item and its neighbours as
  written
- `mto config derive` finds the layout settings that keep the most of an
  existing project where it is, and `--write` saves them

### Commands

- `mto verify REV` proves a formatting or layout commit holds only what mto
  writes, before it goes into `.git-blame-ignore-revs`;
  `mto verify --ignored-revs` checks every commit already listed there
- `mto pins` lists every place a project keeps code from mto, with its
  reason; `directive_reasons = "required"` makes a directive give one
- `mto stats` shows what mto has done to date (runs, files, lines, time),
  from a ledger kept on your machine that names nothing; `--no-stats`,
  `stats.enabled = "off"` and `mto stats forget` leave it out
- `--diff` shows what a run changed, and `--diff=stat` a line per file
- Take only the files git names: `--changed`, `--staged`, `--since REV`
  and `--tracked`, with hooks for pre-commit and the guide's snippets for
  lefthook, husky and GitHub Actions
- `mto config update` adds the settings a newer mto brings to `mto.toml`
  at their defaults, `mto config reset` writes it as `mto init` does, and
  `mto config mirror` writes the formatting settings into a `rustfmt.toml`
  for tools that still run the standard formatter
- `--message-format json` reports a run as JSON Lines, and `--strict`
  makes configuration warnings errors, for CI
- `mto_version` in `mto.toml` pins the mto a project is formatted with
- Completions for bash, zsh, fish and PowerShell (`mto completions
  install`), down to run ids, report ids and revisions
- Color in the code, lists and completion scripts mto prints to a
  terminal, and a progress bar for long runs
- `mto --version` says whether a build is a release or a development one

### Safety

- A project can declare its build guard's command in `mto.toml`; it runs
  only once you trust it with `mto doctor trust`, and a guarded run that
  cannot check the build writes nothing and exits 6
- `// mto: hold` and `mto doctor keep` keep edits the guard held back, and
  stdin formatting keeps them too
- A bug report says what kind of machine it came from (the system, whether
  file names fold case, threads, git), and nothing that names you or it

### Performance

- Format with fewer allocations, tree walks that reuse their memory, and
  one disk barrier per batch of writes instead of three per file

## [0.1.0] - 2026-10-31

### Added

- Format Rust source as the standard formatter writes it with the same
  settings, byte for byte: 9358 of 9361 files and 99.99% of the lines of a
  pinned corpus of real crates match, and 99.92% of lines once their
  formatting is scrambled
- Read the same `rustfmt.toml` and `.rustfmt.toml` files, with `--config`,
  `--config-path`, `--edition` and `--style-edition`; `mto config options`
  lists every setting and whether mto honors it yet
- Warn when a setting mto does not honor yet is set, naming it, instead of
  ignoring it silently
- Follow the modules a file declares, through `cfg_attr`, `cfg_if` and
  `cfg_select`, as the standard formatter does; `--skip-children` formats the
  named files alone
- Honor every spelling of `#[rustfmt::skip]`, `rustfmt::skip::macros` and
  `rustfmt::skip::attributes`, and keep code the standard formatter gives up
  on as written; `mto debug layout` says where and why
- Format macro calls whose arguments parse, `macro_rules!` bodies, doc
  comments and comments in lists, chains, patterns and headers as the
  standard formatter places them
- Show what would change with `--check` (a diff in the standard formatter's
  format, exit code 1), `-l`, `--emit stdout` or `--emit json` for editors
  and CI, and read stdin with `mto -` and `--stdin-filepath`
- Print the settings as the standard formatter does with `--print-config`;
  `mto config path` and `mto config print` show which files and settings
  apply where
- Format many files in parallel, with the output in a stable order
- `cargo mto` formats every target of a package or workspace, with `-p`,
  `--all`, `--manifest-path` and `--message-format`
- Layout, for projects that opt in with `mto init` or `layout = true`:
  a file's items reordered top-down, imports and modules sorted, impls
  grouped, tests last, set by `mto.toml`
- Leave code exactly as written between `// mto: off` and `// mto: on`, and
  keep an item in place with `// mto: pin`; a marker is a line of its own,
  may give a reason after ` -- `, belongs to no item, and a near miss gets a
  warning instead of being followed
- Leave generated files' items where they are (`layout_generated_files`),
  with `generated_markers` saying what marks one; formatting follows the
  standard formatter's `format_generated_files`
- Skip folders by name anywhere or, for an `ignore_dirs` entry with a `/`,
  by path from the file that sets it
- Check every write for meaning first: the tokens before and after are
  compared, and a file whose meaning would change is left as it is (exit
  code 4)
- Write files durably and in one step, so a crash leaves the old file or the
  new one, never half of each; read-only files are left as they are
- Record every run that writes files: `mto history` lists them and
  `mto restore last` undoes one, file by file if asked; old runs are pruned
  by the user config's limits
- With history off, refuse files with uncommitted changes unless
  `--allow-dirty`
- The build guard, `mto doctor on`: after each run the project is built, and
  edits that break it are held back; `mto doctor last` finds the edit that
  broke a build
- `mto report` keeps a bug report when mto finds a bug in itself, cut down to
  the smallest part of the file that still fails, and `mto report send`
  opens an issue filled in with it for you to read over
- Grouped help with a page per command (`mto <command> -h`) and the full
  manual (`mto --man`), wrapped to the terminal and colored when it is one
- Run on Linux, macOS and Windows with no runtime dependencies: the Rust
  standard library alone, on Rust 1.88 or later

### Fixed

- End quietly when the output pipe closes, on Windows too
- Use plain paths on Windows, retry a rename a busy file blocks, color the
  console, find the home folder through `USERPROFILE` and pass long target
  lists in a file
- Parse each file in the configured edition first
- Stop a write that turns `->` into `- >`, which the meaning check read as
  the same tokens
- Leave nothing for a second run to change in every input fuzzing found
  that moved again
- Keep report links within 8000 characters and leave local paths out of the
  issue they open

### Performance

- Check a laid-out corpus 20 times as fast as the standard formatter, on ten
  cores
- Lay out and format 9.1 MB/s on one thread, with at most 31.5 MB of memory
  live while formatting serde
- Take time in step with the size of the code: growth exponents near 1 for
  items, lines, chains and matches, and 1.43 for nesting
- Skip a second pass over a file the first left as it was
