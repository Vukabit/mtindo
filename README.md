# mtindo

`mtindo` is a dependency-free, structural block reorganizer for Rust source files. Its command is `mto`.

While the official `rustfmt` is fantastic at handling internal spacing, indentation, and line wrapping, it doesn't care about the vertical organization of your file. `mtindo` takes over where `rustfmt` leaves off. It physically reorders your source code—moving structs, enums, impls, and functions into a predictable, strictly organized top-down layout.

No more scrolling past 800 lines of helper functions just to find the main `struct` definition.

## Core Features

* **AST-Aware Block Sorting:** This isn't a naive regex script. `mtindo` uses a custom standard-library state machine to accurately track brace depth, escape sequences, block comments, and raw strings (`r#"..."#`).
* **Hierarchical Configuration:** Drop an `mto.toml` file anywhere in your project. `mtindo` automatically traverses up the directory tree to inherit or override settings, making it perfect for complex Cargo workspaces.
* **Sticky Impl Grouping:** Automatically anchors `impl Target` and `impl Trait for Target` blocks directly beneath their parent data structures so your logic stays with its data.
* **Visibility Hierarchy:** Sorts identical item types internally by visibility rank (`pub` -> `pub(super)` -> `pub(crate)` -> private).
* **Failsafe Backups:** Modifying ASTs is inherently risky. Before a single byte is changed, an exact snapshot of your source file is saved to your system's temporary directory with a millisecond-precision timestamp.
* **Atomic Writes:** Executes file updates using OS-level atomic renames. Your file is never left in a partially written or corrupted state, even if the power fails mid-write.
* **Polyrepo & Cycle Protection:** Safely traverses massive codebases while strictly ignoring Unix symlink cycles and intelligently skipping ignored directories.

## Installation

Because `mtindo` relies entirely on the Rust standard library, building it is incredibly fast. 

Clone this repository, navigate to the root directory, and run:

```bash
cargo install --path .
```

This compiles the tool in highly-optimized release mode and automatically places the resulting binary into Cargo's global binary directory (`~/.cargo/bin`), which should already be in your system's `PATH`.

## Command Line Interface

You can pass a single file or a whole directory. Directories are traversed recursively, automatically skipping ignored folders defined in your config.

```bash
# Format a single file
mto src/main.rs

# Format an entire project
mto src/

# See what would change without modifying files
mto --dry-run src/
```

### Options
* `-c, --config <FILE>`: Specify a custom `.toml` config path.
* `-d, --dry-run`: Evaluate the target path and output the intended actions without writing to disk.
* `--check`: Run in validation mode. Exits with code `1` if any files are structurally unformatted (useful for CI pipelines).
* `-r, --restore <BACKUP> <TARGET>`: Atomically replace the target file with a specified backup payload.
* `--man`: Print the embedded manual directly to stdout.
* `-h, --help`: Print the short help menu.

## Configuration (`mto.toml`)

You control the sorting logic via an `mto.toml` file. If the file is missing, `mtindo` falls back to its internal defaults. The parser is strict—it will halt and report an error if it encounters invalid syntax, ensuring a bad config doesn't silently corrupt your formatting.

Here is the default configuration:

```toml
# General spacing and line constraints
max_width = 100
tab_width = 4
blank_lines_between_blocks = 1

# Enable or disable alphabetical sorting within matching categories and visibilities.
sort_alphabetically = true

# Keep `impl` blocks directly below the struct/enum they implement.
group_impls = true

# Reorder use declarations alphabetically
sort_imports = true

# Consolidate imports with the same root path
import_grouping = true

# Define the absolute top-down ordering of your source files.
# Items will be grouped exactly in this order.
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
    "FnDecl"
]

# Directories that should be strictly ignored during recursive traversal.
ignore_dirs = [
    ".git",
    "target",
    "node_modules",
    "vendor",
    "dist",
    ".venv"
]
```

*Note: Any tokens the lexer doesn't explicitly match against your `type_order` are safely bubbled to the bottom of the file to prevent data loss.*

## Safety and Disaster Recovery

`mtindo` is designed to make permanent data loss impossible.

1. **Snapshot:** Before formatting begins, a complete copy of `main.rs` is saved to `/tmp/mtindo_backups/1700000000000_src_main.rs.bak`.
2. **Atomic Write:** The newly sorted code is written to a temporary file alongside the target. Once fully written, it is atomically swapped into place.
3. **Verification:** `mtindo` triggers `rustfmt` to clean up the interior spacing. If `rustfmt` throws a syntax error (indicating the structural layout broke the code), `mtindo` automatically reverts the file back to the snapshot.

If you ever need to manually revert a file because you didn't like the resulting layout, the CLI will print the exact undo command to your terminal after a successful run:

```bash
# Example Undo Command
mto --restore /tmp/mtindo_backups/1700000000000_src_main.rs.bak src/main.rs
```
