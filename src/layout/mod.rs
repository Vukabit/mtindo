//! Layer 3: arranging the items of a file, mto's own feature.
//!
//! The file is parsed; a file with syntax errors is left alone. Its top-level elements are
//! then cut into:
//!
//! - **the header:** a shebang, frontmatter, inner attributes, inner doc comments, and
//!   comment runs set apart from the first item by a blank line (a licence notice). It
//!   stays first, byte for byte;
//! - **units:** each item with the loose comments above it (a section heading moves with
//!   the item under it). The item's own attached comments are part of the item already
//!   (`docs/design/comment-attachment.md`);
//! - **pinned units** that keep their position: `extern crate`, item-position macro calls,
//!   and `// mto: off` ... `// mto: on` regions. A marker may be a loose comment or one
//!   attached to an item; an item with `// mto: on` attached above it stays inside the
//!   region, and a region with no `// mto: on` runs to the end of the file;
//! - **the tail:** comments after the last item, kept last.
//!
//! [`order`] sorts the units by the [`LayoutConfig`]; [`layout`] puts the file back
//! together with the blank lines between units the config asks for.

mod classify;
mod order;

pub use classify::import_group_of_text;

use crate::options::layout::{ImportGroup, ItemKind, LayoutConfig, Visibility};
use crate::syntax::ast::{Attr, Item};
use crate::syntax::{Edition, Parse, SyntaxElement, SyntaxKind, parse};
use crate::text::source::normalize;

/// What one layout pass did: the units as found, as arranged, and the new text.
pub struct Layout {
    pub before: Vec<Unit>,
    pub after: Vec<Unit>,
    pub text: String,
    /// The edition the file was parsed with (see [`parse_any_edition`]).
    pub edition: Edition,
}

/// The parsed file, cut into header, units and tail.
struct Split {
    edition: Edition,
    header: String,
    units: Vec<Unit>,
    tail: String,
}

/// An item with the loose comments above it, or a pinned region; what layout moves.
#[derive(Clone, Debug)]
pub struct Unit {
    pub kind: ItemKind,
    pub visibility: Visibility,
    /// The name sorted on: the item's name, an import's path, `Trait for Type` for impls.
    pub identifier: String,
    /// For an impl, the type it is for; what impls are grouped by.
    pub parent_target: String,
    /// The exact text, from the first loose comment above the item to the item's end.
    pub text: String,
    /// Keeps its position: `extern crate`, item macro calls, `// mto: off` regions.
    pub pinned: bool,
    /// Position among the units of the input.
    pub index: usize,
    /// `#[cfg(test)] mod`.
    pub test_module: bool,
    /// `#[macro_use] mod`.
    pub macro_use: bool,
    /// `mod name;`, a module declared without a body.
    pub module_declaration: bool,
    /// For a `use`: standard library, external crate or this crate.
    pub import_group: Option<ImportGroup>,
}

/// Arranges the items of `source`; `Err` with the reason when the file is left alone.
pub fn layout(source: &str, config: &LayoutConfig) -> Result<Layout, String> {
    let normalized = normalize(source)?;
    let apart = "\n".repeat(config.blank_lines_between_blocks + 1);
    let split = split(&normalized.text, &apart)?;
    let mut macros = std::collections::HashSet::new();
    for unit in split.units.iter() {
        if unit.kind == ItemKind::MacroDecl && !macros.insert(unit.identifier.as_str()) {
            return Err(format!(
                "macro `{}` is defined more than once",
                unit.identifier
            ));
        }
    }
    let after = order::arrange(&split.units, config)?;
    let text = render(&split, &after, config);
    Ok(Layout {
        edition: split.edition,
        before: split.units,
        after,
        text: normalized.restore(&text),
    })
}

/// Parses `text` with the first edition it has no syntax errors in (2021, 2018, 2015,
/// 2024), so code that is only valid in an older edition (`async` as a name) still parses.
/// Without a clean parse, 2021's result is returned.
pub fn parse_any_edition(text: &str) -> (Edition, Parse) {
    let first = parse(text, Edition::E2021);
    if first.errors().is_empty() {
        return (Edition::E2021, first);
    }
    for edition in [Edition::E2018, Edition::E2015, Edition::E2024] {
        let parsed = parse(text, edition);
        if parsed.errors().is_empty() {
            return (edition, parsed);
        }
    }
    (Edition::E2021, first)
}

/// The text of `pieces` (element ranges) in order. A gap of one line break or less is kept
/// as written; a gap with a blank line becomes `apart`, the configured spacing, as between
/// units, so the result does not depend on where an element sat before.
fn joined(text: &str, pieces: &[(usize, usize)], apart: &str) -> String {
    let mut out = String::new();
    for (idx, (start, end)) in pieces.iter().enumerate() {
        if idx > 0 {
            let gap = &text[pieces[idx - 1].1..*start];
            if gap.matches('\n').count() >= 2 {
                out.push_str(apart);
            } else {
                out.push_str(gap);
            }
        }
        out.push_str(&text[*start..*end]);
    }
    out
}

/// The plain comments attached above a node, trimmed, in order.
fn leading_comments(node: &crate::syntax::SyntaxNode) -> Vec<String> {
    let mut comments = Vec::new();
    for element in node.children_with_tokens() {
        match element {
            SyntaxElement::Token(token) if token.kind().is_trivia() => {
                if matches!(
                    token.kind(),
                    SyntaxKind::LineComment | SyntaxKind::BlockComment
                ) {
                    comments.push(token.text().trim().to_string());
                }
            }
            _ => break,
        }
    }
    comments
}

/// The header, the units in their new order and the tail, joined by the blank lines the
/// config asks for: imports of one group and `mod name;` declarations sit on consecutive
/// lines, everything else is `blank_lines_between_blocks` apart.
fn render(split: &Split, units: &[Unit], config: &LayoutConfig) -> String {
    let mut output = String::new();
    let apart = "\n".repeat(config.blank_lines_between_blocks + 1);
    let header = split.header.trim_start();
    if !header.is_empty() {
        output.push_str(header);
        if !units.is_empty() || !split.tail.is_empty() {
            output.push_str(&apart);
        }
    }
    let mut previous: Option<&Unit> = None;
    for unit in units {
        if let Some(prev) = previous {
            let both_imports = prev.kind == ItemKind::UseDecl && unit.kind == ItemKind::UseDecl;
            let new_group = config.sort_imports
                && config.import_grouping
                && prev.import_group != unit.import_group;
            let both_declarations = prev.module_declaration && unit.module_declaration;
            if (both_imports && !new_group) || both_declarations {
                output.push('\n');
            } else {
                output.push_str(&apart);
            }
        }
        output.push_str(&unit.text);
        previous = Some(unit);
    }
    if !split.tail.is_empty() {
        if !units.is_empty() {
            output.push_str(&apart);
        }
        output.push_str(&split.tail);
    }
    // Every piece ends where its last element ends, so only the final line break is added;
    // trailing spaces inside a last comment stay.
    output.push('\n');
    output
}

/// Cuts the file into header, units and tail; see the module documentation.
fn split(text: &str, apart: &str) -> Result<Split, String> {
    let (edition, parsed) = parse_any_edition(text);
    if !parsed.errors().is_empty() {
        return Err("the file has syntax errors".to_string());
    }
    let tree = parsed.syntax_node();
    let mut header_end = 0;
    let mut in_body = false;
    // Loose comments waiting for the item they head, as (start, end).
    let mut pending: Vec<(usize, usize)> = Vec::new();
    let mut region_start: Option<usize> = None;
    let mut units: Vec<Unit> = Vec::new();
    let mut last_element_end = 0;
    for element in tree.children_with_tokens() {
        let (kind, range) = match &element {
            SyntaxElement::Node(node) => (node.kind(), node.text_range()),
            SyntaxElement::Token(token) => (token.kind(), token.text_range()),
        };
        let (start, end) = (range.start as usize, range.end as usize);
        if kind == SyntaxKind::Whitespace {
            continue;
        }
        last_element_end = end;
        let comment = matches!(kind, SyntaxKind::LineComment | SyntaxKind::BlockComment);
        // Markers: the comment itself, or the comments attached above an item.
        let markers: Vec<String> = match &element {
            SyntaxElement::Token(_) if comment => vec![text[start..end].trim().to_string()],
            SyntaxElement::Node(node) => leading_comments(node),
            _ => Vec::new(),
        };
        let marker = |word: &str| markers.iter().any(|m| *m == format!("// mto: {}", word));
        let header_element = match &element {
            SyntaxElement::Token(_) => matches!(
                kind,
                SyntaxKind::Bom
                    | SyntaxKind::Shebang
                    | SyntaxKind::Frontmatter
                    | SyntaxKind::InnerLineDoc
                    | SyntaxKind::InnerBlockDoc
            ),
            SyntaxElement::Node(node) => Attr::cast(node.clone()).is_some_and(|a| a.is_inner()),
        };
        if !in_body {
            if header_element || (comment && !marker("off")) {
                header_end = end;
                continue;
            }
            in_body = true;
        }
        if let Some(region) = region_start {
            if marker("on") {
                units.push(classify::pinned_region(
                    text[region..end].to_string(),
                    units.len(),
                ));
                region_start = None;
            }
            continue;
        }
        if marker("off") {
            let region = pending.first().map(|r| r.0).unwrap_or(start);
            pending.clear();
            // Both markers above one item: a region of just that item.
            if !comment && marker("on") {
                units.push(classify::pinned_region(
                    text[region..end].to_string(),
                    units.len(),
                ));
            } else {
                region_start = Some(region);
            }
            continue;
        }
        if comment {
            pending.push((start, end));
            continue;
        }
        let item = match &element {
            SyntaxElement::Node(node) => Item::cast(node.clone()),
            SyntaxElement::Token(_) => None,
        };
        let Some(item) = item else {
            if header_element {
                return Err("an inner attribute or doc comment comes after an item".to_string());
            }
            return Err(format!("unexpected {:?} at the top level", kind));
        };
        let mut pieces = std::mem::take(&mut pending);
        pieces.push((start, end));
        units.push(classify::unit(
            &item,
            joined(text, &pieces, apart),
            units.len(),
        ));
    }
    if let Some(region) = region_start {
        // A region with no `// mto: on` runs to the end of the file.
        units.push(classify::pinned_region(
            text[region..last_element_end].to_string(),
            units.len(),
        ));
    }
    let tail = joined(text, &pending, apart);
    Ok(Split {
        edition,
        header: text[..header_end].to_string(),
        units,
        tail,
    })
}

#[cfg(test)]
mod tests;
