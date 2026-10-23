# Formatting

`mto` formats Rust source in the Rust style: spacing, indentation and line
breaks exactly as the standard formatter writes them with the same settings.
Formatting is on by default, so `mto` can take the standard formatter's place
in scripts, CI and editors.

## Settings

`mto` reads `rustfmt.toml` (and `.rustfmt.toml`) as the standard formatter
does and honors its settings, including the ones the standard formatter only
offers on its nightly channel: `mto` applies every setting on every channel.
`mto config options` shows how far each setting is honored, and the
[settings reference](../generated/config/README.md) lists them all with their
values. `format = false` in an `mto.toml` turns formatting off and leaves only
layout.

Settings can also be given for one run: `--config max_width=80,hard_tabs=true`,
`--edition 2021`, `--style-edition 2024`. See
[Configuration](configuration.md) for how files combine.

## What is left as written

Like the standard formatter, `mto` leaves as written what it would give up
on:

* `macro_rules!` definitions, and macro calls whose arguments are not code;
* anything marked `#[rustfmt::skip]`, and `// mto: off` regions;
* a statement or item that cannot fit within `max_width`;
* a file with syntax errors, which is reported and left unchanged.

## Every format is checked

Formatting changes whitespace and the few tokens the Rust style rewrites (a
trailing comma, the order of imports, braces around a closure's body). Before
anything is written, a meaning check compares the tokens of the input and the
output: they must match apart from those rewrites, and every comment must
survive. A second run changes nothing. See
[Safety and history](safety.md#every-write-is-checked), and the
[meaning check design](../design/meaning-check.md) for the full list of
rewrites.

## Formatting and layout together

With layout on, `mto` lays a file out first and formats the result, in one
run and one write. Layout keeps to the standard formatter's import order and
blank-line bounds, so formatting never undoes it.

## How close it is

`mto` is measured against the standard formatter's own tests and against a
pinned set of real-world repositories, as fetched and with their whitespace
scrambled. The [compatibility paper](../generated/metrics/compatibility.md)
has the current numbers, and the [metrics index](../generated/metrics/README.md)
the rest.
