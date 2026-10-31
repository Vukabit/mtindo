# Layout

Layout reorders a file's items (modules, imports, macros, constants, types
with their impls, functions) into a predictable top-down order: the vertical
organization of a file, which the standard formatter leaves alone. It moves
items whole and changes nothing inside them.

## Turning layout on

Layout is opt-in. Until an `mto.toml` with `layout = true` applies to a file,
`mto` moves none of its items; it only formats it. To opt in, run this in the
project root:

```bash
mto init
```

It writes an `mto.toml` with `layout = true` and every other setting at its
default, each with a comment. See [Configuration](configuration.md) for the
settings and how files combine.

## What layout does

* **Parsed, not guessed:** every file is parsed into a syntax tree that keeps
  every byte. A file with syntax errors is left untouched and reported as
  skipped.
* **Items move whole:** layout moves each item byte for byte. Only the order
  of items and the blank lines between them change, at the top level and
  inside inline `mod name { ... }` blocks, at every level. Line endings (LF
  or CRLF) and a leading byte-order mark are preserved.
* **Keeps to the Rust style:** imports and `mod` declarations are sorted
  exactly as the standard formatter sorts them for the style edition, and
  blank lines stay within its bounds, so formatting after `mto` leaves the
  layout alone. This is checked on 44 real-world repositories.
* **Sticky impl grouping:** `impl Target` and `impl Trait for Target` blocks
  stay directly beneath the type they implement: the type's own impls first,
  then trait impls by trait name (or a list of your choice).
* **Visibility hierarchy:** items of the same kind are ordered by visibility,
  widest first (`pub`, `pub(crate)`, `pub(in path)`, `pub(super)`, private),
  then by name with the Rust style guide's version sorting (`u8` before
  `u16`). Modules are the exception: like the standard formatter, mto orders
  them by name alone.
* **Comments travel with their item:** a comment directly above an item, or
  at the end of its last line, moves with it.
* **Impl items:** with `reorder_impl_items`, items inside an impl block are
  sorted exactly as the standard formatter sorts them; with
  `impl_method_order = "trait"`, the functions of an impl of a trait declared
  in the same file follow the order of the trait's declaration.

The order itself is set by `type_order`, `visibility_order`, `name_sort`,
`group_impls`, `trait_impl_sort`, `trait_impl_order` and `test_module`; every
setting is listed in the [settings reference](../generated/config/README.md).

## What mto never moves

Some items depend on their position in the file. `mto` keeps them in place:

* **The file header:** a shebang line, inner doc comments (`//!`, `/*! */`),
  inner attributes (`#![...]`), and a leading comment block separated from
  the first item by a blank line (such as a license header) always stay
  first.
* **`macro_rules!` definitions** keep their original order relative to each
  other, and never end up below an item that originally came after them,
  since a macro is only visible to code that follows it. When the configured
  order would break that, macros are placed first.
* **A file that defines the same macro name twice** keeps its items where
  they are, because there the order of definitions changes behavior. It is
  still formatted, and a note says layout passed it over.
* **`#[macro_use]` modules** move ahead of the other modules, in their
  original order, so every module that used their macros still can; like
  `macro_rules!`, nothing that came after one moves in front of it.
* **`extern crate` declarations and item-position macro calls**
  (`thread_local! { .. }`, `include!(..)`, your own `make_items!();`) stay at
  their original position in the file.
* **Items marked to stay:** `#[rustfmt::skip]` items, and the item under a
  `// mto: pin` line (a comment may stand between them), keep their position.
  A file whose header has `#![rustfmt::skip]` is left alone.
* **Generated files:** a file with a comment near its top holding one of
  `generated_markers` (`@generated` by default) keeps its items where they
  are, unless `layout_generated_files = true`. Whether it is formatted follows
  the standard formatter's `format_generated_files`. A folder of generated
  code is better left out whole with `ignore_dirs`.
* **`#[cfg]` variants** of one item (`#[cfg(unix)] fn f` and
  `#[cfg(windows)] fn f`, written next to each other) move as one.
* **A `#[cfg(test)]` module** goes last (or stays where it is, with
  `test_module = "in_place"`).
* **Anything between `// mto: off` and `// mto: on`** is left exactly as
  written, in its original position; see
  [Leaving code as written](formatting.md#leaving-code-as-written) for the
  rules.
* **A file with syntax errors** is left unchanged, and the run exits with
  code `3`.
* **A file nesting more than 1024 levels deep** (generated code; hand-written
  code stays far below) is skipped and left unchanged, neither laid out nor
  formatted, as mto does not follow it that far.

## Adopting layout in an existing project

Laying out a whole project once moves many items. Do it in a commit of its
own, and add that commit to `.git-blame-ignore-revs` so `git blame` looks past
it:

```bash
mto init
mto .
git add mto.toml
git commit -am "Lay out every file with mto"
git rev-parse HEAD >> .git-blame-ignore-revs
git config blame.ignoreRevsFile .git-blame-ignore-revs
```

`mto restore last` undoes the run if the result is not what you want; see
[Safety and history](safety.md). How layout works inside is described in the
[layout design](../design/layout.md).
