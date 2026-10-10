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
//!   `#[rustfmt::skip]` items, items under a `// mto: pin` comment, and `// mto: off` ...
//!   `// mto: on` regions. A marker may be a loose comment or one attached to an item; an
//!   item with `// mto: on` attached above it stays inside the region, and a region with no
//!   `// mto: on` runs to the end of the file;
//! - adjacent `#[cfg]` items of the same kind and name (`#[cfg(unix)] fn f` and
//!   `#[cfg(windows)] fn f`) are one unit, so the variants stay together;
//! - **the tail:** comments after the last item, kept last.
//!
//! A file whose header has `#![rustfmt::skip]` is left as it is, as the standard formatter
//! leaves it.
//!
//! The body of an inline module (`mod name { ... }`) is laid out the same way, at every
//! level, at its own indentation; a body on one line with its braces is left as written.
//! Impl bodies are ordered only when asked ([`impls`]).
//!
//! [`order`] sorts the units by the [`LayoutConfig`]; [`layout`] puts the file back
//! together with the blank lines between units the config asks for.

mod classify;
mod impls;
pub mod imports;
mod order;

use std::collections::HashSet;

pub use classify::import_group_of_text;

use crate::options::layout::{ImportGroup, ItemKind, LayoutConfig, Visibility};
use crate::syntax::ast::{Attr, Item};
use crate::syntax::{Edition, Parse, SyntaxElement, SyntaxKind, SyntaxNode, parse};
use crate::text::source::normalize;

/// Why a file with syntax errors is left alone.
pub const SYNTAX_ERRORS: &str = "the file has syntax errors";

/// What one layout pass did: the units as found, as arranged, and the new text.
pub struct Layout {
    pub before: Vec<Unit>,
    pub after: Vec<Unit>,
    /// The file tells the standard formatter to skip it (`#![rustfmt::skip]`), so it was
    /// left as it is.
    pub skipped: bool,
    pub text: String,
    /// The edition the file was parsed with (see [`parse_any_edition`]).
    pub edition: Edition,
}

/// The parsed file, cut into header, units and tail.
struct Split {
    edition: Edition,
    /// The header has `#![rustfmt::skip]`.
    skip_file: bool,
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
    /// For a trait impl, the trait's name without its path (`Display`).
    pub trait_name: Option<String>,
    /// The exact text, from the first loose comment above the item to the item's end.
    pub text: String,
    /// Where the unit's text came from in the parsed text: (start, end).
    pub span: (usize, usize),
    /// Keeps its position: `extern crate`, item macro calls, `#[rustfmt::skip]` items,
    /// `// mto: pin` items, `// mto: off` regions.
    pub pinned: bool,
    /// An item-position macro call (pinned at the top level and in modules).
    pub macro_call: bool,
    /// Has a `#[cfg]` attribute.
    pub cfg: bool,
    /// Position among the units of the input.
    pub index: usize,
    /// `#[cfg(test)] mod`.
    pub test_module: bool,
    /// `#[macro_use] mod`.
    pub macro_use: bool,
    /// Defines macros the units after it may use (see `Item::defines_macros`), so no unit
    /// that came after it may move in front of it.
    pub defines_macros: bool,
    /// `mod name;`, a module declared without a body.
    pub module_declaration: bool,
    /// For a `use`: standard library, external crate or this crate.
    pub import_group: Option<ImportGroup>,
    /// For a `use`: its tree as the standard formatter sorts it.
    pub use_tree: Option<imports::UseTree>,
}

/// Arranges the items of `source`; `Err` with the reason when the file is left alone.
pub fn layout(source: &str, config: &LayoutConfig) -> Result<Layout, String> {
    let normalized = normalize(source)?;
    let apart = "\n".repeat(config.blank_lines_between_blocks + 1);
    let split = split(&normalized.text, &apart, config)?;
    if split.skip_file {
        return Ok(Layout {
            edition: split.edition,
            before: split.units.clone(),
            after: split.units,
            skipped: true,
            text: source.to_string(),
        });
    }
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
    let text = render(&split, &after, config, "", true);
    Ok(Layout {
        edition: split.edition,
        before: split.units,
        after,
        skipped: false,
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

/// Whether `next` is another `#[cfg]` variant of `previous`: both have a `#[cfg]`, the same
/// kind and the same name, and neither is pinned.
fn cfg_variants(previous: &Unit, next: &Unit) -> bool {
    previous.cfg
        && next.cfg
        && !previous.pinned
        && !next.pinned
        && previous.kind == next.kind
        && previous.identifier == next.identifier
}

/// The text of `pieces` (element ranges) in order. A gap of one line break or less is kept
/// as written; a gap with a blank line becomes `apart`, the configured spacing, as between
/// units, so the result does not depend on where an element sat before. It keeps at least
/// one blank line, even when the spacing is none: a comment set apart from the item below
/// it must not come to touch it, or it would attach to that item.
///
/// A comment that shared its line with the code before it (`loose`, by start) belongs to
/// neither neighbour; on a line of its own it would head what follows, so a blank line
/// always follows it.
fn joined(text: &str, pieces: &[(usize, usize)], apart: &str, loose: &HashSet<usize>) -> String {
    let mut out = String::new();
    for (idx, (start, end)) in pieces.iter().enumerate() {
        if idx > 0 {
            let gap = &text[pieces[idx - 1].1..*start];
            if gap.matches('\n').count() >= 2 || loose.contains(&pieces[idx - 1].0) {
                if apart.matches('\n').count() < 2 {
                    out.push('\n');
                }
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

/// The text of an inline module (`start..end` of `item`) with its body laid out, or `None`
/// when it is not one, the body is empty or on one line with its braces, or the body cannot
/// be laid out (it is then left as written).
fn module_with_body_laid_out(
    item: &Item,
    text: &str,
    apart: &str,
    config: &LayoutConfig,
    traits: &impls::Traits,
) -> Option<String> {
    let node = item.syntax();
    if node.kind() != SyntaxKind::Module {
        return None;
    }
    let body = node
        .children()
        .into_iter()
        .find(|c| c.kind() == SyntaxKind::ItemList)?;
    let range = body.text_range();
    let (open_end, close_start) = (range.start as usize + 1, range.end as usize - 1);
    let significant = |e: &SyntaxElement| {
        !matches!(
            e.kind(),
            SyntaxKind::Whitespace | SyntaxKind::LBrace | SyntaxKind::RBrace
        )
    };
    let elements = body.children_with_tokens();
    let first = elements.iter().find(|e| significant(e))?.text_range().start as usize;
    let last = elements.iter().rfind(|e| significant(e))?.text_range().end as usize;
    let leading = &text[open_end..first];
    if !leading.contains('\n') {
        return None;
    }
    let indent = &leading[leading.rfind('\n')? + 1..];
    let base_apart = apart.trim_end_matches(|c| c != '\n');
    let inner_apart = format!("{}{}", base_apart, indent);
    let split = split_container(&body, text, open_end, &inner_apart, config, traits).ok()?;
    if split.skip_file || split.units.is_empty() {
        return None;
    }
    let mut macros = std::collections::HashSet::new();
    if split
        .units
        .iter()
        .any(|u| u.kind == ItemKind::MacroDecl && !macros.insert(u.identifier.as_str()))
    {
        return None;
    }
    let arranged = order::arrange(&split.units, config).ok()?;
    let inner = render(&split, &arranged, config, indent, false);
    let item_range = node.text_range();
    Some(format!(
        "{}{}{}{}{}",
        &text[item_range.start as usize..open_end],
        leading,
        inner,
        &text[last..close_start],
        &text[close_start..item_range.end as usize]
    ))
}

/// The header, the units in their new order and the tail, joined by the blank lines the
/// config asks for: imports of one group and `mod name;` declarations sit on consecutive
/// lines, everything else is `blank_lines_between_blocks` apart.
fn render(split: &Split, units: &[Unit], config: &LayoutConfig, indent: &str, top: bool) -> String {
    let mut output = String::new();
    let apart = format!(
        "{}{}",
        "\n".repeat(config.blank_lines_between_blocks + 1),
        indent
    );
    let line = format!("{}{}", "\n".repeat(config.blank_lines_tight + 1), indent);
    // At least one blank line, whatever `blank_lines_between_blocks` says, where touching
    // would change what the text means to a reader or to the standard formatter.
    let parted = format!(
        "{}{}",
        "\n".repeat(config.blank_lines_between_blocks.max(1) + 1),
        indent
    );
    let header = split.header.trim_start();
    if !header.is_empty() {
        output.push_str(header);
        if !units.is_empty() || !split.tail.is_empty() {
            let comment_last = header.trim_end().ends_with("*/")
                || header
                    .trim_end()
                    .lines()
                    .last()
                    .is_some_and(|l| l.trim_start().starts_with("//"));
            output.push_str(if comment_last { &parted } else { &apart });
        }
    }
    let mut previous: Option<&Unit> = None;
    for unit in units {
        if let Some(prev) = previous {
            let both_imports = prev.kind == ItemKind::UseDecl && unit.kind == ItemKind::UseDecl;
            let new_group = config.sort_imports
                && config.import_grouping
                && prev.import_group != unit.import_group;
            // A `#[cfg(test)] mod tests;` sorted last stands apart, or the standard
            // formatter would sort it back among the declarations above it.
            let declarations = prev.module_declaration && unit.module_declaration;
            let both_declarations = declarations && prev.test_module == unit.test_module;
            if (both_imports && !new_group) || both_declarations {
                output.push_str(&line);
            } else if both_imports || declarations {
                // The standard formatter sorts imports, and `mod` declarations, within runs
                // not broken by a blank line, so groups touching would be sorted into one.
                output.push_str(&parted);
            } else {
                output.push_str(&apart);
            }
        }
        output.push_str(&unit.text);
        previous = Some(unit);
    }
    if !split.tail.is_empty() {
        if !units.is_empty() {
            // The tail is what follows the last item: comments that belong to no item.
            output.push_str(&parted);
        }
        output.push_str(&split.tail);
    }
    // Every piece ends where its last element ends, so only the final line break is added;
    // trailing spaces inside a last comment stay. A module body keeps what was after its
    // last element instead.
    if top {
        output.push('\n');
    }
    output
}

/// Cuts the file into header, units and tail; see the module documentation.
fn split(text: &str, apart: &str, config: &LayoutConfig) -> Result<Split, String> {
    let (edition, parsed) = parse_any_edition(text);
    if !parsed.errors().is_empty() {
        return Err(SYNTAX_ERRORS.to_string());
    }
    let tree = parsed.syntax_node();
    // Walking every node is costly, so traits are only gathered when they are used.
    let traits = if config.impl_method_order_trait {
        impls::collect_traits(&tree)
    } else {
        impls::Traits::new()
    };
    let mut split = split_container(&tree, text, 0, apart, config, &traits)?;
    split.edition = edition;
    Ok(split)
}

/// Cuts the elements of `container` (the file, or a module's `{ ... }` body starting at
/// `base`) into header, units and tail. Inline modules among the units are laid out too.
fn split_container(
    container: &SyntaxNode,
    text: &str,
    base: usize,
    apart: &str,
    config: &LayoutConfig,
    traits: &impls::Traits,
) -> Result<Split, String> {
    let mut header_end = base;
    // The header's elements so far, as (start, end, is a comment).
    let mut header_elements: Vec<(usize, usize, bool)> = Vec::new();
    let mut in_body = false;
    // Loose comments waiting for the item they head, as (start, end).
    let mut pending: Vec<(usize, usize)> = Vec::new();
    // Comments that shared their line with the code before them, by start.
    let mut loose: HashSet<usize> = HashSet::new();
    let mut code_before: Option<usize> = None;
    let mut region_start: Option<usize> = None;
    let mut units: Vec<Unit> = Vec::new();
    let mut last_element_end = 0;
    let mut skip_file = false;
    // Where the last item unit ended, to join `#[cfg]` variants to it.
    let mut last_item_end: Option<usize> = None;
    for element in container.children_with_tokens() {
        let (kind, range) = match &element {
            SyntaxElement::Node(node) => (node.kind(), node.text_range()),
            SyntaxElement::Token(token) => (token.kind(), token.text_range()),
        };
        let (start, end) = (range.start as usize, range.end as usize);
        if matches!(
            kind,
            SyntaxKind::Whitespace | SyntaxKind::LBrace | SyntaxKind::RBrace
        ) {
            continue;
        }
        last_element_end = end;
        let comment = matches!(kind, SyntaxKind::LineComment | SyntaxKind::BlockComment);
        if comment && code_before.is_some_and(|p| !text[p..start].contains('\n')) {
            loose.insert(start);
        }
        code_before = Some(end);
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
            if let SyntaxElement::Node(node) = &element
                && Attr::cast(node.clone()).is_some_and(|a| a.is_rustfmt_skip())
            {
                skip_file = true;
            }
            if header_element || (comment && !marker("off")) {
                header_elements.push((start, end, comment));
                header_end = end;
                continue;
            }
            in_body = true;
            // Comments right above the first item (no blank line between) head it, as they
            // would any other item; only runs set apart by a blank line stay in the header.
            let mut next_start = start;
            while let Some(&(s, e, true)) = header_elements.last() {
                // A comment trailing the element before it on its line stays with it.
                let before = header_elements
                    .len()
                    .checked_sub(2)
                    .map_or(base, |i| header_elements[i].1);
                let own_line = before == base || text[before..s].contains('\n');
                if text[e..next_start].matches('\n').count() >= 2 || !own_line {
                    break;
                }
                pending.insert(0, (s, e));
                header_elements.pop();
                next_start = s;
            }
            header_end = header_elements.last().map_or(base, |&(_, e, _)| e);
        }
        if let Some(region) = region_start {
            if marker("on") {
                units.push(classify::pinned_region(text, region, end, units.len()));
                region_start = None;
            }
            continue;
        }
        if marker("off") {
            let region = pending.first().map(|r| r.0).unwrap_or(start);
            pending.clear();
            // Both markers above one item: a region of just that item.
            if !comment && marker("on") {
                units.push(classify::pinned_region(text, region, end, units.len()));
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
        let pinned_by_comment = marker("pin")
            || pending
                .iter()
                .any(|(s, e)| text[*s..*e].trim() == "// mto: pin");
        let loose_comments = !pending.is_empty();
        let mut pieces = std::mem::take(&mut pending);
        pieces.push((start, end));
        let unit_start = pieces[0].0;
        let mut unit = classify::unit(
            &item,
            joined(text, &pieces, apart, &loose),
            units.len(),
            config,
        );
        unit.pinned |= pinned_by_comment || (unit.test_module && config.test_module_in_place);
        // An inline module's body is laid out by the same rules; its new text replaces the
        // item's at the end of the unit.
        let item_text = if unit.pinned {
            None
        } else {
            module_with_body_laid_out(&item, text, apart, config, traits)
                .or_else(|| impls::impl_with_items_ordered(&item, text, apart, config, traits))
        };
        let with_item = |mut joined: String| {
            if let Some(new) = &item_text {
                joined.truncate(joined.len() - (end - start));
                joined.push_str(new);
            }
            joined
        };
        unit.text = with_item(std::mem::take(&mut unit.text));
        unit.span = (unit_start, end);
        if let (Some(previous), Some(previous_end)) = (units.last_mut(), last_item_end)
            && cfg_variants(previous, &unit)
            && !loose_comments
        {
            let joined_text = with_item(joined(
                text,
                &[(previous_end, previous_end), (start, end)],
                apart,
                &loose,
            ));
            previous.text.push_str(&joined_text);
            // The variants render as one block, set apart like a module with a body
            // unless every variant is a `mod name;` declaration.
            previous.module_declaration &= unit.module_declaration;
            previous.span.1 = end;
            last_item_end = Some(end);
            continue;
        }
        units.push(unit);
        last_item_end = Some(end);
    }
    if let Some(region) = region_start {
        // A region with no `// mto: on` runs to the end of the file.
        units.push(classify::pinned_region(
            text,
            region,
            last_element_end,
            units.len(),
        ));
    }
    let tail = joined(text, &pending, apart, &loose);
    Ok(Split {
        edition: Edition::E2021,
        skip_file,
        header: text[base..header_end].to_string(),
        units,
        tail,
    })
}

#[cfg(test)]
mod tests;
