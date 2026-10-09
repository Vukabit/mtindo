//! The Rust grammar, one module per area. Each function parses one construct and leaves the
//! parser after it; on unexpected input it reports an error and recovers without looping.

mod attributes;
mod generics;
mod items;
mod params;
mod paths;
mod types;

use crate::syntax::SyntaxKind;
use crate::syntax::parser::{CompletedMarker, Parser, is_keyword_in};

/// The whole file: inner attributes and doc comments, then items.
pub(super) fn source_file(p: &mut Parser) {
    let m = p.start();
    attributes::inner_attrs(p);
    items::item_list_contents(p, false);
    m.complete(p, SyntaxKind::SourceFile);
}

/// `extern` with an optional ABI string.
fn abi(p: &mut Parser) {
    let m = p.start();
    p.eat_kw("extern", SyntaxKind::ExternKw);
    if matches!(p.current(), SyntaxKind::Str | SyntaxKind::RawStr) {
        p.bump_any();
    }
    m.complete(p, SyntaxKind::Abi);
}

/// True if the current token can be a name: an identifier that is not a keyword here, or a
/// raw identifier. Contextual keywords such as `union` are valid names.
fn at_name(p: &Parser) -> bool {
    nth_at_name(p, 0)
}

/// True for an opening delimiter.
fn at_open_delim(p: &Parser) -> bool {
    matches!(
        p.current(),
        SyntaxKind::LParen | SyntaxKind::LBrack | SyntaxKind::LBrace
    )
}

fn closing_delim(open: SyntaxKind) -> SyntaxKind {
    match open {
        SyntaxKind::LParen => SyntaxKind::RParen,
        SyntaxKind::LBrack => SyntaxKind::RBrack,
        _ => SyntaxKind::RBrace,
    }
}

/// Wraps the current token (a whole token tree if it opens a delimiter) in an `ERROR` node.
fn error_bump(p: &mut Parser, message: &str) {
    p.error(message);
    let m = p.start();
    if at_open_delim(p) {
        token_tree(p);
    } else {
        p.bump_any();
    }
    m.complete(p, SyntaxKind::Error);
}

/// A name being defined.
fn name(p: &mut Parser) -> bool {
    if at_name(p) {
        let m = p.start();
        p.bump_remap(SyntaxKind::Ident);
        m.complete(p, SyntaxKind::Name);
        true
    } else {
        p.error("expected a name");
        false
    }
}

/// A name being used, including `self`, `super`, `crate` and `Self` as path segments.
fn name_ref(p: &mut Parser) -> bool {
    let keyword = match p.nth_text(0) {
        "self" if p.at_kw("self") => Some(SyntaxKind::SelfKw),
        "super" if p.at_kw("super") => Some(SyntaxKind::SuperKw),
        "crate" if p.at_kw("crate") => Some(SyntaxKind::CrateKw),
        "Self" if p.at_kw("Self") => Some(SyntaxKind::SelfTypeKw),
        _ => None,
    };
    if keyword.is_none() && !at_name(p) {
        p.error("expected a name");
        return false;
    }
    let m = p.start();
    match keyword {
        Some(kind) => p.bump_remap(kind),
        None => p.bump_any(),
    }
    m.complete(p, SyntaxKind::NameRef);
    true
}

fn nth_at_name(p: &Parser, n: usize) -> bool {
    match p.nth(n) {
        SyntaxKind::RawIdent => true,
        SyntaxKind::Ident => !is_keyword_in(p.nth_text(n), p.edition()),
        _ => false,
    }
}

/// A `{ ... }` body kept unparsed (function bodies until expressions are parsed).
fn opaque_block(p: &mut Parser) -> CompletedMarker {
    let tree = token_tree(p);
    match tree {
        Some(done) => {
            let m = done.precede(p);
            m.complete(p, SyntaxKind::OpaqueBlock)
        }
        None => {
            let m = p.start();
            m.complete(p, SyntaxKind::Error)
        }
    }
}

/// Tokens up to (not including) one of `terminators` at nesting depth 0, kept unparsed in an
/// `OPAQUE` node. Stops early at an unmatched closing delimiter. Returns false if empty.
fn opaque_until(p: &mut Parser, terminators: &[SyntaxKind]) -> bool {
    let m = p.start();
    let mut consumed = false;
    while !p.is_eof() && !terminators.iter().any(|kind| p.at(*kind)) {
        if at_open_delim(p) {
            token_tree(p);
        } else if matches!(
            p.current(),
            SyntaxKind::RParen | SyntaxKind::RBrack | SyntaxKind::RBrace
        ) {
            break;
        } else {
            p.bump_any();
        }
        consumed = true;
    }
    if consumed {
        m.complete(p, SyntaxKind::Opaque);
    } else {
        m.abandon(p);
    }
    consumed
}

/// A balanced `( ... )`, `[ ... ]` or `{ ... }` with its contents as raw tokens; nested
/// delimiters become nested token trees.
fn token_tree(p: &mut Parser) -> Option<CompletedMarker> {
    if !at_open_delim(p) {
        p.error("expected `(`, `[` or `{`");
        return None;
    }
    let m = p.start();
    let close = closing_delim(p.current());
    p.bump_any();
    loop {
        if p.is_eof() {
            p.error(format!(
                "expected {} to close this",
                crate::syntax::parser::describe(close)
            ));
            break;
        }
        if p.at(close) {
            p.bump_any();
            break;
        }
        if at_open_delim(p) {
            token_tree(p);
        } else if matches!(
            p.current(),
            SyntaxKind::RParen | SyntaxKind::RBrack | SyntaxKind::RBrace
        ) {
            p.error(format!(
                "mismatched delimiter: expected {}",
                crate::syntax::parser::describe(close)
            ));
            break;
        } else {
            p.bump_any();
        }
    }
    Some(m.complete(p, SyntaxKind::TokenTree))
}

/// `pub`, `pub(crate)`, `pub(self)`, `pub(super)` or `pub(in path)`.
fn visibility(p: &mut Parser) -> bool {
    if !p.at_kw("pub") {
        return false;
    }
    let m = p.start();
    p.bump_remap(SyntaxKind::PubKw);
    if p.at(SyntaxKind::LParen) {
        let restricted =
            (p.nth_at_kw(1, "crate") || p.nth_at_kw(1, "self") || p.nth_at_kw(1, "super"))
                && p.nth(2) == SyntaxKind::RParen;
        if restricted {
            p.bump(SyntaxKind::LParen);
            name_ref(p);
            p.bump(SyntaxKind::RParen);
        } else if p.nth_at_kw(1, "in") {
            p.bump(SyntaxKind::LParen);
            p.bump_remap(SyntaxKind::InKw);
            paths::path(p, paths::Mode::Simple);
            p.expect(SyntaxKind::RParen);
        }
    }
    m.complete(p, SyntaxKind::Visibility);
    true
}
