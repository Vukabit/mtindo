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
  in the same file follow the order of the trait's declaration. In an impl
  with no trait, `inherent_method_order = "constructors_first"` puts the
  functions that make the type (no receiver, returning `Self` or the type)
  first, and `"by_receiver"` orders them: no receiver, `&self`, `&mut self`,
  `self`. Each part keeps its written order.

The order itself is set by `type_order`, `visibility_order`, `name_sort`,
`group_impls`, `trait_impl_sort`, `trait_impl_order` and `test_module`; every
setting is listed in the [settings reference](../generated/config/README.md).

## Keeping your own structure

Sorting item by item can split up code you grouped on purpose. These settings,
all off by default, keep that grouping:

* **`keep_paragraphs = true`:** items written with no blank line between them
  form a paragraph that moves as one. It is placed by its first item, never
  sorted or split inside, and keeps its tight spacing:

  ```rust
  fn z() {}

  fn parse() {}     // a paragraph:
  fn parse_one() {} // these two stay together, placed as `parse`

  fn a() {}
  ```

  becomes `a`, then the `parse` paragraph, then `z`. Imports, `mod name;`
  declarations, `#[cfg(test)]` modules, macro definitions and pinned items
  never join a paragraph. `mto init` turns it on, as it must be on before the
  first layout run: a run without it puts blank lines between those items,
  and the paragraphs are gone.
* **`sections`:** cuts a file (and each module and impl body) into sections
  at comments that stand on their own, with a blank line after them. Items
  are ordered within their section and never cross into another, and the
  comment stays where it was written:

  ```rust
  fn parse() {}
  fn lex() {}

  // Helpers

  fn skip_spaces() {}
  fn is_digit() {}
  ```

  keeps `skip_spaces` and `is_digit` below `// Helpers`, each part sorted on
  its own. Which comments cut:

  * `"comments"`: every such comment;
  * `"headings"`: only heading-like ones, one or two lines that read as
    prose and not as code, and are not a TODO, FIXME, SAFETY or NOTE. Other
    comments that stand on their own keep their place instead of moving with
    the item below;
  * `"markers"`: only `// mto: section` lines (a reason may follow
    ` -- `, as with every directive).

  Off by default: a comment set apart by a blank line moves with the item
  under it. A `#[cfg(test)]` module goes last in its section.
* **`keep_helpers_near = true`:** a private function, constant, static, type
  alias, struct, enum or union that exactly one other item uses stays
  directly next to that item and moves with it, on the side it was written
  on, instead of sorting off to its own group:

  ```rust
  const LIMIT: usize = 8; // used only by `parse`: stays above it

  pub fn parse() { /* uses LIMIT */ }
  ```

  A private type takes its impls along, and a helper of a helper stays next to
  the item it helps. Uses are found by name, without type checking, so
  anything unclear sorts as usual: a name defined twice, an item used by two
  others, or one whose user is in another section or keeps its place.

## Choosing what decides the order

`order_keys` names what items are ordered by, in turn; each key breaks the
ties the one before it leaves. The default, `["kind", "visibility", "name"]`,
is the order described above.

* `["visibility", "kind", "name"]` puts the public items first: the API, then
  what it is built from. An impl goes with its type, at the type's visibility.
  Unless `kind` comes first, imports, `mod` declarations and macros still come
  first, each kind with its own rules.
* `[]` keeps the order the items were written in. Only runs of imports and of
  `mod name;` declarations written together are sorted where they stand, and
  `#[cfg(test)]` modules go last; `sections`, `keep_paragraphs` and
  `keep_helpers_near` still apply. Use it to take only the parts of layout a
  project wants.

`flow` then orders items by their uses, within one kind (functions among
functions, types among types; among everything but imports, modules and
macros when `kind` is not a key):

* `"top_down"`: an item goes above the items it uses, so the file reads from
  the entry points down to what they are built from;
* `"bottom_up"`: an item goes below the items it uses, each defined before
  its first use.

```rust
fn parse() { lex(); } // top_down: parse, lex, skip_spaces
fn lex() { skip_spaces(); }
fn skip_spaces() {}
```

Where no use decides, the keys do: the order is the keys' order with only
the moves the uses ask for. A type moves with its impls and `#[cfg]` variants
together; items that use each other in a circle keep the keys' order among
themselves. Uses are found by name, without type checking, like
`keep_helpers_near`.

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
  The item under a `// mto: hold` line keeps its neighbours too: nothing moves
  across it.
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
own, holding nothing but what mto wrote, and add that commit to
`.git-blame-ignore-revs` so `git blame` looks past it. Start from a clean tree:
anything else in the commit would be hidden from `git blame` too.

```bash
git status --porcelain   # prints nothing on a clean tree; commit or stash first
mto init
mto -l . > laid-out.txt  # names each file mto writes
git add mto.toml $(cat laid-out.txt)
rm laid-out.txt
git commit -m "Lay out every file with mto"
mto verify HEAD          # every Rust file reproduced: only mto's work
git rev-parse HEAD >> .git-blame-ignore-revs
git add .git-blame-ignore-revs
git commit -m "Ignore the layout commit in git blame"
git config blame.ignoreRevsFile .git-blame-ignore-revs
```

A commit `git blame` looks past is a good place to hide a change, so a
reviewer, or CI, can check one: `mto verify REV` runs `mto` on each Rust file
the commit changed, from its parent, and passes only when every one comes out
byte for byte as committed, its checks passing on the way. CI can run it for
each commit listed in `.git-blame-ignore-revs`.

`mto restore last` undoes the run if the result is not what you want; see
[Safety and history](safety.md). How layout works inside is described in the
[layout design](../design/layout.md).
