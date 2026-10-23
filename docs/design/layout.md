# Layout

Layout reorders the items of a Rust file, and of every inline module and
optionally every impl body in it, by the configured order. It moves whole items
with their comments, keeps what depends on its position where it was, and
leaves a file alone when it cannot do that safely.

The code is in `src/layout/`: `mod.rs` cuts a file into pieces and puts it back
together, `classify.rs` reads what each item is, `order.rs` sorts, and
`impls.rs` orders impl bodies. `src/order/mod.rs` holds the standard
formatter's import order. The user-facing rules are in the README section
"What mto never moves"; this page explains how they are kept. See
[overview.md](overview.md) for where layout sits in a run,
[syntax.md](syntax.md) for the tree it reads,
[comment-attachment.md](comment-attachment.md) for which comments belong to
which item, and [meaning-check.md](meaning-check.md) for the check that the
result means what the input meant. Tests named below are in
`src/layout/tests.rs`.

## Parsing and refusals

`parse_any_edition` (`mod.rs`) parses the text as edition 2021, then 2018,
2015 and 2024, and keeps the first parse without errors, so code valid only in
an older edition (`fn async()`) still parses
(`test_code_only_valid_in_an_older_edition_still_parses`). The edition found
is returned with the layout and used by formatting and the meaning check.
The configured `edition` plays no part here; it only decides whether a
leading `::` in an import names the root.

A file is refused, and left unchanged, when:

- it has syntax errors in every edition (`SYNTAX_ERRORS`);
- it nests deeper than `MAX_DEPTH` (1024, `src/syntax/mod.rs`); a parse too
  deep in one edition is too deep in all, so no other edition is tried;
- it defines one `macro_rules!` name twice among the units of the file, since
  there the order of definitions changes behavior;
- the sort cannot keep macro definitions ahead of the items after them (see
  below).

`test_files_that_cannot_be_laid_out_say_why` shows the messages. Inside an
inline module or impl body, the same failures leave that body as written
instead of refusing the file. A header with an inner `#[rustfmt::skip]` (the
`#!` form) leaves the whole file as written, with `skipped` set.

## Header, units and tail

`split_container` walks the top-level elements of a container (the file, or
the braces of a module or impl body) and cuts them into three parts:

- **The header:** a byte-order mark, shebang, frontmatter, inner attributes,
  inner doc comments, and comment runs set apart from the first item by a blank
  line, such as a licence notice. It stays first, byte for byte. A comment run
  touching the first item heads that item instead
  (`test_a_comment_touching_the_first_item_heads_it`).
- **Units:** what layout moves. A unit is one item with the loose comments
  above it. The comments directly above an item, its attributes and doc
  comments, and a comment ending its last line are inside the item node
  already (see comment-attachment.md). Loose comments, set apart by a blank
  line, join the next item, so a section heading moves with the item under it
  (`test_section_comments_move_with_their_item`).
- **The tail:** comments after the last item, kept last
  (`test_trailing_comments_stay_last`).

Adjacent items that both have a `#[cfg]`, share a kind and a name, and are not
pinned merge into one unit, so `#[cfg(unix)] fn f` and `#[cfg(windows)] fn f`
move as one (`test_cfg_variants_move_together`). Two `#[cfg]` variants of one
macro are therefore one unit, not a duplicate name. A loose comment between
them keeps them apart.

## Classification

`classify::unit` gives each unit what the sort reads:

- **kind:** one of the `ItemKind` values in `src/options/layout.rs`
  (`ModDecl`, `UseDecl`, `MacroDecl`, `FnDecl`, `ImplBlock` and so on);
- **visibility:** `pub`, `pub(crate)`, `pub(in path)`, `pub(super)` or
  private; `pub(in crate)` counts as `pub(crate)`, `pub(in super)` as
  `pub(super)` and `pub(in self)` as private, because the standard formatter
  rewrites them so;
- **identifier:** the item's name, an import's tree without spaces or
  comments, or `Trait for Type` for a trait impl;
- **parent target** and **trait name:** for an impl, the type it is for and
  the trait's last path segment (`Debug` for `fmt::Debug`);
- **import group:** `Std` for `std`, `core` and `alloc`, `Local` for `crate`,
  `super` and `self`, `External` otherwise, and the `UseTree` the standard
  formatter sorts by;
- flags: `cfg`, `test_module` (`#[cfg(test)] mod`), `macro_use`,
  `module_declaration` (`mod name;`), `macro_call`, `defines_macros` and
  `pinned`.

`defines_macros` (`Item::defines_macros` in `src/syntax/ast/mod.rs`) is true
for `macro_rules!`, `macro` definitions, `#[macro_use]` modules, and item macro
calls whose tokens hold `macro_rules !` or `macro name`. An `// mto: off`
region whose text holds `macro_rules!` counts too.

## Pinned and held units

A **pinned** unit keeps its slot: its position among the units. `sorted` in
`order.rs` sorts the free units and fills the slots the pinned ones leave, so
free units may pass a pinned one (`test_pinned_items_keep_their_slot`):

```text
fn b() {}            fn a() {}
extern crate alloc;  extern crate alloc;
fn a() {}            fn b() {}
```

Pinned are: `extern crate`, item-position macro calls, items with
`#[rustfmt::skip]` (also under `cfg_attr`), items under a `// mto: pin`
comment, loose or attached (`test_pinned_items_keep_their_place`), a
`#[cfg(test)]` module with `test_module = "in_place"`, and `// mto: off`
regions. A region runs from its `// mto: off` (with any loose comments above
it) to the element carrying `// mto: on`, so an item with `// mto: on`
attached above it stays inside; with no `// mto: on` it runs to the end of the
container (`test_mto_off_regions_keep_their_slot`). A region is never looked
into. A pinned module or impl keeps its body as written.

A **held** unit is stronger: `layout_holding` takes a set of unit starts, and
`around_held` sorts only the runs between held units, so a held unit stays
between the same neighbours and nothing crosses it. The build guard
(`src/doctor/`) holds units back this way; a held module still has its body
laid out (`test_held_units_keep_their_places`).

## Fewest moves

Each `Move` names one unit that changed place. `moves_of` takes the arranged
units' original indexes, finds a longest strictly increasing run of them
(`longest_increasing`, by patience sorting), and reports only the units
outside it. The units in the run kept their relative order, so the moves are
the fewest units that, put back, restore the written order. Bringing one
import up from the end of a file is one move, not one for every item it passed
(`test_moves_are_the_fewest_units_out_of_order`). A move records its span, its
kind and name, and the unit it now precedes. Moves inside module and impl
bodies are collected through `Cx::note`, so holding works at every level.

## The order

`order::compare` decides between two free units; the sort is stable, so units
the rules call equal keep their written order. In turn:

1. A `#[cfg(test)]` module goes last (`test_test_module_last_or_in_place`).
2. Two imports: with `sort_imports` off, as written; on, by import group when
   `import_grouping` is on, then by `UseTree::compare` from `src/order/mod.rs`.
   Through style edition 2021 names compare snake_case, then CamelCase, then
   UPPER_SNAKE_CASE, each by bytes; from 2024 by version sort. `import_sort`
   overrides the style edition for imports
   (`test_imports_sort_as_the_standard_formatter_does_for_the_style_edition`).
3. The `type_order` rank, an impl taking the rank of the type it is for when
   `group_impls` is on. Kinds missing from `type_order` follow the listed ones
   in their default order, so no two kinds share a rank and the comparison
   stays one order (`test_items_compare_as_one_order_under_any_type_order`).
4. With `group_impls`, structs, enums, unions, traits and impls of different
   types order by the type's name, which puts each type's impls next to it. Of
   types sharing a name, impls join the one ranked first
   (`test_an_impl_of_a_shared_name_settles`).
5. The unit's own `type_order` rank, so a type comes before its impls.
6. Two `macro_rules!` keep their written order.
7. Two modules: `#[macro_use]` modules first, as written; then, with
   `sort_alphabetically`, by name without `r#`, by bytes through style edition
   2024 and by version sort after it
   (`test_modules_sort_as_the_standard_formatter_does`).
8. Two impls of one type: inherent impls first, as written; then trait impls
   listed in `trait_impl_order`, in its order; then by trait name with
   `trait_impl_sort = "name"`, or as written (`test_impls_under_a_type`).
9. Visibility, by `visibility_order`.
10. With `sort_alphabetically`, the name by `name_sort`: version sort (the
    Rust style guide's), bytes, or case-insensitive version sort with case as
    the tie-break (`test_name_sort_and_visibility_order`).

## Macro definitions

A macro defined by `macro_rules!`, a `#[macro_use]` module or a defining macro
call is visible only below its definition. After sorting, `arrange` checks
that no unit written after a definer now comes before it. If one does, it
sorts again with every definer first, in written order; if that still fails
(a pinned definer, such as a macro call, with free slots above it that only
units written after it can fill), the container is refused
(`test_nothing_moves_above_a_macro_call_that_defines_macros`).

## Inline modules and impl bodies

`module_with_body_laid_out` lays out the body of `mod name { ... }` by the
same rules at the body's own indentation, at every depth. A body on one line
with its braces, an empty body, or a body that cannot be laid out is left as
written (`test_inline_modules_are_laid_out_at_every_level`).

`impls::impl_with_items_ordered` changes an impl body only when asked:

- `reorder_impl_items` orders as the standard formatter does: plain
  associated types, then `impl Trait` types, each by name, then consts by
  name, then macro calls (no longer pinned here), then other items, then
  functions. Consecutive types or consts of one rank sit on consecutive lines;
  everything else is a blank line apart
  (`test_reorder_impl_items_as_the_standard_formatter_does`).
- `impl_method_order = "trait"` orders a trait impl's functions as the trait
  declares them, when the trait is declared in the file under one name only
  (`collect_traits`). Functions trade places among the functions' slots;
  functions the trait lacks go after, and everything else, blank lines
  included, stays (`test_impl_functions_follow_the_trait`).

A body closed on its last item's line is left as written, as moving that item
would strand a comment ending its line.

## Blank lines

`render` joins the header, units and tail. Between units it writes
`blank_lines_between_blocks` blank lines, except:

- imports of one group, and `mod name;` declarations with the same
  `#[cfg(test)]` status, sit `blank_lines_tight` apart;
- import groups, and a test module declaration after the others, are at least
  one blank line apart, because the standard formatter sorts runs not broken
  by a blank line and would merge them
  (`test_imports_group_and_declarations_stay_together`);
- a header ending in a comment, and the tail, keep at least one blank line, so
  a comment set apart never comes to touch an item and attach to it.

Inside a unit, `joined` keeps a gap of one line break as written and turns a
gap holding a blank line into the configured spacing, at least one blank line.
A comment that shared its line with the code before it always gets a blank
line after it (`test_spacing_zero_keeps_loose_comments_loose`).

The settings loader (`src/config/mod.rs`) clamps a configured
`blank_lines_between_blocks` to the standard formatter's
`blank_lines_lower_bound` and `blank_lines_upper_bound`, and sets
`blank_lines_tight` to the lower bound, so the standard formatter leaves the
spacing alone. Line endings and a byte-order mark are normalized before layout
and restored after (`test_crlf_and_bom_come_back`).
