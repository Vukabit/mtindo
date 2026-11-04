# CI and editors

`mto` takes the standard formatter's flags and exit codes, so it fits where
the standard formatter already runs.

## Checking in CI

`--check` prints a diff for every file that would change and exits with code
`1` if there are any; it writes nothing and records no history.

```bash
mto --check .          # a folder
cargo mto --check      # every target of a package or workspace
```

`--emit json` with `--check` prints a machine-readable list of what would
change on stdout (the summary line goes to stderr), for annotating pull
requests:

```json
[{"name":"src/main.rs","mismatches":[{"original_begin_line":1,
  "original_end_line":1,"expected_begin_line":1,"expected_end_line":1,
  "original":"fn  main(){}\n","expected":"fn main() {}\n"}]}]
```

On GitHub Actions:

```yaml
- run: cargo install mtindo
- run: cargo mto --check
```

When `CI` is set, `mto` does not create the user config it would otherwise
write on a first run. Exit codes are listed in the
[command line guide](command-line.md#exit-codes).

## Only the files git names

These flags ask git which files to take, so a hook or a CI job on a large
project runs on what changed instead of everything. The run finds its files
as always (ignored folders, settings, modules), and only those git names go
on:

* `--changed`: files with changes not committed: modified, staged or
  untracked.
* `--staged`: files staged for the next commit.
* `--since REV`: files changed since `REV` (where `HEAD` left it), committed
  or not; with `--staged`, the committed and the staged ones.
* `--tracked`: in a folder walk, only the files git tracks, so `.gitignore`
  is respected.

```bash
mto --changed            # before committing
mto --check --since main # in CI, on a pull request's branch
mto --tracked .          # every tracked file, none git ignores
```

With no path, a run with one of these flags takes the whole repository, from
any folder in it. A run git leaves nothing to take says so, even under `-q`,
and exits `0`. Outside a repository, or without git installed, nothing runs
and the exit code is `2`. With `--staged`, a file with changes that are not
staged as well is left alone, with a warning, so the commit gets what was
staged; a staged file is written even with history off, as git keeps it
as staged.

## Git hooks

A plain pre-commit hook that refuses a commit while a staged file would
change:

```bash
#!/bin/sh
# .git/hooks/pre-commit (make it executable)
exec mto --check --staged -q
```

or that lays them out, and refuses the commit so you can look and stage the
result:

```bash
#!/bin/sh
# .git/hooks/pre-commit (make it executable)
mto --staged -q -l | grep . && exit 1
exit 0
```

A pre-push hook checking what the branch adds:

```bash
#!/bin/sh
# .git/hooks/pre-push (make it executable)
exec mto --check --since origin/main -q
```

**pre-commit** ([pre-commit.com](https://pre-commit.com)): this repository
publishes hooks, `mto` (lays out and formats the files committed; the
framework fails the commit when one changes) and `mto-check` (`--check`, at
commit and at push), each built from source; `mto-system` and
`mto-check-system` use the `mto` already installed. `mto-verify` runs
`mto verify --ignored-revs` at push, so a commit `git blame` looks past is
proved to be mto's work alone (see
[Adopting layout](layout.md#adopting-layout-in-an-existing-project)). In
`.pre-commit-config.yaml`:

```yaml
default_install_hook_types: [pre-commit, pre-push]
repos:
  - repo: https://github.com/Vukabit/mtindo
    rev: vX.Y.Z  # a release tag
    hooks:
      - id: mto
      - id: mto-check
        stages: [pre-push]
      - id: mto-verify
```

The framework passes the files itself, those a push carries included, so the
hooks need no git flag.

**lefthook** (`lefthook.yml`):

```yaml
pre-commit:
  commands:
    mto:
      glob: "*.rs"
      run: mto --skip-children {staged_files}
      stage_fixed: true
```

**husky** (`.husky/pre-commit`):

```bash
mto --check --staged -q
```

**GitHub Actions**, on a pull request:

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0  # --since needs the history
- run: cargo install mtindo
- run: mto --check --since origin/${{ github.base_ref }}
- run: mto verify --ignored-revs
```

## Editors

An editor formats a buffer by piping it through a command that reads stdin
and writes the result to stdout. `mto -` does that; `--stdin-filepath` says
where the buffer belongs, so the right `rustfmt.toml` and `mto.toml` apply:

```bash
mto - --stdin-filepath src/main.rs < src/main.rs
```

A run reading stdin is never guarded by the build guard and records no
history; the editor's own undo covers it.

**rust-analyzer** (and so most editors): point its formatting command at
`mto`. In VS Code's `settings.json`:

```json
{
  "rust-analyzer.rustfmt.overrideCommand": ["mto", "-"]
}
```

rust-analyzer passes the buffer on stdin. Without `--stdin-filepath`, `mto`
finds settings from the folder it runs in, so a project whose folders hold
different settings is better served by an editor that can pass the file's
path.

**Other editors:** any formatter setting that takes a command can use
`mto - --stdin-filepath FILE`, with the editor's placeholder for the file's
path in place of `FILE`.

## Switching from the standard formatter

`mto` formats as the standard formatter does and reads its `rustfmt.toml`, so a
project can switch one tool at a time. While some tools still run the
standard formatter (an editor not yet pointed at `mto`, a script calling `cargo
fmt`), they must agree with `mto`, which they do when the formatting settings
are where both look:

* Keep the formatting settings in `rustfmt.toml`, and `mto`'s own (layout) in
  `mto.toml`. Both tools read `rustfmt.toml`, so nothing differs.
* Or keep everything in `mto.toml`, and run `mto config mirror` after changing
  a formatting setting: it writes the ones the standard formatter's stable
  release takes into a `rustfmt.toml` beside it, names the rest, and never
  overwrites a `rustfmt.toml` it did not write.

`mto` warns when both files set a key to different values, and uses
`mto.toml`'s. Once every tool runs `mto` (editors as above, `cargo mto` in
place of `cargo fmt` in CI and scripts, hooks calling `mto`), `mto.toml` alone
is enough and the mirrored `rustfmt.toml` can go.
