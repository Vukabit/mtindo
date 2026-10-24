# Build guard

The build guard (`mto doctor`, `src/doctor/`) runs a build command before
and after each run that writes files. When the run breaks the build, it
narrows the break down to the edits that cause it, holds those back, writes
the rest, and remembers the held edits for later runs.

See also: [overview](overview.md), [meaning check](meaning-check.md) and
[writes and history](writes.md).

## Turning it on

The guard is off by default (`doctor.enabled = false`), and while it is off
mto runs no command. `mto doctor on` turns it on, `mto doctor on -- COMMAND`
also sets the command, and `mto doctor off` turns it off; each edits only the
`[doctor]` table of the user config (`turn` in `src/cli/doctor.rs`). `mto
doctor` prints whether the guard is on, its command, its limits and how many
edits it holds.

The default command is `cargo check --workspace --all-targets --keep-going`.
`Check::run` in `src/doctor/check.rs` runs it as words, not through a shell,
in the project's folder (the nearest folder holding `.git`, else
`Cargo.toml`), with stdin closed and `CARGO_TERM_COLOR=never`. A command
still running after `doctor.timeout` seconds (default 600; 0 for no limit) is
killed and counts as failed, as does one that cannot start.

A writing run is guarded unless it names a single file, as editors do, or
passes `--unguarded`.

## Where the command comes from

The command is read only from the user config. `Resolver::user_value` in
`src/config/mod.rs` reads per-user settings from that file and nowhere else.
A `[doctor]` table in a project's `mto.toml`, or in a file named by
`--config-path`, is ignored with a warning, and `--config` refuses per-user
keys. A cloned repository therefore cannot turn the guard on or choose what
it runs. The one other source is the user's own command line:
`mto doctor RUN -- COMMAND` checks with the command after `--`.

## Before and after

`src/doctor/guard.rs` drives a guarded run:

1. The command runs on a thread while mto lays out and formats the files in
   memory. Nothing is written until it ends.
2. If that check fails, nothing is written and mto exits with code 5. A
   build that already fails cannot guard a run: a crate that does not build
   hides every error an edit would cause in the crates after it, so the run
   would look checked when it is not. mto says to fix the build, give a
   command that passes, or pass `--unguarded`.
3. Otherwise every file is written (stored, recorded, then replaced, as
   [writes](writes.md) describes), and the command runs again.
4. The run broke the build when the check passed before and fails now, or
   fails now with an error it did not have. Errors are the compiler's
   `error[CODE]: message` and `error: message` lines, without their
   locations, which formatting moves (`errors_in`).

The run's record gets `check before` and `check after` lines with each
result.

## Narrowing

A run's edits are the items layout moved and the runs of lines formatting
changed. Either kind can be held back: a moved item stays where it was, and
changed lines stay as written. Holding back edits always leaves a whole file.

`narrow` in `src/doctor/mod.rs` works in turns, each holding back one more
edit (or, failing that, whole files):

1. Tries every file with what is held so far. A pass ends the narrowing.
2. Finds the fewest files whose edits still break the build, the other files
   left as written. The files found on the previous turn are tried alone
   first; failing that, `ddmin` (Zeller and Hildebrandt's delta debugging)
   searches the changed files, trying each subset at most once.
3. Holds back every move in those files. If the build still breaks, the
   changed lines are at fault; otherwise a move is.
4. Lists the edits of that kind in order and bisects on how many of them,
   from the first, are held back (`decisive`): it finds the fewest held from
   the start that let the build pass, and the last of those is the edit held
   back. This takes about log2 of the number of edits in checks.

The search is greedy. Each turn holds one edit and starts again, so the
edits held are each needed on their turn, but the set as a whole is not
proven to be the smallest. The file search is minimal in the `ddmin` sense:
no single file can be dropped from the set it finds.

Each candidate written during a narrowing is stored and recorded in history
before it replaces the file, and the record is marked `doctor open` until the
narrowing ends with `doctor closed`. If mto is cut short in between, the
next writing run (or `mto doctor RUN`) finds the open record among the 20
newest runs and puts back each file that still holds what the run last wrote
(`recover`).

## The check budget

`doctor.max_checks` (default 64) limits how many times one narrowing runs
the command; the checks before and after the run do not count. When the
budget runs out, no further check can confirm what else is safe to write, so
every changed file is held back whole (`hold_all`). That leaves the sources,
which the check before passed, so the narrowing ends with a pass: nothing is
written, mto says it ran out of checks, and the run exits as usual.

## Ending a narrowing

When the build passes, the texts of the last passing candidate are written,
the held edits are listed, and the run's record ends with what the files now
hold, so `mto restore` undoes the run as usual. When the build still fails
with every edit held back, the failure is not mto's (a flaky command, or one
that reads something else that changed): the files are put back as they
were, and mto exits with code 5.

## Holds

Held edits are remembered in `holds.txt` in the doctor's folder, beside
history (`src/doctor/holds.rs`), one tab-separated line each. A hold names
its file by absolute path and its edit by content, not place: a moved item
by its kind, name and the SHA-256 of its text; changed lines by the SHA-256
of the lines before and after. A file held back whole is held for that run
only.

Every later writing run lays the file out with its holds applied, without
running the command, guarded or not. A hold lapses when:

- mto's build changes: each hold carries the full version string, and holds
  of another build are ignored when `holds.txt` is read, and left out when
  it is next saved;
- the text it is about is no longer found in the file.

`mto doctor holds` lists the holds, with how to keep each edit for good
without the guard: `// mto: pin` above a moved item, `// mto: off` and
`// mto: on` around changed lines. `mto doctor release N` drops one, and
`release all` drops every one.

## The report

Each narrowing writes `reports/RUN.md` in the doctor's folder
(`src/doctor/report.rs`; `latest.md` for a run without a record). It names
mto's version, the run's command, the check and its folder, and the results
before and after. It then lists each held edit with its file, the lines as
formatted when they are lines, the errors the build gave with it, and how to
keep it without the guard. When nothing could be held back, it gives the
last failure the command printed instead.

Reports are pruned with history: a report goes when history no longer keeps
its run, unless a remembered hold names that run, or it is younger than ten
minutes.

## Narrowing an earlier run

`mto doctor RUN` narrows down a run already made, guarded or not, where RUN
is `last`, `last~N` or a handle. Every file of the run must still hold what
it wrote. mto puts the files back as they were before the run, recording
that as a new run, checks, and then proceeds as a guarded run does with the
run's texts. With the guard off, a command must be given after `--`.
