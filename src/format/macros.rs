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
  /// The comments on lines of their own before it, `None` for a blank line between them.
  pre: Vec<Option<String>>,
  /// A block comment before it on its line.
  inline: Option<String>,
  /// The comment after it (its `;`) on its line; for the last rule, the one after it before
  /// the `}` (`write_list`'s post-comment).
  post: Option<String>,
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
  let stable = rewrite_definition(&again, indent, edition, config).as_deref() == Some(&laid);
  if !stable {
    super::explain::note("MACRO_RULES", &node.text().to_string(), || {
      "kept as written: its layout would change again on a second run (code in a rule's \
       body kept as written, whose lines keep their columns)"
        .to_string()
    });
  }
  stable.then_some(laid)
}

/// An item standing as a macro call's argument, formatted as a file of its own would be and
/// put at column `indent`: `None` as for [`format_snippet`].
pub(super) fn format_item(text: &str, indent: usize, config: &FormatConfig) -> Option<String> {
  let config = FormatConfig {
    max_width: config.max_width.checked_sub(indent)?,
    ..config.clone()
  };
  let out = format_snippet(text, Edition::E2021, &config)?;
  let out = out.trim_end();
  let continued = in_string(out, Edition::E2021);
  let pad = " ".repeat(indent);
  let lines: Vec<String> = out
    .split('\n')
    .enumerate()
    .map(|(i, line)| {
      if line.is_empty() || continued.get(i).copied().unwrap_or(false) {
        line.to_string()
      } else {
        format!("{}{}", pad, line)
      }
    })
    .collect();
  Some(lines.join("\n"))
}

/// A `lazy_static!` call's braces as the standard formatter's `format_lazy_static` writes
/// them: each `static ref NAME: TYPE = VALUE;`, with its visibility, on a line of its own
/// one level in from `indent`, its value placed as an assignment's is. `None` for a call
/// holding a comment, one not in braces (whose delimiters would change), or what does not
/// read as such entries.
pub(super) fn lazy_static_body(
  call: &SyntaxNode,
  indent: usize,
  config: &FormatConfig,
) -> Option<String> {
  let path = call.children_iter().find(|c| c.kind() == K::Path)?;
  let tree = call.children_iter().find(|c| c.kind() == K::TokenTree)?;
  if path.text() != "lazy_static"
    || tree.first_token()?.kind() != K::LBrace
    || call.subtree_has(|kind| {
      matches!(kind, K::LineComment | K::BlockComment) || super::items::is_doc(kind)
    })
  {
    return None;
  }
  // The entries as `static` items: each `ref` after `static` left out (words, in a token
  // tree).
  let inside: Vec<SyntaxElement> = tree.children_with_tokens_iter().collect();
  let inside = inside.get(1..inside.len().checked_sub(1)?)?;
  let mut source = String::new();
  let mut refs = 0;
  let mut after_static = false;
  for element in inside {
    match element {
      SyntaxElement::Token(t) if t.kind() == K::Whitespace => source.push_str(t.text()),
      SyntaxElement::Token(t) if after_static && t.text() == "ref" => {
        refs += 1;
        after_static = false;
      }
      element => {
        after_static = matches!(element, SyntaxElement::Token(t) if t.text() == "static");
        match element {
          SyntaxElement::Token(t) => source.push_str(t.text()),
          SyntaxElement::Node(n) => source.push_str(&n.text().to_string()),
        }
      }
    }
  }
  let parsed = parse(&source, Edition::E2021);
  if !parsed.errors().is_empty() {
    return None;
  }
  let root = parsed.syntax_node();
  let statics: Vec<SyntaxNode> = root.children_iter().collect();
  if refs == 0
    || statics.len() != refs
    || statics
      .iter()
      .any(|s| s.kind() != K::Static || s.children_iter().any(|c| c.kind() == K::Attr))
  {
    return None;
  }
  let nested = indent + config.tab_spaces;
  let shape = super::stmt::statement_shape(nested, config)?;
  let spacing = shape.limits.spacing;
  let mut entries = Vec::with_capacity(statics.len());
  for item in &statics {
    let visibility = item
      .children_iter()
      .find(|c| c.kind() == K::Visibility)
      .map_or(String::new(), |v| {
        format!("{} ", super::expr::flat(&v, spacing))
      });
    let name = item.children_iter().find(|c| c.kind() == K::Name)?;
    let ty = item
      .children_iter()
      .find(|c| super::types::is_type(c.kind()))?;
    let value = item.children_iter().next_back().filter(|v| v != &ty)?;
    let head = format!(
      "{}static ref {}: {} =",
      visibility,
      super::expr::flat(&name, spacing),
      super::types::rewrite_type(&ty, shape)?
    );
    entries.push(format!(
      "{};",
      super::expr::assign_value(&head, &value, shape.sub_width(1)?)?
    ));
  }
  Some(format!(
    "{{\n{}{}\n{}}}",
    " ".repeat(nested),
    entries.join(&format!("\n{}", " ".repeat(nested))),
    " ".repeat(indent)
  ))
}

/// Notes that a macro call could not be laid out (or was left as written for arguments
/// that do not read as code).
pub(super) fn macro_failed() {
  MACRO_FAILURE.with(|f| f.set(true));
}

/// `f` run with the macro calls it fails to lay out not noted.
pub(super) fn sheltered<T>(f: impl FnOnce() -> T) -> T {
  let before = MACRO_FAILURE.with(|f| f.get());
  let out = f();
  MACRO_FAILURE.with(|f| f.set(before));
  out
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

/// A definition's braces holding only comments, as the standard formatter writes an empty
/// block holding them: one block comment on a line, `{ /* c */ }`, when it fits after the
/// header's `used` columns; else the first comment after `{` on its line when written
/// there, each other on a line of its own one level in, and `}` on a line of its own.
/// `None` when it holds code, nothing, a comment across lines or a blank line.
fn commented_empty_body(
  tree: &SyntaxNode,
  indent: usize,
  used: usize,
  config: &FormatConfig,
) -> Option<String> {
  let tokens: Vec<_> = tree.children_with_tokens_iter().collect();
  let (first, last) = (tokens.first()?, tokens.last()?);
  if first.kind() != K::LBrace || last.kind() != K::RBrace {
    return None;
  }
  let inside = &tokens[1..tokens.len() - 1];
  let mut comments = Vec::new();
  // Whether each comment starts its line.
  let mut on_new_line = false;
  for element in inside {
    let SyntaxElement::Token(t) = element else {
      return None;
    };
    match t.kind() {
      K::Whitespace => {
        let breaks = t.text().matches('\n').count();
        if breaks > 1 {
          return None;
        }
        on_new_line = breaks == 1;
      }
      K::LineComment | K::BlockComment if !t.text().contains('\n') => {
        comments.push((on_new_line, t.text().to_string()));
        on_new_line = false;
      }
      _ => return None,
    }
  }
  if comments.is_empty() {
    return None;
  }
  let width = config.max_width.saturating_sub(indent + used);
  if let [(_, only)] = comments.as_slice()
    && !only.starts_with("//")
    && str_width(only) + 4 <= width
  {
    return Some(format!("{{ {} }}", only));
  }
  let pad = " ".repeat(indent + config.tab_spaces);
  let mut out = String::from("{");
  for (i, (new_line, comment)) in comments.iter().enumerate() {
    if i == 0 && !new_line {
      out.push(' ');
    } else {
      out.push('\n');
      out.push_str(&pad);
    }
    out.push_str(comment.trim_end());
  }
  out.push('\n');
  out.push_str(&" ".repeat(indent));
  out.push('}');
  Some(out)
}

/// Whether formatted code fits as the standard formatter's `filtered_str_fits` measures it:
/// lines of comments aside, the first within `width`, the others within `max_width`, the
/// last within the line from column `indent`.
fn fits(code: &str, max_width: usize, indent: usize, width: usize) -> bool {
  // A line ending inside a comment is not measured (`filter_normal_code` classes a line by
  // its last character).
  let mut in_comment = vec![false; code.split('\n').count()];
  let mut line = 0;
  for (kind, piece) in parse_tokens(code, Edition::E2021) {
    let comment = matches!(kind, K::LineComment | K::BlockComment);
    for _ in piece.matches('\n') {
      if comment {
        in_comment[line] = true;
      }
      line += 1;
    }
    if !piece.trim().is_empty() {
      in_comment[line] = comment;
    }
  }
  let lines: Vec<&str> = code
    .split('\n')
    .zip(&in_comment)
    .filter(|&(l, &comment)| {
      let t = l.trim_start();
      !comment && !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
    })
    .map(|(l, _)| l)
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
  let out = super::file::format_text(text, edition, config, false);
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
  // A comment after its `}` is not the definition's.
  let end = super::imports::code_end(node);
  let elements: Vec<SyntaxElement> = node
    .children_with_tokens_iter()
    .filter(|e| e.text_range().start as usize >= start && (e.text_range().end as usize) <= end)
    .collect();
  if elements
    .iter()
    .any(|e| e.kind().is_trivia() && e.kind() != K::Whitespace)
  {
    super::explain::note("MACRO_RULES", &node.text().to_string(), || {
      "kept as written: a comment in its header".to_string()
    });
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
  let name = name.text().to_string();
  let header = format!("macro_rules! {}", name.trim());
  if let Some(body) = commented_empty_body(tree, indent, str_width(&header) + 1, config) {
    return Some(format!("{} {}", header, body));
  }
  let Some(rules) = rules(tree) else {
    super::explain::note("MACRO_RULES", &node.text().to_string(), || {
      "kept as written: between its braces, something that is not a rule or a comment \
       the layout places (a comment inside a rule's head, or across lines)"
        .to_string()
    });
    return None;
  };
  if rules.is_empty() {
    return Some(format!("{} {{}}", header));
  }
  let tab = config.tab_spaces;
  let arm_indent = indent + tab;
  let arm_width = config.max_width.saturating_sub(arm_indent);
  let mut out = format!("{} {{\n{}", header, " ".repeat(arm_indent));
  for (i, rule) in rules.iter().enumerate() {
    // Each comment on its line at the rules' indentation, which `out` ends with.
    for line in &rule.pre {
      match line {
        Some(comment) => out.push_str(comment),
        None => out.truncate(out.len() - arm_indent),
      }
      out.push('\n');
      out.push_str(&" ".repeat(arm_indent));
    }
    let used = rule.inline.as_ref().map_or(0, |c| str_width(c) + 1);
    if let Some(comment) = &rule.inline {
      out.push_str(comment);
      out.push(' ');
    }
    // Its body indented from the rule's line; the comment narrows its first line.
    out.push_str(&rewrite_rule(
      rule,
      arm_indent,
      arm_width.checked_sub(used)?,
      edition,
      config,
    )?);
    out.push(';');
    if let Some(comment) = &rule.post {
      out.push(' ');
      out.push_str(comment);
    }
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
  let failed = |why: &str| {
    if super::explain::active() {
      let text = format!("{} => {}", rule.matcher.text(), rule.transcriber.text());
      super::explain::note("MACRO_RULES", &text, || {
        format!("kept as written: this rule's {}", why)
      });
    }
    None
  };
  if rule.matcher.first_token()?.kind() != K::LParen {
    return failed("matcher is not in parentheses");
  }
  let tab = config.tab_spaces;
  let whole = rule.transcriber.text().to_string();
  let Some(old_body) = transcriber_body(&whole).map(str::trim) else {
    return failed("body is not in braces");
  };
  let block_body = old_body.starts_with('{');
  let prefix_width = if block_body && config.style_edition >= 2024 {
    6
  } else {
    5
  };
  if width < prefix_width {
    return failed("head leaves no room on its line");
  }
  let mut out = rule.matcher.text().to_string();
  out.push_str(" =>");
  let Some((body, names)) = replace_names(old_body, edition) else {
    return failed("body's `$` names cannot be stood in for (`$$`, or one inside a string)");
  };
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
      match format_code_block(&body, edition, &inner) {
        Some(formatted) => formatted,
        None => {
          return failed(
            "body does not format as items or statements (a macro in it not laid out, \
             or code that does not parse)",
          );
        }
      }
    }
  };
  if !fits(&formatted, inner.max_width, indent, width) {
    return failed("body is wider than max_width once laid out");
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
      return failed("body already holds a name standing in for a `$` name");
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

/// A definition's rules, each `(matcher) => {transcriber}` with an optional `;`, and the
/// comments around them (`rewrite_macro_def` reads the rules as a list); `None` for
/// anything else between its braces, a comment inside a rule's head or a block comment
/// across lines too.
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
  // Comments waiting for the rule after them, and whether a line ended since the last
  // rule or comment.
  let mut pending: Vec<Option<String>> = Vec::new();
  let mut inline: Option<String> = None;
  let mut broke = true;
  for element in &elements[1..elements.len() - 1] {
    match element {
      SyntaxElement::Token(t) if t.kind() == K::Whitespace => {
        let lines = t.text().matches('\n').count();
        if lines > 0 {
          broke = true;
          // A comment on its line before a rule is not inline; one before a blank line
          // keeps it.
          if let Some(comment) = inline.take() {
            pending.push(Some(comment));
          }
          if lines > 1 && !pending.is_empty() {
            pending.push(None);
          }
        }
        if !rules.is_empty()
          && matcher.is_none()
          && pending.is_empty()
          && inline.is_none()
          && lines > 1
        {
          rules.last_mut()?.blank_after = true;
        }
      }
      SyntaxElement::Token(t) if matches!(t.kind(), K::LineComment | K::BlockComment) => {
        let text = t.text().trim_end().to_string();
        if matcher.is_some() || arrow || eq || text.contains('\n') {
          return None;
        }
        match rules.last_mut() {
          // After a rule on its line: its comment.
          Some(rule) if !broke && pending.is_empty() && inline.is_none() => {
            if rule.post.is_some() {
              return None;
            }
            rule.post = Some(text);
          }
          _ if t.kind() == K::BlockComment && broke => inline = Some(text),
          _ => {
            if let Some(comment) = inline.take() {
              pending.push(Some(comment));
            }
            pending.push(Some(text));
          }
        }
        broke = false;
        ended = false;
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
          // Blank lines between the comments and the rule go.
          while pending.last().is_some_and(Option::is_none) {
            pending.pop();
          }
          rules.push(Rule {
            matcher: matcher.take()?,
            transcriber: n.clone(),
            blank_after: false,
            pre: std::mem::take(&mut pending),
            inline: inline.take(),
            post: None,
          });
          arrow = false;
          ended = true;
          broke = false;
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
  // Comments after the last rule: one is its comment, on its line.
  if let Some(comment) = inline.take() {
    pending.push(Some(comment));
  }
  pending.retain(Option::is_some);
  match (pending.as_slice(), rules.last_mut()) {
    ([], _) => {}
    ([Some(comment)], Some(rule)) if rule.post.is_none() => rule.post = Some(comment.clone()),
    _ => return None,
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
