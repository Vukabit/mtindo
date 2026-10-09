//! Types.

use super::{abi, expressions, generics, params, paths, token_tree};
use crate::syntax::SyntaxKind;
use crate::syntax::parser::{CompletedMarker, Parser};

/// True if a type can start here.
pub(super) fn at_type_start(p: &Parser) -> bool {
    p.at(SyntaxKind::LParen)
        || p.at(SyntaxKind::Bang)
        || p.at(SyntaxKind::Star)
        || p.at(SyntaxKind::LBrack)
        || p.at(SyntaxKind::Amp)
        || p.at(SyntaxKind::Lt)
        || p.at(SyntaxKind::Question)
        || p.at_kw("fn")
        || p.at_kw("unsafe")
        || p.at_kw("extern")
        || p.at_kw("for")
        || p.at_kw("impl")
        || p.at_kw("dyn")
        || at_dyn_2015(p)
        || p.nth_text(0) == "_"
        || paths::at_path_start(p)
}

/// A type path such as `Vec<T>` or `<T as Trait>::Item`.
pub(super) fn path_type(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    paths::path(p, paths::Mode::Type);
    m.complete(p, SyntaxKind::PathType)
}

/// `-> Type`.
pub(super) fn ret_type(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::ThinArrow);
    type_no_bounds(p);
    m.complete(p, SyntaxKind::RetType);
}

/// A type, where a trailing `+ Bound` extends a trait object (`Box<Trait + Send>`).
pub(super) fn type_(p: &mut Parser) {
    type_inner(p, true);
}

/// A type that must not absorb a following `+` (after `&`, in `-> T`, inside `<T as ...>`).
pub(super) fn type_no_bounds(p: &mut Parser) {
    type_inner(p, false);
}

/// `[T]` or `[T; N]`.
fn array_or_slice(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::LBrack);
    type_(p);
    let kind = if p.eat(SyntaxKind::Semi) {
        expressions::expr(p);
        SyntaxKind::ArrayType
    } else {
        SyntaxKind::SliceType
    };
    p.expect(SyntaxKind::RBrack);
    m.complete(p, kind);
}

/// In 2015, `dyn` is only a keyword when a bound follows it.
fn at_dyn_2015(p: &Parser) -> bool {
    p.at_contextual_kw("dyn")
        && p.edition() < crate::syntax::Edition::E2018
        && (p.nth(1) == SyntaxKind::Lifetime
            || p.nth(1) == SyntaxKind::Question
            || p.nth(1) == SyntaxKind::LParen
            || p.nth_at_kw(1, "for")
            || paths::nth_at_path_start(p, 1))
}

/// `fn(A, B) -> C` with optional `for<'a>`, `unsafe` and `extern "abi"`.
fn fn_ptr(p: &mut Parser, m: crate::syntax::parser::Marker) {
    p.eat_kw("unsafe", SyntaxKind::UnsafeKw);
    if p.at_kw("extern") {
        abi(p);
    }
    if !p.eat_kw("fn", SyntaxKind::FnKw) {
        p.error("expected `fn`");
    }
    if p.at(SyntaxKind::LParen) {
        params::param_list(p, params::Flavor::FnPtr);
    } else {
        p.error("expected `(`");
    }
    if p.at(SyntaxKind::ThinArrow) {
        ret_type(p);
    }
    m.complete(p, SyntaxKind::FnPtrType);
}

/// `(T)` or a tuple `()`, `(T,)`, `(A, B)`. A parenthesized type may start a trait object:
/// `(Trait) + 'static`.
fn paren_or_tuple(p: &mut Parser, allow_bounds: bool) {
    let m = p.start();
    p.bump(SyntaxKind::LParen);
    let mut count = 0;
    let mut trailing_comma = false;
    while !p.at(SyntaxKind::RParen) && !p.is_eof() {
        if !at_type_start(p) {
            p.error("expected a type");
            break;
        }
        type_(p);
        count += 1;
        trailing_comma = p.eat(SyntaxKind::Comma);
        if !trailing_comma {
            break;
        }
    }
    p.expect(SyntaxKind::RParen);
    if count == 1 && !trailing_comma {
        let paren = m.complete(p, SyntaxKind::ParenType);
        if allow_bounds && p.at(SyntaxKind::Plus) {
            let bound = paren.precede(p).complete(p, SyntaxKind::TypeBound);
            let list = bound.precede(p);
            p.bump(SyntaxKind::Plus);
            generics::bounds_rest(p);
            let list = list.complete(p, SyntaxKind::TypeBoundList);
            list.precede(p).complete(p, SyntaxKind::DynTraitType);
        }
    } else {
        m.complete(p, SyntaxKind::TupleType);
    }
}

/// A path type, a macro in type position (`ty!(...)`), or a bare trait object
/// (`Trait + Send`, 2015 style).
fn path_or_macro_type(p: &mut Parser, allow_bounds: bool) {
    let m = p.start();
    paths::path(p, paths::Mode::Type);
    if p.at(SyntaxKind::Bang)
        && matches!(
            p.nth(1),
            SyntaxKind::LParen | SyntaxKind::LBrack | SyntaxKind::LBrace
        )
    {
        p.bump(SyntaxKind::Bang);
        token_tree(p);
        let call = m.complete(p, SyntaxKind::MacroCall);
        call.precede(p).complete(p, SyntaxKind::MacroType);
        return;
    }
    let path_ty = m.complete(p, SyntaxKind::PathType);
    if allow_bounds && p.at(SyntaxKind::Plus) {
        let bound = path_ty.precede(p).complete(p, SyntaxKind::TypeBound);
        let list = bound.precede(p);
        p.bump(SyntaxKind::Plus);
        generics::bounds_rest(p);
        let list = list.complete(p, SyntaxKind::TypeBoundList);
        list.precede(p).complete(p, SyntaxKind::DynTraitType);
    }
}

fn type_inner(p: &mut Parser, allow_bounds: bool) {
    if p.at(SyntaxKind::LParen) {
        paren_or_tuple(p, allow_bounds);
    } else if p.at(SyntaxKind::Bang) {
        let m = p.start();
        p.bump(SyntaxKind::Bang);
        m.complete(p, SyntaxKind::NeverType);
    } else if p.at(SyntaxKind::Star) {
        let m = p.start();
        p.bump(SyntaxKind::Star);
        if !p.eat_kw("const", SyntaxKind::ConstKw) && !p.eat_kw("mut", SyntaxKind::MutKw) {
            p.error("expected `const` or `mut` after `*`");
        }
        type_no_bounds(p);
        m.complete(p, SyntaxKind::PtrType);
    } else if p.at(SyntaxKind::LBrack) {
        array_or_slice(p);
    } else if p.at(SyntaxKind::Amp) {
        let m = p.start();
        p.bump(SyntaxKind::Amp);
        if p.at(SyntaxKind::Lifetime) {
            p.bump_any();
        }
        p.eat_kw("mut", SyntaxKind::MutKw);
        type_no_bounds(p);
        m.complete(p, SyntaxKind::RefType);
    } else if p.nth_text(0) == "_" && p.current() == SyntaxKind::Ident {
        let m = p.start();
        p.bump_any();
        m.complete(p, SyntaxKind::InferType);
    } else if p.at_kw("fn") || p.at_kw("unsafe") || p.at_kw("extern") {
        let m = p.start();
        fn_ptr(p, m);
    } else if p.at_kw("for") {
        let m = p.start();
        generics::for_binder(p);
        if p.at_kw("fn") || p.at_kw("unsafe") || p.at_kw("extern") {
            fn_ptr(p, m);
        } else {
            let list = p.start();
            let bound = p.start();
            path_type(p);
            bound.complete(p, SyntaxKind::TypeBound);
            if allow_bounds && p.eat(SyntaxKind::Plus) {
                generics::bounds_rest(p);
            }
            list.complete(p, SyntaxKind::TypeBoundList);
            m.complete(p, SyntaxKind::DynTraitType);
        }
    } else if p.at(SyntaxKind::Lifetime) && allow_bounds {
        // A bare trait object led by a lifetime: `'static + Trait` (2015 style).
        let m = p.start();
        generics::bounds(p);
        m.complete(p, SyntaxKind::DynTraitType);
    } else if p.at_kw("impl") {
        let m = p.start();
        p.bump_remap(SyntaxKind::ImplKw);
        generics::bounds(p);
        m.complete(p, SyntaxKind::ImplTraitType);
    } else if p.at_kw("dyn") || at_dyn_2015(p) {
        let m = p.start();
        p.bump_remap(SyntaxKind::DynKw);
        generics::bounds(p);
        m.complete(p, SyntaxKind::DynTraitType);
    } else if paths::at_path_start(p) || p.at(SyntaxKind::Lt) {
        path_or_macro_type(p, allow_bounds);
    } else {
        p.error("expected a type");
    }
}
