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
* [Metrics](generated/metrics/README.md): compatibility, speed, memory,
  fuzzing and more, as measured.

## Design

How `mto` works inside: the [design pages](design/README.md).

## Development and credits

* [CONTRIBUTING.md](../CONTRIBUTING.md): building, testing, fuzzing,
  benchmarking, and every development command.
* [CREDITS.md](../CREDITS.md): everything `mto` builds on or tests against.
