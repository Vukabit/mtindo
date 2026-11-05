# Command line

You can pass a single file or a whole directory. Directories are traversed
recursively, skipping the folders listed in `ignore_dirs`: by name anywhere,
or, for an entry with a `/`, that one folder
([Configuration](configuration.md#mtotoml)).

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

Code printed to a terminal, by `-` or `--emit stdout`, is in color while color
is on, and so are the settings `mto config print` shows and the script
`mto completions` prints. Code printed to a pipe or a file is never colored,
even with `--color always`.

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
* `--changed`, `--staged`, `--since <REV>`, `--tracked`: Take only the files
  git names: changed, staged, changed since `REV`, or tracked. With no path,
  the whole repository. See
  [Only the files git names](ci-and-editors.md#only-the-files-git-names).
* `--config <KEY=VALUE,...>`: Set settings for this run:
  `--config max_width=80,hard_tabs=true`.
* `--config-path <FILE>`: Use only this configuration file.
* `--edition <YEAR>`, `--style-edition <YEAR>`: Set the edition and the style
  edition, which decides the defaults.
* `--print-config default|minimal|current [PATH]`: Print the formatting
  settings as the standard formatter does (to PATH for `default` and
  `minimal`).
* `-q, --quiet`, `-v, --verbose`: Print less, or name each file. On a
  terminal, a run over many files that takes more than a second shows how
  far it has got on one line, redrawn in place and gone when it ends;
  `--quiet` and `--message-format json` leave it out.
* `--color always|never|auto`: Color the output.
* `--strict`: Make every configuration warning (an unknown key, a value mto
  does not honor) an error with exit code 2, so a typo cannot pass in CI.
* `--message-format human|json`: How the run reports; `json` prints one JSON
  object per line for each file, message and the summary (see
  [JSON output](#json-output)).
* `--unstable-features`: Accepted; each setting mto supports applies on
  every channel.
* `--stdin-filepath <PATH>`: With `-`, where the source belongs, for its
  settings and in messages.
* `-d, --dry-run`: List the files that would change without writing to disk.
* `-R, --recursive`: Format every `.rs` file under the current directory.
* `--follow-links`: Also follow symlinked files and directories.
* `--no-history`: Do not record this run, so it cannot be restored.
* `--no-report`: Keep no [bug report](safety.md#bug-reports); name each bug
  `mto` finds in itself in one line.
* `--allow-dirty`: With history off, rewrite files that have uncommitted
  changes.
* `--unguarded`: Do not check this run with the [build guard](build-guard.md),
  nor hold back the edits it holds.
* `mto debug units FILE` and `mto debug moves FILE` show how layout cut a
  file into items and why each moves; see [mto debug](#mto-debug).
* `--man`: Print the manual, which documents every setting and its values.
* `-h, --help`: Print the short help menu.
* `-V, --version`: Print the version.

Output is colored only when it goes to a terminal, unless `--color` says
otherwise. Set `NO_COLOR` to turn color off, or `FORCE_COLOR` to turn it on;
`NO_COLOR` wins if both are set.

On a terminal, mto never passes on a control character it did not write
itself, such as an escape code in a file's name or source. It shows the
character instead: `^[` for an escape, `<U+202E>` for a character that
reverses text. Errors and warnings are treated so wherever they go, logs
included. Other output keeps its bytes unchanged in a pipe or a file.

## Walking directories

Symlinked directories are skipped unless `--follow-links` is given, and every
real path is visited at most once, so symlink cycles cannot cause repeated
work. A folder below the starting one that holds a repository of its own
(`.git`) is another project: the walk passes over it and says so; name it to
format it.

## Cargo

`cargo mto` formats a package or workspace the way `cargo fmt` does (and lays
it out where layout is on):
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
and `-v`, as `cargo fmt` does, and `--no-history` and `--allow-dirty`. Any
other `mto` option goes after `--`, as `cargo mto -- --unguarded`.

## Commands

| Command | What it does |
|---|---|
| `mto init` | Write an `mto.toml` that turns layout on |
| `mto pins` | Every directive and `#[rustfmt::skip]`, with its reason |
| `mto verify REV` | Prove a commit is mto's work: reproduced, file by file |
| `mto config ...` | Settings in effect ([Configuration](configuration.md)) |
| `mto config derive` | The mto.toml that moves the fewest items as written |
| `mto history`, `restore` | Past runs, undoing them ([Safety](safety.md)) |
| `mto report ...` | Bug reports mto kept ([Safety](safety.md#bug-reports)) |
| `mto doctor ...` | The build guard ([The build guard](build-guard.md)) |
| `mto debug ...` | What mto sees in a file and why ([mto debug](#mto-debug)) |
| `mto completions ...` | Completions for bash, zsh, fish and PowerShell |

## mto debug

`mto debug` shows what mto sees and why it does what it does, without writing
anything. Each view takes a file or `-` (stdin), with the settings that apply
to it and any given with `--config` or `--config-path`:

* `units FILE`: how layout cuts the file into items, in each module, and why
  each was cut and classed so: pinned, a paragraph, a section's start, a
  helper and its one user, or why an item is not a helper.
* `moves FILE`: the items that move, where to, and the rule behind each
  (`type_order`, `visibility_order`, `keep_helpers_near` and so on).
* `settle PATH...`: files or folders through a real run in memory, checked
  as a write would be, then run again over their own output. Every outcome is
  counted apart: as written, changed and settled, left alone, the check
  failing, a panic, or a run changing the output again. It exits 4 when a
  check fails or mto panics, 1 when a file does not settle. `--runs N`, `--diff`
  and `--layout-only` adjust it.
* `kept FILE`: each place formatting leaves code as written, and why.
* `tokens FILE`, `tree FILE`: the tokens and syntax tree mto reads.

These views are for people and may change.

## JSON output

A command whose help lists `--json` prints, with it, one JSON object on one
line whose first field is `"version"` and second `"kind"`. Within a version
fields are only added; a field renamed, removed or given another meaning
raises the version. `--emit json` is separate: it keeps the standard
formatter's own shape.

`--message-format json` makes a whole run speak JSON Lines, for editors and
CI: one object per line for each file (`"kind":"file"`, with its outcome), each
message about it (`"kind":"message"`: level, a short code such as `syntax` or
`directive`, line and column when known, the text), warnings and errors about
the run itself, and a closing `"kind":"summary"` with the counts and the exit
code. Nothing else is printed, and the exit code is the usual one. `cargo mto
--message-format json` keeps `cargo fmt`'s meaning (`--emit json`); give mto's
own after `--` (`cargo mto -- --message-format json`).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | `--check` found files that would change; `mto restore` skipped files |
| 2 | Bad arguments or configuration, or a path that does not exist |
| 3 | A file could not be read, written or parsed, or has uncommitted changes |
| 4 | A bug in mto, kept as a report; the file is unchanged |
| 5 | The guard's command still failed with every edit held back |
| 6 | The guard's command already fails here, so it cannot guard the run |

Every flag and command, with its details, is in the
[manual](../generated/manual.md) (`mto --man`).
