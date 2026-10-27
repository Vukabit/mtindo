# Writes and history

mto replaces a file only through an atomic rename, after the new text has
passed the meaning check and the old text is safe in history. This page
describes how a file is written, how much of a write lasts after a crash on
each system, and how runs are recorded, listed, undone and pruned.

See also: [overview](overview.md), [meaning check](meaning-check.md) and
[build guard](build-guard.md).

## The order of steps

For each file a run changes, `run_file` in `src/lib.rs` takes these steps,
in this order:

1. **Verify.** The new text must pass the meaning check. A file that fails
   is left alone and a bug report is kept (see below).
2. **Guard.** With history off, a file git sees uncommitted changes in is
   left alone (see "The git guard").
3. **Store.** The file's current contents go into the history store, and
   reach the disk before anything written after them.
4. **Record.** A line naming the file, the hash of its contents before and
   the hash of the new text is appended to the run's record, which then
   reaches the disk before anything written after it.
5. **Write.** The new text replaces the file through `write_atomic`.

If step 3 or 4 fails, the file is not written; when the store itself
fails, mto says that `--no-history` writes without a record. When the
build guard is on, the files are written together once its first check has
run; `write_all` in
`src/doctor/guard.rs` stores every file, records every file, orders the
record once, and only then replaces them.

A crash at any point leaves each file old or new. Before step 5, the file is
untouched. During step 5, the rename either happened or did not. After it,
the record already names the file, and its old contents are already stored,
so `mto restore` can give them back. A record line that names a file the
run never got to write is harmless: the file still holds its old contents,
and a restore reports that it already holds what it held before.

## The atomic write

`write_atomic` in `src/lib.rs`:

1. Resolves a symlink to its target, so the link stays a link and the file
   it points at is the one replaced.
2. Creates a temporary file beside the target, in the same folder, named
   `.NAME.PID.N.tmp_mtindo` (process id and a per-process counter, so
   concurrent writers never share one).
3. Writes the whole text, then gives the temporary file the target's
   permissions, when the target exists.
4. Orders the temporary file (see "Durability"), then renames it over the
   target and notes the rename in the folder.
5. On any error, removes the temporary file; the target is unchanged.

A later run that walks the folder removes a temporary file left by a killed
run once it is ten minutes old (`is_stale_temp`), so a run never removes
the temporary file of another run still going on.

## Durability

`src/durable/mod.rs` turns every write into three steps. `ordered` makes a
file just written reach the disk before anything written after it, so a
rename can never last without the contents it names. `renamed` notes that a
folder gained an entry. `settle` runs once, as `mto` exits, and makes
everything the run wrote last; if it fails, mto reports an error.

`safety.sync` in the user config chooses how much each step does:

- `batch` (the default) does no more than each step promises, as git's
  `core.fsyncMethod = batch` does. Writes stay in order, and all of them last
  once the run ends.
- `full` makes each file, and its folder entry, last as soon as it is
  written. A power cut mid-run loses nothing already written, at the cost of
  a full flush per file.

What each system does:

| System        | `ordered` (batch)   | `settle` (batch)                    |
| ------------- | ------------------- | ----------------------------------- |
| macOS         | `F_BARRIERFSYNC`    | barrier on each folder, then        |
|               |                     | `F_FULLFSYNC` once per device       |
| Linux, Unixes | `fdatasync`         | `fsync` of each folder renamed into |
| Windows       | `FlushFileBuffers`  | nothing                             |

On macOS, `F_BARRIERFSYNC` keeps the order of writes without emptying the
drive's cache, and `F_FULLFSYNC` empties it. Both come from
`src/durable/apple.rs`, one of three files where mto allows unsafe code.
Where a file system refuses either command (a network share, say), mto
falls back to the standard library's full sync. In `full` mode every file
gets a full sync.

On Linux, `fdatasync` already makes the contents last, so `settle` only
syncs each folder a file was renamed into, once per folder. In `full` mode
each file and each folder is synced as it is written.

On Windows, file contents are flushed before each rename in either mode, but
folders are never synced: the standard library cannot open a folder to sync
it, and mto relies on NTFS keeping a rename with its journal. The guarantee
is therefore weaker. Each file still comes back old or new, never part of
each, but after a power cut the last renames of a run may be lost, so some
files the run reported as formatted may hold their old contents.

## The history store

History lives in one folder (`src/history/mod.rs`):

- `objects/AB/CDEF...`: file contents, named by their SHA-256 and split by
  the first two hex digits. Each is stored once however many runs share it.
  It is written under a temporary name and renamed into place, so runs at
  the same time never see half an object. Storing contents already present
  only touches the file, so a concurrent prune leaves it alone.
- `runs/ID.run`: one text record per run, named by a UUIDv7 id
  (`src/history/id.rs`), so records sort by time. It starts with the line
  `mto history 1` and names the command, the folder it ran in, and one
  `file BEFORE AFTER PATH` line per file written. The build guard adds
  `check` and `doctor` lines.

A record is created with its first file and appended to as files are
written, so a run cut short still records what it did. A last line cut short
is ignored when the record is read. A file written more than once in a run
(the build guard does this) keeps its first `before` and its last `after`.

Only the contents before a run are stored; the hash of what it wrote is
enough to tell whether the file still holds it.

## Runs, projects and handles

`src/cli/history.rs` presents the store:

- `mto history` lists the runs of the current project, newest first; `--all`
  lists every run. A run belongs to a project when it wrote a file under the
  project's root: the nearest folder holding `.git`, else `Cargo.toml`,
  else the folder itself.
- `last` names the project's newest run and `last~N` the one N runs before
  it, so a restore never reaches into another project.
- A handle is the shortest suffix of a run id's hex digits, at least four
  long, that no other id ends with. Handles name a run from anywhere.
- `mto history show RUN` lists a run's files and where each stands now: as
  the run wrote it, as before the run, changed since, or missing.

## Restore

`mto restore RUN [FILES]` puts back the contents a run replaced, but only in
files that still hold exactly what the run wrote. A file changed since, or
removed, is skipped with a note and the command exits with code 1;
`--force` restores those too. A file that already holds its old contents is
left alone.

A restore is a run of its own, recorded with `restore-of`, so it can be
undone in turn. Before replacing a file it stores the file's current
contents; it writes through `write_atomic` like any other run.

## Pruning

After each recorded run, `prune` removes the runs the limits in the user
config's `[history]` table do not keep:

- `keep_days` (default 30): runs older than this.
- `keep_runs` (default 100): runs past this many, newest first.
- `max_size` (default `256MB`): the oldest runs, until the contents the
  remaining runs use fit.

Zero means no limit, and the newest run is always kept. Contents no
remaining run uses are then removed, except those younger than ten minutes,
which a run going on at the same time may not have recorded yet. When any
record cannot be read, no contents are removed, since that record may use
any of them. `mto history prune` (with `--older-than` and `--keep`) also
sweeps contents left by runs cut short. The build guard's reports of pruned
runs go with them.

## The git guard

With history off (`--no-history`, or `enabled = false` under `[history]`)
there is no undo. `src/history/git.rs` then asks git, once per repository,
for `git status --porcelain=v1 -z --untracked-files=all`, and mto leaves
alone every file listed as modified, staged or untracked. `--allow-dirty`, or
`allow_dirty = true` under `[safety]`, turns this off. When git is not
installed or fails, the guard is skipped with a note. Files outside any
repository are never treated as dirty.

## Bug reports

When the meaning check stops a write, or laying out a file panics, mto keeps
a bug report in a `reports` folder beside history (`src/report/mod.rs`,
`src/cli/report.rs`). A report holds the smallest part of the file that
still fails, never the whole file unless asked, and the same bug is counted
rather than kept twice. `mto report` lists, shows, saves and prunes reports;
`mto report send` opens a prefilled issue in the browser and sends nothing
itself. `report.save = false` prints the report instead of keeping it.

## Where the files live

- User config: `~/.config/mtindo/config.toml` on macOS and Linux,
  `%APPDATA%\mtindo\config.toml` on Windows.
- History: `~/.local/state/mtindo/history` on macOS and Linux,
  `%LOCALAPPDATA%\mtindo\history` on Windows. Bug reports (`reports`) and
  the build guard's folder (`doctor`) sit beside it.

`$XDG_CONFIG_HOME` and `$XDG_STATE_HOME` take the place of `~/.config` and
`~/.local/state` when set, on every system, and `$MTINDO_CONFIG` names the
user config directly (`src/config/user.rs`). Without a home folder, history
is off and mto says so.
