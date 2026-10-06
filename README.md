# lofmt (Layout Formatter)

`lofmt` is a dependency-free, structural block reorganizer for Rust source files. 

While the official `rustfmt` is fantastic at handling internal spacing, indentation, and line wrapping, it doesn't care about the vertical organization of your file. `lofmt` takes over where `rustfmt` leaves off. It physically reorders your source code—moving structs, enums, impls, and functions into a predictable, strictly organized top-down layout.

No more scrolling past 800 lines of helper functions just to find the main `struct` definition.

## Core Features

* **AST-Aware Block Sorting:** This isn't a naive regex script. `lofmt` uses a custom standard-library state machine to accurately track brace depth, escape sequences, block comments, and raw strings (`r#"..."#`).
* **Sticky Impl Grouping:** Automatically anchors `impl Target` and `impl Trait for Target` blocks directly beneath their parent data structures so your logic stays with its data.
* **Visibility Hierarchy:** Sorts identical item types internally by visibility rank (`pub` -> `pub(super)` -> `pub(crate)` -> private).
* **Failsafe Backups:** Modifying ASTs is inherently risky. Before a single byte is changed, an exact snapshot of your source file is saved to your system's temporary directory with a millisecond-precision timestamp.
* **Atomic Writes:** Executes file updates using OS-level atomic renames (`fs::rename`). Your file is never left in a partially written or corrupted state, even if the power fails mid-write.
* **Format Hooking:** Automatically executes `cargo +nightly fmt` after rearranging blocks to repair any interior spacing or blank line inconsistencies.

## Installation

Because `lofmt` relies entirely on the Rust standard library, building it is incredibly fast. 

Clone this repository, navigate to the root directory, and run:

```bash
cargo install --path .
```

This compiles the tool in highly-optimized release mode and automatically places the resulting binary into Cargo's global binary directory (`~/.cargo/bin`), which should already be in your system's `PATH`.

## Command Line Interface

You can pass a single file or a whole directory. Directories are traversed recursively, automatically skipping `target/`, `.git/`, and `node_modules/` folders.

```bash
# Format a single file
lofmt src/main.rs

# Format an entire project
lofmt src/

# See what would change without modifying files
lofmt --dry-run src/
```

### Options
* `-c, --config <FILE>`: Specify a custom `.toml` config path. (Defaults to `.lofmt.toml` in the current directory).
* `-d, --dry-run`: Evaluate the target path and output the intended actions without writing to disk.
* `-r, --restore <BACKUP> <TARGET>`: Atomically replace the target file with a specified backup payload.
* `--man`: Print the embedded manual directly to stdout.
* `-h, --help`: Print the short help menu.

## Configuration (`.lofmt.toml`)

You control the sorting logic via a `.lofmt.toml` file in the root of your project. If the file is missing, `lofmt` falls back to its internal defaults. 

Here is the default configuration:

```toml
# Enable or disable alphabetical sorting within matching categories and visibilities.
sort_alphabetically = true

# Keep `impl` blocks directly below the struct/enum they implement.
# If set to false, all impl blocks will be grouped together based on `type_order`.
group_impls = true

# Define the absolute top-down ordering of your source files.
# Items will be grouped exactly in this order.
type_order = [
    "UseDecl",
    "MacroDecl",
    "TraitDecl",
    "EnumDecl",
    "StructDecl",
    "ImplBlock",
    "FnDecl",
    "Unknown"
]
```

*Note: The `Unknown` token catches anything the lexer doesn't explicitly categorize (like static variables or consts) and bubbles them to the bottom of the file.*

## Safety and Disaster Recovery

`lofmt` is designed to make permanent data loss impossible.

1. **Snapshot:** Before formatting begins, a complete copy of `main.rs` is saved to `/tmp/lofmt_backups/1700000000000_src_main.rs.bak`.
2. **Atomic Write:** The newly sorted code is written to `main.tmp_lofmt`. Once fully written, it is atomically renamed to `main.rs`.
3. **Verification:** `lofmt` triggers `rustfmt`. If `rustfmt` throws a syntax error (indicating the structural layout broke the code), `lofmt` automatically reverts the file back to the snapshot.

If you ever need to manually revert a file because you didn't like the resulting layout, the CLI will print the exact undo command to your terminal after a successful run:

```bash
# Example Undo Command
lofmt --restore /tmp/lofmt_backups/1700000000000_src_main.rs.bak src/main.rs
```
