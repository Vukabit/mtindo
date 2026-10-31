# mtindo

`mtindo` formats and lays out Rust source files. Its command is `mto`.

* **Formatting**, on by default: spacing, indentation and line breaks in the
  Rust style, exactly as the standard formatter writes them, reading the same
  `rustfmt.toml` (`mto config options` shows which of its settings are
  honored yet). It can take the standard formatter's place in scripts, CI and
  editors.
* **Layout**, when a project opts in: it reorders a file's items (modules,
  imports, macros, constants, types with their impls, functions) into a
  predictable top-down order, which the standard formatter leaves alone. No
  more scrolling past 800 lines of helper functions just to find the main
  `struct` definition.

Every write is checked first, so the code still means what it meant. Every
run is recorded, so `mto restore last` undoes it. `mto` has no dependencies.

## Installation

Download a prebuilt archive from the
[releases page](https://github.com/Vukabit/mtindo/releases): Linux (x86_64
and aarch64, glibc 2.35 or newer), macOS (Apple silicon and Intel) and
Windows (x86_64). Each holds `mto` and `cargo-mto`; put both on your `PATH`.
Check an archive against the release's `SHA256SUMS` first:

```bash
sha256sum --check --ignore-missing SHA256SUMS      # Linux
shasum -a 256 --check --ignore-missing SHA256SUMS  # macOS
```

Or build it with Cargo, which needs Rust 1.88 or later and nothing beyond
the Rust standard library:

```bash
cargo install mtindo
```

This places `mto` and `cargo-mto` in Cargo's binary directory
(`~/.cargo/bin`); the second makes `cargo mto` work. From a clone of the
repository, run `cargo install --path .` in its root.

## Quick start

```bash
mto src/main.rs        # format one file (and the modules it declares)
cargo mto              # every target of a package or workspace
mto --check .          # show what would change, exit 1 if anything would
mto init               # opt this project into layout (writes mto.toml)
mto restore last       # undo the last run
```

## Documentation

The [documentation index](docs/README.md) lists every page. The guide:

* [Layout](docs/guide/layout.md): turning it on, what it does, and what
  never moves.
* [Formatting](docs/guide/formatting.md): the Rust style, settings, and what
  is left as written.
* [Command line](docs/guide/command-line.md): options, `cargo mto`, commands
  and exit codes.
* [Configuration](docs/guide/configuration.md): `rustfmt.toml`, `mto.toml`
  and the user config.
* [Safety and history](docs/guide/safety.md): checked writes, bug reports,
  history and restore.
* [The build guard](docs/guide/build-guard.md): holding back edits that
  break the build.
* [CI and editors](docs/guide/ci-and-editors.md): `--check`, JSON output,
  hooks and rust-analyzer.

The [manual](docs/generated/manual.md) (`mto --man`) documents every
command, flag and setting, and the
[settings reference](docs/generated/config/README.md) every setting with its
legal values.

## Where it's headed

`mto` aims to be a complete Rust formatter with per-team control over file
layout: output in the Rust style, identical to the standard formatter's by
default, with no dependencies and nothing else to install.

## Development

`cargo xtask test` runs every gate and test suite. How to build, test, fuzz,
benchmark and watch the project, and every development command, are in
[CONTRIBUTING.md](CONTRIBUTING.md). How `mto` works inside is described in
the [design pages](docs/design/README.md).

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
* MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. The identifier and width tables in `src/text/unicode` are
derived from the Unicode Character Database, under the terms in
[LICENSE-UNICODE](LICENSE-UNICODE).

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

## Credits

`mto` takes after [rustfmt](https://github.com/rust-lang/rustfmt) in its
formatting rules, its settings and its command line, and checks itself
against rustfmt's own tests. Thanks to its authors and maintainers. Every
project `mto` builds on or tests against is listed in [CREDITS.md](CREDITS.md).
