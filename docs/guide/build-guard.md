# The build guard

The verifier proves each file holds what it held, not that the project still
builds. `mto doctor on` turns on a guard that checks: each run that writes
files runs a command before and after, `cargo check --workspace
--all-targets --keep-going` unless you give another, and holds back the
edits that break the build, as few as it can find.

```bash
mto doctor                     # on or off, its command, the edits it holds
mto doctor on                  # or: mto doctor on -- cargo test -q
mto .                          # checked before and after
mto doctor holds               # edits held back, and how to keep them
mto doctor release 2           # write one of them after all; or: release all
mto doctor last -- cargo build # narrow down a run already made
mto doctor off
```

## When a run breaks the build

`mto` narrows its edits down, running the command on each candidate: first to
the fewest files that break the build, then, within them, one moved item or
changed line at a time until the build passes. It
writes every other edit, checks once more, and writes a report naming each
held edit, the errors it caused and how to keep it (`// mto: pin`,
`// mto: off`); reports are pruned with the runs they belong to. Held edits
stay held on later runs, without the command and with the guard off too,
until mto's build or the text changes; `--unguarded` writes every edit for
its run.

## When the build already fails

A tree whose command already fails before the run cannot be guarded: a crate
that does not build hides what an edit would break after it. `mto` writes
nothing, says why, and exits with code `5`. Fix the build, give the guard a
command that passes (`mto doctor on -- COMMAND`), or pass `--unguarded` for
one run.

## What runs, and when

`mto` runs the command only with the guard on, and only that command. The
setting lives in the user config, never in a project's files, so no
repository can make `mto` run anything. A run naming a single file (how
editors call `mto`) or reading stdin is not guarded; neither is one given
`--unguarded`.

The `[doctor]` section of the [user config](configuration.md#the-user-config)
sets the command, how long a check may take (`timeout`) and how many checks
one narrowing may run (`max_checks`). How the narrowing works is described in
the [build guard design](../design/build-guard.md).
