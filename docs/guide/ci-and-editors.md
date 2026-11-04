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

## A git hook

A pre-commit hook that refuses a commit while any file would change:

```bash
#!/bin/sh
# .git/hooks/pre-commit (make it executable)
exec cargo mto --check -q
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
