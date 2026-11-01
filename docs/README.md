# mtindo documentation

Every public page, by what it is for. The [README](../README.md) has the
overview, installation and a quick start.

## Guide

How to use `mto`.

* [Layout](guide/layout.md): turning layout on, what it does, what never
  moves.
* [Formatting](guide/formatting.md): the Rust style, settings, what is left
  as written.
* [Command line](guide/command-line.md): options, `cargo mto`, commands, exit
  codes.
* [Configuration](guide/configuration.md): `rustfmt.toml`, `mto.toml`, the
  user config.
* [Safety and history](guide/safety.md): checked writes, bug reports,
  history, restore.
* [The build guard](guide/build-guard.md): holding back edits that break the
  build.
* [CI and editors](guide/ci-and-editors.md): `--check`, JSON output, hooks,
  rust-analyzer.

## Reference

Generated from the code and the recorded measurements; never edited by hand.

* [Settings](generated/config/README.md): every setting, its values and
  default, and a JSON Schema of `mto.toml`.
* [Manual](generated/manual.md): every command, flag, setting and value, as
  `mto --man` prints it.
* [Development commands](generated/commands.md): every `cargo xtask`
  command and its arguments.
* [Metrics](generated/metrics/README.md): compatibility, speed, memory,
  fuzzing and more, as measured.
* [Generated files](generated/README.md): every generated page, what it
  holds and what writes it.

## Design

How `mto` works inside: the [design pages](design/README.md).

## Project

* [CHANGELOG.md](../CHANGELOG.md): what each release changed.
* [CONTRIBUTING.md](../CONTRIBUTING.md): setting up, what a change keeps
  true, commits, and the checks.
* [MAINTAINING.md](../MAINTAINING.md): cutting and fixing releases, tool
  pins, credentials and the repository's settings.
* [SECURITY.md](../SECURITY.md): reporting a security problem.
* [CREDITS.md](../CREDITS.md): everything `mto` builds on or tests against.
