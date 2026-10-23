# The meaning check

Before mto writes a file, it checks that the new text means what the old
one meant, and refuses the write when it cannot show that. This page
describes the two checks in `src/verify/`, what happens when one fails, and
how the checks themselves are kept honest.

See also: [overview](overview.md), [syntax](syntax.md), [layout](layout.md),
[formatting](formatting.md) and [writes](writes.md).

## Where the checks run

`verified_all` in `src/lib.rs` runs both checks before anything is written
or shown. When layout is on and changed the text, `verify_layout` compares
layout's output with the source. When formatting is on and changed the text,
`verify_format` compares the formatted text with layout's output, not with
the source. Each returns a list of `Violation`s, each a `Rule` and a
detail; an empty list means the text is safe to write.

## The layout check

Layout may move whole items and change the whitespace between them, and
nothing else. `verify_layout` in `src/verify/mod.rs` parses both texts and
splits each into parts (`Parts`), which the rules compare. Before parsing,
every `\r\n` reads as `\n`, so that one changed line ending is reported once
by its own rule and not as every item changed.

Scopes: the file is a scope, and so is the body of every inline module and
every impl block. A module or impl counts in its parent as its shell,
`mod name {…}`, and its body is collected as a scope of its own, named after
the shell. Every entry carries its scope's name, so an item cannot move
between scopes unnoticed.

The parts:

- **Header:** the shebang, frontmatter, inner attributes and inner doc
  comments, in order, scope by scope; and whether every header element still
  comes before the first item.
- **Items:** each item's exact text, with its attached comments, and its
  core text without them. Items are compared as a multiset by exact text, so
  order is free and repeats are counted.
- **Comments:** every plain line and block comment anywhere in the file,
  attached or not, compared as a multiset, byte for byte.
- **Others:** anything else at a scope's top level that is not whitespace,
  a comment or a brace: damaged text, stray tokens, a lone `;`.
- **Macros:** `macro_rules!` names in order, scope by scope. A macro defined
  in a `#[macro_use]` module also counts in its parent's scope, and further
  up for as long as the modules between are `#[macro_use]`.
- **Sequence:** each item in order with whether it defines macros. A
  definer is a `macro_rules!`, a `macro` item, a `#[macro_use]` module, or an
  item macro call whose tokens hold `macro_rules !` or `macro name`
  (`defines_macros` in `src/syntax/ast/mod.rs`).

The rules, by their names in reports:

| Rule                   | What it catches                                 |
|------------------------|-------------------------------------------------|
| `bom-changed`          | a byte-order mark added or removed              |
| `line-endings-changed` | `\n` endings became `\r\n` or mixed, or back    |
| `output-broken`        | more syntax errors in the output than the input |
| `header-changed`       | header changed, or no longer before every item  |
| `item-lost`            | an item missing from the output, or changed     |
| `item-added`           | an item not in the input, or one more copy      |
| `comment-reattached`   | an item unchanged but for its attached comments |
| `comment-lost`         | a comment of the input missing                  |
| `comment-added`        | a comment the input did not have                |
| `other-changed`        | other top-level text lost or added              |
| `macro-order-changed`  | `macro_rules!` order changed within a scope     |
| `macro-user-moved`     | an item moved above a macro definer it followed |

An input with no line break, or with mixed endings, may come out with any.
`comment-reattached` pairs a lost item with an added one of the same core
text: one item whose attached comments changed (see [syntax](syntax.md)).
`macro-user-moved` exists because a macro is visible only below its
definition: every item after a definer in its scope must stay after it,
items known by scope, core text and occurrence number.

A detail names up to three entries, each cut to its first non-blank line
and 60 characters, then "and N more".

## The formatting check

Formatting may change whitespace, move comments, and make a fixed list of
token rewrites. `verify_format` in `src/verify/format.rs` parses both texts
and walks each tree into a canonical token stream: whitespace and comments
dropped, doc comments kept (they are attributes) as their text without
whitespace, and every allowed rewrite resolved to one form. The two streams
must be equal; any other difference is `meaning-changed`. It also reports
`bom-changed`, and `output-broken` when the output has more syntax errors,
in which case it stops there. Line endings are not compared here, since
formatting may change them (`newline_style`).

The allowed rewrites are the `REWRITES` table in `src/verify/rewrites.rs`,
in the order the canonicalizer applies them. A rewrite in either direction
compares equal:

- `trailing-comma`: a comma before a list's closing delimiter is optional;
  a one-element tuple keeps its comma.
- `closure-body-braces`: `|x| { x }` is `|x| x`, however deep.
- `match-arm-body`: an arm's body may be a block holding only an
  expression, or that expression; the comma after an arm is optional.
- `match-arm-leading-pipe`: a `|` before a pattern's first alternative.
- `nested-parens`: `((a))` is `(a)`.
- `trailing-semicolon`: the `;` after a `return`, `break` or `continue`
  ending a block.
- `block-semicolon`: a `;` after `while`, `for`, a `loop` whose breaks
  carry no value, or a block-like statement another statement follows.
- `empty-generics`: an empty `<>`, an empty `where`, a `:` with no bounds,
  a `+` ending a bound list, and `::` before generic arguments in a type.
- `stray-semicolon`: a `;` alone among items or statements is empty.
- `explicit-abi`: `extern` without an ABI string means `extern "C"`.
- `range-pattern`: `...` in a range pattern is the old spelling of `..=`.
- `visibility-in`: `pub(in crate)` is `pub(crate)`, and likewise for
  `super` and `self`; `pub(in ::a)` is `pub(in a)`.
- `merge-derives`: consecutive `#[derive(...)]` attributes derive what one
  listing them all, in order, derives.
- `use-trees`: a run of `use` items is compared as the set of paths it
  imports, each with its attributes and visibility.
- `import-and-module-order`: runs of imports, `mod name;` and non
  `#[macro_use]` `extern crate` items compare sorted.
- `impl-item-order`: an impl block's items mean the same in any order.
- `doc-text`: rewrapping a doc comment or writing `#[doc = "..."]` as
  `///` changes only whitespace and markers.
- `comment-text`: rewrapping or re-marking a comment keeps its words.
- `number-literals`: `1.`, `1.0` and `1f32` as `1.0f32` are one float,
  hex digits mean the same in either case; floats stay apart from integers.
- `empty-closure-params`: `| |` and `||` are both no parameters.
- `wildcard-suffix`: two or more `_` ending a tuple pattern are `..`.
- `field-init-shorthand`: `Foo { x: x }` is `Foo { x }`.
- `crlf-in-tokens`: the compiler reads CRLF as LF, in strings too.
- `string-continuation`: a `\` ending a line in a string, the break and
  the next line's leading whitespace are not part of the string.
- `vec-delimiters`: `vec!(..)` and `vec!{..}` build what `vec![..]` does.
- `macro-arguments`: arguments in parentheses or brackets that parse as
  expressions are compared as those expressions, by these same rules.
- `macro-rules-arms`: an arm's matcher and transcriber mean the same in
  any delimiters, and the `;` between arms is a separator.
- `macro-rules-transcribers`: a transcriber that parses as items or
  statements once `$x` is renamed `zx` is compared as that code.
- `try-shorthand`: `try!(e)` is `e?`.

A test in `src/verify/format/tests.rs` pins the table at 29 entries.

Comments are compared apart from the tokens, as a multiset of words: the
comment markers become spaces, the text is split on whitespace, and words
made only of `*`, `/` and `!` are dropped. This lets formatting re-indent a
block comment, rewrap one, or move a comment across a comma, and still
reports `comment-lost` or `comment-added` for any word that went missing
or appeared.

The first difference is reported in this shape:

```text
the tokens differ: input line N: `...`, output line M: `...`
```

It is the position where the streams first differ (or where the shorter one
ends), with up to four canonical tokens before it and four after from each
side, joined by spaces, and the 1-based line of the token there.
Tokens that came out of a re-parsed macro argument or transcriber carry the
start of the macro, so the line points at the macro.

### Tokens as the parser sees them

The lexer emits one token per punctuation character. The parser glues
adjacent ones into `::`, `=>`, `..=` and the like only when nothing
separates them (`src/syntax/parser/mod.rs`), and the tree holds the glued
token. So outside macros `a::b` and `a: :b` are different streams. Inside a
macro call's token trees the parser glues nothing: there the check compares
the characters as the lexer split them, except where `macro-arguments`
re-parses the arguments as expressions. A test pins that `m!(a: :b)` and
`m!(a::b)` differ.

The parser also refuses two spellings the compiler refuses: `...` as a
range operator in an expression, whose first two dots it does not take for
`..`, and chained comparisons such as `a < b < c`
(`src/syntax/parser/grammar/expressions.rs`). Both are syntax errors, and a
file with syntax errors is left unchanged.

## On a violation

A violation stops the write: the file is left exactly as it was, the rest of
the run goes on, and the run exits with code 4 ("left unchanged because of a
bug in mto"). mto keeps a bug report (`src/report/mod.rs`), shown with
`mto report`. A report holds what failed and the smallest part of the file
that still fails, never the whole file unless asked.

The smallest part comes from `minimize` in `src/verify/report.rs`. It splits
the input into top-level elements (items, comments, damaged text, each with
the whitespace after it) and tries dropping each one; a drop that still
fails is kept. Passes repeat until none can go. For a layout violation the
test is "layout on this text still fails `verify_layout`"; for a panic while
laying out or checking, it is "this text still panics". A formatting
violation is reported without a smallest part.

## The second run

Output must also settle: running mto on its own output changes nothing.
This is not checked on each write; the test helpers in
`src/format/*/tests.rs` and `src/layout/tests.rs`, the fuzz targets and the
compatibility harness check it.

## Keeping the checks honest

- **Fuzz targets** in `fuzz/fuzz_targets/` call the checks in
  `src/fuzzing/`. `layout` and `format` require a clean check and a stable
  second run on any input; `comments` inserts numbered comments into
  existing gaps and requires each to survive layout; `generated` builds
  valid source and runs layout, formatting and both together; `verify`
  requires no false alarm (a file against itself, and a file with its items
  moved in a safe order, are clean) and no panic. Finds kept as tests live
  in `tests/fuzz_regressions/`, replayed by `tests/fuzz_regressions.rs`.
- **The standard formatter's output:** `cargo xtask diff` runs
  `verify_format` on the standard formatter's own output over the corpus
  and its test suite. A flag there is either a bug of that formatter's or a
  safe rewrite the table lacks. Known cases are listed, each with its
  reason, in `compat/meaning-exceptions.txt`; an unlisted flag fails the
  run, and a listed file no longer flagged is reported for removal.
  The same harness requires mto's own output to pass the check and to
  settle on a second run.
