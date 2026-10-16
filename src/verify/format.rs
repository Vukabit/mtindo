//! The meaning check for formatted output: the output's tokens equal the input's,
//! apart from whitespace, comments and the allowed rewrites ([`super::rewrites`]), and every
//! comment survives.
//!
//! Both texts are parsed and walked into a canonical token stream: whitespace and comments
//! dropped, doc comments kept (they are attributes) with each line trimmed, and every
//! allowed rewrite resolved to one form. The streams must be equal. Comments are compared as
//! a multiset, each with its lines trimmed, since formatting may re-indent a block comment
//! and move a comment across a comma.

use std::borrow::Cow;

use super::{Rule, Violation, describe_list, difference};
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, TextRange, parse_cached};

/// Marks the start of one imported path in a `use`'s canonical tokens.
const USE_ENTRY: &str = "\u{0}use";

/// Line numbers by byte offset, and the text they index: the text of a tree parsed from
/// it, whose tokens' texts are slices of it.
struct LineIndex<'a> {
  starts: Vec<usize>,
  text: &'a str,
  edition: Edition,
}

impl<'a> LineIndex<'a> {
  fn new(text: &'a str, edition: Edition) -> LineIndex<'a> {
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    LineIndex {
      starts,
      text,
      edition,
    }
  }

  fn line(&self, offset: usize) -> usize {
    self.starts.partition_point(|&start| start <= offset)
  }

  /// The text at `range`, borrowed for as long as the text is.
  fn at(&self, range: TextRange) -> &'a str {
    &self.text[range.start as usize..range.end as usize]
  }
}

/// One canonical token, with the line it came from (1-based) for reports. Its text is
/// borrowed from the source unless a rule rewrote it.
#[derive(Clone, Debug)]
struct Token<'a> {
  text: Cow<'a, str>,
  line: usize,
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
  if let Some(detail) = first_difference(&old_tokens, &new_tokens) {
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
  let lines = LineIndex::new(text, edition);
  let mut tokens = Vec::new();
  let mut comments = Vec::new();
  emit_node(root, &lines, &mut tokens, &mut comments);
  // doc-text: consecutive doc comments of one kind are one text.
  let mut merged: Vec<Token> = Vec::with_capacity(tokens.len());
  for token in tokens {
    if let Some(last) = merged.last_mut()
      && let (Some((kind, more)), Some((last_kind, _))) = (
        token.text.split_once('\u{0}'),
        last.text.split_once('\u{0}'),
      )
      && kind == last_kind
    {
      last.text.to_mut().push_str(more);
      continue;
    }
    merged.push(token);
  }
  // comment-text: comments as the multiset of their words.
  let mut words: Vec<String> = comments
    .iter()
    .flat_map(|c| c.split_whitespace())
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
fn derive_arguments<'a>(attr: &SyntaxNode, lines: &LineIndex<'a>) -> Option<Vec<Token<'a>>> {
  let tokens: Vec<(Cow<'a, str>, usize)> = attr
    .descendants_with_tokens_iter()
    .filter_map(|e| match e {
      // Each token's canonical text, as everywhere: a doc comment written in the list
      // (`/// x`, which keeps a `\r` in a CRLF file) is its text.
      SyntaxElement::Token(t) if !t.kind().is_trivia() => Some((
        token_text(&t.parent(), t.kind(), lines.at(t.text_range())),
        lines.line(t.text_range().start as usize),
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
      .map(|(text, line)| Token {
        text: text.clone(),
        line: *line,
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
  lines: &LineIndex<'a>,
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
        return emit_node(&expr, lines, out, notes);
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
            return emit_node(expr, lines, out, notes);
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
          return emit_node(expr, lines, out, notes);
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
          line: lines.line(node.text_range().start as usize),
        });
        return;
      }
      if node.kind() == SyntaxKind::Attr && merged_derive(node, lines, out) {
        return;
      }
      emit_node(node, lines, out, notes);
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
      let after_item = previous_significant(element).is_some_and(|k| {
        !matches!(
          k,
          SyntaxKind::LParen | SyntaxKind::LBrack | SyntaxKind::LBrace | SyntaxKind::Lt
        )
      });
      if kind == SyntaxKind::Comma
        && !single_tuple(parent)
        && (parent.kind() == SyntaxKind::MatchArm
          || after_item && next.is_none_or(is_closer)
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
      out.push(Token {
        text: token_text(parent, kind, lines.at(token.text_range())),
        line: lines.line(token.text_range().start as usize),
      });
    }
  }
}

/// The node's significant children, after the rewrites that act on a whole node.
fn emit_node<'a>(
  node: &SyntaxNode,
  lines: &LineIndex<'a>,
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
  match node.kind() {
    SyntaxKind::ParenExpr => {
      // nested-parens: `((a))` is `(a)`.
      let inner: Vec<SyntaxNode> = node.children();
      // number-literals: parentheses around a literal (a float receiver) group nothing.
      if let [only] = inner.as_slice()
        && matches!(only.kind(), SyntaxKind::ParenExpr | SyntaxKind::Literal)
      {
        return emit_node(only, lines, out, notes);
      }
    }
    SyntaxKind::Use => return emit_use(node, lines, out),
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
          emit_element(node, &element, lines, out, notes);
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
        emit_node(&child, lines, out, notes);
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
        line: lines.line(node.text_range().start as usize),
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
          emit_element(node, attr, lines, out, notes);
        }
        out.push(Token {
          text: words(name, "").into(),
          line: lines.line(node.text_range().start as usize),
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
          emit_element(node, element, lines, out, notes);
        }
        out.push(Token {
          text: "..".into(),
          line: lines.line(node.text_range().start as usize),
        });
        if let Some(close) = children.last() {
          emit_element(node, close, lines, out, notes);
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
      if let Some(tokens) = macro_arguments(node, lines, true) {
        out.extend(tokens);
        return;
      }
      return emit_tree(node, "[", "]", lines, out, notes);
    }
    // macro-arguments: a macro call's arguments in parentheses or brackets that read as
    // expressions, as those expressions.
    SyntaxKind::TokenTree
      if node
        .parent()
        .is_some_and(|p| p.kind() == SyntaxKind::MacroCall)
        && let Some(tokens) = macro_arguments(node, lines, false) =>
    {
      out.extend(tokens);
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
      let line = lines.line(node.text_range().start as usize);
      out.push(Token {
        text: "{".into(),
        line,
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
            let code = (!matcher).then(|| transcriber(arm, lines)).flatten();
            match code {
              Some(tokens) => {
                out.push(Token {
                  text: "{".into(),
                  line,
                });
                out.extend(tokens);
                out.push(Token {
                  text: "}".into(),
                  line,
                });
              }
              None => emit_tree(arm, "{", "}", lines, out, notes),
            }
            matcher = !matcher;
          }
          SyntaxElement::Token(t) if t.kind() == SyntaxKind::Semi => {}
          other => emit_element(node, other, lines, out, notes),
        }
      }
      out.push(Token {
        text: "}".into(),
        line,
      });
      return;
    }
    SyntaxKind::MacroCall if words(node, "").starts_with("try!(") => {
      // try-shorthand: `try!(e)` is `e?`.
      if let Some(tree) = node
        .children_iter()
        .find(|c| c.kind() == SyntaxKind::TokenTree)
      {
        let inner: Vec<SyntaxElement> = tree
          .children_with_tokens_iter()
          .filter(significant)
          .collect();
        if inner.len() >= 2 {
          for element in &inner[1..inner.len() - 1] {
            emit_element(&tree, element, lines, out, notes);
          }
          out.push(Token {
            text: "?".into(),
            line: lines.line(node.text_range().start as usize),
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
      if let ["pub", "(", "in", target @ ("crate" | "super" | "self"), ")"] = words.as_slice() {
        let line = lines.line(node.text_range().start as usize);
        for text in ["pub", "(", target, ")"] {
          out.push(Token {
            text: text.to_string().into(),
            line,
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
        let line = lines.line(node.text_range().start as usize);
        out.push(Token {
          text: "extern".into(),
          line,
        });
        out.push(Token {
          text: "\"C\"".into(),
          line,
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
              let line = lines.line(t.text_range().start as usize);
              out.push(Token {
                text: "extern".into(),
                line,
              });
              out.push(Token {
                text: "\"C\"".into(),
                line,
              });
            }
            other => emit_element(node, other, lines, out, notes),
          }
        }
        return;
      }
    }
    _ => {}
  }
  let children: Vec<SyntaxElement> = node
    .children_with_tokens_iter()
    .filter(significant)
    .collect();
  let container = matches!(
    node.kind(),
    SyntaxKind::SourceFile | SyntaxKind::ItemList | SyntaxKind::BlockExpr
  );
  // impl-item-order: an impl's items compare in sorted order.
  if node.kind() == SyntaxKind::AssocItemList
    && node.parent().is_some_and(|p| p.kind() == SyntaxKind::Impl)
  {
    let mut items: Vec<Vec<Token>> = Vec::new();
    for element in &children {
      if let SyntaxElement::Node(_) = element {
        let mut item = Vec::new();
        emit_element(node, element, lines, &mut item, notes);
        items.push(item);
      } else {
        emit_element(node, element, lines, out, notes);
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
        emit_element(node, element, lines, &mut item, notes);
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
    emit_element(node, element, lines, out, notes);
    i += 1;
  }
}

/// A token tree with its delimiters replaced by `open` and `close`.
fn emit_tree<'a>(
  tree: &SyntaxNode,
  open: &'a str,
  close: &'a str,
  lines: &LineIndex<'a>,
  out: &mut Vec<Token<'a>>,
  notes: &mut Vec<String>,
) {
  let children: Vec<SyntaxElement> = tree
    .children_with_tokens_iter()
    .filter(significant)
    .collect();
  let line = lines.line(tree.text_range().start as usize);
  out.push(Token {
    text: open.into(),
    line,
  });
  for element in children
    .iter()
    .skip(1)
    .take(children.len().saturating_sub(2))
  {
    emit_element(tree, element, lines, out, notes);
  }
  out.push(Token {
    text: close.into(),
    line,
  });
}

/// use-trees: a `use` as one entry per imported path, each with the `use`'s attributes and
/// visibility, sorted.
fn emit_use<'a>(node: &SyntaxNode, lines: &LineIndex<'a>, out: &mut Vec<Token<'a>>) {
  let line = lines.line(node.text_range().start as usize);
  let mut head = Vec::new();
  let mut paths = Vec::new();
  for child in node.children_with_tokens_iter().filter(significant) {
    match &child {
      SyntaxElement::Node(n) if n.kind() == SyntaxKind::UseTree => {
        flatten_tree(n, "", &mut paths);
      }
      SyntaxElement::Node(n) => {
        let mut notes = Vec::new();
        emit_node(n, lines, &mut head, &mut notes);
      }
      SyntaxElement::Token(t) if t.kind() == SyntaxKind::Semi => {}
      SyntaxElement::Token(t) => head.push(Token {
        text: token_text(node, t.kind(), lines.at(t.text_range())),
        line,
      }),
    }
  }
  paths.sort();
  paths.dedup();
  // `split_use` cuts the entries apart again, for use-trees to compare a run as a set.
  for path in paths {
    out.push(Token {
      text: USE_ENTRY.into(),
      line,
    });
    out.extend(head.iter().cloned());
    out.push(Token {
      text: path.into(),
      line,
    });
  }
}

/// Whether a statement is the last thing in its block, with no tail expression after it.
fn ends_block(statement: &SyntaxNode) -> bool {
  let mut next = statement.next_sibling_or_token();
  while let Some(element) = next {
    if significant(&element) {
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
fn first_difference(old: &[Token], new: &[Token]) -> Option<String> {
  let at = old
    .iter()
    .zip(new)
    .position(|(a, b)| a.text != b.text)
    .or_else(|| (old.len() != new.len()).then(|| old.len().min(new.len())))?;
  let around = |tokens: &[Token]| -> String {
    let start = at.saturating_sub(4);
    let end = (at + 5).min(tokens.len());
    let line = tokens.get(at).or(tokens.last()).map_or(0, |t| t.line);
    let shown: Vec<&str> = tokens[start..end].iter().map(|t| t.text.as_ref()).collect();
    format!("line {}: `{}`", line, shown.join(" "))
  };
  Some(format!(
    "the tokens differ: input {}, output {}",
    around(old),
    around(new)
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
  let mut entry = if glob { join(&full, "*") } else { full };
  if !rename.is_empty() {
    entry = format!("{} {}", entry, rename);
  }
  paths.push(entry);
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
  lines: &LineIndex<'_>,
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
  let parsed = crate::syntax::parse(&text, lines.edition);
  if !parsed.errors().is_empty() {
    return None;
  }
  let root = parsed.syntax_node();
  let list = root.descendant_nodes_where(|k| k == kind).next()?;
  let inner_lines = LineIndex::new(&text, lines.edition);
  let mut tokens = Vec::new();
  // Their comments are the file's, counted there.
  let mut comments = Vec::new();
  emit_node(&list, &inner_lines, &mut tokens, &mut comments);
  let line = lines.line(tree.text_range().start as usize);
  Some(
    tokens
      .into_iter()
      .map(|t| Token {
        text: Cow::Owned(t.text.into_owned()),
        line,
      })
      .collect(),
  )
}

/// merge-derives: the first of consecutive `#[derive(...)]` attributes stands for all of
/// them (their traits, in order, as one list); the others emit nothing. Returns whether
/// `attr` was handled here.
fn merged_derive<'a>(attr: &SyntaxNode, lines: &LineIndex<'a>, out: &mut Vec<Token<'a>>) -> bool {
  let Some(own) = derive_arguments(attr, lines) else {
    return false;
  };
  let previous_is_derive = match previous_significant_node(attr) {
    Some(previous) => {
      previous.kind() == SyntaxKind::Attr && derive_arguments(&previous, lines).is_some()
    }
    None => false,
  };
  if previous_is_derive {
    return true;
  }
  let line = lines.line(attr.text_range().start as usize);
  let mut traits = own;
  let mut next = next_significant_node(attr);
  while let Some(node) = next {
    match (node.kind() == SyntaxKind::Attr)
      .then(|| derive_arguments(&node, lines))
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
      line,
    });
  }
  // Traits are compared as one list without their commas, which is enough to tell
  // `#[derive(A)] #[derive(B)]` and `#[derive(A, B)]` equal and any other list apart.
  out.extend(traits);
  for text in [")", "]"] {
    out.push(Token {
      text: text.into(),
      line,
    });
  }
  true
}

fn next_significant(element: &SyntaxElement) -> Option<SyntaxKind> {
  let mut next = match element {
    SyntaxElement::Token(t) => t.next_sibling_or_token(),
    SyntaxElement::Node(n) => n.next_sibling_or_token(),
  };
  while let Some(e) = next {
    if significant(&e) {
      return Some(kind_of(&e));
    }
    next = match &e {
      SyntaxElement::Token(t) => t.next_sibling_or_token(),
      SyntaxElement::Node(n) => n.next_sibling_or_token(),
    };
  }
  None
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
  let mut previous = match element {
    SyntaxElement::Token(t) => t.prev_sibling_or_token(),
    SyntaxElement::Node(n) => n.prev_sibling_or_token(),
  };
  while let Some(e) = previous {
    if significant(&e) {
      return Some(kind_of(&e));
    }
    previous = match &e {
      SyntaxElement::Token(t) => t.prev_sibling_or_token(),
      SyntaxElement::Node(n) => n.prev_sibling_or_token(),
    };
  }
  None
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
  let inner: Vec<SyntaxElement> = block
    .children_with_tokens_iter()
    .filter(significant)
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

/// A token's canonical text: doc comments as their text, numbers normalized, `...` in a
/// range pattern as `..=`.
fn token_text<'a>(parent: &SyntaxNode, kind: SyntaxKind, text: &'a str) -> Cow<'a, str> {
  match kind {
    // range-pattern: `...` is `..=`.
    SyntaxKind::Dot3 if parent.kind() == SyntaxKind::RangePat => "..=".into(),
    SyntaxKind::OuterLineDoc | SyntaxKind::OuterBlockDoc => doc_text("outer", text).into(),
    SyntaxKind::InnerLineDoc | SyntaxKind::InnerBlockDoc => doc_text("inner", text).into(),
    SyntaxKind::IntNumber | SyntaxKind::FloatNumber => number(kind, text).into(),
    SyntaxKind::Str | SyntaxKind::ByteStr | SyntaxKind::CStr => continued(text).into(),
    _ => Cow::Borrowed(text),
  }
}

/// A `macro_rules!` transcriber's canonical tokens when it reads as items, or else as
/// statements in a function, once its metavariables are renamed; `None` when it reads as
/// neither (or holds a repetition).
fn transcriber<'a>(arm: &SyntaxNode, lines: &LineIndex<'_>) -> Option<Vec<Token<'a>>> {
  let whole = arm.text().to_string();
  let body = whole.get(1..whole.len().checked_sub(1)?)?;
  let (renamed, _) = crate::syntax::metavariables::replace_names(body.trim(), lines.edition)?;
  let as_items = crate::syntax::parse(&renamed, lines.edition);
  let (text, wrapped) = if as_items.errors().is_empty() {
    (renamed, false)
  } else {
    (format!("fn main() {{\n{}\n}}", renamed), true)
  };
  let parsed = crate::syntax::parse(&text, lines.edition);
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
  let inner = LineIndex::new(&text, lines.edition);
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
  let line = lines.line(arm.text_range().start as usize);
  Some(
    tokens
      .into_iter()
      .map(|t| Token {
        text: Cow::Owned(t.text.into_owned()),
        line,
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
