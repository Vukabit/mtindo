//! The spaces between two tokens on one line, as the Rust style writes them: whether a pair
//! takes one space, none, or is left as written because no rule here covers it.
//!
//! A rule looks at the two tokens and the nodes they belong to: `*` is spaced as a binary
//! operator and not as a dereference, `<` as a comparison and not as a generic list. A pair
//! that would lex differently once joined (`- -`, `/ /`) always keeps a space.

use SyntaxKind as K;

use crate::options::format::FormatConfig;
use crate::syntax::lexer::tokenize;
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxToken};

/// Character pairs that begin a longer token or a comment: two tokens that end and start
/// with one of these would be read as something else once joined.
const GLUES: &[&str] = &[
  "::", "..", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "%=", "^=",
  "&=", "|=", "<<", ">>", "<-", "//", "/*", "*/", ".=", "--", "++",
];

/// Whether text ending with `left` and text starting with `right` would read as something
/// else written together: a longer operator, a comment, a range.
pub fn glues(left: &str, right: &str) -> bool {
  match (left.chars().last(), right.chars().next()) {
    (Some(last), Some(first)) => {
      let pair: String = [last, first].iter().collect();
      GLUES.contains(&pair.as_str())
    }
    _ => false,
  }
}

/// Whether `a` and `b` (tokens on one line with spaces between) take one space
/// (`Some(true)`), none (`Some(false)`), or keep what is written (`None`).
pub fn spacing(
  a: &SyntaxToken,
  b: &SyntaxToken,
  edition: Edition,
  config: &FormatConfig,
) -> Option<bool> {
  let wanted = rule(a, b, config)?;
  if !wanted && !separate_when_joined(a, b, edition) {
    return Some(true);
  }
  Some(wanted)
}

/// Whether two tokens written touching take a space: only when a rule asks for one, since
/// they already read as themselves.
pub fn wants_space(a: &SyntaxToken, b: &SyntaxToken, config: &FormatConfig) -> bool {
  rule(a, b, config) == Some(true)
}

/// A token a call's or index's delimiter follows with no space.
fn callee_end(kind: SyntaxKind, parent: SyntaxKind) -> bool {
  if kind == K::Gt {
    // `::<T>(` and `fn f<T>(`; not `impl<T> ()` or `for<'a> (`.
    return parent == K::GenericArgList;
  }
  if kind == K::RParen && parent == K::Visibility {
    return false;
  }
  parent != K::BinExpr
    && parent != K::Attr
    && matches!(
      kind,
      K::Ident
        | K::RawIdent
        | K::SelfKw
        | K::SelfTypeKw
        | K::SuperKw
        | K::CrateKw
        | K::Gt
        | K::RParen
        | K::RBrack
        | K::Question
        | K::Bang
        | K::FnKw
    )
}

/// A closing delimiter a trailing comma sits against: `(a,)`, `[a,]`, `<A,>`.
fn is_closer(kind: SyntaxKind) -> bool {
  matches!(kind, K::RParen | K::RBrack | K::Gt)
}

fn is_keyword(kind: SyntaxKind) -> bool {
  kind.name().ends_with("_KW")
}

/// Whether a closure's `|` opens its parameters (the first `|` of the list).
fn is_opening_pipe(token: &SyntaxToken) -> bool {
  token
    .parent()
    .children_with_tokens()
    .iter()
    .find(|e| e.kind() == K::Pipe)
    .is_some_and(|e| e.text_range() == token.text_range())
}

/// A binary operator, assignment, arrow, `as`, an or-pattern's `|`, a bound's `+` or a
/// binding's `@`.
fn is_operator(kind: SyntaxKind, parent: SyntaxKind) -> bool {
  match kind {
    K::Eq | K::FatArrow | K::ThinArrow | K::At => true,
    K::Pipe => matches!(parent, K::BinExpr | K::OrPat),
    K::Plus => matches!(parent, K::BinExpr | K::TypeBoundList),
    _ => parent == K::BinExpr && !is_range(kind, parent),
  }
}

fn is_prefix(kind: SyntaxKind, parent: SyntaxKind) -> bool {
  match kind {
    K::Minus | K::Bang | K::Star => {
      matches!(parent, K::PrefixExpr | K::LiteralPat | K::PtrType)
    }
    K::Amp | K::AmpAmp => matches!(
      parent,
      K::RefExpr | K::RefType | K::RefPat | K::SelfParam | K::PrefixExpr
    ),
    _ => false,
  }
}

fn is_range(kind: SyntaxKind, parent: SyntaxKind) -> bool {
  matches!(kind, K::Dot2 | K::Dot2Eq | K::Dot3)
    && matches!(
      parent,
      K::RangeExpr | K::RangePat | K::RestPat | K::RecordExprFieldList | K::SlicePat
    )
}

/// A token a path may continue after with `::` and no space: a name, `self`, `Self`,
/// `super`, `crate`, a generic list's end, or an opening delimiter.
fn path_start(kind: SyntaxKind) -> bool {
  matches!(
    kind,
    K::Ident
      | K::RawIdent
      | K::SelfKw
      | K::SelfTypeKw
      | K::SuperKw
      | K::CrateKw
      | K::Gt
      | K::Lt
      | K::LParen
      | K::LBrack
      | K::Amp
      | K::Star
      | K::Bang
      | K::Minus
  )
}

fn rule(a: &SyntaxToken, b: &SyntaxToken, config: &FormatConfig) -> Option<bool> {
  let (ka, kb) = (a.kind(), b.kind());
  let (pa, pb) = (a.parent().kind(), b.parent().kind());
  if ka.is_trivia() || kb.is_trivia() {
    return None;
  }
  // A `macro_rules!` header is left as written, and so are two generic lists opening
  // together (`<<T as A>::B>` and `< <T as A>::B>` are both written).
  if pa == K::MacroRules || pb == K::MacroRules {
    return None;
  }
  if ka == K::Lt && kb == K::Lt && pa != K::BinExpr {
    return None;
  }
  // `builtin # offset_of(..)`: the `#` spaced as written.
  if (ka == K::Pound && pa == K::Builtin) || (kb == K::Pound && pb == K::Builtin) {
    return None;
  }
  // A macro's name, `!` and arguments are tight, unless the arguments hold what no
  // expression list can (`quote! (span => #x)`): the standard formatter leaves such a call
  // whole as written.
  let bang = if kb == K::Bang { b } else { a };
  if bang.kind() == K::Bang
    && matches!(
      bang.parent().kind(),
      K::MacroCall | K::MacroExpr | K::MacroPat | K::MacroType
    )
  {
    if ka == K::Bang && kb == K::LBrace {
      // `name! { ... }`: one space, as the standard formatter always writes it.
      return Some(true);
    }
    return (!unreadable_arguments(bang)).then_some(false);
  }
  // After a restricted visibility: `pub(crate) fn`, `pub(super) x: u8`.
  if ka == K::RParen && pa == K::Visibility && (kb == K::Ident || is_keyword(kb)) {
    return Some(true);
  }
  // A tuple field of a tuple field (`x.0 .0`): spaced before style edition 2024, as the
  // standard formatter's chains write it; joined from it.
  if ka == K::IntNumber
    && pa == K::NameRef
    && kb == K::Dot
    && pb == K::FieldExpr
    && matches!(
      b.next_sibling_or_token(),
      Some(SyntaxElement::Node(n))
        if n.kind() == K::NameRef && n.first_token().is_some_and(|t| t.kind() == K::IntNumber)
    )
  {
    return Some(config.style_edition < 2024);
  }
  // After a field's restriction, before its type: `mut(self) ()`, `mut(self) u8`.
  if ka == K::RParen && pa == K::Restriction && !is_closer(kb) && kb != K::Comma {
    return Some(true);
  }
  // After an attribute: `#[a] x`.
  if ka == K::RBrack && pa == K::Attr {
    return Some(true);
  }
  // Lists and statements.
  if matches!(kb, K::Comma | K::Semi) {
    return Some(false);
  }
  if ka == K::Comma {
    return Some(!is_closer(kb) && !(kb == K::Pipe && pb == K::ParamList && !is_opening_pipe(b)));
  }
  if ka == K::Semi {
    return Some(!matches!(kb, K::RParen | K::RBrack));
  }
  // Delimiters: nothing inside parentheses and brackets.
  if matches!(ka, K::LParen | K::LBrack) || matches!(kb, K::RParen | K::RBrack) {
    return Some(false);
  }
  // Braces: `use a::{b, c}` and `{}` are tight; blocks, struct literals and patterns on
  // one line are `{ a }`.
  if kb == K::LBrace {
    if is_prefix(ka, pa) {
      return Some(false);
    }
    return match ka {
      K::Colon2 => Some(false),
      K::Dollar | K::LBrace | K::Pound | K::Bang | K::Lt | K::LParen | K::LBrack => None,
      _ => Some(true),
    };
  }
  if ka == K::LBrace || kb == K::RBrace {
    if ka == K::LBrace && kb == K::RBrace {
      return Some(false);
    }
    let braces = if ka == K::LBrace { pa } else { pb };
    return match braces {
      K::UseTreeList => Some(false),
      K::TokenTree => None,
      _ => Some(true),
    };
  }
  // Paths, fields and methods.
  if ka == K::Colon2 {
    return Some(false);
  }
  if kb == K::Colon2 {
    if ka == K::Gt && pa != K::GenericArgList {
      return None;
    }
    return path_start(ka).then_some(false);
  }
  if (ka == K::Dot || kb == K::Dot)
    && matches!(
      if ka == K::Dot { pa } else { pb },
      K::FieldExpr | K::MethodCallExpr | K::AwaitExpr | K::UseExpr
    )
  {
    return Some(false);
  }
  // Ranges and rest patterns: tight, unless `spaces_around_ranges` (a rest pattern or a
  // range open at that end stays tight against its delimiter either way).
  if is_range(ka, pa) {
    return if config.spaces_around_ranges && matches!(pa, K::RangeExpr | K::RangePat) {
      (!matches!(kb, K::RParen | K::RBrack | K::RBrace | K::Comma | K::Semi)).then_some(true)
    } else {
      Some(false)
    };
  }
  if is_range(kb, pb) && config.spaces_around_ranges && matches!(pb, K::RangeExpr | K::RangePat) {
    return (!matches!(ka, K::LParen | K::LBrack | K::LBrace | K::Comma)).then_some(true);
  }
  if is_range(kb, pb) {
    return if matches!(
      ka,
      K::Ident | K::RawIdent | K::SelfKw | K::RParen | K::RBrack | K::Question
    ) || ka.is_literal()
    {
      Some(false)
    } else if is_keyword(ka) {
      Some(true)
    } else {
      None
    };
  }
  // Calls, indexes, macros and attributes.
  if kb == K::LParen {
    // `pub(crate)`, `impl(crate)`, `#[unsafe(no_mangle)]`.
    if matches!(pb, K::Visibility | K::Restriction | K::Attr) || pa == K::Meta {
      return Some(false);
    }
    // `fn f<T>(`, `struct S<T>(`.
    if ka == K::Gt && pa == K::GenericParamList && matches!(pb, K::ParamList | K::TupleFieldList) {
      return Some(false);
    }
    return if callee_end(ka, pa) {
      Some(false)
    } else if is_keyword(ka) || is_operator(ka, pa) || matches!(ka, K::Eq | K::FatArrow) {
      Some(true)
    } else {
      None
    };
  }
  if kb == K::LBrack {
    return if (ka != K::Gt && callee_end(ka, pa)) || matches!(ka, K::Pound | K::Bang | K::Amp) {
      Some(false)
    } else {
      None
    };
  }
  if ka == K::Pound || (ka == K::Bang && pa == K::Attr) {
    return Some(false);
  }
  // Generic lists.
  if kb == K::Lt && pb != K::BinExpr {
    return matches!(
      ka,
      K::Ident | K::RawIdent | K::SelfTypeKw | K::Colon2 | K::ImplKw | K::FnKw | K::ForKw
    )
    .then_some(false);
  }
  if ka == K::Lt && pa != K::BinExpr {
    return Some(false);
  }
  if kb == K::Gt && pb != K::BinExpr {
    return Some(false);
  }
  if ka == K::Gt && pa != K::BinExpr && word_like(b) {
    return Some(true);
  }
  // Operators and arrows: spaced; with `type_punctuation_density = "Compressed"`, a
  // bound's `+` and a type's `=` are tight.
  let compressed = |kind: SyntaxKind, parent: SyntaxKind| {
    config.compressed_type_punctuation
      && ((kind == K::Plus && parent == K::TypeBoundList)
        || (kind == K::Eq && matches!(parent, K::AssocTypeArg | K::TypeParam | K::ConstParam)))
  };
  if compressed(ka, pa) || compressed(kb, pb) {
    return Some(false);
  }
  if is_operator(ka, pa) || is_operator(kb, pb) {
    return Some(true);
  }
  // Prefix operators bind to their operand.
  if is_prefix(ka, pa) {
    return Some(false);
  }
  // `?`: after its expression, before a relaxed bound.
  if kb == K::Question && pb == K::TryExpr {
    return Some(false);
  }
  if ka == K::Question && pa == K::TypeBound {
    return Some(false);
  }
  // Type ascriptions, fields and bounds: `a: T`.
  if kb == K::Colon {
    return Some(config.space_before_colon);
  }
  if ka == K::Colon {
    return Some(config.space_after_colon);
  }
  // Closure parameters: `|a, b| body`.
  if pa == K::ParamList && ka == K::Pipe && is_opening_pipe(a) {
    return Some(false);
  }
  if pb == K::ParamList && kb == K::Pipe && !is_opening_pipe(b) {
    return Some(false);
  }
  if (ka == K::Pipe && pa == K::ParamList) || (ka == K::PipePipe && pa == K::ClosureExpr) {
    return Some(true);
  }
  if word_like(a) && word_like(b) {
    return Some(true);
  }
  None
}

/// Whether the two tokens still read as themselves with nothing between them: words and
/// numbers stay apart, and so do punctuation that would glue into a longer token, except
/// where the grammar reads them apart anyway (`>>` closing two generic lists, `&&` taking
/// a reference to a reference).
fn separate_when_joined(a: &SyntaxToken, b: &SyntaxToken, edition: Edition) -> bool {
  let (Some(last), Some(first)) = (a.text().chars().last(), b.text().chars().next()) else {
    return false;
  };
  let pair: String = [last, first].iter().collect();
  if GLUES.contains(&pair.as_str()) {
    let (pa, pb) = (a.parent().kind(), b.parent().kind());
    let generic_ends =
      a.kind() == K::Gt && b.kind() == K::Gt && pa != K::BinExpr && pb != K::BinExpr;
    let references =
      a.kind() == K::Amp && b.kind() == K::Amp && is_prefix(K::Amp, pa) && is_prefix(K::Amp, pb);
    return generic_ends || references;
  }
  // A number before a field or method: `1.into()` and `x.0.f()` read as written, but a
  // number would swallow the dot before digits.
  if matches!(a.kind(), K::IntNumber) && b.kind() == K::Dot {
    let mut after = b.next_sibling_or_token();
    while let Some(SyntaxElement::Token(t)) = &after
      && t.kind().is_trivia()
    {
      after = t.next_sibling_or_token();
    }
    return after
      .and_then(|e| match e {
        SyntaxElement::Node(n) => n.first_token(),
        SyntaxElement::Token(t) => Some(t),
      })
      .is_some_and(|t| {
        t.text()
          .starts_with(|c: char| c.is_alphabetic() || c == '_')
      });
  }
  // Only words and numbers can merge with their neighbor beyond the pairs above.
  let wordy = |t: &SyntaxToken| {
    t.kind().is_literal() || word_like(t) || t.text().ends_with(|c: char| c.is_alphanumeric())
  };
  if !wordy(a) && !wordy(b) {
    return true;
  }
  let kinds =
    |text: &str| -> Vec<SyntaxKind> { tokenize(text, edition).iter().map(|t| t.kind).collect() };
  let mut apart = kinds(a.text());
  apart.extend(kinds(b.text()));
  kinds(&format!("{}{}", a.text(), b.text())) == apart
}

/// Whether the arguments after a macro's `!` hold tokens no expression list can: `=>`,
/// `#` or `$`.
fn unreadable_arguments(bang: &SyntaxToken) -> bool {
  let mut next = bang.next_sibling_or_token();
  while let Some(SyntaxElement::Token(t)) = &next
    && t.kind().is_trivia()
  {
    next = t.next_sibling_or_token();
  }
  let Some(SyntaxElement::Node(tree)) = next else {
    return false;
  };
  tree.subtree_has(|kind| matches!(kind, K::FatArrow | K::Pound | K::Dollar))
}

fn word_like(token: &SyntaxToken) -> bool {
  let kind = token.kind();
  kind.is_literal() || is_keyword(kind) || matches!(kind, K::Ident | K::RawIdent | K::Lifetime)
}

#[cfg(test)]
mod tests;
