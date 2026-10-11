//! The whole-file rules: what formatting does before expressions are laid out.
//!
//! Only the whitespace between tokens changes, so a program means what it meant. For each
//! line break the formatter decides how many blank lines stay and how far the next line is
//! indented:
//!
//! - **Lines that start something in a container** (an item, statement, field, variant,
//!   match arm, list element, or a comment among them) are indented one level past the line
//!   the container's owner starts on: the closure, `if`, match arm or item a block belongs
//!   to, the call an argument list belongs to (for a method call, the line of its `.`). A
//!   line starting with the container's closing delimiter is indented as that line.
//!   Containers are blocks, item and field lists, match arms, and delimited lists
//!   (arguments, parameters, arrays, tuples, struct literals, use lists).
//! - **Other lines** continue the construct above them and move by as much as it moved, so a
//!   multi-line expression keeps its shape. A comment just before a closing delimiter moves
//!   with its container, and a line comment written under a comment that ends a line of
//!   code stays under it.
//! - Trailing whitespace is trimmed after code and line comments (doc comments keep
//!   theirs), blank lines are kept within `blank_lines_upper_bound` and dropped at the start
//!   and end of a container (a module body keeps its first), and the file ends with one line
//!   break, in the `newline_style`.
//!
//! Left as written, as the standard formatter leaves them: macro bodies, anything marked
//! `#[rustfmt::skip]`, `// mto: off` regions, files that skip themselves, files with syntax
//! errors, and a statement with a line of code past its first longer than `max_width`,
//! which the standard formatter gives up on.

use std::collections::{HashMap, HashSet};

use crate::options::format::{FormatConfig, NewlineStyle};
use crate::syntax::ast::Attr;
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, parse};

/// The roles tokens play in the indentation.
#[derive(Default)]
struct Roles {
    /// A token that starts something in a container, with the container's opening offset
    /// (`None` at the top level).
    starts: HashMap<usize, Option<usize>>,
    /// A container's closing delimiter, with its opening offset.
    closers: HashMap<usize, usize>,
    /// Containers' opening delimiters, with where the line their indentation follows is.
    openers: HashMap<usize, usize>,
    /// Comments just before a container's closing delimiter, with its opening offset.
    trailing: HashMap<usize, usize>,
    /// Containers whose blank lines after the opening delimiter stay (module bodies).
    keeps_leading_blank: HashSet<usize>,
    /// Ranges left exactly as written.
    verbatim: Vec<(usize, usize)>,
}

/// A token, with where it starts.
struct Tok {
    kind: SyntaxKind,
    text: String,
    start: usize,
}

/// The file formatted; the input itself when it cannot be formatted safely.
pub fn format_file(source: &str, edition: Edition, config: &FormatConfig) -> String {
    let (bom, rest) = match source.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", source),
    };
    let first_crlf = rest
        .find('\n')
        .is_some_and(|i| i > 0 && rest.as_bytes()[i - 1] == b'\r');
    let newline = match config.newline_style {
        NewlineStyle::Auto if first_crlf => "\r\n",
        NewlineStyle::Auto | NewlineStyle::Unix => "\n",
        NewlineStyle::Windows => "\r\n",
        NewlineStyle::Native if cfg!(windows) => "\r\n",
        NewlineStyle::Native => "\n",
    };
    let text = rest.replace("\r\n", "\n");
    if text.contains('\r') {
        // A carriage return of its own (in a string, most likely): left as written.
        return source.to_string();
    }
    let parsed = parse(&text, edition);
    if !parsed.errors().is_empty() {
        return source.to_string();
    }
    let root = parsed.syntax_node();
    if skips_itself(&root) {
        return source.to_string();
    }
    let body = reindent(&root, &text, config);
    // Line endings inside a multi-line string are part of the string: a file whose strings
    // hold line breaks keeps its own endings.
    let strings_span_lines = root.descendants_with_tokens().iter().any(|e| {
        matches!(e, SyntaxElement::Token(t) if t.kind().is_literal() && t.text().contains('\n'))
    });
    let newline = if strings_span_lines {
        if first_crlf { "\r\n" } else { "\n" }
    } else {
        newline
    };
    let body = if newline == "\n" {
        body
    } else {
        body.replace('\n', newline)
    };
    format!("{}{}", bom, body)
}

/// Where a container's child starts, if it is something that starts a line: a node, or a
/// comment standing among them.
fn construct_start(element: &SyntaxElement) -> Option<usize> {
    match element {
        SyntaxElement::Node(node) => Some(node.text_range().start as usize),
        SyntaxElement::Token(token)
            if matches!(
                token.kind(),
                SyntaxKind::LineComment | SyntaxKind::BlockComment
            ) =>
        {
            Some(token.text_range().start as usize)
        }
        _ => None,
    }
}

fn container(node: &SyntaxNode, roles: &mut Roles) {
    let children = node.children_with_tokens();
    let is_open = |k: SyntaxKind| {
        matches!(
            k,
            SyntaxKind::LBrace | SyntaxKind::LParen | SyntaxKind::LBrack
        )
    };
    let is_close = |k: SyntaxKind| {
        matches!(
            k,
            SyntaxKind::RBrace | SyntaxKind::RParen | SyntaxKind::RBrack
        )
    };
    let open = children.iter().find_map(|e| match e {
        SyntaxElement::Token(t) if is_open(t.kind()) => Some(t.text_range().start as usize),
        _ => None,
    });
    let Some(open) = open else {
        return;
    };
    // The line the contents' indentation follows: the owner's first line, or for a method
    // call's arguments the line of its `.`. Arrays, tuples and anything standing in another
    // container (a block that is a statement) are their own owners.
    let owner = match node.parent() {
        Some(parent)
            if !is_container(parent.kind())
                && !matches!(node.kind(), SyntaxKind::ArrayExpr | SyntaxKind::TupleExpr) =>
        {
            parent
        }
        _ => node.clone(),
    };
    let base = match Some(owner) {
        Some(owner) if owner.kind() == SyntaxKind::MethodCallExpr => owner
            .children_with_tokens()
            .iter()
            .find_map(|e| match e {
                SyntaxElement::Token(t) if t.kind() == SyntaxKind::Dot => {
                    Some(t.text_range().start as usize)
                }
                _ => None,
            })
            .unwrap_or(open),
        Some(owner) if owner.kind() != SyntaxKind::SourceFile => owner
            .first_token()
            .map_or(open, |t| t.text_range().start as usize),
        _ => open,
    };
    roles.openers.insert(open, base);
    if node.kind() == SyntaxKind::ItemList {
        roles.keeps_leading_blank.insert(open);
    }
    let comment =
        |e: &SyntaxElement| matches!(e.kind(), SyntaxKind::LineComment | SyntaxKind::BlockComment);
    // The comments between the last code and the closing delimiter.
    if let Some(close_at) = children.iter().rposition(|e| is_close(e.kind())) {
        for e in children[..close_at].iter().rev() {
            if let SyntaxElement::Token(t) = e
                && comment(e)
            {
                roles.trailing.insert(t.text_range().start as usize, open);
            } else if e.kind() != SyntaxKind::Whitespace {
                break;
            }
        }
    }
    let mut inside = false;
    for child in &children {
        if let SyntaxElement::Token(t) = child {
            let at = t.text_range().start as usize;
            if at == open {
                inside = true;
                continue;
            }
            if is_close(t.kind()) && inside {
                roles.closers.insert(at, open);
                inside = false;
                continue;
            }
        }
        if inside && let Some(start) = construct_start(child) {
            roles.starts.insert(start, Some(open));
        }
    }
}

fn indentation(width: usize, config: &FormatConfig) -> String {
    if config.hard_tabs && config.tab_spaces > 0 {
        let mut out = "\t".repeat(width / config.tab_spaces);
        out.push_str(&" ".repeat(width % config.tab_spaces));
        out
    } else {
        " ".repeat(width)
    }
}

fn is_container(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::ItemList
            | SyntaxKind::AssocItemList
            | SyntaxKind::ExternItemList
            | SyntaxKind::BlockExpr
            | SyntaxKind::RecordFieldList
            | SyntaxKind::TupleFieldList
            | SyntaxKind::VariantList
            | SyntaxKind::MatchArmList
            | SyntaxKind::ArgList
            | SyntaxKind::ParamList
            | SyntaxKind::ArrayExpr
            | SyntaxKind::TupleExpr
            | SyntaxKind::RecordExprFieldList
            | SyntaxKind::UseTreeList
    )
}

/// The statements (`statements` in tree order) with a line of their own code wider than
/// `max_width`, as written: a line belongs to the innermost statement it starts in, and its
/// code ends where a line comment starts. A statement's first line is not measured: it is
/// indented anew, so measuring it would decide differently on a second run.
fn overflowing(
    statements: &[(usize, usize)],
    line_comments: &[usize],
    text: &str,
    config: &FormatConfig,
) -> Vec<(usize, usize)> {
    let tab = config.tab_spaces.max(1);
    let mut found = Vec::new();
    let mut open: Vec<(usize, usize, bool)> = Vec::new();
    let mut next = 0;
    let mut at = 0;
    for line in text.split('\n') {
        let line_end = at + line.len();
        let first = at + (line.len() - line.trim_start().len());
        // The statements that start before this line's first token are entered; those
        // ending before it are left.
        while next < statements.len() && statements[next].0 <= first {
            open.retain(|&(_, end, _)| end > statements[next].0);
            open.push((statements[next].0, statements[next].1, false));
            next += 1;
        }
        open.retain(|&(_, end, _)| end > first);
        if let Some(innermost) = open.last_mut()
            && innermost.0 < at
            && !innermost.2
        {
            let comment = line_comments.partition_point(|&c| c < at);
            let code_end = line_comments
                .get(comment)
                .filter(|&&c| c < line_end)
                .copied()
                .unwrap_or(line_end);
            if width_of(text[at..code_end].trim_end(), tab) > config.max_width {
                innermost.2 = true;
                found.push((innermost.0, innermost.1));
            }
        }
        at = line_end + 1;
    }
    found
}

fn reindent(root: &SyntaxNode, text: &str, config: &FormatConfig) -> String {
    let roles = roles(root, text, config);
    let in_verbatim = |at: usize| {
        let idx = roles.verbatim.partition_point(|&(start, _)| start < at);
        idx > 0 && at < roles.verbatim[idx - 1].1
    };
    let tokens: Vec<Tok> = root
        .descendants_with_tokens()
        .into_iter()
        .filter_map(|e| match e {
            SyntaxElement::Token(t) => Some(Tok {
                kind: t.kind(),
                text: t.text().to_string(),
                start: t.text_range().start as usize,
            }),
            _ => None,
        })
        .collect();
    let tab = config.tab_spaces.max(1);
    let max_newlines = config.blank_lines_upper_bound + 1;
    let mut out = String::with_capacity(text.len());
    // Each line's first token, with the line's indentation as written and as output.
    let mut lines: Vec<(usize, usize, usize)> = vec![(0, 0, 0)];
    // How far the construct being continued moved.
    let mut delta: isize = 0;
    // Each container's indentation, as written and as output.
    let mut open_indent: HashMap<usize, (usize, usize)> = HashMap::new();
    let mut previous_significant: Option<usize> = None;
    // A line comment that ended a line of code: its column as written and as output.
    let mut comment_column: Option<(usize, usize)> = None;
    let mut out_line_start = 0usize;
    let mut text_line_start = 0usize;
    let mut code_on_line = false;
    for (i, tok) in tokens.iter().enumerate() {
        let next = tokens.get(i + 1);
        if out.is_empty() && tok.kind == SyntaxKind::Whitespace {
            // Nothing before the first token.
            continue;
        }
        if tok.kind == SyntaxKind::Whitespace && tok.text.contains('\n') {
            let written_text = tok.text.rsplit('\n').next().unwrap_or("");
            let written = width_of(written_text, tab);
            let next_start = tok.start + tok.text.len();
            text_line_start = next_start - written_text.len();
            code_on_line = false;
            if in_verbatim(tok.start) {
                out.push_str(&tok.text);
                out_line_start = out.len() - written_text.len();
                lines.push((next_start, written, written));
                comment_column = None;
                continue;
            }
            let Some(next) = next else {
                break;
            };
            let newlines = tok.text.matches('\n').count();
            let after_open = previous_significant.filter(|p| roles.openers.contains_key(p));
            let newlines = match after_open {
                Some(open) if roles.keeps_leading_blank.contains(&open) => {
                    newlines.min(max_newlines)
                }
                Some(_) => 1,
                None if roles.closers.contains_key(&next.start) => 1,
                None => newlines.min(max_newlines),
            };
            let aligned = comment_column.filter(|&(column, _)| {
                newlines == 1 && next.kind == SyntaxKind::LineComment && written == column
            });
            let indent = if let Some((_, column)) = aligned {
                column
            } else if let Some(open) = roles.closers.get(&next.start) {
                let new = open_indent.get(open).map_or(0, |&(_, new)| new);
                delta = new as isize - written as isize;
                new
            } else if let Some(open) = roles.trailing.get(&next.start) {
                let (was, new) = open_indent.get(open).copied().unwrap_or((0, 0));
                (written as isize + new as isize - was as isize).max(0) as usize
            } else if let Some(open) = roles.starts.get(&next.start) {
                let new = match open {
                    Some(open) => open_indent.get(open).map_or(0, |&(_, new)| new) + tab,
                    None => 0,
                };
                delta = new as isize - written as isize;
                new
            } else {
                (written as isize + delta).max(0) as usize
            };
            if aligned.is_none() {
                comment_column = None;
            }
            for _ in 0..newlines {
                out.push('\n');
            }
            out_line_start = out.len();
            out.push_str(&indentation(indent, config));
            lines.push((next_start, written, indent));
            continue;
        }
        if tok.kind == SyntaxKind::Whitespace && next.is_none() {
            break;
        }
        if let Some(&base) = roles.openers.get(&tok.start) {
            let idx = lines.partition_point(|&(start, _, _)| start <= base);
            let (_, was, new) = lines[idx.saturating_sub(1)];
            open_indent.insert(tok.start, (was, new));
        }
        if tok.kind == SyntaxKind::LineComment && code_on_line {
            let written = width_of(&text[text_line_start..tok.start], tab);
            let output = width_of(&out[out_line_start..], tab);
            comment_column = Some((written, output));
        }
        // Doc comments keep their trailing whitespace, as the standard formatter leaves it.
        let trimmed = tok.kind == SyntaxKind::LineComment && !in_verbatim(tok.start);
        out.push_str(if trimmed {
            tok.text.trim_end()
        } else {
            &tok.text
        });
        if !tok.kind.is_trivia() || tok.kind == SyntaxKind::BlockComment {
            code_on_line = true;
        }
        if !tok.kind.is_trivia()
            || matches!(tok.kind, SyntaxKind::LineComment | SyntaxKind::BlockComment)
        {
            previous_significant = Some(tok.start);
        }
    }
    // The end of the file gets exactly one line break.
    let mut out = out.trim_end_matches([' ', '\t', '\n']).to_string();
    out.push('\n');
    out
}

fn roles(root: &SyntaxNode, text: &str, config: &FormatConfig) -> Roles {
    let mut roles = Roles::default();
    for child in root.children_with_tokens() {
        if let Some(start) = construct_start(&child) {
            roles.starts.insert(start, None);
        }
    }
    let mut region: Option<usize> = None;
    let mut statements = Vec::new();
    let mut line_comments = Vec::new();
    for element in root.descendants_with_tokens() {
        match &element {
            SyntaxElement::Node(node) => {
                let range = node.text_range();
                let skipped = node.kind() != SyntaxKind::SourceFile
                    && node
                        .children()
                        .into_iter()
                        .filter_map(Attr::cast)
                        .any(|a| a.is_rustfmt_skip());
                if node
                    .parent()
                    .is_some_and(|p| p.kind() == SyntaxKind::BlockExpr)
                {
                    statements.push((range.start as usize, range.end as usize));
                }
                if node.kind() == SyntaxKind::TokenTree || skipped {
                    roles
                        .verbatim
                        .push((range.start as usize, range.end as usize));
                }
                if is_container(node.kind()) {
                    container(node, &mut roles);
                }
            }
            SyntaxElement::Token(token) => {
                let marker = token.text().trim();
                let start = token.text_range().start as usize;
                if token.kind() == SyntaxKind::LineComment {
                    line_comments.push(start);
                    if marker == "// mto: off" && region.is_none() {
                        region = Some(start);
                    } else if marker == "// mto: on"
                        && let Some(from) = region.take()
                    {
                        roles.verbatim.push((from, token.text_range().end as usize));
                    }
                }
            }
        }
    }
    if let Some(from) = region {
        roles.verbatim.push((from, text.len()));
    }
    roles
        .verbatim
        .extend(overflowing(&statements, &line_comments, text, config));
    // Sorted and merged, for looking up by binary search.
    roles.verbatim.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(roles.verbatim.len());
    for (start, end) in roles.verbatim.drain(..) {
        match merged.last_mut() {
            Some(last) if start < last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    roles.verbatim = merged;
    roles
}

/// A file with `#![rustfmt::skip]` is left alone.
fn skips_itself(root: &SyntaxNode) -> bool {
    root.children()
        .into_iter()
        .filter_map(Attr::cast)
        .any(|a| a.is_inner() && a.is_rustfmt_skip())
}

/// The column width of an indentation, tabs counted as `tab_spaces`.
fn width_of(indent: &str, tab: usize) -> usize {
    indent
        .chars()
        .map(|c| if c == '\t' { tab } else { 1 })
        .sum()
}

#[cfg(test)]
mod tests;
