# Maintaining mtindo

What a maintainer does beyond [CONTRIBUTING.md](CONTRIBUTING.md): cutting and
fixing releases, keeping the tool pins and the corpus record current, the
credentials CI uses, and the repository's settings on GitHub. Everything here
runs through `cargo xtask flow` and [flow.toml](flow.toml); the
[command reference](docs/generated/commands.md) has every argument.

## The release hold

`main` and release tags (`v*`) move only through a release. The `pre-push`
hook refuses any other push to them; `cargo xtask flow release` marks the
commit and tag it makes, and those alone pass. Work happens on branches,
pushed and checked by CI like anyone's.

## Cutting a release

1. **Land the work** on a branch that `main` fast-forwards to, push it, and
   wait for CI to pass on its last commit.
2. **Run the corpus** on that exact tree: `cargo xtask diff`. A release needs
   a passing run for the commit it is cut from. To record new numbers, run
   `cargo xtask diff --update` and commit `compat/compatibility.txt` first.
3. **See the plan**: `cargo xtask flow release 0.2.0`. It checks the tree is
   clean and pushed, CI passed, the corpus run, the credentials' expiry and
   that the crate builds as crates.io would get it, then says what it would
   do.
4. **Make it**: `cargo xtask flow release 0.2.0 --go`. It sets the version in
   `Cargo.toml` and `Cargo.lock`, names `CHANGELOG.md`'s newest section for
   the release (`cargo xtask changelog --tag v0.2.0`), commits
   `chore(release): v0.2.0`, and tags it. If any step fails, those files are
   put back as they were.
5. **Push** what it prints: `git push origin HEAD:refs/heads/main
   refs/tags/v0.2.0`.
6. **The release workflow** (`.github/workflows/release.yml`) then builds the
   binaries for every target in `release.targets`, makes the GitHub release
   with them, `SHA256SUMS` and the changelog section as its notes, and waits
   for approval before publishing to crates.io. Approve it in the run's page.

Before a release that changes how the binaries are built, push a branch named
`dry-run/<anything>`: `release.yml` builds and packs every target and writes
the checksums as a release would, publishes nothing, and runs the packed
Linux binary, saying the oldest glibc it needs (on its summary page).

The changelog is written from commit subjects by git-cliff, under the
headings `flow.toml`'s `[kinds]` give; `cargo xtask changelog` shows the
next section at any time, and wraps it to 80 columns.

## The first release

v0.1.0 goes out quietly. In order:

1. Write `CHANGELOG.md`'s `## [0.1.0]` section by hand (there is no earlier
   tag for git-cliff to start from).
2. **Before going public**, scan every commit ever made, not only the tree:
   `cargo xtask flow check public`. Nothing that looks like a credential, and
   nothing from `docs/internal/`.
3. Make the repository public, then the GitHub settings below (rulesets and
   approval on environments need a public repository on a free plan).
   `cargo xtask flow check github` lists any setting still missing; a release
   checks the same and stops until they are made.
4. Run a dry run (above) and look at its summary.
5. Cut the release as above. `release.first_as_draft` leaves the GitHub
   release a draft: look over the binaries and notes, then publish it.
6. crates.io's trusted publishing needs the crate to exist, so publish 0.1.0
   by hand once (`cargo publish`, with an API token, revoked after), then add
   the repository as a trusted publisher on the crate's settings page
   (workflow `release.yml`, environment `crates-io`). Reject the run's
   `crates-io` job for 0.1.0; later releases publish from CI.

## Fixing an earlier release

When `main` has moved on and a fix belongs in an earlier release, cut it from
a branch for that release line:

```bash
git switch -c release/0.1 v0.1.0
git cherry-pick <the fix>
git push -u origin release/0.1
# CI passes, cargo xtask diff passes, then:
cargo xtask flow release 0.1.1 --go
```

A `release/X.Y` branch takes only `X.Y.*` versions and is pushed itself,
not `main`.

## Taking a release back

`cargo xtask flow yank 0.2.0` prints the steps and runs none of them: yank it
on crates.io (projects that already depend on it keep building), mark the
GitHub release as a yanked pre-release so pinned binaries stay reachable, and
cut the fix as the next patch release. Tags others have fetched are not
deleted.

## Tool pins

`flow.toml`'s `[tools]` pins every tool development uses; `cargo xtask
bootstrap` installs them, and CI uses the same. To update one, change its
version there and run `bootstrap --go`. Two pins are also written elsewhere,
and the `tools-pinned` check keeps them in step:

- **The stable toolchain** is also `rust-toolchain.toml`'s channel. Its
  rustfmt is the corpus's answer key, so moving it can move the corpus
  numbers: run `cargo xtask diff`, and if the reference changed, record it
  with `--update` and say so in the commit.
- **The oldest supported Rust** is also `Cargo.toml`'s `rust-version`.

GitHub Actions are pinned by commit, with the release in a comment
(`actions/checkout@<commit> # v5.1.0`); the `actions-pinned` check refuses a
tag or branch. To move one, look up the commit its new release points to
(`gh api repos/OWNER/ACTION/commits/vX.Y.Z --jq .sha`) and change both.

## The corpus

`compat/corpus.txt` pins real crates at fixed commits; `cargo xtask diff`
formats them with `mto` and the standard formatter and compares, failing
below `compat/compatibility.txt`. `cargo xtask corpus update` moves every pin
to its repository's current head; record the new numbers after. Findings are
filed as incidents (`cargo xtask incidents`), as fuzzing's are
(`cargo xtask fuzz run TARGET`).

## Measurements only a maintainer takes

The memory series (`memst.serde.*`), their limits in
`compat/performance.txt` and the papers' memory rows are measured with memst
(`cargo xtask memst`), a profiler of its own that is not yet published. Until
it is, keeping them current is the maintainer's job: `flow.toml` marks memst
optional, `cargo xtask test` skips its task without it, and nothing a
contributor or a release runs needs it. Once memst is on crates.io, its
`[tools.memst]` pin becomes a `cargo` tool like the others and these notes
go.

## Credentials

Nothing secret is kept in the repository; the `secrets` check refuses what
looks like one, and `flow check public` looks through all of history.

- **crates.io** uses trusted publishing: each release run is given a
  short-lived token by crates.io itself, so there is none to store or rotate.
- **Any secret CI comes to need** is stored on the environment of the job
  that uses it, not on the repository, and its expiry written in
  `flow.toml`'s `[release.tokens]` (`NAME = "2027-01-28"`). GitHub never
  shows a secret again, so that line is how the expiry is known: `flow
  release` warns from 14 days before it and refuses once it has passed.

## GitHub settings

Made by hand on the repository's settings pages; `cargo xtask flow check
github` says which are missing:

- **Branch `main`**: a ruleset requiring the `flow`, `msrv` and every `test`
  check, refusing force pushes and deletion.
- **Tags `v*`**: a ruleset refusing updates and deletion.
- **Environments**: `crates-io`, requiring your approval, limited to tags
  `v*`.
- **Actions**: workflow permissions read-only by default (each job asks for
  more itself, as `release.yml` does).
- **Security**: private vulnerability reporting on, for
  [SECURITY.md](SECURITY.md).
