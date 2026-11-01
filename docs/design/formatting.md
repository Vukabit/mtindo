# Formatting

mto formats a file in the Rust style by porting the standard formatter's
width-budget layout and by copying where it gives up and keeps code as written.
This page describes the passes in `src/format/`, how an expression is fitted
into the space left for it, and how the output is kept stable on a second run.

Formatting runs on the text the item layout of [layout.md](layout.md)
produced (`src/lib.rs` calls `format_file`). It changes whitespace, comments'
places and a few tokens the standard formatter also rewrites;
[meaning-check.md](meaning-check.md) describes how the result is checked, and
[syntax.md](syntax.md) the lossless tree it reads. [overview.md](overview.md)
places formatting among the other parts of mto.

## The file pass

`format_file` in `src/format/file.rs` runs one pass, `format_text`, and runs
it again on its own output, at most twice more, when a statement was kept as
written or a list holds a comment, keeping the text it settles on. Each pass
opens the layout memo, keeps the edition, style edition and macros to skip
in thread-local cells, and runs `format_text_with`. That returns the input
unchanged under `disable_all_formatting`, for a generated file (a comment
near the top holding one of `generated_markers`) unless
`format_generated_files` is on, for a lone carriage return, a
parse error, or a file that skips itself with an inner `#[rustfmt::skip]`.

It then applies text rewrites, each in `src/format/reorder.rs`, each followed by
a fresh parse when it changed anything:

1. `extern` without an ABI becomes `extern "C"` (`force_explicit_abi`).
2. Empty statements (`;` alone) are dropped, but not in macro bodies.
3. `use` declarations of empty lists (`use a::{};`) are dropped.
4. Comments on lines after a `match`'s last arm move to the end of that arm's
   line, as the standard formatter attaches them.
5. Comments between a `loop`, `while`, `for` or `if` head and its block go on
   lines of their own, the `{` after them.
6. A field's or variant's comma written on the next line joins the field.
7. Runs of `#[derive(..)]` merge into one (`merge_derives`).
8. Runs of `use`, `extern crate` and `mod name;` are sorted
   (`reorder_imports`, `reorder_modules`), unless a comment is among them.

The rewrites leave `// mto: off` regions alone. Then `reindent` writes the
file, and line endings come last: `newline_style` picks `\n` or `\r\n` (`Auto`
follows the first line), but a file whose strings span lines keeps its own
endings, which are part of the strings. A byte-order mark is put back.

## Roles and the token pass

`reindent` walks every token once. Before it starts, `roles` in
`src/format/file.rs` marks what each token does, in a `Roles` table keyed by
byte offset:

- **Containers and starts.** Blocks, item and field lists, match arms and
  delimited lists are containers. A token that starts something in one (an
  item, statement, field, arm, element or comment) is indented one level past
  the line the container's owner starts on. Other lines continue the construct
  above them and move by as much as it moved, so a multi-line expression keeps
  its shape.
- **Verbatim regions.** Ranges written out exactly as they are: anything under
  `#[rustfmt::skip]`, `// mto: off` regions as `syntax::marker` finds them,
  `macro_rules!` definitions whose rules cannot be formatted, attributes that do
  not read as names and values, comments the standard formatter cannot place,
  and statements with a written line past `max_width` that are not laid out
  whole. A braced macro call's arguments are re-indented as a block of text,
  keeping their layout (`preserve_layout`).
- **Layable nodes.** Statements in blocks, `use` declarations, attributes,
  fields, variants, match arms and item headers that the width-budget code may
  lay out. When the pass reaches the first token of one at the start of a line,
  it knows the column, and asks the right module for the text.
- **Breaks and spaces.** Tokens the Rust style starts a line with, tokens
  that follow one space after the one before (`} else`), and braces that close
  up (`{}`). Spaces within a line follow `src/format/space.rs`.

For a layable node the pass tries, in order:

- **A statement laid out whole** by `rewrite_statement` in
  `src/format/stmt.rs`. The pass writes the text and skips the node's tokens.
- **The head of a statement holding one block.** A closure's body, an `async`
  block or a block as a `let`'s value: `rewrite_head` (or
  `rewrite_let_control`, `rewrite_control_head`) lays out everything up to the
  block's `{`, the token pass writes the block's insides as usual, and the tail
  after `}` (`});`, `?`, chain links) is glued onto the closing line.
- **An item's header** by `rewrite_header` in `src/format/items.rs`: all
  before the body's `{`, or the whole item when it ends in `;`.

Otherwise the written lines are kept, re-indented. A statement whose first
line passes `max_width` at its new indentation is kept as written, as the
standard formatter gives up on it; measured as the pass reaches it, so a
second run decides the same.

## Shapes

An expression is laid out into a `Shape` (`src/format/shape.rs`): `indent`,
the column of its line's indentation; `offset`, how far past that it starts;
`width`, the columns left on its first line; and `align`, a column that a
type's later lines line up to. `block_indent` gives the shape one level in on a
line of its own; `offset_left` and `sub_width` take room for text before and
after. `Limits` carries the settings the layouts read (`max_width`,
`tab_spaces`, the list, chain and single-line widths, the style edition) and
flags set inside attributes or macro arguments.

## Expressions

`rewrite(node, shape)` in `src/format/expr.rs` returns the node's text in that
shape, or `None` when it does not fit. Callers try layouts in the standard
formatter's order and take the first that fits; a value after `=` goes on its
line, else on the next, and `prefer_next_line` decides when both fit.

- **Binary operators.** Operands joined by the same operator are taken as a
  list: on one line when it fits, else each after the first on a line of its
  own, one level in, its operator first. A let chain goes one operand per line.
- **Chains.** A chain is cut into a root and links. A root no wider than one
  indentation level takes the first links onto its line. The rest go on one
  line within `chain_width`, else one link per line one level in. The last
  link may overflow, and is placed on a line of its own when that saves lines.
- **Calls and lists.** `rewrite_list` lays out arguments, arrays, tuples and
  generic lists. Short lists stay on the line; otherwise items go one per line,
  or packed for short array elements. The last item may overflow (a closure, a
  block, or a lone call, struct literal or tuple) so that the list stays on the
  opening line and only the last item breaks. A list with two closures does not
  overflow. A method chain overflowing a list keeps its links on one line.
- **Closures.** A body of one expression goes without braces on the closure's
  line when it fits; a block's statements go on lines; an expression too long
  for the line is wrapped in a block of its own, except where the standard
  formatter refuses that (an `async` block, an empty macro call, or a call or
  operator whose leftmost operand is block-like).
- **Macros.** A call in parentheses or brackets whose arguments parse as a
  call's arguments or an array's elements is laid out as one, under the macro's
  name. Format-like macros (`println!`, `assert_eq!` and the rest) keep the
  arguments before the format string together. Arguments in braces, ones that
  do not parse, and ones holding a comment are kept as written, as are calls
  that `skip_macro_invocations` names.

## Blocks, arms and give-ups

`src/format/block.rs` lays out blocks inside an expression (a closure's
body, an `async` block, a loop, a `match`, an `if`'s branches) as the standard
formatter does, with the visitor of a function body: each statement as the
file pass would lay it out at that indentation. Match arms come from
`src/format/arms.rs`: patterns, guard, `=>`, then a body on the arm's line
when it fits, or a block's `{` after `=>`.

The standard formatter gives up on a statement it cannot fit and keeps its
text. `gives_up` reproduces that rule: it lays the statement out with no line
width and gives up when a line still passes `max_width` and no layout within
the width fits, or when the statement holds a string spanning lines. It does
not give up on a statement holding a comment, an attribute, or one inside macro
arguments. Raw strings, and from the 2024 style strings continued with `\`,
are not measured. Comments inside nested blocks do not count, because those
blocks are laid out apart. `given_up` then keeps the statement's text. Inside
a closure or a one-expression block, only a given-up text on one line is kept;
one across lines would weigh the layouts around it by how it was written.

## Statements, items and lists

`src/format/stmt.rs` takes `let` statements, expression statements and block
tails. Before layout, `clean` turns away a statement holding what a layout
would lose or misplace (a comment outside an argument list or chain gap, a doc
comment, a string spanning lines). After it, `same_tokens` requires the text
to hold the statement's code tokens, but for commas, semicolons and braces,
which a layout may add or drop; a layout failing that is not taken.

`src/format/items.rs` ports the item headers: function signatures, `impl` and
trait headers, generics, type aliases, constants, and fields. A `where` clause
goes on lines of its own, one predicate per line. The header's tokens are
compared with the written ones (`same_header_tokens`). When an `impl`, trait or
type alias header fits no layout even with no line width, it is kept as
written (`header_gives_up`); for an `impl`, struct, union or trait the whole
item is kept, for other items only the header.

`src/format/lists.rs` places comments among list items as the standard
formatter's `itemize_list` and `write_list` do: a comment before an item goes
above it, or on its line when it is a block comment written there; a comment
after an item's comma stays on its line, aligned with its neighbours'. Line
comments put the list one item per line.

`src/format/imports.rs` puts a `use` on one line when it fits, else its items
fill lines one level in (the default `imports_layout`). One the standard
formatter would rewrite (`a::{b}` to `a::b`, an unsorted list, duplicates), or
holding a comment, is left as written.

## The memo

Trying layouts in order means the same node is laid out many times in the same
shape, and a block inside an expression is laid out once per layout of what
holds it, at every level of nesting. `RewriteMemo` in
`src/format/expr.rs` holds, per thread and per file, the result of `rewrite`
for each node identity and shape, plus macro arguments as read, flat texts and
yes-or-no answers about nodes. It keeps the trees alive so no node identity is
reused, and frees them when each pass returns. Nodes under 16 bytes are
not stored, since laying them out is cheaper than the lookup.

`remembered_block` keys a block's layout by only the part of the shape it
depends on: `block_body` in `src/format/block.rs` drops the offset and width
(keeping the width only when a comment is inside), so every layout tried
around a block reuses one layout of its insides; closures are kept by their
full shape, and `remembered_tokens` keeps token checks by the text without its
indentation. These keep deep nesting from growing exponentially.

## A second run changes nothing

The formatter's output must be its own fixed point. Where the standard
formatter is unstable, mto takes the settled result in one run: the file
pass formats its own output again where a kept statement or a commented list
could still move (above). It merges
derives across a blank line, which the standard formatter does only on its
second run. Where a choice could
depend on how code was written, mto decides from what a second run would see:
widths are measured at the new indentation, and a macro kept as written inside
an expression is taken only when its closing line already sits where a run
would put it. The fuzz target in `src/fuzzing/format.rs` checks both
properties on random input and settings: the meaning check is clean, and
formatting the output again gives the same text.

## Settings

`Config::format` in `src/config/mod.rs` builds a `FormatConfig`
(`src/options/format.rs`) from `rustfmt.toml`, `mto.toml`, the package's
edition and the command line; it scales the width settings from
`use_small_heuristics`. The file pass reads `FormatConfig` directly, and
expression layouts read it through `Limits::of` and `Spacing::of`.
`hard_tabs` writes indentation as tabs (`indentation`); with it set, or
`tab_spaces` at 0, the width-budget
layouts decline, and the file is only re-indented and spaced. The style edition
(`style_edition`, else `version`, else `edition`) selects the places where the
2024 style differs, such as continued strings and comments after a block's
last statement.

## Measuring compatibility

`cargo xtask diff` compares mto's output with the standard formatter's own
test pairs and with its output on a pinned corpus, as written and with the
whitespace scrambled. Exact file and line matches are recorded in
`records/results/compatibility.txt` and may only rise. `mto debug layout FILE`
explains each place code was left as written (`src/format/explain.rs`).
