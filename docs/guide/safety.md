# Safety and history

`mto` never writes a file it has not checked, writes each file in one atomic
step, and records every run that writes files so it can be undone.

## Every write is checked

Before `mto` writes a file, it parses the old and the new text and checks
that the code means what it did:

* **Layout** may change only the order of items and the blank lines between
  them: every item keeps its exact text and its comments, every comment
  survives, the file header stays first, `macro_rules!` keep their order,
  and the byte-order mark and line endings are kept.
* **Formatting** may change only whitespace and the tokens the Rust style
  rewrites.

If anything else would change, the file is left alone, `mto` keeps a bug
report and names it in one line, and it exits with code `4`. Laying out a
file that fails (a panic) is reported the same way, and the rest of the run
goes on. The [meaning check design](../design/meaning-check.md) lists every
rule.

## Bug reports

A report holds what failed, the smallest part of the file that still fails
(never the whole file), the command, the settings that differ from the
defaults, and the build. The same bug seen again is counted, not kept twice.
Reports are kept beside history; one not seen for 90 days goes by itself.

```bash
mto report                     # every report, the latest first
mto report show 7f3a2c         # one report, as it would be filed
mto report save 7f3a2c bug.md  # --with-file adds the whole file
mto report send 7f3a2c         # a new issue, filled in, in the browser
mto report prune               # remove them all
```

`mto report send` opens the issue for you to read over and submit: `mto`
itself sends nothing. `save = false` under `[report]` in the user config
prints the whole report instead and keeps none.

## How a write happens

1. **Refusal:** if a file has syntax errors, or cannot be split into items
   with certainty, it is skipped and left byte-for-byte unchanged.
2. **Verification:** the new text must mean what the old one did; anything
   else stops the write.
3. **Record:** the file's contents before the run are stored in history, and
   the file is added to the run's record, before the file changes.
4. **Atomic write:** the new content is written to a temp file next to the
   target, then renamed over it in one step. Permissions are kept, and a
   symlinked file stays a symlink.

Each step reaches the disk before the next, so a run cut short anywhere, even
by a crash, leaves every file as it was or as written, and `mto restore` can
undo whatever it changed. Unchanged files are never rewritten.

By default (`sync = "batch"` under `[safety]` in the user config) the steps
are kept in order as they are written, and everything a run wrote is flushed
to disk once, when it ends, as git's `core.fsyncMethod = batch` does: on
macOS, where a full flush costs milliseconds, a run writing hundreds of files
takes a fraction of the time. `sync = "full"` flushes each file before the
next. On Windows, which cannot flush a folder, each file still comes back old
or new after a power cut, but the last renames of a run may be lost; the
[writes design](../design/writes.md) has the details for each system.

## History and restore

Every run that writes files is recorded under an id, shown by a short handle
(the end of the id). Contents are stored once however many runs share them.

```bash
mto history                  # this project's runs, newest first
mto history --all            # every project's
mto history show last        # one run's files, and where each stands now
mto restore last             # undo this project's last run
mto restore last~1           # the one before it
mto restore 3f9a src/lib.rs  # one file of run 3f9a
mto history prune --keep 10  # also --older-than 30 (days; or 2w, 12h)
```

History is shared by every project. A run belongs to the project it wrote
files in: the nearest folder above holding `.git`, else `Cargo.toml`. Bare
`mto history` lists the current project's runs, with a `PROJECT` column, or
every run with a note when the project has none. `last` and `last~N` count
the current project's runs too, so a restore never reaches into another
project; a handle names its run from anywhere.

A restore puts back only the files that still hold what the run wrote; one
edited or deleted since is skipped (exit code `1`) unless `--force`. A
restore is a run of its own, so it can be undone too.

History lives in `~/.local/state/mtindo/history` (`$XDG_STATE_HOME` is
honored; `%LOCALAPPDATA%\mtindo\history` on Windows); `mto config path` shows
it. After each run it is pruned to the user config's limits.

## With history off

With history off there is nothing to undo a run with, so `mto` leaves alone
any file that git sees uncommitted changes in, and says so (exit code `3`).
`--allow-dirty` lets it through. git is used only if it is installed;
without it, the guard is skipped with a note.

The verifier proves each file holds what it held, not that the project still
builds; for that, see [The build guard](build-guard.md).
