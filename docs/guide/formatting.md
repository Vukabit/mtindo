# Formatting

`mto` formats Rust source in the Rust style: spacing, indentation and line
breaks exactly as the standard formatter writes them with the same settings.
Formatting is on by default, so `mto` can take the standard formatter's place
in scripts, CI and editors.

## Settings

`mto` reads `rustfmt.toml` (and `.rustfmt.toml`) as the standard formatter
does. Each setting it supports applies on every channel, the ones the
standard formatter only offers on nightly too. Some settings are not honored
yet, and those are ignored: `mto config options` says for each one whether it
is honored, in part or not yet (its `MTO` column), and the
[settings reference](../generated/config/README.md) lists them all with their
values. `format = false` in an `mto.toml` turns formatting off and leaves only
layout.

Settings can also be given for one run: `--config max_width=80,hard_tabs=true`,
`--edition 2021`, `--style-edition 2024`. See
[Configuration](configuration.md) for how files combine.

## What is left as written

Like the standard formatter, `mto` leaves as written what it would give up
on:

* `macro_rules!` definitions and macro calls whose contents are not code;
* anything marked `#[rustfmt::skip]`, and `// mto: off` regions (below);
* a statement or item that cannot fit within `max_width`;
* with `format_generated_files = false`, a generated file: one with a comment
  near its top holding one of `generated_markers` (`@generated` by default);
* a file with syntax errors, which is reported and left unchanged, and one
  nesting more than 1024 levels deep.

`mto debug layout FILE` lists each place in a file left as written, and why.

## Leaving code as written

Put `// mto: off` and `// mto: on` around code that must stay exactly as it
is, such as a table aligned by hand:

```rust
// mto: off
const IDENTITY: [u8; 4] = [
    1, 0,
    0, 1,
];
// mto: on
```

Between them nothing is moved or changed, the markers included.

* **A marker is a line of its own:** a line comment that is only the marker.
  A reason may follow after ` -- `: `// mto: off -- aligned by hand`.
* **A marker belongs to no item.** A comment under `// mto: off` belongs to
  the code below it, and the item after `// mto: on` is formatted and laid
  out as any other.
* **A region stays in its block.** It ends at `// mto: on` in the same
  block, or at the end of that block when there is none. A region among the
  parts of one statement or item (between list items or match arms) keeps
  that whole statement or item as written.
* **Mistakes are reported, never followed:** `//mto: off`, `// MTO: off`,
  `// mto: of`, words after a marker without ` -- `, a marker after code on
  its line or in a `/* */` or doc comment, and a marker without its partner
  each get a warning naming the line.

`// mto: pin` on a line of its own keeps the item below it in its place in
the [layout](layout.md).

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
run and one write; when formatting puts a body written on one line on lines
of its own, its items are laid out too, so a second run changes nothing.
Layout keeps to the standard formatter's import order and blank-line bounds,
so formatting never undoes it.

## How close it is

`mto` is measured against the standard formatter's own tests and against a
pinned set of real-world repositories, as fetched and with their whitespace
scrambled. The [compatibility paper](../generated/metrics/compatibility.md)
has the current numbers, and the [metrics index](../generated/metrics/README.md)
the rest.
