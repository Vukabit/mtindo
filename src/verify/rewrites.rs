//! The rewrites formatting may make to a program's tokens, each with why it keeps the
//! program's meaning. The meaning check canonicalizes both texts by these rules
//! ([`super::format`]), so a rewrite in either direction compares equal, and anything else
//! is a change of meaning. Each rule has its own test in `tests.rs`.

/// Every allowed rewrite, in the order the canonicalizer applies them.
pub const REWRITES: &[Rewrite] = &[
  Rewrite {
    name: "trailing-comma",
    reason: "A comma before the closing delimiter of a list (arguments, parameters, \
                 closure parameters, fields, variants, generics, arrays, patterns, `where` \
                 clauses, macro arguments) is optional. A one-element tuple keeps its comma: `(a,)` is a tuple, `(a)` is not.",
  },
  Rewrite {
    name: "closure-body-braces",
    reason: "A closure whose body is a block holding only an expression means the same with \
                 the braces removed, however deep: `|x| { x }` and `|x| { { x } }` are `|x| x`.",
  },
  Rewrite {
    name: "match-arm-body",
    reason: "A match arm's body may be a block holding only an expression, or that \
                 expression: `A => { 1 }` is `A => 1`, `A => { loop {} }` is `A => loop {}`, \
                 and `A => { return x; }` is `A => return x`. The comma after an arm is \
                 optional.",
  },
  Rewrite {
    name: "match-arm-leading-pipe",
    reason: "A `|` before the first alternative of a pattern means nothing: `| A | B =>` \
                 is `A | B =>`, `let (| a, b)` is `let (a, b)`.",
  },
  Rewrite {
    name: "nested-parens",
    reason: "Parentheses directly inside parentheses group nothing more: `((a))` is `(a)`.",
  },
  Rewrite {
    name: "trailing-semicolon",
    reason: "A `return`, `break` or `continue` ending a block never yields a value, so the \
                 semicolon after it changes nothing.",
  },
  Rewrite {
    name: "block-semicolon",
    reason: "A `;` after `while`, `for`, or a `loop` whose breaks carry no value changes \
                 nothing (their value is `()` or never), and neither does one after any \
                 block-like statement (`if`, `match`, a block) that another statement follows.",
  },
  Rewrite {
    name: "empty-generics",
    reason: "An empty `<>`, an empty `where`, a `:` with no bounds after it, a `+` ending \
                 a bound list, and the `::` before generic arguments or `Fn(..)` sugar say \
                 nothing: `S<>` is `S`, `T:` is `T`, `'a:` is `'a`, `T: A +` is `T: A`, \
                 `Item::<T>` is `Item<T>` in a type, `Fn::(i32)` is `Fn(i32)`.",
  },
  Rewrite {
    name: "stray-semicolon",
    reason: "A `;` standing alone among items or statements is an empty item or statement.",
  },
  Rewrite {
    name: "explicit-abi",
    reason: "`extern` without an ABI string means `extern \"C\"`.",
  },
  Rewrite {
    name: "range-pattern",
    reason: "`...` in a range pattern is the old spelling of `..=`.",
  },
  Rewrite {
    name: "visibility-in",
    reason: "`pub(in crate)` is `pub(crate)`, `pub(in super)` is `pub(super)`, and \
                 `pub(in self)` is `pub(self)`; a path from the root names a module from the \
                 crate's, so `pub(in ::a)` is `pub(in a)`.",
  },
  Rewrite {
    name: "merge-derives",
    reason: "Consecutive `#[derive(...)]` attributes on one item derive the same traits as \
                 one `#[derive(...)]` listing them all, in the same order.",
  },
  Rewrite {
    name: "use-trees",
    reason: "A run of `use` items imports the same names however its trees are braced, \
                 nested, ordered, merged into fewer items or split into more (`a::{b}` is \
                 `a::b`, `a::{self}` is `a`, `::a` is `a`, `a as a` is `a`; \
                 `imports_granularity`), so each run is compared as the set of paths it \
                 imports, each with its attributes and visibility. A `use` importing nothing \
                 (`use a::{};`) imports nothing either way.",
  },
  Rewrite {
    name: "import-and-module-order",
    reason: "Imports never depend on each other's order, and neither do `mod name;` \
                 declarations or `extern crate` items (except `#[macro_use]` ones, which are \
                 not moved), so each run of them is compared in sorted order.",
  },
  Rewrite {
    name: "impl-item-order",
    reason: "The items of an impl block mean the same in any order \
                 (`reorder_impl_items`).",
  },
  Rewrite {
    name: "doc-text",
    reason: "Rewrapping a doc comment, formatting the code in it, or writing \
                 `#[doc = \"...\"]` as `///` (`wrap_comments`, \
                 `format_code_in_doc_comments`, `normalize_doc_attributes`) changes only its \
                 whitespace and markers, so consecutive doc comments are compared as their \
                 text without whitespace, nor the block quote markers (`>`) starting a line.",
  },
  Rewrite {
    name: "doc-code",
    reason: "Formatting a doc comment's code blocks (`format_code_in_doc_comments`) adds or \
                 drops a trailing comma, and a block's last `;`, as laying out code does, so \
                 inside a fenced block a comma before a closing delimiter and a `;` before a \
                 `}` are not compared.",
  },
  Rewrite {
    name: "comment-text",
    reason: "Rewrapping, re-indenting or re-marking a comment (`wrap_comments`, \
                 `normalize_comments`) keeps its words, so comments are compared as the \
                 multiset of their words without a custom comment's marker (`//@`), a \
                 word broken after each punctuation mark in it.",
  },
  Rewrite {
    name: "number-literals",
    reason: "`1.`, `1.0` and the `1.0f32` that `1f32` is are the same float, and hex \
                 digits mean the same in either case (`float_literal_trailing_zero`, `hex_literal_case`). Floats stay apart \
                 from integers. Parentheses around a literal group nothing (`(1.).abs()`).",
  },
  Rewrite {
    name: "empty-closure-params",
    reason: "`| |` and `||` are both a closure without parameters.",
  },
  Rewrite {
    name: "wildcard-suffix",
    reason: "Two or more `_` ending a tuple pattern match what `..` matches there, and \
                 `_` just before a final `..` adds nothing (`condense_wildcard_suffixes`); \
                 not after a `..`, as a pattern holds one at most.",
  },
  Rewrite {
    name: "field-init-shorthand",
    reason: "`Foo { x: x }` is `Foo { x }` (`use_field_init_shorthand`).",
  },
  Rewrite {
    name: "crlf-in-tokens",
    reason: "rustc reads every CRLF in a file as LF, inside string literals and block \
                 comments too, so writing a CRLF there as LF (or LF as CRLF) changes nothing.",
  },
  Rewrite {
    name: "string-continuation",
    reason: "A `\\` ending a line inside a string, the line break and the next line's \
                 leading whitespace are not part of the string, so re-indenting it, or \
                 splitting a long string that way (`format_strings`), changes nothing.",
  },
  Rewrite {
    name: "vec-delimiters",
    reason: "`vec!(..)` and `vec!{..}` build the same vector as `vec![..]`.",
  },
  Rewrite {
    name: "macro-arguments",
    reason: "A macro call's arguments in parentheses or brackets that read as expressions \
                 are compared as those expressions, by these same rules: the standard \
                 formatter formats them as such.",
  },
  Rewrite {
    name: "macro-rules-arms",
    reason: "A `macro_rules!` arm's matcher and transcriber mean the same in any \
                 delimiters, and the `;` between arms (or after the last) is a separator.",
  },
  Rewrite {
    name: "macro-rules-transcribers",
    reason: "A `macro_rules!` transcriber that reads as items, or as statements, once its \
                 metavariables are renamed (`$x` to `zx`) is compared as that code, by these \
                 same rules: formatting it as code keeps what it expands to.",
  },
  Rewrite {
    name: "try-shorthand",
    reason: "`try!(e)` is `e?` (`use_try_shorthand`).",
  },
];

/// One allowed rewrite: a short name and the reason it is safe.
pub struct Rewrite {
  pub name: &'static str,
  pub reason: &'static str,
}
