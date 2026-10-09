//! Parameter lists of functions and function pointer types.

use super::attributes::outer_attrs;
use super::{patterns, types};
use crate::syntax::SyntaxKind;
use crate::syntax::parser::Parser;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Flavor {
    /// A function definition or declaration: may start with `self`.
    FnDef,
    /// A function pointer type: parameter names are optional.
    FnPtr,
}

/// `(self, a: A, b: B)`.
pub(super) fn param_list(p: &mut Parser, flavor: Flavor) {
    let m = p.start();
    p.bump(SyntaxKind::LParen);
    let mut first = true;
    while !p.at(SyntaxKind::RParen) && !p.is_eof() {
        if !param(p, first && flavor == Flavor::FnDef) {
            break;
        }
        first = false;
        if !p.eat(SyntaxKind::Comma) {
            break;
        }
    }
    p.expect(SyntaxKind::RParen);
    m.complete(p, SyntaxKind::ParamList);
}

/// `self`, `mut self`, `&self`, `&'a mut self`, or `self: Type`.
fn at_self_param(p: &Parser) -> bool {
    let self_at = |n: usize| p.nth_at_kw(n, "self") && !p.nth_at(n + 1, SyntaxKind::Colon2);
    if self_at(0) || (p.at_kw("mut") && self_at(1)) {
        return true;
    }
    if !p.at(SyntaxKind::Amp) {
        return false;
    }
    let mut n = 1;
    if p.nth(n) == SyntaxKind::Lifetime {
        n += 1;
    }
    if p.nth_at_kw(n, "mut") {
        n += 1;
    }
    self_at(n)
}

/// One parameter: `pattern: Type`, a bare `Type`, or a C-variadic `...`.
/// One parameter, or a `self` parameter when `allow_self`; attributes may precede either.
fn param(p: &mut Parser, allow_self: bool) -> bool {
    let m = p.start();
    outer_attrs(p);
    if allow_self && at_self_param(p) {
        self_param_body(p);
        m.complete(p, SyntaxKind::SelfParam);
        return true;
    }
    if p.eat(SyntaxKind::Dot3) {
        m.complete(p, SyntaxKind::Param);
        return true;
    }
    if pattern_before_colon(p) {
        patterns::pattern_single(p);
        p.expect(SyntaxKind::Colon);
        if !p.eat(SyntaxKind::Dot3) {
            types::type_(p);
        }
    } else if types::at_type_start(p) {
        types::type_(p);
    } else {
        m.abandon(p);
        p.error("expected a parameter");
        return false;
    }
    m.complete(p, SyntaxKind::Param);
    true
}

/// True if a `pattern:` comes before the end of this parameter: a single `:` (not `::`)
/// at nesting depth 0, before the next `,` or `)`.
fn pattern_before_colon(p: &Parser) -> bool {
    let mut n = 0;
    let mut depth = 0;
    loop {
        match p.nth(n) {
            SyntaxKind::Eof => return false,
            SyntaxKind::LParen | SyntaxKind::LBrack | SyntaxKind::LBrace => depth += 1,
            SyntaxKind::RParen | SyntaxKind::RBrack | SyntaxKind::RBrace => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            SyntaxKind::Comma if depth == 0 => return false,
            SyntaxKind::Colon if depth == 0 => {
                if p.nth_at(n, SyntaxKind::Colon2) {
                    n += 1;
                } else {
                    return n > 0;
                }
            }
            _ => {}
        }
        n += 1;
    }
}

fn self_param_body(p: &mut Parser) {
    if p.eat(SyntaxKind::Amp) && p.at(SyntaxKind::Lifetime) {
        p.bump_any();
    }
    p.eat_kw("mut", SyntaxKind::MutKw);
    p.bump_remap(SyntaxKind::SelfKw);
    if p.eat(SyntaxKind::Colon) {
        types::type_(p);
    }
}
