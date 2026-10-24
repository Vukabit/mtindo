# Command line

You can pass a single file or a whole directory. Directories are traversed
recursively, skipping the folders listed in `ignore_dirs`.

```bash
# Format (and, with layout on, lay out) a single file
mto src/main.rs

# A whole project
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

## Options

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
* `--unstable-features`: Accepted; mto applies every setting on every
  channel.
* `--stdin-filepath <PATH>`: With `-`, where the source belongs, for its
  settings and in messages.
* `-d, --dry-run`: List the files that would change without writing to disk.
* `-R, --recursive`: Format every `.rs` file under the current directory.
* `--follow-links`: Also follow symlinked files and directories.
* `--no-history`: Do not record this run, so it cannot be restored.
* `--allow-dirty`: With history off, rewrite files that have uncommitted
  changes.
* `--dbg`: Print how each file was split into items.
* `--man`: Print the manual, which documents every setting and its values.
* `-h, --help`: Print the short help menu.
* `-V, --version`: Print the version.

Output is colored only when it goes to a terminal, unless `--color` says
otherwise. Set `NO_COLOR` to turn color off, or `FORCE_COLOR` to turn it on;
`NO_COLOR` wins if both are set.

## Walking directories

Symlinked directories are skipped unless `--follow-links` is given, and every
real path is visited at most once, so symlink cycles cannot cause repeated
work. A folder below the starting one that holds a repository of its own
(`.git`) is another project: the walk passes over it and says so; name it to
lay it out.

## Cargo

`cargo mto` lays out a package or workspace the way `cargo fmt` formats it:
every member when run from the workspace root, otherwise the current package.
Each target (library, binaries, examples, tests, benches, build script, as
Cargo discovers them) brings the modules it declares. `mto` reads the
manifests itself; Cargo is not run.

```bash
cargo mto --check          # every target; exit 1 if any file would change
cargo mto -p mycrate       # one member (repeatable)
cargo mto --all            # also local path dependencies outside the workspace
cargo mto -- --config max_width=80   # options after -- go to mto
```

It also takes `--manifest-path`, `--message-format short|json|human`, `-q`
and `-v`, as `cargo fmt` does, and `--no-history` and `--allow-dirty`.

## Commands

| Command | What it does |
|---|---|
| `mto init` | Write an `mto.toml` that turns layout on |
| `mto config ...` | Settings in effect ([Configuration](configuration.md)) |
| `mto history`, `restore` | Past runs, undoing them ([Safety](safety.md)) |
| `mto report ...` | Bug reports mto kept ([Safety](safety.md#bug-reports)) |
| `mto doctor ...` | The build guard ([The build guard](build-guard.md)) |
| `mto debug ...` | What mto sees in a file, and why it leaves code as written |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | `--check` found files that would change; `mto restore` skipped files |
| 2 | Bad arguments or configuration |
| 3 | A file could not be read, written or parsed, or has uncommitted changes |
| 4 | A bug in mto, kept as a report; the file is unchanged |
| 5 | The guard's command failed before the run, or with every edit held back |

Every flag and command, with its details, is in the
[manual](../generated/manual.md) (`mto --man`).
