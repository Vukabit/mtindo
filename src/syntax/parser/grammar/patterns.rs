//! Patterns.

use super::attributes::outer_attrs;
use super::expressions::block_expr;
use super::{error_bump, name, name_ref, nth_at_name, paths, token_tree};
use crate::syntax::SyntaxKind;
use crate::syntax::parser::{CompletedMarker, Parser};

/// One pattern without top-level alternatives (function and closure parameters use this).
/// Returns false (after reporting) if no pattern starts here.
pub(super) fn pattern_single(p: &mut Parser) -> bool {
    let Some(done) = pattern_atom(p) else {
        p.error("expected a pattern");
        return false;
    };
    if matches!(
        done.kind(),
        SyntaxKind::LiteralPat | SyntaxKind::PathPat | SyntaxKind::ConstBlockPat
    ) {
        range_rest(p, done);
    }
    true
}

/// A pattern that may have alternatives: `A | B`, with an optional leading `|`.
pub(super) fn pattern_top(p: &mut Parser) -> bool {
    let m = p.start();
    let leading = p.eat(SyntaxKind::Pipe);
    if !pattern_single(p) {
        m.abandon(p);
        return false;
    }
    let mut alternatives = leading;
    while p.at(SyntaxKind::Pipe) && !p.at(SyntaxKind::PipePipe) {
        p.bump(SyntaxKind::Pipe);
        pattern_single(p);
        alternatives = true;
    }
    if alternatives {
        m.complete(p, SyntaxKind::OrPat);
    } else {
        m.abandon(p);
    }
    true
}

/// True if a range pattern's end can start here.
fn at_range_end(p: &Parser) -> bool {
    (p.at_kw("const") && p.nth(1) == SyntaxKind::LBrace)
        || p.current().is_literal()
        || (p.at(SyntaxKind::Minus) && p.nth(1).is_literal())
        || paths::at_path_start(p)
        || p.at(SyntaxKind::Lt)
}

/// True if a range end follows `n` tokens ahead (for `..5` versus a rest pattern `..`).
fn at_range_end_after(p: &Parser, n: usize) -> bool {
    p.nth(n).is_literal()
        || (p.nth(n) == SyntaxKind::Minus && p.nth(n + 1).is_literal())
        || paths::nth_at_path_start(p, n)
}

/// `ref mut name @ subpattern`.
fn ident_pat(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.eat_kw("ref", SyntaxKind::RefKw);
    p.eat_kw("mut", SyntaxKind::MutKw);
    name(p);
    if p.eat(SyntaxKind::At) {
        pattern_single(p);
    }
    m.complete(p, SyntaxKind::IdentPat)
}

/// `-1`, `'a'`, `"s"`, `true`.
fn literal_pat(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.eat(SyntaxKind::Minus);
    if p.at_kw("true") {
        p.bump_remap(SyntaxKind::TrueKw);
    } else if p.at_kw("false") {
        p.bump_remap(SyntaxKind::FalseKw);
    } else {
        p.bump_any();
    }
    m.complete(p, SyntaxKind::LiteralPat)
}

/// `Path`, `Path(..)`, `Path { .. }` or `path!(..)`.
fn path_pattern(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    paths::path(p, paths::Mode::Expr);
    if p.at(SyntaxKind::Bang)
        && matches!(
            p.nth(1),
            SyntaxKind::LParen | SyntaxKind::LBrack | SyntaxKind::LBrace
        )
    {
        p.bump(SyntaxKind::Bang);
        token_tree(p);
        let call = m.complete(p, SyntaxKind::MacroCall);
        return call.precede(p).complete(p, SyntaxKind::MacroPat);
    }
    if p.at(SyntaxKind::LParen) {
        p.bump(SyntaxKind::LParen);
        pattern_list(p, SyntaxKind::RParen);
        return m.complete(p, SyntaxKind::TupleStructPat);
    }
    if p.at(SyntaxKind::LBrace) {
        record_pat_fields(p);
        return m.complete(p, SyntaxKind::RecordPat);
    }
    m.complete(p, SyntaxKind::PathPat)
}

fn pattern_atom(p: &mut Parser) -> Option<CompletedMarker> {
    let kind = p.current();
    if kind.is_literal()
        || p.at_kw("true")
        || p.at_kw("false")
        || (p.at(SyntaxKind::Minus) && p.nth(1).is_literal())
    {
        return Some(literal_pat(p));
    }
    if kind == SyntaxKind::Ident && p.nth_text(0) == "_" {
        let m = p.start();
        p.bump_any();
        return Some(m.complete(p, SyntaxKind::WildcardPat));
    }
    if p.at(SyntaxKind::Dot2Eq)
        || p.at(SyntaxKind::Dot3)
        || (p.at(SyntaxKind::Dot2) && at_range_end_after(p, 2))
    {
        let m = p.start();
        if !p.eat(SyntaxKind::Dot2Eq) && !p.eat(SyntaxKind::Dot3) {
            p.bump(SyntaxKind::Dot2);
        }
        range_end(p);
        return Some(m.complete(p, SyntaxKind::RangePat));
    }
    if p.at(SyntaxKind::Dot2) {
        let m = p.start();
        p.bump(SyntaxKind::Dot2);
        return Some(m.complete(p, SyntaxKind::RestPat));
    }
    if p.at(SyntaxKind::Amp) {
        let m = p.start();
        p.bump(SyntaxKind::Amp);
        p.eat_kw("mut", SyntaxKind::MutKw);
        pattern_single(p);
        return Some(m.complete(p, SyntaxKind::RefPat));
    }
    if p.at(SyntaxKind::LParen) {
        return Some(tuple_like(p, SyntaxKind::RParen, true));
    }
    if p.at(SyntaxKind::LBrack) {
        return Some(tuple_like(p, SyntaxKind::RBrack, false));
    }
    if p.at_kw("const") && p.nth(1) == SyntaxKind::LBrace {
        let m = p.start();
        p.bump_remap(SyntaxKind::ConstKw);
        block_expr(p);
        return Some(m.complete(p, SyntaxKind::ConstBlockPat));
    }
    if p.at_contextual_kw("box")
        && !matches!(
            p.nth(1),
            SyntaxKind::Comma | SyntaxKind::RParen | SyntaxKind::Colon | SyntaxKind::Eq
        )
    {
        let m = p.start();
        p.bump_remap(SyntaxKind::BoxKw);
        pattern_single(p);
        return Some(m.complete(p, SyntaxKind::BoxPat));
    }
    if p.at_kw("ref") || p.at_kw("mut") {
        return Some(ident_pat(p));
    }
    if nth_at_name(p, 0) && single_segment_binding(p) {
        return Some(ident_pat(p));
    }
    if paths::at_path_start(p) || p.at(SyntaxKind::Lt) {
        return Some(path_pattern(p));
    }
    None
}

/// Comma-separated patterns up to (and including) `close`.
fn pattern_list(p: &mut Parser, close: SyntaxKind) -> usize {
    let mut count = 0;
    let mut trailing_comma = false;
    while !p.at(close) && !p.is_eof() {
        outer_attrs(p);
        if !pattern_top(p) {
            break;
        }
        count += 1;
        trailing_comma = p.eat(SyntaxKind::Comma);
        if !trailing_comma {
            break;
        }
    }
    p.expect(close);
    if trailing_comma { count + 1 } else { count }
}

fn range_end(p: &mut Parser) {
    if p.at_kw("const") && p.nth(1) == SyntaxKind::LBrace {
        let m = p.start();
        p.bump_remap(SyntaxKind::ConstKw);
        block_expr(p);
        m.complete(p, SyntaxKind::ConstBlockPat);
    } else if p.current().is_literal() || p.at(SyntaxKind::Minus) {
        literal_pat(p);
    } else if paths::at_path_start(p) || p.at(SyntaxKind::Lt) {
        let m = p.start();
        paths::path(p, paths::Mode::Expr);
        m.complete(p, SyntaxKind::PathPat);
    } else {
        p.error("expected the end of the range");
    }
}

/// After a literal or path pattern: `..=end`, `...end`, `..end` or an open `..`.
fn range_rest(p: &mut Parser, start: CompletedMarker) {
    if !(p.at(SyntaxKind::Dot2Eq) || p.at(SyntaxKind::Dot3) || p.at(SyntaxKind::Dot2)) {
        return;
    }
    let m = start.precede(p);
    if !p.eat(SyntaxKind::Dot2Eq) && !p.eat(SyntaxKind::Dot3) {
        p.bump(SyntaxKind::Dot2);
    }
    if at_range_end(p) {
        range_end(p);
    }
    m.complete(p, SyntaxKind::RangePat);
}

/// `{ field: pattern, shorthand, ref mut other, 0: pattern, .. }`.
fn record_pat_fields(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && !p.is_eof() {
        let field = p.start();
        outer_attrs(p);
        if p.at(SyntaxKind::Dot2) {
            let rest = p.start();
            p.bump(SyntaxKind::Dot2);
            rest.complete(p, SyntaxKind::RestPat);
            field.complete(p, SyntaxKind::RecordPatField);
            break;
        }
        let named = (nth_at_name(p, 0) || p.at(SyntaxKind::IntNumber))
            && p.nth(1) == SyntaxKind::Colon
            && !p.nth_at(1, SyntaxKind::Colon2);
        if named {
            if p.at(SyntaxKind::IntNumber) {
                let n = p.start();
                p.bump_any();
                n.complete(p, SyntaxKind::NameRef);
            } else {
                name_ref(p);
            }
            p.bump(SyntaxKind::Colon);
            pattern_top(p);
        } else if p.at_contextual_kw("box") && nth_at_name(p, 1) {
            let b = p.start();
            p.bump_remap(SyntaxKind::BoxKw);
            ident_pat(p);
            b.complete(p, SyntaxKind::BoxPat);
        } else if p.at_kw("ref") || p.at_kw("mut") || nth_at_name(p, 0) {
            ident_pat(p);
        } else {
            field.abandon(p);
            error_bump(p, "expected a field pattern");
            continue;
        }
        field.complete(p, SyntaxKind::RecordPatField);
        if !p.eat(SyntaxKind::Comma) {
            break;
        }
    }
    p.expect(SyntaxKind::RBrace);
    m.complete(p, SyntaxKind::RecordPatFieldList);
}

/// True if a name here is a plain binding (`x`, `x @ ..`) rather than the start of a path
/// pattern (`A::B`, `A(..)`, `A { .. }`, `a!()`, or a range bound `A..=B`).
fn single_segment_binding(p: &Parser) -> bool {
    !(p.nth_at(1, SyntaxKind::Colon2)
        || matches!(
            p.nth(1),
            SyntaxKind::LParen | SyntaxKind::LBrace | SyntaxKind::Bang
        )
        || p.nth_at(1, SyntaxKind::Dot2)
        || p.nth_at(1, SyntaxKind::Dot2Eq)
        || p.nth_at(1, SyntaxKind::Dot3))
}

/// `(a, b)` tuple (or `(a)` parenthesized) and `[a, b]` slice patterns.
fn tuple_like(p: &mut Parser, close: SyntaxKind, parens: bool) -> CompletedMarker {
    let m = p.start();
    p.bump_any();
    let count_with_comma = pattern_list(p, close);
    let kind = if !parens {
        SyntaxKind::SlicePat
    } else if count_with_comma == 1 {
        SyntaxKind::ParenPat
    } else {
        SyntaxKind::TuplePat
    };
    m.complete(p, kind)
}
