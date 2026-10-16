//! Declarative macro definitions (`macro_rules! name { .. }`) as the standard formatter's
//! `rewrite_macro_def` writes them: the header on one line, then each rule on lines of its
//! own one level in, its matcher as written, ` => {`, its transcriber formatted as code and
//! `};`. A transcriber is formatted with its metavariables renamed (`$name` to `zname`, of
//! the same width) as items, else as statements in a function, at the width left inside
//! the rule; then the names are put back. When any rule cannot be formatted (a matcher in
//! brackets or braces, a repetition `$(..)` in the transcriber, code that does not parse,
//! a macro call in it that cannot be laid out, a line too long), the whole definition is
//! left as written. Matchers are left as written, as `format_macro_matchers` is off.

use std::cell::Cell;

use crate::options::format::FormatConfig;
use crate::syntax::metavariables::replace_names;
use crate::syntax::{Edition, SyntaxElement, SyntaxKind as K, SyntaxNode, parse, parse_tokens};
use crate::text::unicode::str_width;

thread_local! {
  /// Whether a macro call could not be laid out since this was last cleared: the standard
  /// formatter's `macro_rewrite_failure`, which makes the transcriber it is in fail.
  static MACRO_FAILURE: Cell<bool> = const { Cell::new(false) };
}

/// One rule of a definition: its matcher, its transcriber, and whether a blank line
/// follows it.
struct Rule {
  matcher: SyntaxNode,
  transcriber: SyntaxNode,
  blank_after: bool,
}

/// A `macro_rules!` definition from its `macro_rules` on (its attributes are laid out on
/// their own), when its header starts a line at column `indent`; `None` to leave it as
/// written.
pub fn rewrite_macro_rules(
  node: &SyntaxNode,
  indent: usize,
  edition: Edition,
  config: &FormatConfig,
) -> Option<String> {
  let laid = rewrite_definition(node, indent, edition, config)?;
  // Code in a transcriber left as written keeps its indentation from the file, which the
  // rule's indentation then adds to: only a layout a second run keeps is taken.
  let again = parse(&laid, edition).syntax_node();
  let again = again
    .descendant_nodes_where(|kind| kind == K::MacroRules)
    .next()?;
  (rewrite_definition(&again, indent, edition, config)? == laid).then_some(laid)
}

/// Notes that a macro call could not be laid out (or was left as written for arguments
/// that do not read as code).
pub(super) fn macro_failed() {
  MACRO_FAILURE.with(|f| f.set(true));
}

/// `config` with the line width `max_width`, the widths that follow it from
/// `use_small_heuristics` recomputed and any set apart moved by as much.
fn adjusted(config: &FormatConfig, max_width: usize) -> FormatConfig {
  let old = config.max_width;
  let delta = max_width as isize - old as isize;
  let scaled = |max: usize, percent: usize| {
    let tenths = if max > 100 { (max * 10 + 50) / 100 } else { 10 };
    (percent * tenths + 5) / 10
  };
  let width = |w: usize, percent: usize| {
    if w == usize::MAX {
      w
    } else if w == scaled(old, percent) {
      scaled(max_width, percent)
    } else if w == old {
      max_width
    } else {
      w.saturating_add_signed(delta)
    }
  };
  let mut new = config.clone();
  new.max_width = max_width;
  new.fn_call_width = width(config.fn_call_width, 60);
  new.chain_width = width(config.chain_width, 60);
  new.struct_lit_width = width(config.struct_lit_width, 18);
  new.struct_variant_width = width(config.struct_variant_width, 35);
  new.array_width = width(config.array_width, 60);
  new.attr_fn_like_width = width(config.attr_fn_like_width, 70);
  new.single_line_if_else_max_width = width(config.single_line_if_else_max_width, 50);
  new.single_line_let_else_max_width = width(config.single_line_let_else_max_width, 50);
  new
}

/// Whether formatted code fits as the standard formatter's `filtered_str_fits` measures it:
/// lines of comments aside, the first within `width`, the others within `max_width`, the
/// last within the line from column `indent`.
fn fits(code: &str, max_width: usize, indent: usize, width: usize) -> bool {
  let lines: Vec<&str> = code
    .split('\n')
    .filter(|l| {
      let t = l.trim_start();
      !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
    })
    .collect();
  let code = lines.join("\n");
  let code = code.strip_suffix('\n').unwrap_or(&code);
  if code.is_empty() {
    return true;
  }
  let mut lines = code.split('\n');
  if str_width(lines.next().unwrap_or("")) > width {
    return false;
  }
  if !code.contains('\n') {
    return true;
  }
  if code.split('\n').skip(1).any(|l| str_width(l) > max_width) {
    return false;
  }
  str_width(code.rsplit('\n').next().unwrap_or("")) <= indent + width
}

/// Statements formatted inside `fn main() { .. }`, then taken out of it one level less
/// indented, as the standard formatter's `format_code_block` does; `None` when they do not
/// format, or a line is longer than the line width.
fn format_code_block(text: &str, edition: Edition, config: &FormatConfig) -> Option<String> {
  const PREFIX: &str = "fn main() {\n";
  let tab = config.tab_spaces;
  let indent_str = " ".repeat(tab);
  let mut wrapped = String::from(PREFIX);
  let continued = in_string(text, edition);
  for (i, line) in text.split('\n').enumerate() {
    if !continued[i] {
      wrapped.push_str(&indent_str);
    }
    wrapped.push_str(line);
    wrapped.push('\n');
  }
  wrapped.push('}');
  let formatted = format_snippet(&wrapped, edition, config)?;
  let block_len = formatted.rfind('}').map_or(formatted.len(), |i| {
    formatted[..i]
      .strip_suffix(char::is_whitespace)
      .map_or(i, str::len)
  });
  let block_start = PREFIX.len().min(block_len);
  let inside = &formatted[block_start..block_len];
  let continued = in_string(inside, edition);
  let mut out = String::with_capacity(inside.len());
  for (i, line) in inside.split('\n').enumerate() {
    if i > 0 {
      out.push('\n');
    }
    let line = if continued[i] {
      line
    } else if line.len() > config.max_width {
      return None;
    } else if line.len() > indent_str.len() && line.starts_with(&indent_str) {
      &line[tab..]
    } else {
      line
    };
    out.push_str(line);
  }
  Some(out)
}

/// Code formatted as a file of its own, as the standard formatter's `format_snippet`
/// does for a macro definition: `None` when it does not parse, when a macro call in it
/// could not be laid out, or when a line keeps trailing whitespace.
fn format_snippet(text: &str, edition: Edition, config: &FormatConfig) -> Option<String> {
  let parsed = parse(text, edition);
  if !parsed.errors().is_empty() || untyped_parameter(&parsed.syntax_node(), edition) {
    return None;
  }
  let before = MACRO_FAILURE.with(|f| f.replace(false));
  let out = super::file::format_file(text, edition, config);
  let failed = MACRO_FAILURE.with(|f| f.replace(before));
  if failed || out.is_empty() && !text.is_empty() {
    return None;
  }
  let continued = in_string(&out, edition);
  let lines: Vec<&str> = out.split('\n').collect();
  let trailing = lines.iter().enumerate().any(|(i, line)| {
    let ends_in_string = continued.get(i + 1).copied().unwrap_or(false);
    !ends_in_string && line.ends_with([' ', '\t'])
  });
  (!trailing).then_some(out)
}

/// For each line of `text`, whether it starts inside a string literal.
fn in_string(text: &str, edition: Edition) -> Vec<bool> {
  let mut starts = vec![false];
  for (kind, piece) in parse_tokens(text, edition) {
    let literal = kind.is_literal();
    for _ in piece.matches('\n') {
      starts.push(literal);
    }
  }
  starts
}

fn rewrite_definition(
  node: &SyntaxNode,
  indent: usize,
  edition: Edition,
  config: &FormatConfig,
) -> Option<String> {
  if node.kind() != K::MacroRules || config.hard_tabs || config.tab_spaces == 0 {
    return None;
  }
  let start = super::items::header_start(node)?;
  let elements: Vec<SyntaxElement> = node
    .children_with_tokens_iter()
    .filter(|e| e.text_range().start as usize >= start)
    .collect();
  if elements
    .iter()
    .any(|e| e.kind().is_trivia() && e.kind() != K::Whitespace)
  {
    return None;
  }
  let code: Vec<&SyntaxElement> = elements.iter().filter(|e| !e.kind().is_trivia()).collect();
  let [
    keyword,
    bang,
    SyntaxElement::Node(name),
    SyntaxElement::Node(tree),
  ] = code.as_slice()
  else {
    return None;
  };
  if keyword.kind() != K::MacroRulesKw || bang.kind() != K::Bang || name.kind() != K::Name {
    return None;
  }
  let rules = rules(tree)?;
  let name = name.text().to_string();
  let header = format!("macro_rules! {}", name.trim());
  if rules.is_empty() {
    return Some(format!("{} {{}}", header));
  }
  let tab = config.tab_spaces;
  let arm_indent = indent + tab;
  let arm_width = config.max_width.saturating_sub(arm_indent);
  let mut out = format!("{} {{\n{}", header, " ".repeat(arm_indent));
  for (i, rule) in rules.iter().enumerate() {
    out.push_str(&rewrite_rule(rule, arm_indent, arm_width, edition, config)?);
    out.push(';');
    if i + 1 < rules.len() {
      out.push('\n');
      if rule.blank_after {
        out.push('\n');
      }
      out.push_str(&" ".repeat(arm_indent));
    }
  }
  out.push('\n');
  out.push_str(&" ".repeat(indent));
  out.push('}');
  Some(out)
}

/// One rule from its matcher to its transcriber's `}`, at column `indent` with `width`
/// left on its first line.
fn rewrite_rule(
  rule: &Rule,
  indent: usize,
  width: usize,
  edition: Edition,
  config: &FormatConfig,
) -> Option<String> {
  // Only rules whose matcher is in parentheses are formatted.
  if rule.matcher.first_token()?.kind() != K::LParen {
    return None;
  }
  let tab = config.tab_spaces;
  let whole = rule.transcriber.text().to_string();
  let old_body = transcriber_body(&whole)?.trim();
  let block_body = old_body.starts_with('{');
  let prefix_width = if block_body && config.style_edition >= 2024 {
    6
  } else {
    5
  };
  width.checked_sub(prefix_width)?;
  let mut out = rule.matcher.text().to_string();
  out.push_str(" =>");
  let (body, names) = replace_names(old_body, edition)?;
  out.push_str(" {");
  let body_indent = if block_body { indent } else { indent + tab };
  let mut inner = adjusted(config, config.max_width.saturating_sub(body_indent));
  // No `;` is added after a last `return`, `break` or `continue` in a macro definition:
  // used where an expression goes, it would not compile.
  inner.trailing_semicolon = false;
  let formatted = match format_snippet(&body, edition, &inner) {
    Some(formatted) => formatted,
    None => {
      inner = adjusted(&inner, inner.max_width + tab);
      format_code_block(&body, edition, &inner)?
    }
  };
  if !fits(&formatted, inner.max_width, indent, width) {
    return None;
  }
  // Indented as a block's insides, but for lines inside a string.
  let continued = in_string(formatted.trim_end(), edition);
  let indent_str = " ".repeat(body_indent);
  let mut new_body = String::new();
  for (i, line) in formatted.trim_end().lines().enumerate() {
    if !line.trim().is_empty() && !continued[i] {
      new_body.push_str(&indent_str);
    }
    new_body.push_str(line);
    new_body.push('\n');
  }
  for (old, new) in &names {
    if old_body.contains(new.as_str()) {
      return None;
    }
    new_body = new_body.replace(new.as_str(), old);
  }
  if block_body {
    out.push_str(new_body.trim());
  } else if !new_body.is_empty() {
    out.push('\n');
    out.push_str(&new_body);
    out.push_str(&" ".repeat(indent));
  }
  out.push('}');
  Some(out)
}

/// A definition's rules, each `(matcher) => {transcriber}` with an optional `;`; `None`
/// for anything else between its braces, a comment among them too.
fn rules(tree: &SyntaxNode) -> Option<Vec<Rule>> {
  let elements: Vec<SyntaxElement> = tree.children_with_tokens_iter().collect();
  let (first, last) = (elements.first()?, elements.last()?);
  if first.kind() != K::LBrace || last.kind() != K::RBrace || elements.len() < 2 {
    return None;
  }
  let mut rules: Vec<Rule> = Vec::new();
  let mut matcher: Option<SyntaxNode> = None;
  let mut arrow = false;
  let mut eq = false;
  let mut eq_end = 0u32;
  let mut ended = false;
  for element in &elements[1..elements.len() - 1] {
    match element {
      SyntaxElement::Token(t) if t.kind() == K::Whitespace => {
        if !rules.is_empty() && matcher.is_none() && t.text().matches('\n').count() > 1 {
          rules.last_mut()?.blank_after = true;
        }
      }
      SyntaxElement::Token(t) if t.kind() == K::FatArrow && matcher.is_some() && !arrow => {
        arrow = true;
      }
      // In a token tree `=>` is `=` and `>`, touching.
      SyntaxElement::Token(t) if t.kind() == K::Eq && matcher.is_some() && !arrow && !eq => {
        eq = true;
        eq_end = t.text_range().end;
      }
      SyntaxElement::Token(t) if t.kind() == K::Gt && eq && t.text_range().start == eq_end => {
        eq = false;
        arrow = true;
      }
      SyntaxElement::Token(t) if t.kind() == K::Semi && ended => {
        ended = false;
      }
      SyntaxElement::Node(n) if n.kind() == K::TokenTree => {
        if eq {
          return None;
        }
        if arrow {
          rules.push(Rule {
            matcher: matcher.take()?,
            transcriber: n.clone(),
            blank_after: false,
          });
          arrow = false;
          ended = true;
        } else if matcher.is_none() {
          matcher = Some(n.clone());
          ended = false;
        } else {
          return None;
        }
      }
      _ => return None,
    }
  }
  if matcher.is_some() {
    return None;
  }
  Some(rules)
}

/// A transcriber's text between its delimiters.
fn transcriber_body(whole: &str) -> Option<&str> {
  let first = whole.chars().next()?;
  let last = whole.chars().next_back()?;
  (matches!(first, '(' | '[' | '{') && matches!(last, ')' | ']' | '}') && whole.len() >= 2)
    .then(|| &whole[1..whole.len() - 1])
}

/// Whether a function has a parameter with no type (`fn f(&x)`), which rustc does not
/// parse (but in a trait, before the 2018 edition), though this parser takes it.
fn untyped_parameter(root: &SyntaxNode, edition: Edition) -> bool {
  edition != Edition::E2015
    && root
      .descendant_nodes_where(|kind| kind == K::Param)
      .any(|param| {
        param
          .parent()
          .and_then(|list| list.parent())
          .is_some_and(|f| f.kind() == K::Fn)
          && !param
            .children_with_tokens_iter()
            .any(|e| matches!(e.kind(), K::Colon | K::Dot3))
      })
}

#[cfg(test)]
mod tests;
