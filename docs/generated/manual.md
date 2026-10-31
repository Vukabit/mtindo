# mtindo (mto)

`mtindo` is a formatter for Rust source files, with no dependencies. Its command
is `mto`.

Formatting does what the standard formatter does: spacing, indentation and line
breaks in the Rust style, changing only whitespace and the few tokens the style
rewrites. It is on by default, so with no configuration `mto` can take the
standard formatter's place; `format = false` in an `mto.toml` turns it off.

Layout does what the standard formatter leaves alone: it reorders a file's items
(modules, imports, macros, constants, types with their impls, functions) into a
predictable top-down order, inside inline modules too, and the items of impl
blocks when asked. It moves items whole. Layout is opt-in, with `layout = true`
in an `mto.toml`; `mto init` writes one.

## How it works

*   **Parsed, not guessed:** each file is parsed into a syntax tree that keeps
    every byte. A file with syntax errors is left untouched and reported as
    skipped, as is one nesting more than 1024 levels deep, which mto does not
    follow that far.
*   **Checked before every write:** layout may change only the order of items
    and the blank lines between them, each item keeping its text and its
    comments; formatting may change only whitespace and the tokens the Rust
    style rewrites, and every comment must survive. Anything else stops the
    write (exit code 4), and mto keeps a bug report, named in one line:
    `mto report` shows and saves it (`report.save = false` prints it whole
    instead).
*   **Settled in one run:** formatting mto's output again changes nothing.
*   **Every level:** layout's rules apply inside inline `mod name { ... }`
    blocks, at every level; the items of impl blocks are ordered only when asked
    (`reorder_impl_items`, `impl_method_order`).
*   **Keeps to the Rust style:** imports and `mod` declarations sort exactly as
    the standard formatter sorts them for the style edition, and blank lines
    stay within its bounds, so formatting never undoes the layout.
*   **Impls stay with their type:** `impl Type` and `impl Trait for Type` blocks
    are kept directly beneath the type they implement, its own impls first, then
    trait impls by trait name.
*   **Visibility, then name:** items of the same kind are ordered by visibility
    (`visibility_order`), then by name (`name_sort`); modules by name alone, as
    the standard formatter orders them.
*   **Order-sensitive items stay put:** the file header (shebang, `//!`,
    `#![...]`), the order of `macro_rules!` definitions, `#[macro_use]` modules,
    `extern crate` and item-position macro calls are never moved past each
    other, and a macro never ends up below an item that followed it.
    `#[rustfmt::skip]` items and items under `// mto: pin` keep their place, and
    `#[cfg]` variants of one item move together.
*   **Atomic writes:** a file is replaced by renaming a temp file beside it, on
    disk before the rename. Permissions are kept, and symlinked files stay
    symlinks.
*   **History:** every run that writes files is recorded with each file's
    contents before it; `mto restore last` undoes the last one.
*   **Check mode for CI:** `--check` prints a diff for every file that would
    change, and exits with code 1 if there are any.
*   **Stdin and stdout:** `mto -` reads source from stdin and writes the result
    to stdout.

## Installation

With Homebrew, prebuilt binaries from the GitHub release (Linux, macOS and
Windows, with `SHA256SUMS`), or from crates.io:

```bash
brew install vukabit/tap/mto
cargo install mtindo
```

From a clone of the repository: `cargo install --path .`.

## Command line
```text
mto [OPTIONS] <PATH>...
mto [OPTIONS] - [--stdin-filepath PATH]
mto <COMMAND> [ARGS]
cargo mto [OPTIONS] [-- <MTO OPTIONS>...]
```
*   `<PATH>`: a single `.rs` file or a directory. Directories are traversed
    recursively, skipping `ignore_dirs`, symlinked entries, and folders that
    hold a repository of their own (`.git`), which are named in a note; name one
    to format it. Running `mto` with no arguments prints the short help.
*   `mto -`: Read source from stdin and write the result to stdout;
    `--stdin-filepath` says where it belongs.

### Changes (`mto <PATH>...`)
*   `--check`: Print a diff for every file that would change; exit 1 if any.
*   `-d, --dry-run`: List the files that would change, without writing.
*   `--emit <files|stdout|json>`: Write the files (default), print them, or
    report changes as JSON.
*   `-l, --files-with-diff`: Name the files that change (or would, with --check)
    instead of showing how.

### Files (`mto <PATH>...`)
*   `--follow-links`: Also follow symlinked files and directories.
*   `-R, --recursive`: Format every .rs file under the current directory.
*   `--skip-children`: A file argument is that file alone, not the modules it
    declares.

### Stdin (`mto -`)
*   `--stdin-filepath <PATH>`: With -: where the source belongs, for its
    settings and in messages.

### Configuration (`mto <PATH>... | mto -`)
*   `--config <KEY=VALUE,...>`: Set settings for this run, over every file.
*   `--config-path <FILE>`: Use only this configuration file; search for no
    other.
*   `--edition <YEAR>`: The Rust edition, over Cargo.toml and configuration
    files.
*   `--style-edition <YEAR>`: The style edition, which decides the defaults.
*   `--unstable-features`: Accepted for compatibility; mto applies every setting
    on every channel.

### History and safety (`mto <PATH>...`)
*   `--allow-dirty`: With history off, rewrite files that have uncommitted
    changes.
*   `--no-history`: Do not record this run in history (it cannot be restored).
*   `--unguarded`: Do not check this run with the build guard, nor hold back the
    edits it holds.

### Output (`mto <PATH>... | mto -`)
*   `--color <always|never|auto>`: Color the output (default: auto, only on a
    terminal).
*   `--dbg`: Print how each file was split into items.
*   `-q, --quiet`: Print only errors and what was asked for.
*   `-v, --verbose`: Name each file as it is formatted.

### Compatibility (`mto`)
*   `--print-config <default|minimal|current> [PATH]`: Print the formatting
    settings as the standard formatter prints them, and exit.

### General (`mto`)
*   `-h, --help`: Show this help and exit.
*   `--man`: Print the full manual.
*   `-V, --version`: Print the version and exit.

### Commands
*   `mto init [DIR]`: Writes an mto.toml with layout turned on and every mto
    setting at its default, each with its summary, into DIR (default: the
    current directory). An existing file is never overwritten.
*   `mto config path|print|options [DIR]`: `path` lists the configuration files
    that apply in DIR (default: the current directory), and the history folder.
    `print` shows every setting in effect with where it came from, in sections:
    layout, then formatting. `options` lists every setting a configuration file
    may hold, by file, with its values and default, and for the standard
    formatter's settings whether mto honors it yet.
    *   `--toml` (after `mto config options`): As a commented file, every
        setting at its default.
    *   `--current` (after `mto config print`): Every setting in effect, with
        its source (the default).
    *   `--default` (after `mto config print`): Every setting at its default.
    *   `--minimal` (after `mto config print`): Only the settings that differ
        from their defaults.
    *   `--user` (after `mto config print`): The user config's settings.
    *   `--config <KEY=VALUE,...>` (after `mto config path|print`): Set settings
        for this run, over every file.
    *   `--config-path <FILE>` (after `mto config path|print`): Use only this
        configuration file; search for no other.
    *   `--edition <YEAR>` (after `mto config path|print`): The Rust edition,
        over Cargo.toml and configuration files.
    *   `--style-edition <YEAR>` (after `mto config path|print`): The style
        edition, which decides the defaults.
*   `mto history [--all | show <RUN> | prune]`: Every run that writes files is
    recorded. With no argument, lists the runs that wrote files in this folder's
    project (the nearest folder above holding .git, else Cargo.toml), newest
    first, or every run when there are none; --all lists every project's. `show`
    lists one run's files and where each stands now. `prune` removes old runs,
    and the build guard's reports of them, by the user config's limits unless
    told otherwise. RUN is `last` or `last~N`, counted among this project's
    runs, or a handle from the list, which names its run anywhere.
    *   `--all` (after `mto history`): List every project's runs.
    *   `--keep <N>` (after `mto history prune`): Keep only the newest N runs.
    *   `--older-than <DAYS>` (after `mto history prune`): Remove runs this old:
        30 (days), 2w, 12h.
*   `mto restore <RUN> [FILE...]`: Puts back what RUN replaced, in every file it
    wrote or only the FILEs named. A file changed or deleted since is skipped
    (exit code 1) unless --force. The restore is recorded as a run of its own,
    so it can be undone too. RUN is `last` or `last~N`, counted among this
    project's runs, or a handle from `mto history`, which names its run
    anywhere.
    *   `--force` (after `mto restore <RUN> [FILE...]`): Also restore files
        changed since the run, and missing ones.
*   `mto doctor [on|off|holds|release|<RUN>]`: `on` turns the build guard on:
    each run that writes files then checks that the project still builds, and
    holds back the edits that break it. It runs the command given after `--`
    (default: `cargo check` over the whole workspace and all its targets) and no
    other; with the guard off, mto runs none. `off` turns it off. With no
    argument, says whether it is on, its command and the edits it holds. `holds`
    lists those edits; `release` drops one, or all. RUN narrows down a run
    already made the same way: RUN is `last` or `last~N`, counted among this
    project's runs, or a handle from `mto history`.
*   `mto report [show|save|send|prune]`: When the verifier stops a write, or
    laying out a file fails, mto keeps a report beside history and names it in
    one line (`report.save = false` in the user config prints the whole report
    instead). A report holds what failed, the smallest part of the file that
    still fails (never the whole file), the command, the settings that differ
    from the defaults and the build; the same bug seen again is counted. With no
    argument, lists the reports. `show` prints one. `save` writes one to FILE;
    --with-file adds the whole file it was found in. `send` opens a new issue on
    mto's repository in the browser, filled in with the report, to read over and
    submit: mto itself sends nothing. `prune` removes them all; a report not
    seen for 90 days goes by itself. ID is a report's id from the list, or its
    start.
    *   `--with-file` (after `mto report save <ID> <FILE>`): Add the whole file
        the bug was found in.
*   `mto debug tokens|tree|layout <FILE>`: Prints the tokens or the syntax tree
    of FILE as mto reads it, or (layout) each place the formatter leaves code as
    written in it and why, with the settings that apply to FILE and formatting
    on; nothing is written. The output is not stable; it is meant for bug
    reports and for finding why code was not formatted.
    *   `--edition <YEAR>` (after `mto debug tokens|tree|layout <FILE>`): The
        Rust edition, over Cargo.toml and configuration files.

## Cargo
`cargo mto` formats the targets of the current package or workspace, as
`cargo fmt` does: every member when run from the workspace root, else the
current package. Each target's root file (library, binaries, examples, tests,
benches, build script, as Cargo discovers them) brings the modules it declares.
The manifests are read directly; Cargo is not run. Options after `--` go to
`mto`.
*   `cargo mto --all`: Every workspace member, and local path dependencies
    outside the workspace.
*   `cargo mto --manifest-path <PATH>`: The Cargo.toml to start from (default:
    the nearest one).
*   `cargo mto -p, --package <NAME>...`: Only these members (repeatable).
*   `cargo mto --check`: Print a diff for every file that would change; exit 1
    if any.
*   `cargo mto --message-format <short|json|human>`: short: name the files (-l);
    json: the JSON report (--emit json).
*   `cargo mto --allow-dirty`: With history off, rewrite files that have
    uncommitted changes.
*   `cargo mto --no-history`: Do not record this run in history (it cannot be
    restored).
*   `cargo mto -q, --quiet`: Print only errors and what was asked for.
*   `cargo mto -v, --verbose`: List the targets, and name each file as it is
    formatted.
*   `cargo mto -h, --help`: Show this help and exit.
*   `cargo mto --version`: Print mto's version and exit.

## Exit codes
*   `0`: success.
*   `1`: --check found files that would change; restore skipped files changed
    since.
*   `2`: bad arguments or configuration.
*   `3`: a file could not be read, written, or parsed (it has syntax errors),
    or, with history off, it has uncommitted changes.
*   `4`: a bug in mto: the meaning check stopped a write, or formatting a file
    failed; the file is unchanged.
*   `5`: the build guard's command failed with every edit held back, or already
    failed before the run; the files are as they were.

## Configuration
mto reads the settings for each folder it formats from, lowest precedence first:

1. The defaults below, for the style edition in effect.
2. The nearest `.rustfmt.toml` or `rustfmt.toml`, found as the standard
   formatter finds it: in the folder, then each parent folder, then the home
   folder, then the standard formatter's folder in the user's configuration
   folder. Only that one file is used.
3. Every `.mto.toml` or `mto.toml` from the outermost folder down to the folder
   itself; a nearer file wins key by key. The search upwards stops at a file
   that sets `root = true`. An `mto.toml` may also set the formatting settings,
   and wins over `rustfmt.toml`, with a warning when the two disagree.
4. The edition of the package in the nearest `Cargo.toml` (inherited from the
   workspace when it says `edition.workspace = true`), as `cargo fmt` passes it.
5. The command line: `--config key=value`, `--edition` and `--style-edition`.

`--config-path FILE` replaces steps 2 and 3 with that one file, which may hold
both mto's settings and the formatting settings. `mto config path` lists the
files in effect and `mto config print` shows every value with where it came
from.

The style edition is resolved as the standard formatter resolves it:
`style_edition` if set, else `version` (`One` is 2015, `Two` is 2024), else
`edition`, else 2015. It decides the defaults of the settings that change
between style editions.

An unknown key, or a key in the wrong file, is a warning and is ignored. A value
of the wrong type, or outside the legal values, is an error (exit code 2) naming
the file, line and column. Values are matched without regard to case, as the
standard formatter matches them.

### mto's settings
These go in `mto.toml` or `.mto.toml` only; the standard formatter would not
understand them.

#### `blank_lines_between_blocks`
The number of blank lines placed between items. Imports of the same group and
`mod name;` declarations stay on consecutive lines. It is kept within the
standard formatter's `blank_lines_lower_bound` and `blank_lines_upper_bound` (0
and 1 by default), with a warning, so that the standard formatter leaves the
result alone; imports and declarations get the lower bound.

*   Type: a whole number, zero or more.
*   Default: `1`.

#### `generated_markers`
A file is generated when a comment among its first
`generated_marker_line_search_limit` lines holds one of these texts. Layout
leaves a generated file's items where they are unless `layout_generated_files`
is set, and formatting leaves it as written when
`format_generated_files = false`. A folder of generated code is better left out
with `ignore_dirs`.

*   Type: an array of strings.
*   Default: `["@generated"]`.

#### `group_impls`
Rank each impl block as the struct, enum or trait it is for and place it right
after that item, instead of in the `ImplBlock` group of `type_order`.

*   Type: `true` or `false`.
*   Default: `true`.

#### `ignore_dirs`
Folders skipped when mto walks a directory. A name (`target`) skips every folder
of that name, wherever it is; an entry with a `/` is a path (`tests/fixtures`,
`./cache`, or an absolute one), read from the folder of the file that sets it,
and skips that folder only. Setting this list replaces the default list.

*   Type: an array of strings.
*   Default: `[".git", "target", "node_modules", "vendor", "dist", ".venv"]`.

#### `impl_method_order`
The order of the functions inside an impl of a trait declared in the same file.
Only the functions trade places; other items and blank lines stay where they
are. A trait declared under the same name twice in the file is skipped. The
standard formatter's `reorder_impl_items` is honored as well, as the standard
formatter applies it.

*   Type: one of the values below, as a string.
*   Default: `"written"`.
*   Values:
    *   `written`: Functions in impl blocks keep the order they were written in.
    *   `trait`: In an impl of a trait declared in the same file, the functions
        follow the order of the trait's declaration; functions the trait does
        not declare come after them.

#### `import_grouping`
Separate `use` declarations into three groups, in this order, with a blank line
between groups: the standard library (`std`, `core`, `alloc`), external crates,
and local paths (`crate`, `self`, `super`). Only applies when `sort_imports` is
on.

*   Type: `true` or `false`.
*   Default: `true`.

#### `import_sort`
Imports are sorted exactly as the standard formatter sorts them, so the standard
formatter leaves them alone. This picks which of the standard formatter's two
orders to use when it should differ from the style edition in effect.

*   Type: one of the values below, as a string.
*   Default: `"style_edition"`.
*   Values:
    *   `style_edition`: Sort imports as the standard formatter does for the
        style edition in effect.
    *   `2015`: the standard formatter's order before style edition 2024:
        snake_case, then CamelCase, then UPPER_SNAKE_CASE, each by bytes, so
        `a10` comes before `a2`.
    *   `2024`: Version sorting, as from style edition 2024: numbers by value,
        so `u8` comes before `u16`, and uppercase before lowercase.

#### `layout`
Turns layout on for the files this mto.toml applies to. Off by default, so mto
moves no items until a project opts in; `mto init` writes an mto.toml with it
on. An mto.toml without it may still set formatting and override the standard
formatter's settings.

*   Type: `true` or `false`.
*   Default: `false`.

#### `format`
Formats the files this mto.toml applies to in the Rust style, as the standard
formatter does with the same settings, beside or without `layout`. On by
default, so mto with no configuration is a drop-in for the standard formatter;
`format = false` turns it off. Formatting changes whitespace and the few tokens
the Rust style rewrites (a trailing comma, the order of imports). Like the
standard formatter, it leaves as written what it would give up on: macro bodies
it cannot read as code, `macro_rules!` definitions, anything marked
`#[rustfmt::skip]`, `// mto: off` regions, a statement or item that cannot fit
within `max_width`, and files that `disable_all_formatting` or
`format_generated_files = false` leave alone. Every change is checked to keep
the program's tokens and comments, and a second run changes nothing.

*   Type: `true` or `false`.
*   Default: `true`.

#### `layout_generated_files`
Whether layout reorders the items of a generated file (one holding a
`generated_markers` text near its top). Off by default, so generated code keeps
the order its generator wrote; formatting follows `format_generated_files`
instead.

*   Type: `true` or `false`.
*   Default: `false`.

#### `name_sort`
How `sort_alphabetically` compares names. Imports and modules are not affected:
they are always sorted exactly as the standard formatter sorts them.

*   Type: one of the values below, as a string.
*   Default: `"version"`.
*   Values:
    *   `version`: The Rust style guide's version sorting: numbers by value
        (`u8` before `u16`), underscores first, uppercase before lowercase.
    *   `bytes`: Byte by byte, so `a10` comes before `a2`.
    *   `case_insensitive`: Version sorting with case ignored (`alpha`, `Beta`,
        `gamma`), case deciding only between otherwise equal names.

#### `sort_alphabetically`
Within each kind of `type_order`, items are ordered by `visibility_order`, then
by name (see `name_sort`) when this is on. When off, items of the same kind and
visibility keep their original order.

*   Type: `true` or `false`.
*   Default: `true`.

#### `sort_imports`
Sort `use` declarations: by group when `import_grouping` is on, then `self`,
`super` and `crate` paths ahead of the rest, then by path. When off, imports
keep the order they were written in; pair it with the standard formatter's
`reorder_imports = false`, or the standard formatter sorts them.

*   Type: `true` or `false`.
*   Default: `true`.

#### `test_module`
Where `#[cfg(test)]` modules go: last in the file, or kept where they are.

*   Type: one of the values below, as a string.
*   Default: `"last"`.
*   Values:
    *   `last`: At the end of the file.
    *   `in_place`: Where it was written: it keeps its place, like a pinned
        item.

#### `trait_impl_order`
Trait impls for these traits come right after the type's own impls, in this
order, for example `["Default", "Clone", "Debug", "Display"]`. A trait is
matched by its name without its path, so `fmt::Display` matches `Display`. Other
trait impls follow, as `trait_impl_sort` says.

*   Type: an array of strings.
*   Default: `[]`.

#### `trait_impl_sort`
The impls of one type go in this order: the type's own impls, as written; the
trait impls listed in `trait_impl_order`; then the other trait impls, by trait
name or as written.

*   Type: one of the values below, as a string.
*   Default: `"name"`.
*   Values:
    *   `name`: By trait name, in `name_sort` order.
    *   `original`: As written.

#### `type_order`
The order in which kinds of items appear in a file, top to bottom. Kinds left
out of the list go after the listed ones, in their default order. `#[cfg(test)]`
modules go where `test_module` says. `extern crate` items and item macro calls
(such as `include!`) are not kinds here: they never move, because moving them
can change what the code means.

*   Type: an array of the values below, each at most once.
*   Default: `["ModDecl", "UseDecl", "MacroDecl", "MacroDef", "ConstDecl",
    "StaticDecl", "TypeAlias", "TraitDecl", "EnumDecl", "StructDecl", "Union",
    "ImplBlock", "ExternBlock", "FnDecl", "Unknown"]`.
*   Values:
    *   `ModDecl`: Module declarations: `mod name;` and `mod name { ... }`.
    *   `UseDecl`: Imports: `use` declarations.
    *   `MacroDecl`: `macro_rules!` definitions.
    *   `MacroDef`: `macro` definitions (declarative macros 2.0).
    *   `ConstDecl`: Constants: `const`.
    *   `StaticDecl`: Statics: `static`.
    *   `TypeAlias`: Type aliases: `type Name = ...;`.
    *   `TraitDecl`: Traits, including trait aliases.
    *   `EnumDecl`: Enums.
    *   `StructDecl`: Structs.
    *   `Union`: Unions.
    *   `ImplBlock`: Impl blocks.
    *   `ExternBlock`: `extern` blocks: `extern "C" { ... }`.
    *   `FnDecl`: Functions.
    *   `Unknown`: Items of no other kind, such as `reuse`.

#### `visibility_order`
Within each kind of `type_order`, items are ordered by visibility in this order
before they are sorted by name. Visibilities left out go last. Modules ignore
it, as the standard formatter does.

*   Type: an array of the values below, each at most once.
*   Default: `["Pub", "PubCrate", "PubIn", "PubSuper", "Private"]`.
*   Values:
    *   `Pub`: `pub`.
    *   `PubCrate`: `pub(crate)`, and `pub(in crate)`, which the standard
        formatter writes so.
    *   `PubIn`: `pub(in path)` for any other path.
    *   `PubSuper`: `pub(super)`, and `pub(in super)`.
    *   `Private`: No visibility, `pub(self)` or `pub(in self)`.

### Formatting settings
These go in `rustfmt.toml`, `.rustfmt.toml`, `mto.toml` or `.mto.toml`, as the
standard formatter 1.10.0 documents them. mto reads and checks them all, on
every channel, so an existing `rustfmt.toml` works unchanged; the standard
formatter's own stability is noted for reference. `mto config options` shows how
far mto honors each.

#### `array_width`
Maximum width of an array literal before falling back to vertical formatting.

*   Type: a whole number, zero or more.
*   Default: `60`.
*   In the standard formatter: stable.

#### `attr_fn_like_width`
Maximum width of the args of a function-like attributes before falling back to
vertical formatting.

*   Type: a whole number, zero or more.
*   Default: `70`.
*   In the standard formatter: stable.

#### `binop_separator`
Where to put a binary operator when a binary expression goes multiline.

*   Type: one of the values below, as a string.
*   Default: `"Front"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `Front`
    *   `Back`

#### `blank_lines_lower_bound`
Minimum number of blank lines which must be put between items. If two items have
fewer blank lines between them, additional blank lines are inserted.

*   Type: a whole number, zero or more.
*   Default: `0`.
*   In the standard formatter: unstable, nightly only.

#### `blank_lines_upper_bound`
Maximum number of blank lines which can be put between items. If more than this
number of consecutive empty lines are found, they are trimmed down to match this
integer.

*   Type: a whole number, zero or more.
*   Default: `1`.
*   In the standard formatter: unstable, nightly only.

#### `brace_style`
Brace style for items

*   Type: one of the values below, as a string.
*   Default: `"SameLineWhere"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `AlwaysNextLine`: Put the opening brace on the next line.
    *   `PreferSameLine`: Put the opening brace on the same line, if possible.
    *   `SameLineWhere`: Prefer the same line except where there is a
        where-clause, in which case force the brace to be put on the next line.

#### `chain_width`
Maximum width of a chain to fit on one line.

*   Type: a whole number, zero or more.
*   Default: `60`.
*   In the standard formatter: stable.

#### `color`
Whether to use colored output or not.

*   Type: one of the values below, as a string.
*   Default: `"Auto"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `Always`: Always use color, whether it is a piped or terminal output.
    *   `Never`: Never use color.
    *   `Auto`: Automatically use color, if supported by terminal.

#### `combine_control_expr`
Combine control expressions with function calls.

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: unstable, nightly only.

#### `comment_width`
Maximum length of comments. No effect unless `wrap_comments = true`.

*   Type: a whole number, zero or more.
*   Default: `80`.
*   In the standard formatter: unstable, nightly only.

#### `condense_wildcard_suffixes`
Replace strings of _ wildcards by a single .. in tuple patterns

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `control_brace_style`
Brace style for control flow constructs

*   Type: one of the values below, as a string.
*   Default: `"AlwaysSameLine"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `AlwaysSameLine`: K&R style, Rust community default.
    *   `ClosingNextLine`: Stroustrup style.
    *   `AlwaysNextLine`: Allman style.

#### `disable_all_formatting`
Don't reformat anything.

Note that this option may be soft-deprecated in the future once the ignore
option is stabilized. Nightly toolchain users are encouraged to use ignore
instead when possible.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: stable.

#### `doc_comment_code_block_small_heuristics`
Value for `use_small_heuristics` for use in code blocks in doc comments. Only
used if `format_code_in_doc_comments` is true.

*   Type: one of the values below, as a string.
*   Default: `"Inherit"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `Inherit`: Inherit from the top-level configuration.
    *   `Off`: Turn off any heuristics.
    *   `Max`: Turn on max heuristics.
    *   `Default`: Use scaled values based on the value of `max_width`.

#### `doc_comment_code_block_width`
Max width for code snippets included in doc comments. Only used if
`format_code_in_doc_comments` is true.

*   Type: a whole number, zero or more.
*   Default: `100`.
*   In the standard formatter: unstable, nightly only.

#### `edition`
Specifies which edition is used by the parser.

*   Type: one of the values below, as a string.
*   Default: `"2015"`.
*   In the standard formatter: stable.
*   Values:
    *   `2015`: Edition 2015.
    *   `2018`: Edition 2018.
    *   `2021`: Edition 2021.
    *   `2024`: Edition 2024.

#### `emit_mode`
Internal option

*   Type: one of the values below, as a string.
*   Default: `"Files"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `Files`: Emits to files.
    *   `Stdout`: Writes the output to stdout.
    *   `Coverage`: Displays how much of the input file was processed.
    *   `Checkstyle`: Unfancy stdout.
    *   `Json`: Writes the resulting diffs in a JSON format. Returns an empty
        array `[]` if there were no diffs.
    *   `ModifiedLines`: Output the changed lines (for internal value only)
    *   `Diff`: Checks if a diff can be generated. If so, the formatter outputs
        a diff and quits with exit code 1. This option is designed to be run in
        CI where a non-zero exit signifies non-standard code formatting. Used
        for `--check`.

#### `empty_item_single_line`
Put empty-body functions and impls on a single line

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: unstable, nightly only.

#### `enum_discrim_align_threshold`
The maximum length of enum variant having discriminant, that gets vertically
aligned with others. Variants without discriminants would be ignored for the
purpose of alignment.

Note that this is not how much whitespace is inserted, but instead the longest
variant name that doesn't get ignored when aligning.

*   Type: a whole number, zero or more.
*   Default: `0`.
*   In the standard formatter: unstable, nightly only.

#### `error_on_line_overflow`
Error if the formatter is unable to get all lines within `max_width`, except for
comments and string literals. If this happens, then it is a bug in the
formatter. You might be able to work around the bug by refactoring your code to
avoid long/complex expressions, usually by extracting a local variable or using
a shorter name.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `error_on_unformatted`
Error if unable to get comments or string literals within `max_width`, or they
are left with trailing whitespaces.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `float_literal_trailing_zero`
Control the presence of trailing zero in floating-point literal values

*   Type: one of the values below, as a string.
*   Default: `"Preserve"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `Preserve`: Leave the literal as-is.
    *   `Always`: Add a trailing zero to the literal.
    *   `IfNoPostfix`: Add a trailing zero by default. If the literal contains
        an exponent or a suffix, the zero and the preceding period are removed.
    *   `Never`: Remove the trailing zero. If the literal contains an exponent
        or a suffix, the preceding period is also removed.

#### `fn_call_width`
Maximum width of the args of a function call before falling back to vertical
formatting.

*   Type: a whole number, zero or more.
*   Default: `60`.
*   In the standard formatter: stable.

#### `fn_params_layout`
Control the layout of parameters in function signatures.

*   Type: one of the values below, as a string.
*   Default: `"Tall"`.
*   In the standard formatter: stable.
*   Values:
    *   `Compressed`: Fit as much on one line as possible.
    *   `Tall`: Items are placed horizontally if sufficient space, vertically
        otherwise.
    *   `Vertical`: Place every item on a separate line.

#### `fn_single_line`
Put single-expression functions on a single line

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `force_explicit_abi`
Always print the abi for extern items

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: stable.

#### `force_multiline_blocks`
Force multiline closure and match arm bodies to be wrapped in a block

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `format_code_in_doc_comments`
Format code snippet included in doc comments.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `format_generated_files`
Format generated files. A file is considered generated if any of the first
several lines contain a `@generated` comment marker. The number of lines to
check is configured by `generated_marker_line_search_limit`.

By default, generated files are reformatted, i. e. `@generated` marker is
ignored. This option is currently ignored for stdin (`@generated` in stdin is
ignored.)

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: unstable, nightly only.

#### `format_macro_bodies`
Format the bodies of declarative macro definitions.

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: unstable, nightly only.

#### `format_macro_matchers`
Format the metavariable matching patterns in macros.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `format_strings`
Format string literals where necessary

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `generated_marker_line_search_limit`
Number of lines to check for a `@generated` pragma header, starting from the top
of the file. Setting this value to `0` will treat all files as non-generated.
When`format_generated_files` is `true`, this option has no effect.

*   Type: a whole number, zero or more.
*   Default: `5`.
*   In the standard formatter: unstable, nightly only.

#### `group_imports`
Controls the strategy for how consecutive imports are grouped together.

Controls the strategy for grouping sets of consecutive imports. Imports may
contain newlines between imports and still be grouped together as a single set,
but other statements between imports will result in different grouping sets.

*   Type: one of the values below, as a string.
*   Default: `"Preserve"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `Preserve`: Keep groups as they are.
    *   `StdExternalCrate`: Discard existing groups, and create new groups for
        1. `std` / `core` / `alloc` imports 2. other imports 3. `self` / `crate`
        / `super` imports.
    *   `One`: Discard existing groups, and create a single group for
        everything.

#### `hard_tabs`
Use tab characters for indentation, spaces for alignment

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: stable.

#### `hex_literal_case`
Control the case of the letters in hexadecimal literal values

*   Type: one of the values below, as a string.
*   Default: `"Preserve"`.
*   In the standard formatter: stable.
*   Values:
    *   `Preserve`: Leave the literal as-is.
    *   `Upper`: Ensure all literals use uppercase lettering.
    *   `Lower`: Ensure all literals use lowercase lettering.

#### `ignore`
Skip formatting files and directories that match the specified pattern. The
pattern format is the same as .gitignore. Be sure to use Unix/forwardslash `/`
style paths. This path style will work on all platforms. Windows style paths
with backslashes `\` are not supported.

*   Type: an array of strings.
*   Default: `[]`.
*   In the standard formatter: unstable, nightly only.

#### `imports_granularity`
Controls how imports are structured in `use` statements. Imports will be merged
or split to the configured level of granularity.

Similar to other `import` related configuration options, this option operates
within the bounds of user-defined groups of imports. See `group_imports` for
more information on import groups.

Note that the formatter will not modify the granularity of imports containing
comments if doing so could potentially lose or misplace said comments.

*   Type: one of the values below, as a string.
*   Default: `"Preserve"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `Preserve`: Do not merge imports.
    *   `Crate`: Use one `use` statement per crate.
    *   `Module`: Use one `use` statement per module.
    *   `Item`: Use one `use` statement per imported item.
    *   `One`: Use one `use` statement including all items.

#### `imports_indent`
Indent style of imports

*   Type: one of the values below, as a string.
*   Default: `"Block"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `Visual`: First line on the same line as the opening brace, all lines
        aligned with the first line.
    *   `Block`: First line is on a new line and all lines align with **block**
        indent.

#### `imports_layout`
Item layout inside a imports block

*   Type: one of the values below, as a string.
*   Default: `"Mixed"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `Vertical`: One item per row.
    *   `Horizontal`: All items on one row.
    *   `HorizontalVertical`: Try Horizontal layout, if that fails then
        vertical.
    *   `Mixed`: Pack as many items as possible per row over (possibly) many
        rows.

#### `indent_style`
Indent on expressions or items.

*   Type: one of the values below, as a string.
*   Default: `"Block"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `Visual`: First line on the same line as the opening brace, all lines
        aligned with the first line.
    *   `Block`: First line is on a new line and all lines align with **block**
        indent.

#### `inline_attribute_width`
Write an item and its attribute on the same line if their combined width is
below a threshold

*   Type: a whole number, zero or more.
*   Default: `0`.
*   In the standard formatter: unstable, nightly only.

#### `make_backup`
Internal option, use `--backup`

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `match_arm_blocks`
Controls whether arm bodies are wrapped in cases where the first line of the
body cannot fit on the same line as the `=>` operator.

The Style Guide requires that bodies are block wrapped by default if a line
break is required after the `=>`, but this option can be used to disable that
behavior to prevent wrapping arm bodies in that event, so long as the body
contains neither multiple statements nor line comments.

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: unstable, nightly only.

#### `match_arm_indent`
Controls whether match arms are indented. If disabled, match arms will be
formatted at the same indentation level as the outer `match` statement. Meaning
that match blocks will only be indented once, not twice.

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: unstable, nightly only.

#### `match_arm_leading_pipes`
Controls whether to include a leading pipe on match arms

*   Type: one of the values below, as a string.
*   Default: `"Never"`.
*   In the standard formatter: stable.
*   Values:
    *   `Always`: Place leading pipes on all match arms.
    *   `Never`: Never emit leading pipes on match arms.
    *   `Preserve`: Preserve any existing leading pipes.

#### `match_block_trailing_comma`
Put a trailing comma after a block based match arm (non-block arms are not
affected)

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: stable.

#### `max_width`
Maximum width of each line

*   Type: a whole number, zero or more.
*   Default: `100`.
*   In the standard formatter: stable.

#### `merge_derives`
Merge multiple derives into a single one.

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: stable.

#### `newline_style`
Unix or Windows line endings

*   Type: one of the values below, as a string.
*   Default: `"Auto"`.
*   In the standard formatter: stable.
*   Values:
    *   `Auto`: Auto-detect based on the raw source input.
    *   `Windows`: Force CRLF (`\r\n`).
    *   `Unix`: Force CR (`\n`).
    *   `Native`: `\r\n` in Windows, `\n` on other platforms.

#### `normalize_comments`
Convert /* */ comments to // comments where possible

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `normalize_doc_attributes`
Convert `#![doc]` and `#[doc]` attributes to `//!` and `///` doc comments.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `overflow_delimited_expr`
When structs, slices, arrays, and block/array-like macros are used as the last
argument in an expression list, allow them to overflow (like blocks/closures)
instead of being indented on a new line.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `remove_nested_parens`
Remove nested parens.

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: stable.

#### `reorder_impl_items`
Reorder impl items. `type` and `const` are put first, then macros and methods.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `reorder_imports`
Reorder import and extern crate statements alphabetically in groups (a group is
separated by a newline).

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: stable.

#### `reorder_modules`
Reorder `mod` declarations alphabetically in group.

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: stable.

#### `required_version`
Require a specific version of the formatter. If you want to make sure that the
specific version of the formatter is used in your CI, use this option.

*   Type: a string.
*   Default: `"1.10.0"`.
*   In the standard formatter: unstable, nightly only.

#### `short_array_element_width_threshold`
The width threshold for an array element to be considered "short".

The layout of an array is dependent on the length of each of its elements. If
the length of every element in an array is below this threshold (all elements
are "short") then the array can be formatted in the mixed/compressed style, but
if any one element has a length that exceeds this threshold then the array
elements will have to be formatted vertically.

*   Type: a whole number, zero or more.
*   Default: `10`.
*   In the standard formatter: stable.

#### `show_parse_errors`
Show parse errors if the parser failed to parse files.

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: unstable, nightly only.

#### `single_line_if_else_max_width`
Maximum line length for single line if-else expressions. A value of `0` (zero)
results in if-else expressions always being broken into multiple lines. Note
this occurs when `use_small_heuristics` is set to `Off`.

*   Type: a whole number, zero or more.
*   Default: `50`.
*   In the standard formatter: stable.

#### `single_line_let_else_max_width`
Maximum line length for single line let-else statements. See the let-else
statement section of the Rust Style Guide for more details on when a let-else
statement may be written on a single line. A value of `0` (zero) means the
divergent `else` block will always be formatted over multiple lines. Note this
occurs when `use_small_heuristics` is set to `Off`.

By default this option is set as a percentage of `max_width` provided by
`use_small_heuristics`, but a value set directly for
`single_line_let_else_max_width` will take precedence.

*   Type: a whole number, zero or more.
*   Default: `50`.
*   In the standard formatter: stable.

#### `skip_children`
Don't reformat out of line modules

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `skip_macro_invocations`
Skip formatting the bodies of macro invocations with the following names.

The formatter will not format any macro invocation for macros with names set in
this list. Including the special value "*" will prevent any macro invocations
from being formatted.

Note: This option does not have any impact on how the formatter formats macro
definitions.

*   Type: an array of strings.
*   Default: `[]`.
*   In the standard formatter: unstable, nightly only.

#### `space_after_colon`
Leave a space after the colon.

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: unstable, nightly only.

#### `space_before_colon`
Leave a space before the colon.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `spaces_around_ranges`
Put spaces around the .., ..=, and ... range operators

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `struct_field_align_threshold`
The maximum diff of width between struct fields to be aligned with each other.

*   Type: a whole number, zero or more.
*   Default: `0`.
*   In the standard formatter: unstable, nightly only.

#### `struct_lit_single_line`
Put small struct literals on a single line

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: unstable, nightly only.

#### `struct_lit_width`
Maximum width in the body of a struct literal before falling back to vertical
formatting. A value of `0` (zero) results in struct literals always being broken
into multiple lines. Note this occurs when `use_small_heuristics` is set to
`Off`.

*   Type: a whole number, zero or more.
*   Default: `18`.
*   In the standard formatter: stable.

#### `struct_variant_width`
Maximum width in the body of a struct variant before falling back to vertical
formatting. A value of `0` (zero) results in struct literals always being broken
into multiple lines. Note this occurs when `use_small_heuristics` is set to
`Off`.

*   Type: a whole number, zero or more.
*   Default: `35`.
*   In the standard formatter: stable.

#### `style_edition`
Controls the edition of the Rust Style Guide to use for formatting (RFC 3338)

*   Type: one of the values below, as a string.
*   Default: `"2015"`; `"2024"` from style edition 2024.
*   In the standard formatter: stable.
*   Values:
    *   `2015`: Edition 2015.
    *   `2018`: Edition 2018.
    *   `2021`: Edition 2021.
    *   `2024`: Edition 2024.
    *   `2027`: Edition 2027. (in the standard formatter: unstable, nightly
        only)

#### `tab_spaces`
Number of spaces per tab

*   Type: a whole number, zero or more.
*   Default: `4`.
*   In the standard formatter: stable.

#### `trailing_comma`
How to handle trailing commas for lists

*   Type: one of the values below, as a string.
*   Default: `"Vertical"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `Always`
    *   `Never`
    *   `Vertical`

#### `trailing_semicolon`
Add trailing semicolon after break, continue and return

*   Type: `true` or `false`.
*   Default: `true`.
*   In the standard formatter: unstable, nightly only.

#### `type_punctuation_density`
Determines if `+` or `=` are wrapped in spaces in the punctuation of types

*   Type: one of the values below, as a string.
*   Default: `"Wide"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `Compressed`: No spaces around "=" and "+".
    *   `Wide`: Spaces around " = " and " + ".

#### `unstable_features`
Enable unstable features on the unstable channel.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `use_field_init_shorthand`
Use field initialize shorthand if possible.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: stable.

#### `use_small_heuristics`
This option can be used to simplify the management and bulk updates of the
granular width configuration settings (`fn_call_width`, `attr_fn_like_width`,
`struct_lit_width`, `struct_variant_width`, `array_width`, `chain_width`,
`single_line_if_else_max_width`), that respectively control when formatted
constructs are multi-lined/vertical based on width.

Note that explicitly provided values for the width configuration settings take
precedence and override the calculated values determined by
`use_small_heuristics`.

*   Type: one of the values below, as a string.
*   Default: `"Default"`.
*   In the standard formatter: stable.
*   Values:
    *   `Off`: Turn off any heuristics.
    *   `Max`: Turn on max heuristics.
    *   `Default`: Use scaled values based on the value of `max_width`.

#### `use_try_shorthand`
Replace uses of the try! macro by the ? shorthand

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: stable.

#### `version`
This option is deprecated and has been replaced by `style_edition`.
`version = "One"` is equivalent to `style_edition = "(2015|2018|2021)"` and
`version = "Two"` is equivalent to `style_edition = "2024"`

*   Type: one of the values below, as a string.
*   Default: `"One"`; `"Two"` from style edition 2024.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `One`: 1.x.y. When specified, the formatter will format in the same
        style as 1.0.0.
    *   `Two`: 2.x.y. When specified, the formatter will format in the latest
        style.

#### `where_single_line`
Forces the `where` clause to be laid out on a single line.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `wrap_comments`
Break comments to fit on the line

Note that no wrapping will happen if:
1. The comment is the start of a markdown header doc comment
2. A URL was found in the comment

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

### Hidden formatting settings
The standard formatter leaves these out of its own listings: they are
deprecated, or set by the formatter itself. mto accepts them so that existing
files keep working.

#### `file_lines`
Lines to format; this is not supported in rustfmt.toml, and can only be
specified via the --file-lines option

*   Type: any value.
*   Default: set by the standard formatter itself; mto ignores the value.
*   In the standard formatter: unstable, nightly only.

#### `fn_args_layout`
This option is deprecated and has been renamed to `fn_params_layout` to better
communicate that it affects the layout of parameters in function signatures.

*   Type: one of the values below, as a string.
*   Default: `"Tall"`.
*   In the standard formatter: stable.
*   Deprecated: use `fn_params_layout`.
*   Values:
    *   `Compressed`: Fit as much on one line as possible.
    *   `Tall`: Items are placed horizontally if sufficient space, vertically
        otherwise.
    *   `Vertical`: Place every item on a separate line.

#### `hide_parse_errors`
This option is deprecated and has been renamed to `show_parse_errors` to avoid
confusion around the double negative default of `hide_parse_errors=false`.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.

#### `merge_imports`
This option is deprecated. Use `imports_granularity = "Crate"` instead.

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: unstable, nightly only.
*   Deprecated: use `imports_granularity`.

#### `print_misformatted_file_names`
Internal option, use `-l` or `--files-with-diff`

*   Type: `true` or `false`.
*   Default: `false`.
*   In the standard formatter: stable.

#### `verbose`
How much to information to emit to the user

*   Type: one of the values below, as a string.
*   Default: `"Normal"`.
*   In the standard formatter: unstable, nightly only.
*   Values:
    *   `Verbose`: Emit more.
    *   `Normal`: Default.
    *   `Quiet`: Emit as little as possible.

#### `width_heuristics`
'small' heuristic values

*   Type: any value.
*   Default: set by the standard formatter itself; mto ignores the value.
*   In the standard formatter: unstable, nightly only.

### User settings
The user configuration holds how mto behaves for you, never layout or
formatting. It is `$MTINDO_CONFIG` when set (the file must exist), else
`$XDG_CONFIG_HOME/mtindo/config.toml`, else `~/.config/mtindo/config.toml` on
macOS and Linux and `%APPDATA%\mtindo\config.toml` on Windows. Project files
cannot set user settings, and the user configuration cannot set project ones.

#### `doctor.command`
The build guard runs it in the project's folder before and after each run that
writes files, and while it narrows down a run that broke the build. It is run as
these words, not through a shell. `mto doctor on -- COMMAND` sets it.

*   Type: an array of strings.
*   Default: `["cargo", "check", "--workspace", "--all-targets",
    "--keep-going"]`.

#### `doctor.enabled`
With it on, mto runs `doctor.command` before and after each run that writes
files, and holds back the edits that break the build. With it off, mto runs no
command. `mto doctor on` and `mto doctor off` set it.

*   Type: `true` or `false`.
*   Default: `false`.

#### `doctor.max_checks`
Past this many runs of the command, a narrowing holds back every changed file
whole, so the files stay as they were before the run.

*   Type: a whole number, zero or more.
*   Default: `64`.

#### `doctor.timeout`
A command still running after this many seconds is stopped and counts as failed.
0: no limit.

*   Type: a whole number, zero or more.
*   Default: `600`.

#### `history.enabled`
Every run that writes files is recorded: each file's contents before and after,
kept once however many runs share them. `mto history` lists the runs and
`mto restore` undoes one. `--no-history` skips the record for one run. With
history off, the git guard stops mto from overwriting uncommitted changes.

*   Type: `true` or `false`.
*   Default: `true`.

#### `history.keep_days`
Runs older than this many days are removed after each recorded run and by
`mto history prune`. The newest run is always kept.

*   Type: a whole number, zero or more.
*   Default: `30`.

#### `history.keep_runs`
Only this many of the newest runs are kept.

*   Type: a whole number, zero or more.
*   Default: `100`.

#### `history.max_size`
The oldest runs are removed until the file contents the rest keep fit in this
much room. A number of bytes, or one with `KB`, `MB` or `GB` (powers of 1024).

*   Type: a size such as `"256MB"`.
*   Default: `"256MB"`.

#### `output.color`
`--color` decides over it, and so do the `NO_COLOR` and `FORCE_COLOR`
environment variables.

*   Type: one of the values below, as a string.
*   Default: `"auto"`.
*   Values:
    *   `auto`: Color only on a terminal.
    *   `always`: Always color.
    *   `never`: Never color.

#### `report.save`
When the verifier stops a write, or laying out a file fails, the report is kept
beside history and the run names it in one line; `mto report` shows, saves,
sends and prunes reports. A report holds the smallest part of the file that
still fails, never the whole file unless asked. Off, the whole report is printed
instead, and nothing is kept.

*   Type: `true` or `false`.
*   Default: `true`.

#### `safety.allow_dirty`
With history off there is no `mto restore`, so mto refuses to rewrite a file
that git sees uncommitted changes in. This turns that guard off, as
`--allow-dirty` does for one run. With history on it has no effect.

*   Type: `true` or `false`.
*   Default: `false`.

#### `safety.sync`
Either way a crash leaves each file as it was or as mto wrote it, and history
holds its earlier contents before it changes. `batch` keeps writes in order and
makes them all last once the run ends, as git's `core.fsyncMethod = batch` does:
on macOS a run writing hundreds of files takes a fraction of the time. `full`
makes each file last before the next is written, so a power cut mid-run loses
nothing already written, at a full disk flush per file. On Windows, where a
folder cannot be synced, a power cut may lose the last renames of a run in
either mode: those files come back as they were.

*   Type: one of the values below, as a string.
*   Default: `"batch"`.
*   Values:
    *   `batch`: Each file reaches the disk in order, and everything a run wrote
        lasts once it ends.
    *   `full`: Each file lasts as soon as it is written.

## Safety and history

A file is written in steps:

1. The new text is checked against the original before anything is written.
2. With history on (the default), the file's contents before are stored in the
   history folder (`mto config path` shows it), each content once however many
   runs share it, and the file is added to the run's record.
3. The new text is written to a temp file beside the original, and an atomic
   rename replaces the original.

Each step reaches the disk before the next, so a run cut short anywhere, even by
a power cut, leaves each file as it was or as written, and `mto restore` can
undo it. With `sync = "batch"` under `[safety]` (the default), everything a run
wrote is flushed to disk once, when it ends; `sync = "full"` flushes each file
before the next. Unchanged files are never rewritten.

Each run that writes files is recorded under an id, shown by a short handle:

```bash
mto history            # this project's runs, newest first
mto history show last  # one run's files, and where each stands now
mto restore last       # undo this project's last run
mto restore 3f9a src/lib.rs
```

A restore puts back only files that still hold what the run wrote; `--force`
restores changed and missing files too. A restore is a run of its own, so it can
be undone. Runs are pruned after each run by the user config's `[history]`
settings, or with `mto history prune`, and the build guard's reports of the runs
pruned go with them.

With history off (`--no-history`, or `enabled = false` under `[history]`), there
is no undo, so mto leaves alone any file git sees uncommitted changes in (exit
code 3), unless `--allow-dirty` (or `allow_dirty = true` under `[safety]`). git
is used only if it is installed.

Text between `// mto: off` and `// mto: on` is never moved or changed, the
markers with it. Each marker is a line comment alone on its line; a reason may
follow after ` -- ` (`// mto: off -- aligned by hand`). A marker belongs to no
item: a comment under it belongs to the code below, and the item after
`// mto: on` is formatted and laid out as any other. A region ends in the block
it starts in, and with no `// mto: on` runs to the end of that block; one among
the parts of a statement or item (list items, match arms) keeps that whole
statement or item as written. `// mto: pin` keeps the next item in its place. A
comment that looks meant as a marker and is not one (`//mto: off`,
`// MTO: off`, words after it without ` -- `, one after code or in a `/* */` or
doc comment) is reported and not followed, as is a marker without its partner.

## The build guard: mto doctor

mto proves that each file it writes holds what it held, but not that the project
still builds: a reordered item can meet a macro or a build script that depends
on where it was. The build guard checks.

```bash
mto doctor on                    # guard with cargo check
mto doctor on -- cargo test -q   # or with any command
mto doctor                       # whether it is on, and what it holds
mto doctor off
```

It is off until you turn it on. Turned on, mto runs one command: the one given
to `mto doctor on`, else `cargo check` over the whole workspace and all its
targets. Turned off, it runs none. The setting lives in the user config, under
`[doctor]`, so no project can turn it on or change its command.

A run that writes files is guarded unless it names a single file, which is how
editors call mto, or reads stdin; `--unguarded` skips the guard for one run. A
guarded run:

1. Runs the command in the project's folder (the nearest above holding `.git`,
   else `Cargo.toml`) while it lays out and formats the files in memory. Nothing
   is written until it ends.
2. Writes the files, then runs the command again.
3. Compares the two. When the command passed before and fails now, the run broke
   the build. Errors are compared by their code and message (`error[E0425]:
   cannot find value in this scope`), not their lines, which formatting moves. A
   command that prints no errors the compiler's way is judged by its exit code
   alone.

A run that broke the build is taken apart. Its edits are the moves of items
(each item layout put somewhere else) and the changes formatting made (each run
of changed lines). mto narrows them down, running the command on each candidate:
first to the files that still break the build, then, within them, one edit at a
time until the build passes. It holds back as few edits as it can find: the item
stays where it was, the lines stay as they were written. Every other edit is
written. The run's record in history ends with what was written, so
`mto restore` undoes the run as usual.

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

The report says what each held edit was, the errors it caused, and how to keep
it for good without the guard: `// mto: pin` above a moved item, `// mto: off`
and `// mto: on` around changed lines. It goes when its run is pruned from
history, unless an edit it held is still held.

A narrowing runs the command at most `doctor.max_checks` times. Past that, every
changed file is held back whole, as no check can show what else is safe to
write: the files stay as they were before the run, and mto says so. When the
build still fails with every edit held back, the failure was not mto's (the
command is flaky, or reads something else that changed): the files are put back
as they were, and mto says so (exit code 5).

A command that already fails before the run cannot guard it, as a crate that
does not build hides what an edit would break after it: mto writes nothing, says
why, and exits with code 5. Fix the build, give the guard a command that passes,
or pass `--unguarded`.

Held edits are remembered, in the doctor's folder next to history, so the next
run holds them back again without running the command, with the guard off too;
`--unguarded` writes them for one run. A hold lapses when mto's build changes
(the one `mto --version` names, which changes between releases too) or the text
it is about does; the guard then checks again.

```bash
mto doctor holds        # every held edit, with how to keep it for good
mto doctor release 2    # drop one; release all drops every one
```

While a narrowing runs, the files hold its candidates, each recorded in the
run's history first. If it is cut short, the next run of mto puts the files back
as they were before the run, before anything else.

`mto doctor RUN` narrows down a run already made, guarded or not: RUN is `last`,
`last~N` or a handle from `mto history`. Its files must still hold what it
wrote. The command is the one after `--`, else the guard's; with the guard off
and no command given, it stops and says how to give one.
