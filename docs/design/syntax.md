# Reading Rust

mto reads a Rust file into a lossless syntax tree that holds every byte of the
input, comments and whitespace included. Layout, formatting and both verifiers
work on that tree, so this page describes how it is built and what it promises.

See [overview.md](overview.md) for where reading fits in a run of mto.

## The pipeline

`parse` in `src/syntax/mod.rs` runs three steps over the text, for one edition:

1. The lexer (`src/syntax/lexer/`) splits the text into tokens.
2. The parser (`src/syntax/parser/`) reads the significant tokens and writes a
   flat list of events.
3. The tree builder (`src/syntax/tree/builder/`) replays the events over the
   tokens, puts the trivia back, and builds the tree.

The result, a `Parse`, holds the tree, the errors found and how deep the tree
nests. Parsing never fails: a broken file still gives a tree that spells the
input exactly, with errors beside it.

## The lexer

Every byte of the input belongs to exactly one token, so the token lengths
always add up to the text. A token is a kind, a length in bytes and an optional
`LexError`. The lexer never stops on bad input: an unclosed string, an unknown
character or a lifetime starting with a digit becomes a token carrying an
error, and the token still covers the offending text.

Before ordinary tokens, `tokenize` reads the byte-order mark, a shebang line
(`#!` not followed by `[`, which would be an inner attribute) and cargo-script
frontmatter fenced by `---`. These, whitespace, and plain `//` and `/* */`
comments are trivia. Doc comments get their own kinds (`OuterLineDoc`,
`InnerLineDoc`, `OuterBlockDoc`, `InnerBlockDoc`) and are not trivia: the
parser places them as attributes. Block comments nest, as in rustc.

The lexer emits single-character punctuation only. The parser glues `::`, `->`,
`..=`, `>>=` and the rest from adjacent characters, and only when nothing
separates them, so `a > > b` is two `>` tokens and `Vec<Vec<u8>>` closes two
generic lists without a split. The lexer does not know keywords either: `fn`
and `async` are plain identifiers until the parser reads them.

One rustc quirk is reproduced on purpose. `x.0.1` lexes as `x`, `.`, `0.1`,
and `split_tuple_field_floats` then splits a float that follows a single `.`
into integer and dot tokens, as rustc's parser does, so the tree holds a chain
of two tuple fields.

The lexer is kept honest by `cargo xtask lexdiff`, which runs it beside rustc's
own lexer on real source files and reports any token boundary where the two
disagree.

## Editions

`Edition` in `src/syntax/edition.rs` is one of 2015, 2018, 2021 and 2024, and
editions compare in that order. The edition changes lexing:

- From 2021, an identifier touching `#`, `"` or `'` is a reserved prefix and
  gets an error (`k#x`, `f"..."`); `builtin #` is exempt.
- From 2021, `c"..."` and `cr"..."` are C strings and `'r#a` is a raw lifetime;
  before, they lex as an identifier and a string, or a lifetime.
- From 2024, `#"` and `##` are reserved guarded strings.

It also changes parsing. `is_keyword_in` in `src/syntax/parser/mod.rs` makes
`async`, `await` and `dyn` keywords from 2018 and `gen` from 2024; in 2015,
`dyn` acts as a keyword only when a bound follows it. `try { }` blocks exist
from 2018 and `gen { }` blocks from 2024. A word that is not a keyword in the
edition is an ordinary name, so `async` can name a function in 2015.

mto picks the edition by trial. `parse_any_edition` in `src/layout/mod.rs`
parses with the configured edition first and keeps that parse when it has no
errors; failing that, it tries 2021, 2018, 2015 and 2024 and keeps the first
parse without errors. Code valid only in an older edition still parses; when
no edition parses cleanly, the configured edition's result stands.

## The parser and its events

The parser is hand written recursive descent, one module per area under
`src/syntax/parser/grammar/`: items, expressions, patterns, types, generics,
paths, parameters and attributes. Binary operators use precedence climbing
with the binding powers listed at the top of `expressions.rs`. Macro arguments
and bodies are kept as token trees: balanced delimiters around raw tokens.

The parser builds no tree. It writes `Event`s (`src/syntax/parser/event.rs`):
`Start` with a node kind, `Token` with a kind and how many raw tokens it joins,
`Finish`, `Error`, and `Tombstone` for a node it abandoned. A node is opened
with a `Marker` and later completed with its kind or abandoned. A completed
node can still be wrapped: `CompletedMarker::precede` opens a parent after the
fact and records it as a `forward_parent` link, so `a + b` is parsed left to
right and only then becomes the left side of a `BinExpr`. Events are 8 bytes
and `Copy`; a file makes about three per significant token.

## Errors and recovery

A grammar function that meets unexpected input reports an error and recovers
without looping. `error_bump` wraps the offending token, or a whole token tree
when it opens a delimiter, in an `ERROR` node; `skip_until` skips to a
terminator at nesting depth zero and stops early at an unmatched closing
delimiter. A guard counts lookahead calls without progress and panics past ten
million, which finds parser bugs in tests and fuzzing.

Lexer errors cover their token's range; parser errors are empty ranges at the
start of the next significant token. Beyond malformed input, the parser
rejects what rustc rejects even where a tree could be built:

- `...` in an expression (`a...b`, `...b`) is no range operator; `..` and `..=`
  are. In a pattern, a C variadic parameter and a macro call, `...` is fine.
- Comparisons do not chain: `a < b < c` and `a == b != c` are errors, while
  `a < b && c < d` and `(a < b) < c` are not.

A file with any error is left as written. `refusal` in `src/layout/mod.rs`
turns the errors into the reason given to the user.

## The tree

`src/syntax/tree/` holds a red/green tree. The green side (`green.rs`) is the
data: kinds, lengths and children, and each node's start and parent. A whole
parse is one `Arena` holding the text, every node's children side by side in one
vector, and every token in text order, so a node's tokens are a slice read
without walking down. Building is appending, and dropping a tree frees a few
vectors however deep it nests. A `GreenNode` is a reference-counted arena and a
node id.

The red side (`red.rs`) is the handle the rest of mto uses. A `SyntaxNode`
is an entry in the arena: the counted arena and a node id. The arena records
each node's parent and start, so a handle needs nothing more, and making one
allocates nothing. Walks such as `descendants_where` and
`token_texts` create handles only for what they return.

The tree is lossless: every token's text is a slice of the parsed text, and a
node's `text()` is exactly its bytes in the source. Tests check this for
arbitrary strings in every edition (`prop_tree_text_is_the_input`).

This is what makes layout safe. Layout moves whole items, and an item moves
byte for byte: its text, its attributes and its comments go with it, untouched.
The layout verifier then compares items in the input and the output as exact
bytes, and every comment as well. The meaning check for formatted output parses
both texts and walks their tokens, dropping whitespace and comments and keeping
everything else, then requires the two streams to be equal. Neither check could
be trusted on a tree that lost or normalized text. See [layout.md](layout.md),
[formatting.md](formatting.md) and [meaning-check.md](meaning-check.md).

## Where trivia goes

The parser skips trivia; the tree builder puts each trivia token back into one
node. Most trivia stays with the enclosing node. Around an attachable node (an
item), the rules in `src/syntax/tree/attach.rs` move comments inside it: the
run of comments directly above the item with no blank line between, and a
comment on the item's last line. A macro call inside an expression does not
attach. [comment-attachment.md](comment-attachment.md) specifies the rules.

## How deep a tree may nest

`MAX_DEPTH` in `src/syntax/mod.rs` is 1024 nodes. Layout, formatting and the
verifiers walk the tree recursively, so a deep enough file would overflow the
stack; a file nested deeper is left as written with the error
`nested too deeply`. mto runs that work on a thread with a 16 MB stack
(`STACK` and `with_stack` in `src/lib.rs`), sized for that depth on every
platform.

The limit applies twice. The parser counts open nodes, and past the limit it
reads every further token as the end of the file, so its recursion unwinds at
once. Some nesting needs no recursion in the parser, such as a long method
chain where each call wraps the last; for that, the tree builder stops opening
nodes one level past the limit and adds their tokens to the deepest node. The
tree still holds every byte.

## The parse cache

A file is parsed by layout, by formatting and by both verifiers, mostly from
the same few texts. While a `ParseCache` guard is open on a thread,
`parse_cached` reuses any of the four most recent parses with the same text and
edition, since a parse depends on nothing else. Opening a guard inside another
leaves the outer one in charge, and dropping the outer guard frees every kept
tree. Trees are reference counted and not shared between threads, so the cache
is per thread.

## Querying the tree

Most code walks `SyntaxNode`s by kind. For the questions layout asks often,
`src/syntax/ast/` offers typed views: `SourceFile`, `Item`, `Attr`, `Impl` and
`Use`. They give a file's items and attributes, an item's kind, name,
visibility and doc comments, whether it carries `#[rustfmt::skip]`, an impl's
self type and trait, and the root of a `use` path. A view wraps a node and
reads it on demand; it never copies or changes the tree. Views are lenient: a
part missing from a broken file comes back as `None`.

`parse_tokens` gives a text's tokens, kinds and all, without building a tree,
for callers that only compare tokens. `src/syntax/metavariables.rs` uses it to
rename `$name` metavariables so that a `macro_rules!` body reads as code.
