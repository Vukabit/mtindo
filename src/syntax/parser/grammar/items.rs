//! Items: functions, types, traits, impls, modules, imports, statics, extern blocks and
//! macros, plus the fields, variants and `use` trees inside them.

use super::attributes::{inner_attrs, outer_attrs};
use super::expressions;
use super::{
    abi, at_open_delim, at_restriction, error_bump, generics, name, name_ref, nth_at_name, params,
    paths, restriction, token_tree, types, visibility,
};
use crate::syntax::SyntaxKind;
use crate::syntax::parser::Parser;

/// The item that follows any attributes, or `None` if no item starts here.
pub(super) fn item(p: &mut Parser) -> Option<SyntaxKind> {
    if p.at_contextual_kw("macro_rules") && p.nth(1) == SyntaxKind::Bang && nth_at_name(p, 2) {
        return Some(macro_rules(p));
    }
    if at_macro_call(p) {
        return Some(macro_call(p));
    }
    visibility(p);
    let item_keyword_follows = |p: &Parser| {
        [
            "fn", "const", "type", "impl", "unsafe", "async", "extern", "pub",
        ]
        .iter()
        .any(|kw| p.nth_at_kw(1, kw))
    };
    loop {
        if p.at_contextual_kw("default") && item_keyword_follows(p) {
            p.bump_remap(SyntaxKind::DefaultKw);
        } else if p.at_contextual_kw("final") && item_keyword_follows(p) {
            p.bump_any();
        } else {
            break;
        }
    }
    if at_restriction(p, "impl") {
        restriction(p, SyntaxKind::ImplKw);
    }
    if p.at_contextual_kw("reuse") && (paths::nth_at_path_start(p, 1) || p.nth(1) == SyntaxKind::Lt)
    {
        return Some(reuse_item(p));
    }
    let kind = if p.at_kw("use") {
        use_item(p)
    } else if p.at_kw("mod") {
        module(p)
    } else if p.at_kw("struct") {
        struct_item(p)
    } else if p.at_contextual_kw("union") && nth_at_name(p, 1) {
        union_item(p)
    } else if p.at_kw("enum") {
        enum_item(p)
    } else if p.at_kw("type") && p.nth_at_kw(1, "const") {
        // `type const NAME: T = value;` (nightly).
        p.bump_remap(SyntaxKind::TypeKw);
        const_item(p)
    } else if p.at_kw("type") {
        type_alias(p)
    } else if p.at_kw("static") {
        static_item(p)
    } else if p.at_kw("impl") {
        impl_item(p)
    } else if p.at_kw("trait") || (p.at_contextual_kw("auto") && p.nth_at_kw(1, "trait")) {
        trait_item(p)
    } else if p.at_kw("unsafe") {
        if p.nth_at_kw(1, "impl") {
            impl_item(p)
        } else if p.nth_text(1) == "default" && p.nth_at_kw(2, "impl") {
            p.bump_remap(SyntaxKind::UnsafeKw);
            p.bump_remap(SyntaxKind::DefaultKw);
            impl_item(p)
        } else if p.nth_at_kw(1, "trait") || (p.nth_text(1) == "auto" && p.nth_at_kw(2, "trait")) {
            trait_item(p)
        } else if p.nth_at_kw(1, "mod") {
            p.bump_remap(SyntaxKind::UnsafeKw);
            module(p)
        } else if p.nth_at_kw(1, "static") {
            p.bump_remap(SyntaxKind::UnsafeKw);
            static_item(p)
        } else if p.nth_at_kw(1, "extern")
            && (p.nth(2) == SyntaxKind::LBrace
                || (matches!(p.nth(2), SyntaxKind::Str | SyntaxKind::RawStr)
                    && p.nth(3) == SyntaxKind::LBrace))
        {
            extern_block(p)
        } else {
            fn_item(p)
        }
    } else if p.at_kw("const") {
        let trait_follows = p.nth_at_kw(1, "trait")
            || ((p.nth_at_kw(1, "unsafe") || p.nth_text(1) == "auto")
                && (p.nth_at_kw(2, "trait")
                    || p.nth_text(2) == "auto"
                    || p.nth_at_kw(2, "unsafe")));
        if trait_follows {
            trait_item(p)
        } else if p.nth(1) == SyntaxKind::LBrace {
            p.bump_remap(SyntaxKind::ConstKw);
            expressions::block_expr(p);
            SyntaxKind::Const
        } else if p.nth_at_kw(1, "fn")
            || p.nth_at_kw(1, "unsafe")
            || p.nth_at_kw(1, "async")
            || p.nth_at_kw(1, "extern")
        {
            fn_item(p)
        } else if p.nth_at_kw(1, "impl") {
            p.bump_remap(SyntaxKind::ConstKw);
            impl_item(p)
        } else if p.nth_at_kw(1, "trait") {
            p.bump_remap(SyntaxKind::ConstKw);
            trait_item(p)
        } else {
            const_item(p)
        }
    } else if p.at_kw("async") || (p.at_kw("gen") && p.nth_at_kw(1, "fn")) {
        fn_item(p)
    } else if p.at_contextual_kw("safe") && p.nth_at_kw(1, "static") {
        p.bump_remap(SyntaxKind::SafeKw);
        static_item(p)
    } else if p.at_contextual_kw("safe") && p.nth_at_kw(1, "fn") {
        fn_item(p)
    } else if p.at_kw("extern") {
        if p.nth_at_kw(1, "crate") {
            extern_crate(p)
        } else if p.nth(1) == SyntaxKind::LBrace
            || (matches!(p.nth(1), SyntaxKind::Str | SyntaxKind::RawStr)
                && p.nth(2) == SyntaxKind::LBrace)
        {
            extern_block(p)
        } else {
            fn_item(p)
        }
    } else if p.at_kw("fn") {
        fn_item(p)
    } else if p.at_contextual_kw("macro") && nth_at_name(p, 1) {
        macro_def(p)
    } else {
        return None;
    };
    Some(kind)
}

/// Items until the end of the file, or until `}` when inside braces.
pub(super) fn item_list_contents(p: &mut Parser, in_braces: bool) {
    loop {
        if p.is_eof() {
            return;
        }
        if p.at(SyntaxKind::RBrace) {
            if in_braces {
                return;
            }
            error_bump(p, "unmatched `}`");
            continue;
        }
        item_or_recover(p);
    }
}

/// True if an item could start here; used to resynchronise after an error.
fn at_item_start(p: &Parser) -> bool {
    const KEYWORDS: &[&str] = &[
        "pub", "fn", "struct", "enum", "trait", "impl", "type", "const", "static", "use", "mod",
        "extern", "unsafe", "async", "gen",
    ];
    matches!(
        p.current(),
        SyntaxKind::Pound | SyntaxKind::OuterLineDoc | SyntaxKind::OuterBlockDoc
    ) || KEYWORDS.iter().any(|kw| p.at_kw(kw))
        || (p.at_contextual_kw("union") && nth_at_name(p, 1))
        || (p.at_contextual_kw("auto") && p.nth_at_kw(1, "trait"))
        || (p.at_contextual_kw("macro_rules") && p.nth(1) == SyntaxKind::Bang)
        || at_macro_call(p)
}

/// `path!` followed by a delimiter, at item position.
fn at_macro_call(p: &Parser) -> bool {
    let mut n = 0;
    if p.nth_at(0, SyntaxKind::Colon2) {
        n = 2;
    }
    loop {
        let segment = nth_at_name(p, n)
            || p.nth_at_kw(n, "self")
            || p.nth_at_kw(n, "super")
            || p.nth_at_kw(n, "crate");
        if !segment {
            return false;
        }
        n += 1;
        if p.nth_at(n, SyntaxKind::Colon2) {
            n += 2;
            continue;
        }
        break;
    }
    p.nth(n) == SyntaxKind::Bang
        && matches!(
            p.nth(n + 1),
            SyntaxKind::LParen | SyntaxKind::LBrack | SyntaxKind::LBrace
        )
}

/// `const NAME: Type = value;` (also `const _`).
fn const_item(p: &mut Parser) -> SyntaxKind {
    p.bump_remap(SyntaxKind::ConstKw);
    name(p);
    if p.at(SyntaxKind::Lt) {
        generics::generic_param_list(p);
    }
    if p.eat(SyntaxKind::Colon) {
        types::type_(p);
    }
    if p.eat(SyntaxKind::Eq) {
        expressions::expr(p);
    }
    generics::where_clause(p);
    p.expect(SyntaxKind::Semi);
    SyntaxKind::Const
}

fn enum_item(p: &mut Parser) -> SyntaxKind {
    p.bump_remap(SyntaxKind::EnumKw);
    name(p);
    if p.at(SyntaxKind::Lt) {
        generics::generic_param_list(p);
    }
    generics::where_clause(p);
    if p.at(SyntaxKind::LBrace) {
        variant_list(p);
    } else {
        p.error("expected `{`");
    }
    SyntaxKind::Enum
}

/// `extern { ... }` or `unsafe extern "C" { ... }`.
fn extern_block(p: &mut Parser) -> SyntaxKind {
    p.eat_kw("unsafe", SyntaxKind::UnsafeKw);
    abi(p);
    item_list(p, SyntaxKind::ExternItemList);
    SyntaxKind::ExternBlock
}

/// `extern crate name;` or `extern crate name as alias;`.
fn extern_crate(p: &mut Parser) -> SyntaxKind {
    p.bump_remap(SyntaxKind::ExternKw);
    p.bump_remap(SyntaxKind::CrateKw);
    name_ref(p);
    rename(p);
    p.expect(SyntaxKind::Semi);
    SyntaxKind::ExternCrate
}

/// A function with any of `const`, `async`, `gen` (2024), `unsafe`, `safe` and
/// `extern "abi"` before `fn`.
fn fn_item(p: &mut Parser) -> SyntaxKind {
    loop {
        if p.eat_kw("const", SyntaxKind::ConstKw)
            || p.eat_kw("async", SyntaxKind::AsyncKw)
            || p.eat_kw("gen", SyntaxKind::GenKw)
            || p.eat_kw("unsafe", SyntaxKind::UnsafeKw)
            || p.eat_contextual_kw("safe", SyntaxKind::SafeKw)
        {
            continue;
        }
        if p.at_kw("extern") {
            abi(p);
            continue;
        }
        break;
    }
    if !p.eat_kw("fn", SyntaxKind::FnKw) {
        p.error("expected `fn`");
        return SyntaxKind::Fn;
    }
    name(p);
    if p.at(SyntaxKind::Lt) {
        generics::generic_param_list(p);
    }
    if p.at(SyntaxKind::LParen) {
        params::param_list(p, params::Flavor::FnDef);
    } else {
        p.error("expected `(`");
    }
    if p.at(SyntaxKind::ThinArrow) {
        types::ret_type(p);
    }
    generics::where_clause(p);
    if p.at(SyntaxKind::LBrace) {
        expressions::block_expr(p);
    } else {
        p.expect(SyntaxKind::Semi);
    }
    SyntaxKind::Fn
}

/// After `impl`: whether `<` opens generic parameters (`impl<T> X`) rather than a qualified
/// path (`impl <T as Trait>::Assoc`).
fn impl_generics_follow(p: &Parser) -> bool {
    if !p.at(SyntaxKind::Lt) {
        return false;
    }
    if matches!(p.nth(1), SyntaxKind::Gt | SyntaxKind::Pound) || p.nth_at_kw(1, "const") {
        return true;
    }
    (p.nth(1) == SyntaxKind::Lifetime || nth_at_name(p, 1))
        && matches!(
            p.nth(2),
            SyntaxKind::Gt | SyntaxKind::Comma | SyntaxKind::Colon | SyntaxKind::Eq
        )
}

/// `impl<T> Trait for Type where ... { ... }`, also `unsafe`, `const` and negative impls.
fn impl_item(p: &mut Parser) -> SyntaxKind {
    p.eat_kw("unsafe", SyntaxKind::UnsafeKw);
    p.bump_remap(SyntaxKind::ImplKw);
    if impl_generics_follow(p) {
        generics::generic_param_list(p);
    }
    p.eat_kw("const", SyntaxKind::ConstKw);
    if p.at(SyntaxKind::Bang) && paths::nth_at_path_start(p, 1) {
        p.bump(SyntaxKind::Bang);
    }
    types::type_no_bounds(p);
    if p.eat_kw("for", SyntaxKind::ForKw) {
        types::type_no_bounds(p);
    }
    generics::where_clause(p);
    item_list(p, SyntaxKind::AssocItemList);
    SyntaxKind::Impl
}

/// `{ inner attributes, items }`, completed as `kind`.
fn item_list(p: &mut Parser, kind: SyntaxKind) {
    if !p.at(SyntaxKind::LBrace) {
        p.error("expected `{`");
        return;
    }
    let m = p.start();
    p.bump(SyntaxKind::LBrace);
    inner_attrs(p);
    item_list_contents(p, true);
    p.expect(SyntaxKind::RBrace);
    m.complete(p, kind);
}

/// One item with its attributes, or an `ERROR` node that skips to the next likely item.
fn item_or_recover(p: &mut Parser) {
    let start = p.token_pos();
    let m = p.start();
    outer_attrs(p);
    match item(p) {
        Some(kind) => {
            m.complete(p, kind);
        }
        None if p.token_pos() > start => {
            p.error("expected an item");
            m.complete(p, SyntaxKind::Error);
        }
        None => {
            m.abandon(p);
            recover(p);
        }
    }
}

/// `path!(...)`, `path![...];` or `path! { ... }` at item position.
fn macro_call(p: &mut Parser) -> SyntaxKind {
    paths::path(p, paths::Mode::Simple);
    p.expect(SyntaxKind::Bang);
    let braced = p.at(SyntaxKind::LBrace);
    token_tree(p);
    if !braced {
        p.expect(SyntaxKind::Semi);
    }
    SyntaxKind::MacroCall
}

/// `macro name(...) { ... }` (declarative macros 2.0).
fn macro_def(p: &mut Parser) -> SyntaxKind {
    p.bump_remap(SyntaxKind::MacroKw);
    name(p);
    if p.at(SyntaxKind::LParen) {
        token_tree(p);
    }
    token_tree(p);
    SyntaxKind::MacroDef
}

/// `macro_rules! name { ... }`.
fn macro_rules(p: &mut Parser) -> SyntaxKind {
    p.bump_remap(SyntaxKind::MacroRulesKw);
    p.bump(SyntaxKind::Bang);
    name(p);
    let braced = p.at(SyntaxKind::LBrace);
    token_tree(p);
    if !braced {
        p.expect(SyntaxKind::Semi);
    } else {
        p.eat(SyntaxKind::Semi);
    }
    SyntaxKind::MacroRules
}

/// `mod name;` or `mod name { ... }`.
fn module(p: &mut Parser) -> SyntaxKind {
    p.bump_remap(SyntaxKind::ModKw);
    name(p);
    if p.at(SyntaxKind::LBrace) {
        item_list(p, SyntaxKind::ItemList);
    } else {
        p.expect(SyntaxKind::Semi);
    }
    SyntaxKind::Module
}

/// `{ field: Type, pub other: Type }`.
fn record_field_list(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && !p.is_eof() {
        let f = p.start();
        outer_attrs(p);
        visibility(p);
        if at_restriction(p, "mut") {
            restriction(p, SyntaxKind::MutKw);
        }
        p.eat_kw("unsafe", SyntaxKind::UnsafeKw);
        if at_restriction(p, "mut") {
            restriction(p, SyntaxKind::MutKw);
        }
        if !name(p) {
            f.abandon(p);
            error_bump(p, "expected a field");
            continue;
        }
        p.expect(SyntaxKind::Colon);
        types::type_(p);
        if p.eat(SyntaxKind::Eq) {
            expressions::expr(p);
        }
        f.complete(p, SyntaxKind::RecordField);
        if !p.eat(SyntaxKind::Comma) {
            break;
        }
    }
    p.expect(SyntaxKind::RBrace);
    m.complete(p, SyntaxKind::RecordFieldList);
}

/// Skips tokens up to the next likely item start, inside an `ERROR` node.
fn recover(p: &mut Parser) {
    p.error("expected an item");
    let m = p.start();
    loop {
        let semi = p.at(SyntaxKind::Semi);
        if at_open_delim(p) {
            token_tree(p);
        } else {
            p.bump_any();
        }
        if semi || p.is_eof() || p.at(SyntaxKind::RBrace) || at_item_start(p) {
            break;
        }
    }
    m.complete(p, SyntaxKind::Error);
}

/// `as name` or `as _`.
fn rename(p: &mut Parser) {
    if !p.at_kw("as") {
        return;
    }
    let m = p.start();
    p.bump_remap(SyntaxKind::AsKw);
    name(p);
    m.complete(p, SyntaxKind::Rename);
}

/// `reuse path;`, `reuse path as name;`, `reuse Trait::{a, b} { ... }` (nightly delegation).
fn reuse_item(p: &mut Parser) -> SyntaxKind {
    p.bump_any();
    use_tree(p);
    if p.at(SyntaxKind::LBrace) {
        expressions::block_expr(p);
    } else {
        p.expect(SyntaxKind::Semi);
    }
    SyntaxKind::Reuse
}

/// `static [mut] NAME: Type = value;`.
fn static_item(p: &mut Parser) -> SyntaxKind {
    p.bump_remap(SyntaxKind::StaticKw);
    p.eat_kw("mut", SyntaxKind::MutKw);
    name(p);
    p.expect(SyntaxKind::Colon);
    types::type_(p);
    if p.eat(SyntaxKind::Eq) {
        expressions::expr(p);
    }
    p.expect(SyntaxKind::Semi);
    SyntaxKind::Static
}

/// `struct S { ... }`, `struct S(...);` or `struct S;`, with generics and `where`.
fn struct_item(p: &mut Parser) -> SyntaxKind {
    p.bump_remap(SyntaxKind::StructKw);
    name(p);
    if p.at(SyntaxKind::Lt) {
        generics::generic_param_list(p);
    }
    generics::where_clause(p);
    if p.at(SyntaxKind::LBrace) {
        record_field_list(p);
    } else if p.at(SyntaxKind::LParen) {
        tuple_field_list(p);
        generics::where_clause(p);
        p.expect(SyntaxKind::Semi);
    } else {
        p.expect(SyntaxKind::Semi);
    }
    SyntaxKind::Struct
}

/// `trait T: Bounds where ... { ... }` (also `unsafe`, `auto`) or a trait alias `trait A = B;`.
fn trait_item(p: &mut Parser) -> SyntaxKind {
    p.eat_kw("const", SyntaxKind::ConstKw);
    p.eat_kw("unsafe", SyntaxKind::UnsafeKw);
    p.eat_contextual_kw("auto", SyntaxKind::AutoKw);
    p.eat_kw("unsafe", SyntaxKind::UnsafeKw);
    p.bump_remap(SyntaxKind::TraitKw);
    name(p);
    if p.at(SyntaxKind::Lt) {
        generics::generic_param_list(p);
    }
    if p.eat(SyntaxKind::Colon) {
        generics::bounds(p);
    }
    if p.eat(SyntaxKind::Eq) {
        generics::bounds(p);
        generics::where_clause(p);
        p.expect(SyntaxKind::Semi);
        return SyntaxKind::Trait;
    }
    generics::where_clause(p);
    item_list(p, SyntaxKind::AssocItemList);
    SyntaxKind::Trait
}

/// `(pub Type, Type)`.
fn tuple_field_list(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::LParen);
    while !p.at(SyntaxKind::RParen) && !p.is_eof() {
        let f = p.start();
        outer_attrs(p);
        visibility(p);
        if at_restriction(p, "mut") {
            restriction(p, SyntaxKind::MutKw);
        }
        if !types::at_type_start(p) {
            f.abandon(p);
            error_bump(p, "expected a field type");
            continue;
        }
        types::type_(p);
        f.complete(p, SyntaxKind::TupleField);
        if !p.eat(SyntaxKind::Comma) {
            break;
        }
    }
    p.expect(SyntaxKind::RParen);
    m.complete(p, SyntaxKind::TupleFieldList);
}

/// `type Name<T>: Bounds where ... = Type;`.
fn type_alias(p: &mut Parser) -> SyntaxKind {
    p.bump_remap(SyntaxKind::TypeKw);
    name(p);
    if p.at(SyntaxKind::Lt) {
        generics::generic_param_list(p);
    }
    if p.eat(SyntaxKind::Colon) {
        generics::bounds(p);
    }
    generics::where_clause(p);
    if p.eat(SyntaxKind::Eq) {
        types::type_(p);
    }
    generics::where_clause(p);
    p.expect(SyntaxKind::Semi);
    SyntaxKind::TypeAlias
}

/// `union U { ... }`.
fn union_item(p: &mut Parser) -> SyntaxKind {
    p.bump_remap(SyntaxKind::UnionKw);
    name(p);
    if p.at(SyntaxKind::Lt) {
        generics::generic_param_list(p);
    }
    generics::where_clause(p);
    if p.at(SyntaxKind::LBrace) {
        record_field_list(p);
    } else {
        p.error("expected `{`");
    }
    SyntaxKind::Union
}

/// `use tree;`.
fn use_item(p: &mut Parser) -> SyntaxKind {
    p.bump_remap(SyntaxKind::UseKw);
    use_tree(p);
    p.expect(SyntaxKind::Semi);
    SyntaxKind::Use
}

/// `path`, `path as name`, `path::*`, `path::{...}`, `*` or `{...}`.
fn use_tree(p: &mut Parser) {
    let m = p.start();
    if p.at(SyntaxKind::Star) {
        p.bump(SyntaxKind::Star);
    } else if p.at(SyntaxKind::LBrace) {
        use_tree_list(p);
    } else if p.at(SyntaxKind::Colon2) && matches!(p.nth(2), SyntaxKind::Star | SyntaxKind::LBrace)
    {
        p.bump(SyntaxKind::Colon2);
        if !p.eat(SyntaxKind::Star) {
            use_tree_list(p);
        }
    } else if paths::at_path_start(p) {
        paths::path(p, paths::Mode::Use);
        if p.at(SyntaxKind::Colon2) {
            p.bump(SyntaxKind::Colon2);
            if p.at(SyntaxKind::LBrace) {
                use_tree_list(p);
            } else if !p.eat(SyntaxKind::Star) {
                p.error("expected `*` or `{`");
            }
        } else {
            rename(p);
        }
    } else {
        p.error("expected a use path");
    }
    m.complete(p, SyntaxKind::UseTree);
}

/// `{a, b::c, d as e}`.
fn use_tree_list(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && !p.is_eof() {
        use_tree(p);
        if !p.eat(SyntaxKind::Comma) {
            break;
        }
    }
    p.expect(SyntaxKind::RBrace);
    m.complete(p, SyntaxKind::UseTreeList);
}

/// An enum's `{ Variant, Variant(T), Variant { f: T } = 3 }`.
fn variant_list(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && !p.is_eof() {
        let v = p.start();
        outer_attrs(p);
        visibility(p);
        if !name(p) {
            v.abandon(p);
            error_bump(p, "expected a variant");
            continue;
        }
        if p.at(SyntaxKind::LBrace) {
            record_field_list(p);
        } else if p.at(SyntaxKind::LParen) {
            tuple_field_list(p);
        }
        if p.eat(SyntaxKind::Eq) {
            expressions::expr(p);
        }
        v.complete(p, SyntaxKind::Variant);
        if !p.eat(SyntaxKind::Comma) {
            break;
        }
    }
    p.expect(SyntaxKind::RBrace);
    m.complete(p, SyntaxKind::VariantList);
}
