//! Paths such as `std::vec::Vec<T>`, `<T as Trait>::Item` and `crate::a::b`.

use super::{generics, name_ref, nth_at_name, types};
use crate::syntax::SyntaxKind;
use crate::syntax::parser::Parser;

/// Where a path appears, which decides whether generic arguments may follow segments.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
  /// Attribute and visibility paths: segments only.
  Simple,
  /// Type paths: generic arguments, `Fn(A) -> B` sugar and `<T as Trait>` qualifiers.
  Type,
  /// Expression and pattern paths: generic arguments only after `::` (the turbofish),
  /// and `<T as Trait>` qualifiers.
  Expr,
  /// Paths inside `use` trees: segments only, stopping before `::*` and `::{`.
  Use,
}

/// True if a path starts here.
pub(super) fn at_path_start(p: &Parser) -> bool {
  nth_at_path_start(p, 0)
}

pub(super) fn nth_at_path_start(p: &Parser, n: usize) -> bool {
  nth_at_name(p, n)
    || p.nth_at(n, SyntaxKind::Colon2)
    || p.nth_at_kw(n, "self")
    || p.nth_at_kw(n, "super")
    || p.nth_at_kw(n, "crate")
    || p.nth_at_kw(n, "Self")
    || (p.nth(n) == SyntaxKind::Dollar && p.nth_text(n + 1) == "crate")
}

/// `Fn(A, B) -> C` sugar; also `method(..)` (nightly return type notation) and named
/// parameters such as `Fn(msg: String)`.
pub(super) fn parenthesized_args(p: &mut Parser) {
  let m = p.start();
  p.bump(SyntaxKind::LParen);
  if p.at(SyntaxKind::Dot2) && p.nth(2) == SyntaxKind::RParen {
    p.bump(SyntaxKind::Dot2);
  }
  while !p.at(SyntaxKind::RParen) && !p.is_eof() {
    let named = (nth_at_name(p, 0) || p.nth_text(0) == "_")
      && p.nth(1) == SyntaxKind::Colon
      && !p.nth_at(1, SyntaxKind::Colon2);
    if named {
      let param = p.start();
      super::patterns::pattern_single(p);
      p.bump(SyntaxKind::Colon);
      types::type_(p);
      param.complete(p, SyntaxKind::Param);
    } else if types::at_type_start(p) {
      types::type_(p);
    } else {
      p.error("expected a type");
      break;
    }
    if !p.eat(SyntaxKind::Comma) {
      break;
    }
  }
  p.expect(SyntaxKind::RParen);
  m.complete(p, SyntaxKind::ParenthesizedArgList);
  if p.at(SyntaxKind::ThinArrow) {
    types::ret_type(p);
  }
}

/// A whole path.
pub(super) fn path(p: &mut Parser, mode: Mode) {
  let m = p.start();
  if matches!(mode, Mode::Type | Mode::Expr) && p.at(SyntaxKind::Lt) {
    qualified_segment(p);
  } else {
    p.eat(SyntaxKind::Colon2);
    segment(p, mode);
  }
  while p.at(SyntaxKind::Colon2) && segment_follows_colon2(p, mode) {
    p.bump(SyntaxKind::Colon2);
    segment(p, mode);
  }
  m.complete(p, SyntaxKind::Path);
}

/// `<Type as Trait>` at the start of a qualified path.
fn qualified_segment(p: &mut Parser) {
  let m = p.start();
  p.bump(SyntaxKind::Lt);
  types::type_no_bounds(p);
  if p.eat_kw("as", SyntaxKind::AsKw) {
    types::path_type(p);
  }
  p.expect(SyntaxKind::Gt);
  m.complete(p, SyntaxKind::PathSegment);
}

/// One segment: a name, plus generic arguments in type paths.
fn segment(p: &mut Parser, mode: Mode) {
  let m = p.start();
  if p.at(SyntaxKind::Dollar) && p.nth_text(1) == "crate" {
    p.bump_any();
    p.bump_remap(SyntaxKind::CrateKw);
  } else {
    name_ref(p);
  }
  if mode == Mode::Expr && p.at(SyntaxKind::Colon2) && p.nth(2) == SyntaxKind::Lt {
    generics::generic_arg_list(p);
  }
  if mode == Mode::Type {
    // `<=` never opens generic arguments: `x as u8 <= y` compares.
    let opens_args = p.at(SyntaxKind::Lt) && !p.at(SyntaxKind::LtEq);
    if opens_args || (p.at(SyntaxKind::Colon2) && p.nth(2) == SyntaxKind::Lt) {
      generics::generic_arg_list(p);
    } else if p.at(SyntaxKind::LParen) {
      parenthesized_args(p);
    } else if p.at(SyntaxKind::Colon2) && p.nth(2) == SyntaxKind::LParen {
      p.bump(SyntaxKind::Colon2);
      parenthesized_args(p);
    }
  }
  m.complete(p, SyntaxKind::PathSegment);
}

/// After a `::`: whether another segment follows (in `use` trees, `::*` and `::{` end the path).
fn segment_follows_colon2(p: &Parser, mode: Mode) -> bool {
  match mode {
    Mode::Use => nth_at_path_start(p, 2) && !p.nth_at(2, SyntaxKind::Colon2),
    Mode::Type => nth_at_path_start(p, 2) || p.nth(2) == SyntaxKind::Lt,
    Mode::Simple | Mode::Expr => nth_at_path_start(p, 2),
  }
}
