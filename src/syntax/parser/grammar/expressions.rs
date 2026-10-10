//! Expressions, blocks and statements.
//!
//! Binary operators are parsed by precedence climbing with these binding powers (left, right),
//! loosest first: assignment (2, 1) and so right-associative; ranges (3, 4) with optional
//! operands; `||` (5, 6); `&&` (7, 8); comparisons (9, 10); `|` (11, 12); `^` (13, 14);
//! `&` (15, 16); shifts (17, 18); `+ -` (19, 20); `* / %` (21, 22); then `as` (23). Prefix
//! operators bind tighter still, and postfix calls, indexing, fields, `?` and `.await`
//! tightest of all.

use super::attributes::{inner_attrs, outer_attrs};
use super::{
    at_builtin, builtin, error_bump, generics, items, name_ref, nth_at_name, paths, patterns,
    token_tree, types,
};
use crate::syntax::SyntaxKind;
use crate::syntax::parser::{CompletedMarker, Marker, Parser};

const CAST_BP: u8 = 23;

/// `let` scrutinees bind tighter than `&&` and `||` (so `let a = b && c` is a chain).
const LET_SCRUTINEE_BP: u8 = 9;

const NO_STRUCT: Restrictions = Restrictions {
    forbid_structs: true,
    stmt: false,
};

const NONE: Restrictions = Restrictions {
    forbid_structs: false,
    stmt: false,
};

const RANGE_RHS_BP: u8 = 4;

const STMT: Restrictions = Restrictions {
    forbid_structs: false,
    stmt: true,
};

/// Context that changes how an expression is parsed.
#[derive(Clone, Copy)]
pub(super) struct Restrictions {
    /// No struct literals: in `if`, `while`, `match` and `for` headers a `{` starts the body.
    forbid_structs: bool,
    /// At the start of a statement or match arm: a block-like expression ends there unless a
    /// `.` or `?` continues it.
    stmt: bool,
}

/// True if an expression can start here.
pub(super) fn at_expr_start(p: &Parser) -> bool {
    let kind = p.current();
    kind.is_literal()
        || matches!(
            kind,
            SyntaxKind::LParen
                | SyntaxKind::LBrack
                | SyntaxKind::LBrace
                | SyntaxKind::Pipe
                | SyntaxKind::Bang
                | SyntaxKind::Minus
                | SyntaxKind::Star
                | SyntaxKind::Amp
                | SyntaxKind::Lt
                | SyntaxKind::Pound
        )
        || (kind == SyntaxKind::Lifetime && p.nth(1) == SyntaxKind::Colon)
        || p.at(SyntaxKind::Dot2)
        || p.at(SyntaxKind::Colon2)
        || [
            "true", "false", "if", "match", "loop", "while", "for", "return", "break", "continue",
            "let", "move", "async", "unsafe", "const", "static",
        ]
        .iter()
        .any(|kw| p.at_kw(kw))
        || p.at_contextual_kw("yield")
        || p.at_contextual_kw("become")
        || at_yeet(p)
        || paths::at_path_start(p)
}

/// A `{ ... }` block.
pub(super) fn block_expr(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    block_body(p);
    m.complete(p, SyntaxKind::BlockExpr)
}

/// A whole expression. Returns false (after reporting) if none starts here.
pub(super) fn expr(p: &mut Parser) -> bool {
    if expr_bp(p, NONE, 0).is_none() {
        p.error("expected an expression");
        return false;
    }
    true
}

/// `(a, b)` call or method arguments.
fn arg_list(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::LParen);
    while !p.at(SyntaxKind::RParen) && !p.is_eof() {
        outer_attrs(p);
        if !expr(p) {
            break;
        }
        if !p.eat(SyntaxKind::Comma) {
            break;
        }
    }
    p.expect(SyntaxKind::RParen);
    m.complete(p, SyntaxKind::ArgList);
}

/// `[]`, `[a, b]` or `[x; n]`.
fn array_expr(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::LBrack);
    if !p.at(SyntaxKind::RBrack) {
        outer_attrs(p);
        expr(p);
        if p.eat(SyntaxKind::Semi) {
            expr(p);
        } else {
            while p.eat(SyntaxKind::Comma) {
                if p.at(SyntaxKind::RBrack) {
                    break;
                }
                outer_attrs(p);
                if !expr(p) {
                    break;
                }
            }
        }
    }
    p.expect(SyntaxKind::RBrack);
    m.complete(p, SyntaxKind::ArrayExpr)
}

/// True at a closure: `|x| ...`, `|| ...`, `move |x|`, `async |x|`, `for<'a> |x|`.
fn at_closure_start(p: &Parser) -> bool {
    let mut n = 0;
    if p.at_kw("for") && p.nth(1) == SyntaxKind::Lt {
        return true;
    }
    loop {
        if p.nth_at_kw(n, "move")
            || p.nth_at_kw(n, "async")
            || p.nth_at_kw(n, "static")
            || p.nth_at_kw(n, "const")
            || p.nth_at_kw(n, "use")
        {
            n += 1;
            continue;
        }
        break;
    }
    p.nth(n) == SyntaxKind::Pipe
}

/// True at an item inside a block, as opposed to an expression that starts with the same
/// keyword (`unsafe {`, `const {`, `async move {`, `static ||`).
fn at_item_start(p: &Parser) -> bool {
    const ALWAYS: &[&str] = &[
        "fn", "struct", "enum", "trait", "impl", "type", "mod", "pub", "extern",
    ];
    if ALWAYS.iter().any(|kw| p.at_kw(kw)) {
        return true;
    }
    if p.at_kw("use") {
        return !(p.nth(1) == SyntaxKind::Pipe || p.nth_at(1, SyntaxKind::PipePipe));
    }
    if p.at_kw("const") || p.at_kw("unsafe") {
        return p.nth(1) != SyntaxKind::LBrace
            && p.nth(1) != SyntaxKind::Pipe
            && !p.nth_at(1, SyntaxKind::PipePipe);
    }
    if p.at_kw("async") {
        return p.nth_at_kw(1, "fn")
            || p.nth_at_kw(1, "unsafe")
            || (p.nth_at_kw(1, "gen") && p.nth_at_kw(2, "fn"));
    }
    if p.at_kw("gen") {
        return p.nth_at_kw(1, "fn");
    }
    if p.at_kw("static") {
        return nth_at_name(p, 1) || p.nth_at_kw(1, "mut");
    }
    (p.at_contextual_kw("union") && nth_at_name(p, 1))
        || (p.at_contextual_kw("macro") && nth_at_name(p, 1))
        || (p.at_contextual_kw("auto") && p.nth_at_kw(1, "trait"))
        || (p.at_contextual_kw("macro_rules") && p.nth(1) == SyntaxKind::Bang && nth_at_name(p, 2))
}

/// `do yeet` (nightly); `do` is reserved, so this cannot be anything else.
fn at_yeet(p: &Parser) -> bool {
    p.at_contextual_kw("do") && p.nth(1) == SyntaxKind::Ident && p.nth_text(1) == "yeet"
}

/// The primary expression, with whether it is block-like (`{}`, `if`, `match`, loops).
fn atom(p: &mut Parser, r: Restrictions) -> Option<(CompletedMarker, bool)> {
    if p.current().is_literal() || p.at_kw("true") || p.at_kw("false") {
        let m = p.start();
        if p.at_kw("true") {
            p.bump_remap(SyntaxKind::TrueKw);
        } else if p.at_kw("false") {
            p.bump_remap(SyntaxKind::FalseKw);
        } else {
            p.bump_any();
        }
        return Some((m.complete(p, SyntaxKind::Literal), false));
    }
    if p.at(SyntaxKind::Lifetime)
        && p.nth(1) == SyntaxKind::Colon
        && !p.nth_at(1, SyntaxKind::Colon2)
    {
        let m = p.start();
        let label = p.start();
        p.bump_any();
        p.bump(SyntaxKind::Colon);
        label.complete(p, SyntaxKind::Label);
        return Some((labeled(p, m), true));
    }
    if p.at(SyntaxKind::LParen) {
        return Some((paren_or_tuple(p), false));
    }
    if p.at(SyntaxKind::LBrack) {
        return Some((array_expr(p), false));
    }
    if p.at(SyntaxKind::LBrace) {
        return Some((block_expr(p), true));
    }
    if p.at_kw("if") {
        return Some((if_expr(p), true));
    }
    if p.at_kw("match") {
        return Some((match_expr(p), true));
    }
    if p.at_kw("loop") || p.at_kw("while") || (p.at_kw("for") && p.nth(1) != SyntaxKind::Lt) {
        let m = p.start();
        return Some((labeled(p, m), true));
    }
    if let Some(prefix) = block_prefix_len(p) {
        let m = p.start();
        for _ in 0..prefix {
            let kind = match p.nth_text(0) {
                "unsafe" => SyntaxKind::UnsafeKw,
                "const" => SyntaxKind::ConstKw,
                "async" => SyntaxKind::AsyncKw,
                "move" => SyntaxKind::MoveKw,
                "try" => SyntaxKind::TryKw,
                "gen" => SyntaxKind::GenKw,
                _ => SyntaxKind::GenKw,
            };
            p.bump_remap(kind);
        }
        block_body(p);
        return Some((m.complete(p, SyntaxKind::BlockExpr), true));
    }
    if p.at_contextual_kw("try") && p.nth_text(1) == "bikeshed" {
        // `try bikeshed Type { .. }` (nightly heterogeneous try blocks).
        let m = p.start();
        p.bump_remap(SyntaxKind::TryKw);
        p.bump_any();
        types::type_no_bounds(p);
        block_body(p);
        return Some((m.complete(p, SyntaxKind::BlockExpr), true));
    }
    if at_builtin(p) {
        return Some((builtin(p), false));
    }
    if at_closure_start(p) {
        return Some((closure(p, r), false));
    }
    if p.at_kw("return") || p.at_contextual_kw("yield") || p.at_contextual_kw("become") {
        let m = p.start();
        let (kind, keyword) = match p.nth_text(0) {
            "return" => (SyntaxKind::ReturnExpr, SyntaxKind::ReturnKw),
            "yield" => (SyntaxKind::YieldExpr, SyntaxKind::YieldKw),
            _ => (SyntaxKind::BecomeExpr, SyntaxKind::Ident),
        };
        p.bump_remap(keyword);
        if operand_follows(p, r) {
            expr_bp(p, Restrictions { stmt: false, ..r }, 0);
        }
        return Some((m.complete(p, kind), false));
    }
    if at_yeet(p) {
        // `do yeet` and `do yeet value` (nightly).
        let m = p.start();
        p.bump_any();
        p.bump_any();
        if operand_follows(p, r) {
            expr_bp(p, Restrictions { stmt: false, ..r }, 0);
        }
        return Some((m.complete(p, SyntaxKind::YeetExpr), false));
    }
    if p.at_kw("break") || p.at_kw("continue") {
        let m = p.start();
        let (kind, keyword) = if p.at_kw("break") {
            (SyntaxKind::BreakExpr, SyntaxKind::BreakKw)
        } else {
            (SyntaxKind::ContinueExpr, SyntaxKind::ContinueKw)
        };
        p.bump_remap(keyword);
        if p.at(SyntaxKind::Lifetime) {
            p.bump_any();
        }
        if kind == SyntaxKind::BreakExpr && operand_follows(p, r) {
            expr_bp(p, Restrictions { stmt: false, ..r }, 0);
        }
        return Some((m.complete(p, kind), false));
    }
    if p.at_kw("let") {
        let m = p.start();
        p.bump_remap(SyntaxKind::LetKw);
        patterns::pattern_top(p);
        p.expect(SyntaxKind::Eq);
        if expr_bp(p, Restrictions { stmt: false, ..r }, LET_SCRUTINEE_BP).is_none() {
            p.error("expected an expression");
        }
        return Some((m.complete(p, SyntaxKind::LetExpr), false));
    }
    if p.nth_text(0) == "_" && p.current() == SyntaxKind::Ident {
        let m = p.start();
        p.bump_any();
        return Some((m.complete(p, SyntaxKind::UnderscoreExpr), false));
    }
    if paths::at_path_start(p) || p.at(SyntaxKind::Lt) {
        return Some(path_expr(p, r));
    }
    None
}

/// `{ inner attributes; statements; tail }`, inside an already started block node.
fn block_body(p: &mut Parser) {
    if !p.eat(SyntaxKind::LBrace) {
        p.error("expected `{`");
        return;
    }
    inner_attrs(p);
    while !p.at(SyntaxKind::RBrace) && !p.is_eof() {
        statement(p);
    }
    p.expect(SyntaxKind::RBrace);
}

/// How many prefix keywords start a special block here: `unsafe {`, `const {`, `async {`,
/// `async move {`, `try {` (2018+) or `gen {` (2024+).
fn block_prefix_len(p: &Parser) -> Option<usize> {
    let brace_at = |n: usize| p.nth(n) == SyntaxKind::LBrace;
    if (p.at_kw("unsafe") || p.at_kw("const")) && brace_at(1) {
        return Some(1);
    }
    let gen_at = |n: usize| p.nth_text(n) == "gen" && p.edition() >= crate::syntax::Edition::E2024;
    let mut n = 0;
    if p.at_kw("async") {
        n += 1;
    }
    if gen_at(n) {
        n += 1;
    }
    if n > 0 {
        if p.nth_at_kw(n, "move") {
            n += 1;
        }
        if brace_at(n) {
            return Some(n);
        }
    }
    if p.at_contextual_kw("try") && p.edition() >= crate::syntax::Edition::E2018 && brace_at(1) {
        return Some(1);
    }
    None
}

/// `|params| body` with optional `for<'a>`, `const`, `static`, `async`, `move` and `-> T`.
fn closure(p: &mut Parser, r: Restrictions) -> CompletedMarker {
    let m = p.start();
    if p.at_kw("for") {
        generics::for_binder(p);
    }
    p.eat_kw("const", SyntaxKind::ConstKw);
    p.eat_kw("static", SyntaxKind::StaticKw);
    p.eat_kw("async", SyntaxKind::AsyncKw);
    p.eat_kw("move", SyntaxKind::MoveKw);
    p.eat_kw("use", SyntaxKind::UseKw);
    let params = p.start();
    if p.eat(SyntaxKind::PipePipe) {
        params.complete(p, SyntaxKind::ParamList);
    } else {
        p.expect(SyntaxKind::Pipe);
        while !p.at(SyntaxKind::Pipe) && !p.is_eof() {
            let param = p.start();
            outer_attrs(p);
            if !patterns::pattern_single(p) {
                param.abandon(p);
                break;
            }
            if p.eat(SyntaxKind::Colon) {
                types::type_no_bounds(p);
            }
            param.complete(p, SyntaxKind::Param);
            if !p.eat(SyntaxKind::Comma) {
                break;
            }
        }
        p.expect(SyntaxKind::Pipe);
        params.complete(p, SyntaxKind::ParamList);
    }
    if p.at(SyntaxKind::ThinArrow) {
        types::ret_type(p);
        block_expr(p);
    } else if expr_bp(p, Restrictions { stmt: false, ..r }, 0).is_none() {
        p.error("expected the closure body");
    }
    m.complete(p, SyntaxKind::ClosureExpr)
}

/// An expression whose binary operators all bind at least as tightly as `min_bp`.
/// Returns the expression and whether it is block-like and ended a statement.
fn expr_bp(p: &mut Parser, r: Restrictions, min_bp: u8) -> Option<(CompletedMarker, bool)> {
    let (mut lhs, block_like) = unary_expr(p, r)?;
    if r.stmt && block_like {
        return Some((lhs, true));
    }
    let inner = Restrictions { stmt: false, ..r };
    loop {
        if p.at_kw("as") {
            if CAST_BP < min_bp {
                break;
            }
            let m = lhs.precede(p);
            p.bump_remap(SyntaxKind::AsKw);
            types::type_no_bounds(p);
            lhs = m.complete(p, SyntaxKind::CastExpr);
            continue;
        }
        let Some((op, left_bp, right_bp)) = infix_op(p) else {
            break;
        };
        if left_bp < min_bp {
            break;
        }
        let m = lhs.precede(p);
        p.bump(op);
        if matches!(op, SyntaxKind::Dot2 | SyntaxKind::Dot2Eq) {
            if operand_follows(p, r) {
                expr_bp(p, inner, RANGE_RHS_BP);
            }
            lhs = m.complete(p, SyntaxKind::RangeExpr);
            continue;
        }
        if expr_bp(p, inner, right_bp).is_none() {
            p.error("expected an expression");
        }
        lhs = m.complete(p, SyntaxKind::BinExpr);
    }
    Some((lhs, false))
}

/// A condition or scrutinee in a header (`if`, `while`, `match`, `for ... in`), where a `{`
/// after the expression starts the body. A block is itself a valid condition (`if { f() } {}`).
fn header_expr(p: &mut Parser, what: &str) {
    if expr_bp(p, NO_STRUCT, 0).is_none() {
        p.error(format!("expected {}", what));
    }
}

/// `if cond { } else if cond { } else { }`.
fn if_expr(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump_remap(SyntaxKind::IfKw);
    header_expr(p, "a condition");
    block_expr(p);
    if p.eat_kw("else", SyntaxKind::ElseKw) {
        if p.at_kw("if") {
            if_expr(p);
        } else {
            block_expr(p);
        }
    }
    m.complete(p, SyntaxKind::IfExpr)
}

/// The binary operator here, with its binding powers. Longer operators are matched first.
fn infix_op(p: &Parser) -> Option<(SyntaxKind, u8, u8)> {
    use SyntaxKind as K;
    const OPS: &[(SyntaxKind, u8, u8)] = &[
        (K::ShlEq, 2, 1),
        (K::ShrEq, 2, 1),
        (K::PlusEq, 2, 1),
        (K::MinusEq, 2, 1),
        (K::StarEq, 2, 1),
        (K::SlashEq, 2, 1),
        (K::PercentEq, 2, 1),
        (K::CaretEq, 2, 1),
        (K::AmpEq, 2, 1),
        (K::PipeEq, 2, 1),
        (K::Dot2Eq, 3, 4),
        (K::Dot2, 3, 4),
        (K::PipePipe, 5, 6),
        (K::AmpAmp, 7, 8),
        (K::EqEq, 9, 10),
        (K::Neq, 9, 10),
        (K::LtEq, 9, 10),
        (K::GtEq, 9, 10),
        (K::Shl, 17, 18),
        (K::Shr, 17, 18),
        (K::Lt, 9, 10),
        (K::Gt, 9, 10),
        (K::Pipe, 11, 12),
        (K::Caret, 13, 14),
        (K::Amp, 15, 16),
        (K::Plus, 19, 20),
        (K::Minus, 19, 20),
        (K::Star, 21, 22),
        (K::Slash, 21, 22),
        (K::Percent, 21, 22),
    ];
    if p.at(K::Eq) && !p.at(K::EqEq) && !p.at(K::FatArrow) {
        return Some((K::Eq, 2, 1));
    }
    OPS.iter().find(|(kind, _, _)| p.at(*kind)).copied()
}

/// After a label (or none): `loop`, `while`, `for`, or a labeled block, inside `m`.
fn labeled(p: &mut Parser, m: Marker) -> CompletedMarker {
    if p.eat_kw("loop", SyntaxKind::LoopKw) {
        block_expr(p);
        m.complete(p, SyntaxKind::LoopExpr)
    } else if p.eat_kw("while", SyntaxKind::WhileKw) {
        header_expr(p, "a condition");
        block_expr(p);
        m.complete(p, SyntaxKind::WhileExpr)
    } else if p.eat_kw("for", SyntaxKind::ForKw) {
        // `for await x in stream` (nightly async iteration).
        if p.at_kw("await") {
            p.bump_remap(SyntaxKind::AwaitKw);
        }
        patterns::pattern_top(p);
        if !p.eat_kw("in", SyntaxKind::InKw) {
            p.error("expected `in`");
        }
        header_expr(p, "an expression");
        block_expr(p);
        m.complete(p, SyntaxKind::ForExpr)
    } else {
        p.eat_kw("unsafe", SyntaxKind::UnsafeKw);
        block_body(p);
        m.complete(p, SyntaxKind::BlockExpr)
    }
}

/// `let pattern: Type = value else { ... };`.
fn let_stmt(p: &mut Parser) {
    p.bump_remap(SyntaxKind::LetKw);
    patterns::pattern_top(p);
    if p.eat(SyntaxKind::Colon) {
        types::type_(p);
    }
    if p.eat(SyntaxKind::Eq) {
        expr(p);
        if p.at_kw("else") {
            let m = p.start();
            p.bump_remap(SyntaxKind::ElseKw);
            block_expr(p);
            m.complete(p, SyntaxKind::LetElse);
        }
    }
    p.expect(SyntaxKind::Semi);
}

fn match_arm(p: &mut Parser) {
    let m = p.start();
    outer_attrs(p);
    patterns::pattern_top(p);
    if p.at_kw("if") {
        let guard = p.start();
        p.bump_remap(SyntaxKind::IfKw);
        expr(p);
        guard.complete(p, SyntaxKind::MatchGuard);
    }
    p.expect(SyntaxKind::FatArrow);
    let block_like = match expr_bp(p, STMT, 0) {
        Some((_, block_like)) => block_like,
        None => {
            p.error("expected an expression");
            false
        }
    };
    if !p.eat(SyntaxKind::Comma) && !block_like && !p.at(SyntaxKind::RBrace) {
        p.error("expected `,`");
    }
    m.complete(p, SyntaxKind::MatchArm);
}

/// `{ arms }` of a `match`.
fn match_arm_list(p: &mut Parser) {
    let arms = p.start();
    if p.eat(SyntaxKind::LBrace) {
        inner_attrs(p);
        while !p.at(SyntaxKind::RBrace) && !p.is_eof() {
            let start = p.token_pos();
            match_arm(p);
            if p.token_pos() == start {
                error_bump(p, "expected a match arm");
            }
        }
        p.expect(SyntaxKind::RBrace);
    } else {
        p.error("expected `{`");
    }
    arms.complete(p, SyntaxKind::MatchArmList);
}

/// `match scrutinee { pattern if guard => body, ... }`.
fn match_expr(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump_remap(SyntaxKind::MatchKw);
    header_expr(p, "an expression");
    match_arm_list(p);
    m.complete(p, SyntaxKind::MatchExpr)
}

/// True if an operand follows `return`, `break`, a range operator and the like.
fn operand_follows(p: &Parser, r: Restrictions) -> bool {
    if r.forbid_structs && p.at(SyntaxKind::LBrace) {
        return false;
    }
    at_expr_start(p)
}

/// `()`, `(e)`, `(e,)` or `(a, b)`.
fn paren_or_tuple(p: &mut Parser) -> CompletedMarker {
    let m = p.start();
    p.bump(SyntaxKind::LParen);
    if p.eat(SyntaxKind::RParen) {
        return m.complete(p, SyntaxKind::TupleExpr);
    }
    outer_attrs(p);
    expr(p);
    if !p.at(SyntaxKind::Comma) {
        p.expect(SyntaxKind::RParen);
        return m.complete(p, SyntaxKind::ParenExpr);
    }
    while p.eat(SyntaxKind::Comma) {
        if p.at(SyntaxKind::RParen) {
            break;
        }
        outer_attrs(p);
        if !expr(p) {
            break;
        }
    }
    p.expect(SyntaxKind::RParen);
    m.complete(p, SyntaxKind::TupleExpr)
}

/// A path, macro call (`path!(...)`) or struct literal (`Path { field: value }`).
fn path_expr(p: &mut Parser, r: Restrictions) -> (CompletedMarker, bool) {
    let m = p.start();
    paths::path(p, paths::Mode::Expr);
    if p.at(SyntaxKind::Bang)
        && matches!(
            p.nth(1),
            SyntaxKind::LParen | SyntaxKind::LBrack | SyntaxKind::LBrace
        )
    {
        p.bump(SyntaxKind::Bang);
        let braced = p.at(SyntaxKind::LBrace);
        token_tree(p);
        let call = m.complete(p, SyntaxKind::MacroCall);
        let expr = call.precede(p).complete(p, SyntaxKind::MacroExpr);
        return (expr, braced);
    }
    if p.at(SyntaxKind::LBrace) && !r.forbid_structs {
        record_expr_fields(p);
        return (m.complete(p, SyntaxKind::RecordExpr), false);
    }
    (m.complete(p, SyntaxKind::PathExpr), false)
}

/// Calls, indexing, fields, method calls, `?` and `.await` after `lhs`.
fn postfix(p: &mut Parser, mut lhs: CompletedMarker) -> (CompletedMarker, bool) {
    let mut changed = false;
    loop {
        if p.at(SyntaxKind::LParen) {
            let m = lhs.precede(p);
            arg_list(p);
            lhs = m.complete(p, SyntaxKind::CallExpr);
        } else if p.at(SyntaxKind::LBrack) {
            let m = lhs.precede(p);
            p.bump(SyntaxKind::LBrack);
            expr(p);
            p.expect(SyntaxKind::RBrack);
            lhs = m.complete(p, SyntaxKind::IndexExpr);
        } else if p.at(SyntaxKind::Question) {
            let m = lhs.precede(p);
            p.bump(SyntaxKind::Question);
            lhs = m.complete(p, SyntaxKind::TryExpr);
        } else if p.at(SyntaxKind::Dot) && !p.at(SyntaxKind::Dot2) {
            if p.nth_at_kw(1, "use") {
                // `x.use` (nightly ergonomic clones).
                let m = lhs.precede(p);
                p.bump(SyntaxKind::Dot);
                p.bump_remap(SyntaxKind::UseKw);
                lhs = m.complete(p, SyntaxKind::UseExpr);
            } else if p.nth_at_kw(1, "match") && p.nth(2) == SyntaxKind::LBrace {
                // `x.match { .. }` (nightly postfix match).
                let m = lhs.precede(p);
                p.bump(SyntaxKind::Dot);
                p.bump_remap(SyntaxKind::MatchKw);
                match_arm_list(p);
                lhs = m.complete(p, SyntaxKind::MatchExpr);
            } else if p.nth_at_kw(1, "await") {
                let m = lhs.precede(p);
                p.bump(SyntaxKind::Dot);
                p.bump_remap(SyntaxKind::AwaitKw);
                lhs = m.complete(p, SyntaxKind::AwaitExpr);
            } else if nth_at_name(p, 1)
                && (p.nth(2) == SyntaxKind::LParen || p.nth_at(2, SyntaxKind::Colon2))
            {
                let m = lhs.precede(p);
                p.bump(SyntaxKind::Dot);
                name_ref(p);
                if p.at(SyntaxKind::Colon2) {
                    generics::generic_arg_list(p);
                }
                if p.at(SyntaxKind::LParen) {
                    arg_list(p);
                } else {
                    p.error("expected `(`");
                }
                lhs = m.complete(p, SyntaxKind::MethodCallExpr);
            } else if nth_at_name(p, 1)
                || matches!(p.nth(1), SyntaxKind::IntNumber | SyntaxKind::FloatNumber)
                || (p.nth(1) == SyntaxKind::Ident
                    && matches!(p.nth_text(1), "self" | "Self" | "super" | "crate"))
            {
                // Path keywords are accepted as field names, as rustc's parser does.
                let m = lhs.precede(p);
                p.bump(SyntaxKind::Dot);
                if nth_at_name(p, 0) {
                    name_ref(p);
                } else {
                    let field = p.start();
                    p.bump_any();
                    field.complete(p, SyntaxKind::NameRef);
                }
                lhs = m.complete(p, SyntaxKind::FieldExpr);
            } else {
                break;
            }
        } else {
            break;
        }
        changed = true;
    }
    (lhs, changed)
}

/// `{ field: value, shorthand, 0: value, ..base }` after a struct path.
fn record_expr_fields(p: &mut Parser) {
    let m = p.start();
    p.bump(SyntaxKind::LBrace);
    while !p.at(SyntaxKind::RBrace) && !p.is_eof() {
        if p.eat(SyntaxKind::Dot2) {
            if !p.at(SyntaxKind::RBrace) {
                expr(p);
            }
            break;
        }
        let field = p.start();
        outer_attrs(p);
        let named = (nth_at_name(p, 0) || p.at(SyntaxKind::IntNumber))
            && p.nth(1) == SyntaxKind::Colon
            && !p.nth_at(1, SyntaxKind::Colon2);
        if named {
            if p.at(SyntaxKind::IntNumber) {
                let name = p.start();
                p.bump_any();
                name.complete(p, SyntaxKind::NameRef);
            } else {
                name_ref(p);
            }
            p.bump(SyntaxKind::Colon);
            expr(p);
        } else if !name_ref(p) {
            field.abandon(p);
            error_bump(p, "expected a field");
            continue;
        }
        field.complete(p, SyntaxKind::RecordExprField);
        if !p.eat(SyntaxKind::Comma) {
            break;
        }
    }
    p.expect(SyntaxKind::RBrace);
    m.complete(p, SyntaxKind::RecordExprFieldList);
}

/// One statement inside a block.
fn statement(p: &mut Parser) {
    if p.eat(SyntaxKind::Semi) {
        return;
    }
    let start = p.token_pos();
    let m = p.start();
    outer_attrs(p);
    let had_attrs = p.token_pos() > start;
    if p.at_kw("let") || (p.at_kw("super") && p.nth_at_kw(1, "let")) {
        // `super let` is nightly.
        p.eat_kw("super", SyntaxKind::SuperKw);
        let_stmt(p);
        m.complete(p, SyntaxKind::LetStmt);
        return;
    }
    if at_item_start(p) {
        match items::item(p) {
            Some(kind) => {
                m.complete(p, kind);
            }
            None => {
                p.error("expected an item");
                m.complete(p, SyntaxKind::Error);
            }
        }
        return;
    }
    match expr_bp(p, STMT, 0) {
        None => {
            if had_attrs {
                p.error("expected a statement after the attributes");
                m.complete(p, SyntaxKind::ExprStmt);
            } else {
                m.abandon(p);
                error_bump(p, "expected a statement");
            }
        }
        Some((_, block_like)) => {
            if p.eat(SyntaxKind::Semi) {
                m.complete(p, SyntaxKind::ExprStmt);
            } else if p.at(SyntaxKind::RBrace) || p.is_eof() {
                // The block's tail expression.
                if had_attrs {
                    m.complete(p, SyntaxKind::ExprStmt);
                } else {
                    m.abandon(p);
                }
            } else if block_like {
                m.complete(p, SyntaxKind::ExprStmt);
            } else {
                p.error("expected `;`");
                m.complete(p, SyntaxKind::ExprStmt);
            }
        }
    }
}

/// Prefix operators, then a primary expression with its postfix operators.
fn unary_expr(p: &mut Parser, r: Restrictions) -> Option<(CompletedMarker, bool)> {
    // Attributes on an expression (`#[cfg(x)] { .. }`) stay just before it in the parent node.
    outer_attrs(p);
    let inner = Restrictions { stmt: false, ..r };
    if p.at(SyntaxKind::Minus) || p.at(SyntaxKind::Bang) || p.at(SyntaxKind::Star) {
        let m = p.start();
        p.bump_any();
        if unary_expr(p, inner).is_none() {
            p.error("expected an expression");
        }
        return Some((m.complete(p, SyntaxKind::PrefixExpr), false));
    }
    if p.at(SyntaxKind::Amp) {
        let m = p.start();
        p.bump(SyntaxKind::Amp);
        let pinned =
            p.at_contextual_kw("pin") && (p.nth_at_kw(1, "const") || p.nth_at_kw(1, "mut"));
        if pinned {
            p.bump_any();
            if !p.eat_kw("const", SyntaxKind::ConstKw) {
                p.bump_remap(SyntaxKind::MutKw);
            }
        } else if p.at_contextual_kw("raw") && (p.nth_at_kw(1, "const") || p.nth_at_kw(1, "mut")) {
            p.bump_remap(SyntaxKind::RawKw);
            if !p.eat_kw("const", SyntaxKind::ConstKw) {
                p.bump_remap(SyntaxKind::MutKw);
            }
        } else {
            p.eat_kw("mut", SyntaxKind::MutKw);
        }
        if unary_expr(p, inner).is_none() {
            p.error("expected an expression");
        }
        return Some((m.complete(p, SyntaxKind::RefExpr), false));
    }
    if p.at(SyntaxKind::Dot2Eq) || p.at(SyntaxKind::Dot2) {
        let m = p.start();
        if !p.eat(SyntaxKind::Dot2Eq) {
            p.bump(SyntaxKind::Dot2);
        }
        if operand_follows(p, r) {
            expr_bp(p, inner, RANGE_RHS_BP);
        }
        return Some((m.complete(p, SyntaxKind::RangeExpr), false));
    }
    let (atom, block_like) = atom(p, r)?;
    if r.stmt
        && block_like
        && !(p.at(SyntaxKind::Dot) && !p.at(SyntaxKind::Dot2))
        && !p.at(SyntaxKind::Question)
    {
        return Some((atom, true));
    }
    let (expr, changed) = postfix(p, atom);
    Some((expr, block_like && !changed))
}
