//! The meaning check for formatted output: the output's tokens equal the input's,
//! apart from whitespace, comments and the allowed rewrites ([`super::rewrites`]), and every
//! comment survives.
//!
//! Both texts are parsed and walked into a canonical token stream: whitespace and comments
//! dropped, doc comments kept (they are attributes) with each line trimmed, and every
//! allowed rewrite resolved to one form. The streams must be equal. Comments are compared as
//! the multiset of their words, markers such as `//` and `*` left out, since formatting may
//! re-indent a block comment and move a comment across a comma.
//!
//! Tokens are compared as rustc reads them: touching punctuation it glues into one token
//! (`->`, `<-`, `...`) is one token in a macro's arguments, and noted elsewhere.

use std::borrow::Cow;

use super::{Rule, Violation, describe_list, difference};
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, TextRange, parse_cached};

/// Marks two tokens that touch where rustc would read them as one (see `glues`).
const TOUCH: &str = "\u{1}touch ";

/// Marks the start of one imported path in a `use`'s canonical tokens.
const USE_ENTRY: &str = "\u{0}use";

/// The text of a tree parsed from it, whose tokens' texts are slices of it.
struct Source<'a> {
  text: &'a str,
  edition: Edition,
}

impl<'a> Source<'a> {
  fn new(text: &'a str, edition: Edition) -> Source<'a> {
    Source { text, edition }
  }

  /// The text at `range`, borrowed for as long as the text is.
  fn at(&self, range: TextRange) -> &'a str {
    &self.text[range.start as usize..range.end as usize]
  }
}

/// One canonical token, with the byte offset it came from, made a line number only for a
/// report. Its text is borrowed from the source unless a rule rewrote it.
#[derive(Clone, Debug)]
struct Token<'a> {
  text: Cow<'a, str>,
  start: u32,
}

/// Every rule formatted output breaks; empty when it keeps the input's meaning.
pub fn verify_format(input: &str, output: &str, edition: Edition) -> Vec<Violation> {
  let mut violations = Vec::new();
  if input.starts_with('\u{feff}') != output.starts_with('\u{feff}') {
    violations.push(Violation {
      rule: Rule::BomChanged,
      detail: "the byte-order mark was added or removed".to_string(),
    });
  }
  let (old_tree, old_errors) = tree(input, edition);
  let (new_tree, new_errors) = tree(output, edition);
  if new_errors > old_errors {
    violations.push(Violation {
      rule: Rule::OutputBroken,
      detail: format!(
        "the output has {} syntax error(s), the input {}",
        new_errors, old_errors
      ),
    });
    return violations;
  }
  let (old_tokens, old_comments) = canonical(&old_tree, input, edition);
  let (new_tokens, new_comments) = canonical(&new_tree, output, edition);
  if let Some(detail) = first_difference(&old_tokens, input, &new_tokens, output) {
    violations.push(Violation {
      rule: Rule::MeaningChanged,
      detail,
    });
  }
  let (lost, added) = difference(
    old_comments.iter().map(String::as_str),
    new_comments.iter().map(String::as_str),
  );
  if !lost.is_empty() {
    violations.push(Violation {
      rule: Rule::CommentLost,
      detail: format!("{} comment(s) lost: {}", lost.len(), describe_list(&lost)),
    });
  }
  if !added.is_empty() {
    violations.push(Violation {
      rule: Rule::CommentAdded,
      detail: format!(
        "{} comment(s) added: {}",
        added.len(),
        describe_list(&added)
      ),
    });
  }
  violations
}

/// Items in the order of their tokens' texts, compared without copying them.
fn by_text(a: &[Token], b: &[Token]) -> std::cmp::Ordering {
  a.iter().map(|t| &t.text).cmp(b.iter().map(|t| &t.text))
}

/// The canonical token stream of a file, and its comments.
fn canonical<'a>(
  root: &SyntaxNode,
  text: &'a str,
  edition: Edition,
) -> (Vec<Token<'a>>, Vec<String>) {
  let source = Source::new(text, edition);
  let mut tokens = Vec::new();
  let mut comments = Vec::new();
  emit_node(root, &source, &mut tokens, &mut comments);
  // doc-text: consecutive doc comments of one kind are one text.
  let mut merged: Vec<Token> = Vec::with_capacity(tokens.len());
  // Where the last kept token's kind ends; what is joined to it comes after.
  let mut last_kind = None;
  for token in tokens {
    // A byte loop: most tokens are a few bytes, and `\0` is one.
    let kind = token.text.bytes().position(|b| b == 0);
    if let (Some(last), Some(at), Some(last_at)) = (merged.last_mut(), kind, last_kind)
      && token.text[..at] == last.text[..last_at]
    {
      last.text.to_mut().push_str(&token.text[at + 1..]);
      continue;
    }
    last_kind = kind;
    merged.push(token);
  }
  // comment-text: comments as the multiset of their words.
  // A custom comment's marker (`//@`) is a word of its own, as a space after it may be
  // added (`normalize_comments`).
  let mut words: Vec<String> = comments
    .iter()
    .flat_map(|c| {
      let mut words = c.split_whitespace();
      let first = words.next().map(|w| {
        let at = w.find(|c: char| c.is_alphanumeric()).unwrap_or(w.len());
        if at > 0 && at < w.len() {
          vec![&w[..at], &w[at..]]
        } else {
          vec![w]
        }
      });
      first.into_iter().flatten().chain(words)
    })
    .filter(|w| !w.chars().all(|c| c == '*' || c == '/' || c == '!'))
    .map(str::to_string)
    .collect();
  words.sort();
  (merged, words)
}

/// string-continuation: a `\` ending a line inside a string, the line break and the next
/// line's leading whitespace are not part of the string, so they are dropped here.
fn continued(text: &str) -> String {
  let mut out = String::with_capacity(text.len());
  let mut chars = text.chars().peekable();
  while let Some(c) = chars.next() {
    if c != '\\' {
      out.push(c);
      continue;
    }
    match chars.peek() {
      Some('\n') | Some('\r') => {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
          chars.next();
        }
      }
      Some(_) => {
        out.push(c);
        out.push(chars.next().unwrap_or_default());
      }
      None => out.push(c),
    }
  }
  out
}

/// The arguments of a `#[derive(...)]` attribute, as tokens without commas, or `None`.
fn derive_arguments<'a>(attr: &SyntaxNode, source: &Source<'a>) -> Option<Vec<Token<'a>>> {
  let tokens: Vec<(Cow<'a, str>, u32)> = attr
    .descendants_with_tokens_iter()
    .filter_map(|e| match e {
      // Each token's canonical text, as everywhere: a doc comment written in the list
      // (`/// x`, which keeps a `\r` in a CRLF file) is its text.
      SyntaxElement::Token(t) if !t.kind().is_trivia() => Some((
        token_text(&t.parent(), t.kind(), source.at(t.text_range())),
        t.text_range().start,
      )),
      _ => None,
    })
    .collect();
  let texts: Vec<&str> = tokens.iter().map(|(t, _)| t.as_ref()).collect();
  if texts.len() < 6
    || texts[..4] != ["#", "[", "derive", "("]
    || texts[texts.len() - 2..] != [")", "]"]
  {
    return None;
  }
  Some(
    tokens[4..tokens.len() - 2]
      .iter()
      .filter(|(t, _)| t.as_ref() != ",")
      .map(|(text, start)| Token {
        text: text.clone(),
        start: *start,
      })
      .collect(),
  )
}

/// doc-text for `#[doc = "..."]` and `#![doc = "..."]`: the string's text as a doc
/// comment's, or `None` for any other attribute.
fn doc_attribute(attr: &SyntaxNode) -> Option<String> {
  let texts: Vec<&str> = attr
    .token_texts()
    .filter(|(kind, _, _)| !kind.is_trivia())
    .map(|(_, text, _)| text)
    .collect();
  let (kind, string) = match texts.as_slice() {
    ["#", "[", "doc", "=", string, "]"] => ("outer", *string),
    ["#", "!", "[", "doc", "=", string, "]"] => ("inner", *string),
    _ => return None,
  };
  let inner = string
    .trim_start_matches('r')
    .trim_matches('#')
    .strip_prefix('"')?
    .strip_suffix('"')?
    .replace("\\n", " ")
    .replace("\\t", " ")
    .replace("\\\"", "\"")
    .replace("\\\\", "\\");
  Some(doc_text(kind, &format!("///{}", inner)))
}

/// A doc comment's text without its markers or any whitespace, behind its kind and a NUL:
/// the form `doc-text` compares (`outer` or `inner`).
fn doc_text(kind: &str, text: &str) -> String {
  // A line doc comment loses only its `///` or `//!`; a block one its `/**` or `/*!` and its
  // closing `*/`, each once. A `*/` ending a line doc comment is text (and before a `\r`
  // it would not even be last).
  let body = if let Some(block) = text
    .strip_prefix("/**")
    .or_else(|| text.strip_prefix("/*!"))
  {
    block.strip_suffix("*/").unwrap_or(block)
  } else {
    text
      .strip_prefix("///")
      .or_else(|| text.strip_prefix("//!"))
      .unwrap_or(text)
  };
  let mut out = format!("{}\u{0}", kind);
  for line in body.lines() {
    let line = line.trim_start().trim_start_matches('*');
    out.extend(line.chars().filter(|c| !c.is_whitespace()));
  }
  out
}

/// One child of `parent`, after the rewrites that depend on where it sits.
fn emit_element<'a>(
  parent: &SyntaxNode,
  element: &SyntaxElement,
  source: &Source<'a>,
  out: &mut Vec<Token<'a>>,
  notes: &mut Vec<String>,
) {
  match element {
    SyntaxElement::Node(node) => {
      // closure-body-braces and match-arm-body: a block holding only an expression is
      // that expression.
      if node.kind() == SyntaxKind::BlockExpr
        && matches!(
          parent.kind(),
          SyntaxKind::ClosureExpr | SyntaxKind::MatchArm
        )
        && let Some(mut expr) = sole_expression(node)
      {
        // `{ { x } }` is `x` too.
        while let Some(inner) = sole_expression(&expr) {
          expr = inner;
        }
        return emit_node(&expr, source, out, notes);
      }
      // block-semicolon: `loop {};` is `loop {}`, and so is any block-like statement
      // before another statement.
      if node.kind() == SyntaxKind::ExprStmt {
        let inner = node.children();
        if let [expr] = inner.as_slice() {
          // A `loop` may yield a value through `break value`; then the `;` counts. Only a
          // `loop` is searched: the walk covers all of it.
          let valued_break = || {
            expr
              .descendant_nodes_where(|kind| kind == SyntaxKind::BreakExpr)
              .any(|d| d.children_iter().any(|c| is_expression(c.kind())))
          };
          let looping = matches!(expr.kind(), SyntaxKind::WhileExpr | SyntaxKind::ForExpr)
            || (expr.kind() == SyntaxKind::LoopExpr && !valued_break());
          let block_like = looping
            || matches!(
              expr.kind(),
              SyntaxKind::IfExpr | SyntaxKind::MatchExpr | SyntaxKind::BlockExpr
            );
          if looping || (block_like && !ends_block(node)) {
            return emit_node(expr, source, out, notes);
          }
        }
      }
      // trailing-semicolon: `return x;` ending a block is `return x`.
      if node.kind() == SyntaxKind::ExprStmt && ends_block(node) {
        let inner = node.children();
        if let [expr] = inner.as_slice()
          && matches!(
            expr.kind(),
            SyntaxKind::ReturnExpr | SyntaxKind::BreakExpr | SyntaxKind::ContinueExpr
          )
        {
          return emit_node(expr, source, out, notes);
        }
      }
      // stray-semicolon: the parser wraps a `;` alone among items in an error node.
      if node.kind() == SyntaxKind::Error
        && matches!(
          parent.kind(),
          SyntaxKind::SourceFile | SyntaxKind::ItemList | SyntaxKind::AssocItemList
        )
        && words(node, "") == ";"
      {
        return;
      }
      if node.kind() == SyntaxKind::Attr
        && let Some(text) = doc_attribute(node)
      {
        out.push(Token {
          text: text.into(),
          start: node.text_range().start,
        });
        return;
      }
      if node.kind() == SyntaxKind::Attr && merged_derive(node, source, out) {
        return;
      }
      emit_node(node, source, out, notes);
    }
    SyntaxElement::Token(token) => {
      let kind = token.kind();
      let next = next_significant(element);
      // empty-generics: `::` before a path segment's `<>` or `(..)` sugar is optional
      // (`String::<>::from`, `Fn::(i32)`).
      let turbofish = (parent.kind() == SyntaxKind::PathSegment
        && matches!(
          next,
          Some(SyntaxKind::GenericArgList | SyntaxKind::ParenthesizedArgList)
        ))
        || (parent.kind() == SyntaxKind::GenericArgList && previous_significant(element).is_none());
      if kind == SyntaxKind::Colon2 && turbofish {
        return;
      }
      // empty-generics: a `+` ending a bound list.
      if kind == SyntaxKind::Plus && parent.kind() == SyntaxKind::TypeBoundList && next.is_none() {
        return;
      }
      // trailing-comma, except in a one-element tuple; match-arm-body's comma. A comma
      // before `{` is never dropped: `f(a, { b })` and `f(a { b })` differ.
      let closure_params = parent.kind() == SyntaxKind::ParamList
        && parent
          .parent()
          .is_some_and(|p| p.kind() == SyntaxKind::ClosureExpr);
      // A comma straight after an opening delimiter (`[,]`, `m!(,)`) follows no item: it is
      // not a trailing comma, and dropping it changes the tokens.
      let after_item = || {
        previous_significant(element).is_some_and(|k| {
          !matches!(
            k,
            SyntaxKind::LParen | SyntaxKind::LBrack | SyntaxKind::LBrace | SyntaxKind::Lt
          )
        })
      };
      if kind == SyntaxKind::Comma
        && !single_tuple(parent)
        && (parent.kind() == SyntaxKind::MatchArm
          || next.is_none_or(is_closer) && after_item()
          || (closure_params && next == Some(SyntaxKind::Pipe)))
      {
        return;
      }
      // stray-semicolon: a `;` standing alone in a file, module, impl or block.
      if kind == SyntaxKind::Semi
        && matches!(
          parent.kind(),
          SyntaxKind::SourceFile
            | SyntaxKind::ItemList
            | SyntaxKind::AssocItemList
            | SyntaxKind::BlockExpr
        )
      {
        return;
      }
      // match-arm-leading-pipe.
      let pattern_start = matches!(parent.kind(), SyntaxKind::MatchArm | SyntaxKind::OrPat);
      if kind == SyntaxKind::Pipe && pattern_start && previous_significant(element).is_none() {
        return;
      }
      let range = token.text_range();
      let text = source.at(range);
      // rustc's lexer glues touching punctuation where it can (`->`, `...`, `<<=`), wherever
      // it stands. In a macro's arguments it stays glued, and a macro tells `->` from `- >`.
      // Elsewhere the parser left these two tokens apart; rustc reads `x <-1` as `x`, `<-`,
      // `1` and rejects it, so a touch is noted. rustc splits again only a `>` closing
      // generics (`Vec<Vec<u8>>`), `&&` taking a reference twice and `||` around closures:
      // where the parser left those apart, rustc's parser takes them apart too.
      if let Some(last) = out.last_mut()
        && last.start as usize + last.text.len() == range.start as usize
        && glues(&last.text, text)
      {
        if parent.kind() == SyntaxKind::TokenTree {
          last.text.to_mut().push_str(text);
          return;
        }
        if !last.text.starts_with('>') && !matches!((&*last.text, text), ("&", "&") | ("|", "|")) {
          let touch = format!("{}{}{}", TOUCH, last.text, text);
          out.push(Token {
            text: touch.into(),
            start: range.start,
          });
        }
      }
      out.push(Token {
        text: token_text(parent, kind, text),
        start: range.start,
      });
    }
  }
}

/// The node's significant children, after the rewrites that act on a whole node.
fn emit_node<'a>(
  node: &SyntaxNode,
  source: &Source<'a>,
  out: &mut Vec<Token<'a>>,
  notes: &mut Vec<String>,
) {
  // Comments anywhere in the node, before any child is skipped or replaced.
  if node.parent().is_none() {
    for (kind, text, _) in node.token_texts() {
      if matches!(kind, SyntaxKind::LineComment | SyntaxKind::BlockComment) {
        notes.push(
          text
            .replace("/*", " ")
            .replace("*/", " ")
            .replace("//", " "),
        );
      }
    }
  }
  // A macro call's arguments that read as expressions (bar `vec!`'s, read as an array's),
  // read before the match: its arms cannot keep what their conditions read.
  let read = (node.kind() == SyntaxKind::TokenTree
    && node
      .parent()
      .is_some_and(|p| p.kind() == SyntaxKind::MacroCall && !words(&p, "").starts_with("vec!")))
  .then(|| macro_arguments(node, source, false))
  .flatten();
  match node.kind() {
    SyntaxKind::ParenExpr => {
      // nested-parens: `((a))` is `(a)`.
      let inner: Vec<SyntaxNode> = node.children();
      // number-literals: parentheses around a literal (a float receiver) group nothing.
      if let [only] = inner.as_slice()
        && matches!(only.kind(), SyntaxKind::ParenExpr | SyntaxKind::Literal)
      {
        return emit_node(only, source, out, notes);
      }
    }
    SyntaxKind::Use => return emit_use(node, source, out),
    // empty-generics: `<>` says nothing, nor does a `:` with no bounds after it.
    SyntaxKind::GenericParamList | SyntaxKind::GenericArgList
      if node.children_iter().next().is_none() =>
    {
      return;
    }
    // empty-generics: a `where` with nothing after it.
    SyntaxKind::WhereClause if node.children_iter().next().is_none() => return,
    SyntaxKind::LifetimeParam
      if node
        .children_iter()
        .all(|c| c.kind() == SyntaxKind::TypeBoundList && c.children_iter().next().is_none()) =>
    {
      for element in node.children_with_tokens_iter() {
        if kind_of(&element) == SyntaxKind::Lifetime {
          emit_element(node, &element, source, out, notes);
        }
      }
      return;
    }
    SyntaxKind::TypeParam
      if node.children_iter().all(|c| {
        c.kind() == SyntaxKind::Name
          || (c.kind() == SyntaxKind::TypeBoundList && c.children_iter().next().is_none())
      }) =>
    {
      for child in node
        .children_iter()
        .filter(|c| c.kind() == SyntaxKind::Name)
      {
        emit_node(&child, source, out, notes);
      }
      return;
    }
    SyntaxKind::ParamList
      if node
        .parent()
        .is_some_and(|p| p.kind() == SyntaxKind::ClosureExpr)
        && words(node, " ") == "| |" =>
    {
      // empty-closure-params: `| |` is `||`.
      out.push(Token {
        text: "||".into(),
        start: node.text_range().start,
      });
      return;
    }
    SyntaxKind::RecordExprField => {
      // field-init-shorthand: `x: x` is `x`.
      let parts: Vec<SyntaxElement> = node
        .children_with_tokens_iter()
        .filter(significant)
        .collect();
      let attrs = parts
        .iter()
        .take_while(|e| kind_of(e) == SyntaxKind::Attr)
        .count();
      if let [SyntaxElement::Node(name), colon, SyntaxElement::Node(value)] = &parts[attrs..]
        && kind_of(colon) == SyntaxKind::Colon
        && value.kind() == SyntaxKind::PathExpr
        && words(name, "") == words(value, "")
      {
        for attr in &parts[..attrs] {
          emit_element(node, attr, source, out, notes);
        }
        out.push(Token {
          text: words(name, "").into(),
          start: node.text_range().start,
        });
        return;
      }
    }
    SyntaxKind::TuplePat | SyntaxKind::TupleStructPat => {
      // wildcard-suffix: two or more trailing `_` are `..`.
      let children: Vec<SyntaxElement> = node
        .children_with_tokens_iter()
        .filter(significant)
        .collect();
      let patterns: Vec<usize> = children
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, SyntaxElement::Node(n) if n.kind() != SyntaxKind::Path))
        .map(|(i, _)| i)
        .collect();
      // Wildcards just before a final `..` fold into it too.
      let rest_last = patterns
        .last()
        .is_some_and(|&i| kind_of(&children[i]) == SyntaxKind::RestPat);
      let before = patterns.len() - usize::from(rest_last);
      let trailing = patterns[..before]
        .iter()
        .rev()
        .take_while(|&&i| kind_of(&children[i]) == SyntaxKind::WildcardPat)
        .count();
      if trailing >= 2 || (rest_last && trailing >= 1) {
        let first = patterns[before - trailing];
        for element in &children[..first] {
          emit_element(node, element, source, out, notes);
        }
        out.push(Token {
          text: "..".into(),
          start: node.text_range().start,
        });
        if let Some(close) = children.last() {
          emit_element(node, close, source, out, notes);
        }
        return;
      }
    }
    // vec-delimiters: `vec!(..)` and `vec!{..}` are `vec![..]`.
    SyntaxKind::TokenTree
      if node.parent().is_some_and(|p| {
        p.kind() == SyntaxKind::MacroCall && words(&p, "").starts_with("vec!")
      }) =>
    {
      if let Some(tokens) = macro_arguments(node, source, true) {
        out.extend(tokens);
        return;
      }
      return emit_tree(node, "[", "]", source, out, notes);
    }
    // macro-arguments: a macro call's arguments in parentheses or brackets that read as
    // expressions, as those expressions.
    SyntaxKind::TokenTree if read.is_some() => {
      out.extend(read.into_iter().flatten());
      return;
    }
    // macro-rules-arms: each arm's matcher and transcriber in any delimiters, and the
    // `;` between arms.
    SyntaxKind::TokenTree
      if node
        .parent()
        .is_some_and(|p| p.kind() == SyntaxKind::MacroRules) =>
    {
      let children: Vec<SyntaxElement> = node
        .children_with_tokens_iter()
        .filter(significant)
        .collect();
      let start = node.text_range().start;
      out.push(Token {
        text: "{".into(),
        start,
      });
      let mut matcher = true;
      for element in children
        .iter()
        .skip(1)
        .take(children.len().saturating_sub(2))
      {
        match element {
          SyntaxElement::Node(arm) if arm.kind() == SyntaxKind::TokenTree => {
            // macro-rules-transcribers: a transcriber that reads as code, as that code.
            let code = (!matcher).then(|| transcriber(arm, source)).flatten();
            match code {
              Some(tokens) => {
                out.push(Token {
                  text: "{".into(),
                  start,
                });
                out.extend(tokens);
                out.push(Token {
                  text: "}".into(),
                  start,
                });
              }
              None => emit_tree(arm, "{", "}", source, out, notes),
            }
            matcher = !matcher;
          }
          SyntaxElement::Token(t) if t.kind() == SyntaxKind::Semi => {}
          other => emit_element(node, other, source, out, notes),
        }
      }
      out.push(Token {
        text: "}".into(),
        start,
      });
      return;
    }
    SyntaxKind::MacroCall if words(node, "").starts_with("try!(") => {
      // try-shorthand: `try!(e)` is `e?`; `e` compared as an expression when it reads as one.
      if let Some(tree) = node
        .children_iter()
        .find(|c| c.kind() == SyntaxKind::TokenTree)
      {
        if let Some(tokens) = macro_arguments(&tree, source, false)
          && tokens.len() >= 2
        {
          let inside = tokens.len() - 1;
          out.extend(tokens.into_iter().take(inside).skip(1));
          out.push(Token {
            text: "?".into(),
            start: node.text_range().start,
          });
          return;
        }
        let inner: Vec<SyntaxElement> = tree
          .children_with_tokens_iter()
          .filter(significant)
          .collect();
        if inner.len() >= 2 {
          for element in &inner[1..inner.len() - 1] {
            emit_element(&tree, element, source, out, notes);
          }
          out.push(Token {
            text: "?".into(),
            start: node.text_range().start,
          });
          return;
        }
      }
    }
    SyntaxKind::Visibility => {
      // visibility-in: `pub(in crate)` is `pub(crate)`.
      let words: Vec<&str> = node
        .token_texts()
        .filter(|(kind, _, _)| !kind.is_trivia())
        .map(|(_, text, _)| text)
        .collect();
      // A path from the root names a module from the crate's (`pub(in ::a)` is
      // `pub(in a)`).
      if let ["pub", "(", "in", "::", rest @ ..] = words.as_slice() {
        let start = node.text_range().start;
        for text in ["pub", "(", "in"].iter().chain(rest) {
          out.push(Token {
            text: text.to_string().into(),
            start,
          });
        }
        return;
      }
      if let ["pub", "(", "in", target @ ("crate" | "super" | "self"), ")"] = words.as_slice() {
        let start = node.text_range().start;
        for text in ["pub", "(", target, ")"] {
          out.push(Token {
            text: text.to_string().into(),
            start,
          });
        }
        return;
      }
    }
    SyntaxKind::Abi => {
      // explicit-abi: `extern` alone is `extern "C"`.
      let has_string = node
        .children_with_tokens_iter()
        .any(|e| significant(&e) && kind_of(&e) != SyntaxKind::ExternKw);
      if !has_string {
        let start = node.text_range().start;
        out.push(Token {
          text: "extern".into(),
          start,
        });
        out.push(Token {
          text: "\"C\"".into(),
          start,
        });
        return;
      }
    }
    SyntaxKind::ExternBlock => {
      // explicit-abi for `extern { ... }`, whose `extern` may sit outside an Abi node.
      let children = node.children_with_tokens();
      let bare = children.iter().any(|e| kind_of(e) == SyntaxKind::ExternKw)
        && !children.iter().any(|e| kind_of(e) == SyntaxKind::Abi);
      if bare {
        for element in children.iter().filter(|e| significant(e)) {
          match element {
            SyntaxElement::Token(t) if t.kind() == SyntaxKind::ExternKw => {
              let start = t.text_range().start;
              out.push(Token {
                text: "extern".into(),
                start,
              });
              out.push(Token {
                text: "\"C\"".into(),
                start,
              });
            }
            other => emit_element(node, other, source, out, notes),
          }
        }
        return;
      }
    }
    _ => {}
  }
  let container = matches!(
    node.kind(),
    SyntaxKind::SourceFile | SyntaxKind::ItemList | SyntaxKind::BlockExpr
  );
  let impl_items = node.kind() == SyntaxKind::AssocItemList
    && node.parent().is_some_and(|p| p.kind() == SyntaxKind::Impl);
  // Most nodes are neither reordered nor sorted: their children are emitted as reached.
  if !container && !impl_items {
    for element in node.children_with_tokens_iter().filter(significant) {
      emit_element(node, &element, source, out, notes);
    }
    return;
  }
  let children: Vec<SyntaxElement> = node
    .children_with_tokens_iter()
    .filter(significant)
    .collect();
  // impl-item-order: an impl's items compare in sorted order.
  if impl_items {
    let mut items: Vec<Vec<Token>> = Vec::new();
    for element in &children {
      if let SyntaxElement::Node(_) = element {
        let mut item = Vec::new();
        emit_element(node, element, source, &mut item, notes);
        items.push(item);
      } else {
        emit_element(node, element, source, out, notes);
      }
    }
    // The braces were emitted in place; the items go between them.
    let close = out.pop();
    items.sort_by(|a, b| by_text(a, b));
    out.extend(items.into_iter().flatten());
    out.extend(close);
    return;
  }
  let mut i = 0;
  while i < children.len() {
    let element = &children[i];
    // import-and-module-order: a run of `use` items, or of `mod name;` declarations, is
    // compared sorted.
    if container && let Some(key) = sortable(element) {
      let mut run: Vec<Vec<Token>> = Vec::new();
      while let Some(element) = children.get(i)
        && sortable(element) == Some(key)
      {
        let mut item = Vec::new();
        emit_element(node, element, source, &mut item, notes);
        run.push(item);
        i += 1;
      }
      if key == SyntaxKind::Use {
        // use-trees: the run is the set of what it imports.
        let mut entries: Vec<Vec<Token>> = run.into_iter().flat_map(split_use).collect();
        entries.sort_by(|a, b| by_text(a, b));
        entries.dedup_by(|a, b| a.iter().map(|t| &t.text).eq(b.iter().map(|t| &t.text)));
        out.extend(entries.into_iter().flatten());
        continue;
      }
      run.sort_by(|a, b| by_text(a, b));
      out.extend(run.into_iter().flatten());
      continue;
    }
    emit_element(node, element, source, out, notes);
    i += 1;
  }
}

/// A token tree with its delimiters replaced by `open` and `close`.
fn emit_tree<'a>(
  tree: &SyntaxNode,
  open: &'a str,
  close: &'a str,
  source: &Source<'a>,
  out: &mut Vec<Token<'a>>,
  notes: &mut Vec<String>,
) {
  let children: Vec<SyntaxElement> = tree
    .children_with_tokens_iter()
    .filter(significant)
    .collect();
  let start = tree.text_range().start;
  out.push(Token {
    text: open.into(),
    start,
  });
  for element in children
    .iter()
    .skip(1)
    .take(children.len().saturating_sub(2))
  {
    emit_element(tree, element, source, out, notes);
  }
  out.push(Token {
    text: close.into(),
    start,
  });
}

/// use-trees: a `use` as one entry per imported path, each with the `use`'s attributes and
/// visibility, sorted.
fn emit_use<'a>(node: &SyntaxNode, source: &Source<'a>, out: &mut Vec<Token<'a>>) {
  let start = node.text_range().start;
  let mut head = Vec::new();
  let mut paths = Vec::new();
  for child in node.children_with_tokens_iter().filter(significant) {
    match &child {
      SyntaxElement::Node(n) if n.kind() == SyntaxKind::UseTree => {
        flatten_tree(n, "", &mut paths);
      }
      SyntaxElement::Node(n) => {
        let mut notes = Vec::new();
        emit_node(n, source, &mut head, &mut notes);
      }
      SyntaxElement::Token(t) if t.kind() == SyntaxKind::Semi => {}
      SyntaxElement::Token(t) => head.push(Token {
        text: token_text(node, t.kind(), source.at(t.text_range())),
        start,
      }),
    }
  }
  paths.sort();
  paths.dedup();
  // `split_use` cuts the entries apart again, for use-trees to compare a run as a set.
  for path in paths {
    out.push(Token {
      text: USE_ENTRY.into(),
      start,
    });
    out.extend(head.iter().cloned());
    out.push(Token {
      text: path.into(),
      start,
    });
  }
}

/// Whether a statement is the last thing in its block, with no tail expression after it.
fn ends_block(statement: &SyntaxNode) -> bool {
  let mut next = statement.next_sibling_or_token();
  while let Some(element) = next {
    // A `;` standing alone in the block, which stray-semicolon drops (`return x; ;`).
    let stray = matches!(&element, SyntaxElement::Token(t)
      if t.kind() == SyntaxKind::Semi && t.parent().kind() == SyntaxKind::BlockExpr);
    if significant(&element) && !stray {
      return kind_of(&element) == SyntaxKind::RBrace;
    }
    next = match &element {
      SyntaxElement::Token(t) => t.next_sibling_or_token(),
      SyntaxElement::Node(n) => n.next_sibling_or_token(),
    };
  }
  false
}

/// Where two canonical streams first differ, with a little context from each side.
/// `old` and `new` are the streams of `input` and `output`.
fn first_difference(old: &[Token], input: &str, new: &[Token], output: &str) -> Option<String> {
  let at = old
    .iter()
    .zip(new)
    .position(|(a, b)| a.text != b.text)
    .or_else(|| (old.len() != new.len()).then(|| old.len().min(new.len())))?;
  let around = |tokens: &[Token], text: &str| -> String {
    let start = at.saturating_sub(4);
    let end = (at + 5).min(tokens.len());
    // The 1-based line holding the token's start.
    let line = tokens.get(at).or(tokens.last()).map_or(0, |t| {
      1 + text.as_bytes()[..t.start as usize]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
    });
    let shown: Vec<&str> = tokens[start..end].iter().map(|t| t.text.as_ref()).collect();
    format!("line {}: `{}`", line, shown.join(" "))
  };
  Some(format!(
    "the tokens differ: input {}, output {}",
    around(old, input),
    around(new, output)
  ))
}

/// The paths a use tree imports, each as `a::b::c`, `a::b::*` or `a::b as c`.
fn flatten_tree(tree: &SyntaxNode, prefix: &str, paths: &mut Vec<String>) {
  let mut path = String::new();
  let mut list = None;
  let mut glob = false;
  let mut rename = String::new();
  for child in tree.children_with_tokens_iter().filter(significant) {
    match &child {
      SyntaxElement::Node(n) if n.kind() == SyntaxKind::Path => path = words(n, ""),
      SyntaxElement::Node(n) if n.kind() == SyntaxKind::UseTreeList => list = Some(n.clone()),
      SyntaxElement::Node(n) if n.kind() == SyntaxKind::Rename => rename = words(n, " "),
      SyntaxElement::Token(t) if t.kind() == SyntaxKind::Star => glob = true,
      _ => {}
    }
  }
  let join = |a: &str, b: &str| match (a.is_empty(), b.is_empty()) {
    (true, _) => b.to_string(),
    (_, true) => a.to_string(),
    _ => format!("{}::{}", a, b),
  };
  // `self` inside a list names the list's own prefix.
  let full = if path == "self" && !prefix.is_empty() {
    prefix.to_string()
  } else {
    join(prefix, &path)
  };
  if let Some(list) = list {
    for inner in list
      .children_iter()
      .filter(|c| c.kind() == SyntaxKind::UseTree)
    {
      flatten_tree(&inner, &full, paths);
    }
    return;
  }
  // `a::self` outside a list (an error the formatter still rewrites) is `a`, and a
  // leading `::` names the same crate in a `use`.
  let full = full
    .strip_suffix("::self")
    .map(str::to_string)
    .unwrap_or(full);
  let full = full.strip_prefix("::").map(str::to_string).unwrap_or(full);
  // `a as a` is `a`.
  let last = full.rsplit("::").next().unwrap_or("");
  if !glob && rename == format!("as {}", last) {
    rename.clear();
  }
  let mut entry = if glob { join(&full, "*") } else { full };
  if !rename.is_empty() {
    entry = format!("{} {}", entry, rename);
  }
  paths.push(entry);
}

/// Whether rustc's lexer glues punctuation `next` onto the token `last` it touches.
fn glues(last: &str, next: &str) -> bool {
  matches!(
    (last, next),
    ("=", "=" | ">")
      | ("<", "=" | "<" | "-")
      | ("<<", "=")
      | (">", "=" | ">")
      | (">>", "=")
      | ("!", "=")
      | ("+" | "-" | "*" | "/" | "%" | "^" | "&" | "|", "=")
      | ("&", "&")
      | ("|", "|")
      | ("-", ">")
      | (".", ".")
      | ("..", "." | "=")
      | (":", ":")
  )
}

fn is_closer(kind: SyntaxKind) -> bool {
  matches!(
    kind,
    SyntaxKind::RParen | SyntaxKind::RBrack | SyntaxKind::RBrace | SyntaxKind::Gt
  )
}

fn is_expression(kind: SyntaxKind) -> bool {
  kind.name().ends_with("_EXPR") || kind == SyntaxKind::Literal
}

fn kind_of(element: &SyntaxElement) -> SyntaxKind {
  match element {
    SyntaxElement::Token(t) => t.kind(),
    SyntaxElement::Node(n) => n.kind(),
  }
}

/// A macro call's arguments' canonical tokens when, in parentheses or brackets (any
/// delimiters for `vec!`, as an array), they read as expressions; `None` otherwise.
fn macro_arguments<'a>(
  tree: &SyntaxNode,
  source: &Source<'_>,
  array: bool,
) -> Option<Vec<Token<'a>>> {
  let whole = tree.text().to_string();
  let open = whole.chars().next()?;
  let inner = whole.get(1..whole.len().checked_sub(1)?)?;
  let (text, kind) = match open {
    _ if array => (
      format!("fn main() {{\n[{}];\n}}", inner),
      SyntaxKind::ArrayExpr,
    ),
    '(' => (
      format!("fn main() {{\n__mto_macro({});\n}}", inner),
      SyntaxKind::ArgList,
    ),
    '[' => (
      format!("fn main() {{\n[{}];\n}}", inner),
      SyntaxKind::ArrayExpr,
    ),
    _ => return None,
  };
  let parsed = crate::syntax::parse(&text, source.edition);
  if !parsed.errors().is_empty() {
    return None;
  }
  let root = parsed.syntax_node();
  let list = root.descendant_nodes_where(|k| k == kind).next()?;
  let inner_source = Source::new(&text, source.edition);
  let mut tokens = Vec::new();
  // Their comments are the file's, counted there.
  let mut comments = Vec::new();
  emit_node(&list, &inner_source, &mut tokens, &mut comments);
  let start = tree.text_range().start;
  Some(
    tokens
      .into_iter()
      .map(|t| Token {
        text: Cow::Owned(t.text.into_owned()),
        start,
      })
      .collect(),
  )
}

/// merge-derives: the first of consecutive `#[derive(...)]` attributes stands for all of
/// them (their traits, in order, as one list); the others emit nothing. Returns whether
/// `attr` was handled here.
fn merged_derive<'a>(attr: &SyntaxNode, source: &Source<'a>, out: &mut Vec<Token<'a>>) -> bool {
  let Some(own) = derive_arguments(attr, source) else {
    return false;
  };
  let previous_is_derive = match previous_significant_node(attr) {
    Some(previous) => {
      previous.kind() == SyntaxKind::Attr && derive_arguments(&previous, source).is_some()
    }
    None => false,
  };
  if previous_is_derive {
    return true;
  }
  let start = attr.text_range().start;
  let mut traits = own;
  let mut next = next_significant_node(attr);
  while let Some(node) = next {
    match (node.kind() == SyntaxKind::Attr)
      .then(|| derive_arguments(&node, source))
      .flatten()
    {
      Some(more) => traits.extend(more),
      None => break,
    }
    next = next_significant_node(&node);
  }
  for text in ["#", "[", "derive", "("] {
    out.push(Token {
      text: text.into(),
      start,
    });
  }
  // Traits are compared as one list without their commas, which is enough to tell
  // `#[derive(A)] #[derive(B)]` and `#[derive(A, B)]` equal and any other list apart.
  out.extend(traits);
  for text in [")", "]"] {
    out.push(Token {
      text: text.into(),
      start,
    });
  }
  true
}

fn next_significant(element: &SyntaxElement) -> Option<SyntaxKind> {
  element.kinds_after().find(|kind| !kind.is_trivia())
}

fn next_significant_node(node: &SyntaxNode) -> Option<SyntaxNode> {
  let mut next = node.next_sibling_or_token();
  while let Some(e) = next {
    match &e {
      SyntaxElement::Node(n) => return Some(n.clone()),
      SyntaxElement::Token(t) if !t.kind().is_trivia() => return None,
      SyntaxElement::Token(t) => next = t.next_sibling_or_token(),
    }
  }
  None
}

/// number-literals: hex digits in lowercase, and a float's all-zero fraction dropped
/// (`1.`, `1.0` and `1.00` are `1`), behind `f` so a float never equals an integer.
fn number(kind: SyntaxKind, text: &str) -> String {
  let lower = text.to_ascii_lowercase();
  // `7f32` is the float `7.0f32`.
  let float_suffix = !lower.starts_with("0x") && (lower.ends_with("f32") || lower.ends_with("f64"));
  if kind != SyntaxKind::FloatNumber && !float_suffix {
    return lower;
  }
  let Some(dot) = lower.find('.') else {
    return format!("f{}", lower);
  };
  let rest = &lower[dot + 1..];
  let zeros = rest.chars().take_while(|c| *c == '0' || *c == '_').count();
  let after = &rest[zeros..];
  if after.is_empty() || after.starts_with(|c: char| !c.is_ascii_digit() && c != '_') {
    format!("f{}{}", &lower[..dot], after)
  } else {
    format!("f{}", lower)
  }
}

fn previous_significant(element: &SyntaxElement) -> Option<SyntaxKind> {
  element.kinds_before().find(|kind| !kind.is_trivia())
}

fn previous_significant_node(node: &SyntaxNode) -> Option<SyntaxNode> {
  let mut previous = node.prev_sibling_or_token();
  while let Some(e) = previous {
    match &e {
      SyntaxElement::Node(n) => return Some(n.clone()),
      SyntaxElement::Token(t) if !t.kind().is_trivia() => return None,
      SyntaxElement::Token(t) => previous = t.prev_sibling_or_token(),
    }
  }
  None
}

fn significant(element: &SyntaxElement) -> bool {
  match element {
    SyntaxElement::Token(t) => !t.kind().is_trivia(),
    SyntaxElement::Node(_) => true,
  }
}

/// A tuple with a single element, whose comma makes it a tuple.
fn single_tuple(node: &SyntaxNode) -> bool {
  matches!(
    node.kind(),
    SyntaxKind::TupleExpr | SyntaxKind::TupleType | SyntaxKind::TuplePat
  ) && node.children().len() == 1
}

/// The only expression in a block that has nothing else: no statements, label, attributes
/// or `unsafe`/`async`/`const`. `None` for any other block.
fn sole_expression(block: &SyntaxNode) -> Option<SyntaxNode> {
  if block.kind() != SyntaxKind::BlockExpr {
    return None;
  }
  // A `;` standing alone in the block is an empty statement (stray-semicolon).
  let inner: Vec<SyntaxElement> = block
    .children_with_tokens_iter()
    .filter(significant)
    .filter(|e| kind_of(e) != SyntaxKind::Semi)
    .collect();
  match inner.as_slice() {
    [open, SyntaxElement::Node(expr), close]
      if kind_of(open) == SyntaxKind::LBrace && kind_of(close) == SyntaxKind::RBrace =>
    {
      if is_expression(expr.kind()) {
        return Some(expr.clone());
      }
      // A block-like expression (`loop {}`, `match`) stands as a statement without `;`.
      let parts: Vec<SyntaxElement> = expr
        .children_with_tokens_iter()
        .filter(significant)
        .collect();
      match parts.as_slice() {
        [SyntaxElement::Node(inner)]
          if expr.kind() == SyntaxKind::ExprStmt && is_expression(inner.kind()) =>
        {
          Some(inner.clone())
        }
        // `{ return x; }` is `return x`: neither yields a value.
        [SyntaxElement::Node(inner), semi]
          if expr.kind() == SyntaxKind::ExprStmt
            && kind_of(semi) == SyntaxKind::Semi
            && matches!(
              inner.kind(),
              SyntaxKind::ReturnExpr | SyntaxKind::BreakExpr | SyntaxKind::ContinueExpr
            ) =>
        {
          Some(inner.clone())
        }
        _ => None,
      }
    }
    _ => None,
  }
}

/// What kind of sortable run an item belongs to: imports, or `mod name;` declarations
/// without `#[macro_use]`.
fn sortable(element: &SyntaxElement) -> Option<SyntaxKind> {
  let SyntaxElement::Node(node) = element else {
    return None;
  };
  match node.kind() {
    SyntaxKind::Use => Some(SyntaxKind::Use),
    SyntaxKind::Module => {
      let declaration = !node
        .children_iter()
        .any(|c| c.kind() == SyntaxKind::ItemList);
      let macro_use = node.text().contains("macro_use");
      (declaration && !macro_use).then_some(SyntaxKind::Module)
    }
    SyntaxKind::ExternCrate => {
      (!node.text().contains("macro_use")).then_some(SyntaxKind::ExternCrate)
    }
    _ => None,
  }
}

/// A `use`'s canonical tokens cut into one entry per imported path.
fn split_use(tokens: Vec<Token<'_>>) -> Vec<Vec<Token<'_>>> {
  let mut entries: Vec<Vec<Token>> = Vec::new();
  for token in tokens {
    if token.text == USE_ENTRY {
      entries.push(Vec::new());
    }
    if let Some(entry) = entries.last_mut() {
      entry.push(token);
    }
  }
  entries
}

/// A token's canonical text: CRLF as LF, doc comments as their text, numbers normalized,
/// `...` in a range pattern as `..=`.
fn token_text<'a>(parent: &SyntaxNode, kind: SyntaxKind, text: &'a str) -> Cow<'a, str> {
  // crlf-in-tokens: rustc reads every CRLF as LF, inside strings and block comments too.
  // A byte loop first: most tokens are a few bytes, and hold no `\r`.
  if text.bytes().any(|b| b == b'\r') && text.contains("\r\n") {
    return token_text(parent, kind, &text.replace("\r\n", "\n"))
      .into_owned()
      .into();
  }
  match kind {
    // range-pattern: `...` is `..=`.
    SyntaxKind::Dot3 if parent.kind() == SyntaxKind::RangePat => "..=".into(),
    SyntaxKind::OuterLineDoc | SyntaxKind::OuterBlockDoc => doc_text("outer", text).into(),
    SyntaxKind::InnerLineDoc | SyntaxKind::InnerBlockDoc => doc_text("inner", text).into(),
    // An integer already in lowercase and with no float suffix is its own canonical text.
    SyntaxKind::IntNumber
      if !text.bytes().any(|b| b.is_ascii_uppercase())
        && (text.starts_with("0x") || !(text.ends_with("f32") || text.ends_with("f64"))) =>
    {
      Cow::Borrowed(text)
    }
    SyntaxKind::IntNumber | SyntaxKind::FloatNumber => number(kind, text).into(),
    // A string with no `\` continues no line.
    SyntaxKind::Str | SyntaxKind::ByteStr | SyntaxKind::CStr if !text.contains('\\') => {
      Cow::Borrowed(text)
    }
    SyntaxKind::Str | SyntaxKind::ByteStr | SyntaxKind::CStr => continued(text).into(),
    _ => Cow::Borrowed(text),
  }
}

/// A `macro_rules!` transcriber's canonical tokens when it reads as items, or else as
/// statements in a function, once its metavariables are renamed; `None` when it reads as
/// neither (or holds a repetition).
fn transcriber<'a>(arm: &SyntaxNode, source: &Source<'_>) -> Option<Vec<Token<'a>>> {
  let whole = arm.text().to_string();
  let body = whole.get(1..whole.len().checked_sub(1)?)?;
  let (renamed, _) = crate::syntax::metavariables::replace_names(body.trim(), source.edition)?;
  let as_items = crate::syntax::parse(&renamed, source.edition);
  let (text, wrapped) = if as_items.errors().is_empty() {
    (renamed, false)
  } else {
    (format!("fn main() {{\n{}\n}}", renamed), true)
  };
  let parsed = crate::syntax::parse(&text, source.edition);
  if !parsed.errors().is_empty() {
    return None;
  }
  // A `;` standing alone in the transcriber itself is part of what it expands to, unlike
  // one inside a block of it (stray-semicolon): compared as written.
  let root = parsed.syntax_node();
  let top = if wrapped {
    root
      .descendant_nodes_where(|kind| kind == SyntaxKind::BlockExpr)
      .next()?
  } else {
    root.clone()
  };
  if top
    .children_with_tokens_iter()
    .any(|e| e.kind() == SyntaxKind::Semi)
  {
    return None;
  }
  let inner = Source::new(&text, source.edition);
  let mut tokens = Vec::new();
  // Its comments are the file's, counted there.
  let mut comments = Vec::new();
  emit_node(&root, &inner, &mut tokens, &mut comments);
  // `fn main ( ) {` and the closing `}` are not the transcriber's.
  let tokens = if wrapped {
    let end = tokens.len().checked_sub(1)?;
    tokens.get(5..end)?.to_vec()
  } else {
    tokens
  };
  let start = arm.text_range().start;
  Some(
    tokens
      .into_iter()
      .map(|t| Token {
        text: Cow::Owned(t.text.into_owned()),
        start,
      })
      .collect(),
  )
}

fn tree(text: &str, edition: Edition) -> (SyntaxNode, usize) {
  let parsed = parse_cached(text, edition);
  let errors = parsed.errors().len();
  (parsed.syntax_node(), errors)
}

/// The node's significant token texts, joined by `separator`.
fn words(node: &SyntaxNode, separator: &str) -> String {
  node
    .token_texts()
    .filter(|(kind, _, _)| !kind.is_trivia())
    .map(|(_, text, _)| text)
    .collect::<Vec<_>>()
    .join(separator)
}

#[cfg(test)]
mod tests;
