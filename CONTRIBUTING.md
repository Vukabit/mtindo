# Contributing to mtindo

Thanks for helping. This page is for anyone sending a change: how to set up,
what a change must keep true, how commits are written, and what the checks
look for. [MAINTAINING.md](MAINTAINING.md) covers releases and the project's
tooling. Every development command, with its arguments, is in the
[command reference](docs/generated/commands.md).

## What a change keeps true

- **The output matches the standard formatter.** With the same settings, `mto`
  writes exactly what rustfmt writes, byte for byte. A change to formatting is
  measured against a pinned corpus of real crates (`cargo xtask diff`) and may
  not lower its record.
- **A second run changes nothing.** Formatting `mto`'s own output again gives
  the same text.
- **The meaning never changes.** Every write is checked token for token first;
  a change that trips the check is a bug, however good the layout looks.
- **No runtime dependencies.** The shipped `mto` uses the Rust standard
  library alone and runs no other program. Development tools are fine; they
  live in `xtask/` and are never part of the binary.

## Setting up

```bash
git clone https://github.com/Vukabit/mtindo
cd mtindo
cargo xtask bootstrap        # what would be installed or updated
cargo xtask bootstrap --go   # install it, and turn the git hooks on
```

`bootstrap` reads the tool pins in [flow.toml](flow.toml): the Rust toolchain
(the same one `rust-toolchain.toml` names, whose rustfmt is the answer key the
corpus is compared against), the oldest supported Rust, a nightly for fuzzing,
and the cargo tools the release uses. Run it again whenever `flow.toml`
changes; it updates only what differs. Each check below also looks for the
tools it needs and says to run `bootstrap` when one is missing or at another
version.

## Making a change

Code is layered: each module under `src/` uses only the layers below it (the
`layering` gate checks this). A few house rules, also checked:

- Public Markdown stays within 80 columns.
- Comments in `src/`, `tests/` and `fuzz/` explain the code, not the plan: no
  stage or decision ids.
- `docs/internal/` holds working notes and is never committed.

After editing Rust code, lay it out the way the project does: `mto` itself,
then the standard formatter, then clippy.

```bash
cargo build --release
./target/release/mto --skip-children <the files you changed>
cargo fmt --all
cargo clippy --all-targets
```

`mto` also orders the items in a file, so a private function may move; that
is expected.

A formatting fix comes with a regression test whose expected output is the
standard formatter's, and with a corpus run:

```bash
cargo xtask diff     # files and lines matching, against the record
```

## Running the checks

`cargo xtask test` runs every gate and test suite and reports each as
passed, failed or skipped with a reason; `cargo xtask test --full` adds the
network, fuzzing and benchmark groups. Every task runs under a time limit,
so a green run means every chosen task ran and passed. Runs are logged under
`.dev/` (machine-local, not tracked): `cargo xtask status` shows where the
project stands, `cargo xtask runs` the latest runs.

## Commits

Each commit's first line is `kind(scope): subject`:

```text
fix(format): keep a generic default's header as written
feat(cli,config): read rustfmt.toml from the workspace root
docs(docs): say how holds lapse
```

- **kind** is one of [flow.toml](flow.toml)'s `[kinds]`. Those with a
  changelog heading (`feat` Added, `fix` Fixed, `perf` Performance,
  `refactor` Changed, `revert` Removed) are listed in the changelog;
  `docs`, `test`, `chore`, `ci` and `style` are not.
- **scope** is a section from `[sections]`, which maps names to paths
  (`format` is `src/format/**`), or a path in the tree; several are joined
  with commas. It must cover at least one file the commit changes.
- A `!` after the scope marks a breaking change: `feat(cli)!: ...`.
- The first line is at most 72 columns, with no period at its end. The
  subject is what the changelog shows, so write it for someone using `mto`.

A body is for a change that needs one. It is refused for `docs`, `test`,
`chore`, `ci` and `style` commits and for changes under 40 lines, required
for a breaking change (saying what breaks), and kept to 12 lines of at most
72 columns. Trailers such as `Co-authored-by:` are not counted as a body.

A plain `git commit` starts the message with the scope the staged files are
in (`kind(format,cli): `), so only the kind and subject are left to write.
`fixup!` and `squash!` commits are accepted locally but refused when pushed:
squash them first.

## What the hooks check

The hooks in `.githooks/` are one line each; [flow.toml](flow.toml)'s
`[stages]` says what each runs, and CI runs the same checks.

- **`commit-msg`**, every commit: the message, as above.
- **`pre-commit`**, every commit: `no-internal`, `secrets`, `fmt`, `gates`.
- **`pre-push`**, every push: `messages`, `hold`, `clippy`, `tests`.
- **`ci`**, once per push in CI: `messages`, `no-internal`, `secrets`,
  `fmt`, `gates`, pinned actions and tools, the command reference.
- **`build`**, on each CI platform: `clippy`, `tests`.
- **`msrv`**, in CI: the tests on the oldest supported Rust.

When one fails:

- **`no-internal`**: unstage `docs/internal/`.
- **`secrets`**: something looks like a credential. Remove it; if it was a
  real one, rotate it, since it may already be in a reflog or a push.
- **`fmt`**: run `cargo fmt --all` (after `mto`, as above).
- **`gates`**: `cargo xtask gates` lists each problem with its file and line.
- **`hold`**: `main` and release tags are pushed only by a release; push your
  branch instead.
- **`messages`**: one of the pushed commits breaks the rules above; reword it
  (`git rebase -i`) before pushing.

To run a stage by hand: `cargo xtask flow check ci`.

## Pull requests

Push a branch and open a pull request against `main`. CI checks its commits
and the tree, builds and tests on Linux (x86_64 and aarch64), macOS and
Windows, and tests on the oldest supported Rust. Say what the change does for
someone using `mto`; for a formatting change, include the corpus numbers
`cargo xtask diff` printed.

## Reporting bugs

`mto report` writes a bug report with everything needed to reproduce a
problem: the input, the settings, what `mto` did and which build it was. Open
an issue with it attached. For a security problem, see
[SECURITY.md](SECURITY.md) instead.

## License

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as in the [README](README.md#license),
without any additional terms or conditions.
