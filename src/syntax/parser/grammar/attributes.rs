//! Attributes (`#[...]`, `#![...]`) and doc comments, which are attributes too.

use super::expressions::expr;
use super::{paths, skip_until, token_tree};
use crate::syntax::SyntaxKind;
use crate::syntax::parser::Parser;

/// Inner attributes and inner doc comments, at the start of a file or block.
pub(super) fn inner_attrs(p: &mut Parser) {
    loop {
        if matches!(
            p.current(),
            SyntaxKind::InnerLineDoc | SyntaxKind::InnerBlockDoc
        ) {
            p.bump_any();
        } else if p.at(SyntaxKind::Pound)
            && p.nth(1) == SyntaxKind::Bang
            && p.nth(2) == SyntaxKind::LBrack
        {
            attr(p, true);
        } else {
            break;
        }
    }
}

/// Outer attributes and outer doc comments, before an item, field, variant or parameter.
pub(super) fn outer_attrs(p: &mut Parser) {
    loop {
        if matches!(
            p.current(),
            SyntaxKind::OuterLineDoc | SyntaxKind::OuterBlockDoc
        ) {
            p.bump_any();
        } else if p.at(SyntaxKind::Pound) && p.nth(1) == SyntaxKind::LBrack {
            attr(p, false);
        } else {
            break;
        }
    }
}

/// `#[meta]`, `#![meta]` or `#[unsafe(meta)]`.
fn attr(p: &mut Parser, inner: bool) {
    let m = p.start();
    p.bump(SyntaxKind::Pound);
    if inner {
        p.bump(SyntaxKind::Bang);
    }
    p.bump(SyntaxKind::LBrack);
    if p.at_kw("unsafe") && p.nth(1) == SyntaxKind::LParen {
        p.bump_remap(SyntaxKind::UnsafeKw);
        p.bump(SyntaxKind::LParen);
        meta(p);
        p.expect(SyntaxKind::RParen);
    } else {
        meta(p);
    }
    if !p.eat(SyntaxKind::RBrack) {
        p.error("expected `]` to close the attribute");
        skip_until(p, &[SyntaxKind::RBrack]);
        p.eat(SyntaxKind::RBrack);
    }
    m.complete(p, SyntaxKind::Attr);
}

/// `path`, `path(tokens)`, or `path = value`.
fn meta(p: &mut Parser) {
    let m = p.start();
    paths::path(p, paths::Mode::Simple);
    if matches!(
        p.current(),
        SyntaxKind::LParen | SyntaxKind::LBrack | SyntaxKind::LBrace
    ) {
        token_tree(p);
    } else if p.eat(SyntaxKind::Eq) {
        expr(p);
    }
    m.complete(p, SyntaxKind::Meta);
}
