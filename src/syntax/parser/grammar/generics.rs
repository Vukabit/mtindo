//! Generic parameters and arguments, `where` clauses, trait bounds and `for<'a>` binders.

use super::attributes::outer_attrs;
use super::expressions::block_expr;
use super::{name, name_ref, nth_at_name, paths, types};
use crate::syntax::SyntaxKind;
use crate::syntax::parser::Parser;

/// True if a bound can start here.
pub(super) fn at_bound_start(p: &Parser) -> bool {
    p.at(SyntaxKind::Lifetime)
        || p.at(SyntaxKind::Question)
        || p.at(SyntaxKind::Tilde)
        || p.at(SyntaxKind::LParen)
        || p.at(SyntaxKind::Lt)
        || p.at(SyntaxKind::Bang)
        || p.at_kw("for")
        || p.at_kw("const")
        || p.at_kw("async")
        || p.at_kw("use")
        || (p.at(SyntaxKind::LBrack) && p.nth_at_kw(1, "const"))
        || paths::at_path_start(p)
}

/// `T: Trait + 'a + ?Sized`: a list of bounds joined by `+`.
pub(super) fn bounds(p: &mut Parser) {
    let m = p.start();
    bounds_rest(p);
    m.complete(p, SyntaxKind::TypeBoundList);
}

/// Bounds after the first one, used when a type turns out to start a bound list.
pub(super) fn bounds_rest(p: &mut Parser) {
    while at_bound_start(p) {
        bound(p);
        if !p.eat(SyntaxKind::Plus) {
            break;
        }
    }
}

/// `for<'a, 'b>`.
pub(super) fn for_binder(p: &mut Parser) {
    let m = p.start();
    p.bump_remap(SyntaxKind::ForKw);
    if p.at(SyntaxKind::Lt) {
        generic_param_list(p);
    } else {
        p.error("expected `<` after `for`");
    }
    m.complete(p, SyntaxKind::ForBinder);
}

/// `<'a, T, U = Default, N: usize>` after a path segment, with an optional leading `::`.
pub(super) fn generic_arg_list(p: &mut Parser) {
    let m = p.start();
    p.eat(SyntaxKind::Colon2);
    if !p.eat(SyntaxKind::Lt) {
        p.error("expected `<`");
        m.complete(p, SyntaxKind::GenericArgList);
        return;
    }
    while !p.at(SyntaxKind::Gt) && !p.is_eof() {
        if !generic_arg(p) {
            break;
        }
        if !p.eat(SyntaxKind::Comma) {
            break;
        }
    }
    p.expect(SyntaxKind::Gt);
    m.complete(p, SyntaxKind::GenericArgList);
}

/// `<'a, T: Bound = Default, const N: usize = 3>` after an item's name.
pub(super) fn generic_param_list(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::Lt);
    while !p.at(SyntaxKind::Gt) && !p.is_eof() {
        if !generic_param(p) {
            break;
        }
        if !p.eat(SyntaxKind::Comma) {
            break;
        }
    }
    p.expect(SyntaxKind::Gt);
    m.complete(p, SyntaxKind::GenericParamList);
}

/// `where T: Clone, 'a: 'b, for<'x> F: Fn(&'x u8)`.
pub(super) fn where_clause(p: &mut Parser) {
    if !p.at_kw("where") {
        return;
    }
    let m = p.start();
    p.bump_remap(SyntaxKind::WhereKw);
    while at_where_pred_start(p) {
        where_pred(p);
        if !p.eat(SyntaxKind::Comma) {
            break;
        }
    }
    m.complete(p, SyntaxKind::WhereClause);
}

/// `Name = ...` or `Name: Bound`, possibly with generic arguments in between (`Item<'a> = T`).
fn at_assoc_arg(p: &Parser) -> bool {
    if !nth_at_name(p, 0) {
        return false;
    }
    // The name may carry generic arguments, with or without a turbofish: `Item<T>`,
    // `Item::<T>`, `Item(T)`, `Item::(T)` and, for return type notation, `method(..)`.
    let mut n = 1;
    if p.nth_at(1, SyntaxKind::Colon2) && matches!(p.nth(3), SyntaxKind::Lt | SyntaxKind::LParen) {
        n = 3;
    }
    let (open, close) = match p.nth(n) {
        SyntaxKind::Lt => (SyntaxKind::Lt, SyntaxKind::Gt),
        SyntaxKind::LParen => (SyntaxKind::LParen, SyntaxKind::RParen),
        _ => (SyntaxKind::Eof, SyntaxKind::Eof),
    };
    if open != SyntaxKind::Eof {
        let mut depth = 0;
        loop {
            match p.nth(n) {
                kind if kind == open => depth += 1,
                kind if kind == close => {
                    depth -= 1;
                    if depth == 0 {
                        n += 1;
                        break;
                    }
                }
                SyntaxKind::Eof | SyntaxKind::Semi | SyntaxKind::LBrace => return false,
                _ => {}
            }
            n += 1;
            if n > 64 {
                return false;
            }
        }
    }
    let assigns = p.nth(n) == SyntaxKind::Eq && !p.nth_at(n, SyntaxKind::EqEq);
    let bounds = p.nth(n) == SyntaxKind::Colon && !p.nth_at(n, SyntaxKind::Colon2);
    assigns || bounds
}

fn at_where_pred_start(p: &Parser) -> bool {
    p.at(SyntaxKind::Lifetime)
        || p.at_kw("for")
        || (p.at(SyntaxKind::Pound) && p.nth(1) == SyntaxKind::LBrack)
        || matches!(
            p.current(),
            SyntaxKind::OuterLineDoc | SyntaxKind::OuterBlockDoc
        )
        || types::at_type_start(p)
}

/// One bound: a lifetime, or a trait path with modifiers.
fn bound(p: &mut Parser) {
    let m = p.start();
    if p.at(SyntaxKind::Lifetime) {
        p.bump_any();
    } else if p.at(SyntaxKind::LParen) {
        p.bump(SyntaxKind::LParen);
        bound(p);
        p.expect(SyntaxKind::RParen);
    } else if p.at_kw("use") && p.nth(1) == SyntaxKind::Lt {
        p.bump_remap(SyntaxKind::UseKw);
        use_bound_args(p);
    } else {
        if p.at_kw("for") {
            for_binder(p);
        }
        loop {
            if p.eat(SyntaxKind::Question) || p.eat(SyntaxKind::Bang) {
                continue;
            }
            if p.at(SyntaxKind::Tilde) && p.nth_at_kw(1, "const") {
                p.bump(SyntaxKind::Tilde);
                p.bump_remap(SyntaxKind::ConstKw);
                continue;
            }
            if p.eat_kw("const", SyntaxKind::ConstKw) || p.eat_kw("async", SyntaxKind::AsyncKw) {
                continue;
            }
            if p.at(SyntaxKind::LBrack) && p.nth_at_kw(1, "const") && p.nth(2) == SyntaxKind::RBrack
            {
                // `[const] Trait` (nightly).
                p.bump(SyntaxKind::LBrack);
                p.bump_remap(SyntaxKind::ConstKw);
                p.bump(SyntaxKind::RBrack);
                continue;
            }
            break;
        }
        if p.at_kw("for") {
            for_binder(p);
        }
        types::path_type(p);
    }
    m.complete(p, SyntaxKind::TypeBound);
}

fn bump_literal(p: &mut Parser) {
    if p.at_kw("true") {
        p.bump_remap(SyntaxKind::TrueKw);
    } else if p.at_kw("false") {
        p.bump_remap(SyntaxKind::FalseKw);
    } else {
        p.bump_any();
    }
}

/// One generic argument.
fn generic_arg(p: &mut Parser) -> bool {
    let m = p.start();
    let kind = if p.at(SyntaxKind::Lifetime) && p.nth(1) == SyntaxKind::Plus {
        // `S<'static + Trait>`: a bare trait object led by a lifetime.
        types::type_(p);
        SyntaxKind::TypeArg
    } else if p.at(SyntaxKind::Lifetime) {
        p.bump_any();
        SyntaxKind::LifetimeArg
    } else if p.at(SyntaxKind::LBrace) {
        block_expr(p);
        SyntaxKind::ConstArg
    } else if p.current().is_literal() || p.at_kw("true") || p.at_kw("false") {
        bump_literal(p);
        SyntaxKind::ConstArg
    } else if p.at(SyntaxKind::Minus) && p.nth(1).is_literal() {
        p.bump(SyntaxKind::Minus);
        p.bump_any();
        SyntaxKind::ConstArg
    } else if at_assoc_arg(p) {
        name_ref(p);
        p.eat(SyntaxKind::Colon2);
        if p.at(SyntaxKind::Lt) {
            generic_arg_list(p);
        } else if p.at(SyntaxKind::LParen) {
            paths::parenthesized_args(p);
        }
        if p.eat(SyntaxKind::Colon) {
            bounds(p);
        } else {
            p.expect(SyntaxKind::Eq);
            if p.at(SyntaxKind::LBrace) || p.current().is_literal() {
                let c = p.start();
                if p.at(SyntaxKind::LBrace) {
                    block_expr(p);
                } else {
                    p.bump_any();
                }
                c.complete(p, SyntaxKind::ConstArg);
            } else {
                types::type_(p);
            }
        }
        SyntaxKind::AssocTypeArg
    } else if types::at_type_start(p) {
        types::type_(p);
        SyntaxKind::TypeArg
    } else {
        m.abandon(p);
        p.error("expected a generic argument");
        return false;
    };
    m.complete(p, kind);
    true
}

/// One generic parameter.
fn generic_param(p: &mut Parser) -> bool {
    let m = p.start();
    outer_attrs(p);
    let kind = if p.at(SyntaxKind::Lifetime) {
        p.bump_any();
        if p.eat(SyntaxKind::Colon) {
            bounds(p);
        }
        SyntaxKind::LifetimeParam
    } else if p.at_kw("const") {
        p.bump_remap(SyntaxKind::ConstKw);
        name(p);
        p.expect(SyntaxKind::Colon);
        types::type_(p);
        if p.eat(SyntaxKind::Eq) {
            let c = p.start();
            if p.at(SyntaxKind::LBrace) {
                block_expr(p);
            } else if p.at(SyntaxKind::Minus) {
                p.bump(SyntaxKind::Minus);
                p.bump_any();
            } else if paths::at_path_start(p) {
                let path = p.start();
                paths::path(p, paths::Mode::Expr);
                path.complete(p, SyntaxKind::PathExpr);
            } else {
                p.bump_any();
            }
            c.complete(p, SyntaxKind::ConstArg);
        }
        SyntaxKind::ConstParam
    } else if nth_at_name(p, 0) {
        name(p);
        if p.eat(SyntaxKind::Colon) {
            bounds(p);
        }
        if p.eat(SyntaxKind::Eq) {
            types::type_(p);
        }
        SyntaxKind::TypeParam
    } else {
        m.abandon(p);
        p.error("expected a generic parameter");
        return false;
    };
    m.complete(p, kind);
    true
}

/// `use<'a, T>` precise capturing.
fn use_bound_args(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::Lt);
    while !p.at(SyntaxKind::Gt) && !p.is_eof() {
        if p.at(SyntaxKind::Lifetime) {
            p.bump_any();
        } else if !name_ref(p) {
            break;
        }
        if !p.eat(SyntaxKind::Comma) {
            break;
        }
    }
    p.expect(SyntaxKind::Gt);
    m.complete(p, SyntaxKind::UseBoundGenericArgs);
}

/// `T: Bound`, `'a: 'b`, or `for<'a> T: Bound`.
fn where_pred(p: &mut Parser) {
    let m = p.start();
    outer_attrs(p);
    if p.at_kw("for") {
        for_binder(p);
    }
    if p.at(SyntaxKind::Lifetime) {
        p.bump_any();
    } else {
        types::type_(p);
    }
    if p.eat(SyntaxKind::Colon) {
        bounds(p);
    }
    m.complete(p, SyntaxKind::WherePred);
}
