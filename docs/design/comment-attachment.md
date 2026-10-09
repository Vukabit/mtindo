# Comment attachment

mto's syntax tree keeps every byte of the source, including whitespace and
comments. Each of those trivia tokens sits inside exactly one node. This
document specifies which node, because the layout engine moves whole nodes and
the formatter prints them: a comment that belongs to the wrong node ends up
describing the wrong code.

The rules are implemented in `src/syntax/tree/attach.rs` and applied by the tree
builder in `src/syntax/tree/builder/`.

## Terms

- **Trivia:** whitespace, plain comments (`//`, `/* */`), the byte-order mark
  and the shebang line. The parser skips trivia; the tree builder puts it back.
- **Doc comments** (`///`, `//!`, `/** */`, `/*! */`) are not trivia. They are
  attributes, so the parser places them like any other attribute.
- **Attachable node:** an item: `fn`, `struct`, `enum`, `union`, `trait`,
  `impl`, `type`, `const`, `static`, `use`, `mod`, `extern crate`, `extern`
  blocks, `macro_rules!`, and item-position macro calls. Later stages may add
  fields, enum variants, match arms and statements, and will extend this list
  when they do.
- **Blank line:** a whitespace token containing at least two line breaks.

## Rule 1: comments directly above an item belong to the item

The run of plain comments immediately above an attachable node, with no blank
line between the run and the node, belongs **inside** that node, before its
first token. The whitespace between those comments and the node comes with them.

```rust
// Belongs to `a`.
// Also belongs to `a`.
fn a() {}
```

Only the run touching the node attaches. A blank line ends the run:

```rust
// Section heading: stays with the parent, not with `b`.

// Belongs to `b`.
fn b() {}
```

The byte-order mark and the shebang line never attach; they stay at the top of
the file.

## Rule 2: a comment at the end of an item's last line belongs to the item

A plain comment that starts on the same line as an attachable node's last token
belongs **inside** that node, after its last token, together with the spaces
before it.

```rust
fn a() {} // Belongs to `a`.
const LIMIT: u8 = 3; /* Belongs to `LIMIT`. */
```

A block comment qualifies only if it also ends on that line and nothing but a
line break follows it. A comment on the next line is not trailing; it falls
under rule 1 for whatever follows it.

Rule 2 is applied when the node closes, before rule 1 is applied to the next
node. So in

```rust
fn a() {} // about a
// about b
fn b() {}
```

`// about a` belongs to `a` and `// about b` belongs to `b`.

## Rule 3: everything else belongs to the enclosing node

Trivia that rules 1 and 2 do not claim stays in the node that contains it, at
the position where it occurs: between two children, or inside a node between its
tokens.

## Whitespace

Whitespace is never owned for its own sake. It belongs wherever it falls: inside
a node when it sits between that node's tokens or between an attached comment
and the node, otherwise in the enclosing node.

## Guarantees

- Printing the tree reproduces the source exactly, whatever the attachment.
- Moving an attachable node moves its leading and trailing comments with it.
- Attachment never depends on comment content, except for distinguishing doc
  comments from plain ones.
