//! Layer 2: proves mto's output is safe before it is written.
//!
//! In layout mode, mto may only move whole items and change the whitespace between them,
//! at the top level and inside inline modules and impl blocks, each body being a scope of
//! its own: an item may not move between scopes. [`verify_layout`] parses the input and the output and checks, rule by
//! rule ([`Rule`]):
//!
//! - every item comes out with exactly its original bytes, its attached comments included
//!   (items are compared as a multiset, so order is free);
//! - every comment, attached or not, is still there, byte for byte;
//! - the header (shebang, frontmatter, inner attributes and inner doc comments) is
//!   unchanged and still before every item;
//! - anything else at the top level (damaged text, stray tokens) is unchanged;
//! - `macro_rules!` definitions keep their order relative to each other, because a macro
//!   is only visible below its definition;
//! - the output parses at least as well as the input;
//! - the byte-order mark and the line-ending style are kept.
//!
//! The verifier does not know which engine produced the output, so it checks today's
//! text-scanning engine and the syntax-tree engine of stage 2 alike. [`report`] turns
//! violations into a bug report with the smallest input that still shows them.

pub mod format;
pub mod report;
pub mod rewrites;

use std::collections::{BTreeMap, HashMap};

use crate::syntax::ast::{Attr, Item, ItemKind};
use crate::syntax::lexer::tokenize;
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, parse};

/// Marks a scope whose macros are visible in its parent too (`#[macro_use] mod`).
const MACRO_USE: &str = " [macro_use]";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineEndings {
    None,
    Lf,
    Crlf,
    Mixed,
}

/// One way the output can be unsafe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    /// The output has syntax errors the input did not have.
    OutputBroken,
    /// An item of the input is missing from the output, or comes out changed.
    ItemLost,
    /// The output has an item the input did not have, or more copies of one.
    ItemAdded,
    /// An item comes out unchanged except for the comments attached to it: a comment
    /// that described it came loose, or one that described something else attached to it.
    CommentReattached,
    /// A comment of the input is missing from the output.
    CommentLost,
    /// The output has a comment the input did not have.
    CommentAdded,
    /// The header changed, or is no longer before every item.
    HeaderChanged,
    /// Something that is neither an item, a comment, the header nor whitespace changed.
    OtherChanged,
    /// `macro_rules!` definitions changed order relative to each other.
    MacroOrderChanged,
    /// An item that came after a macro definition now comes before it, where the macro is
    /// not yet defined.
    MacroUserMoved,
    /// Formatted output's tokens differ from the input's beyond the allowed rewrites: the
    /// program's meaning may have changed.
    MeaningChanged,
    /// The byte-order mark was added or removed.
    BomChanged,
    /// The line endings changed style (`\n` against `\r\n`).
    LineEndingsChanged,
}

impl Rule {
    /// The rule's short name, used in bug reports and logs.
    pub fn name(self) -> &'static str {
        match self {
            Rule::OutputBroken => "output-broken",
            Rule::ItemLost => "item-lost",
            Rule::ItemAdded => "item-added",
            Rule::CommentReattached => "comment-reattached",
            Rule::CommentLost => "comment-lost",
            Rule::CommentAdded => "comment-added",
            Rule::HeaderChanged => "header-changed",
            Rule::OtherChanged => "other-changed",
            Rule::MacroOrderChanged => "macro-order-changed",
            Rule::MacroUserMoved => "macro-user-moved",
            Rule::MeaningChanged => "meaning-changed",
            Rule::BomChanged => "bom-changed",
            Rule::LineEndingsChanged => "line-endings-changed",
        }
    }
}

/// The parts of a file the rules compare.
#[derive(Debug, Default)]
struct Parts {
    /// Shebang, frontmatter, inner attributes and inner doc comments, in order, each with
    /// its scope; compared scope by scope.
    header: Vec<(String, String)>,
    /// Whether every header element comes before the first item.
    header_first: bool,
    /// Each top-level item: (exact text, core text without attached comments).
    items: Vec<(String, String)>,
    /// Every plain comment anywhere in the file.
    comments: Vec<String>,
    /// Everything else at the top level that is not whitespace.
    others: Vec<String>,
    /// `macro_rules!` names in order, each with the scope it is visible in; compared scope by
    /// scope. A `#[macro_use]` module's macros also count in its parent's scope.
    macros: Vec<(String, String)>,
    /// Each item in order as (scope, core text, defines macros): what must stay below a
    /// macro definition.
    sequence: Vec<(String, String, bool)>,
    errors: usize,
}

/// One broken rule, with what was seen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub rule: Rule,
    pub detail: String,
}

/// Every rule the output breaks; empty when it is safe to write.
pub fn verify_layout(input: &str, output: &str, edition: Edition) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut add = |rule: Rule, detail: String| violations.push(Violation { rule, detail });

    if input.starts_with('\u{feff}') != output.starts_with('\u{feff}') {
        add(
            Rule::BomChanged,
            if input.starts_with('\u{feff}') {
                "the byte-order mark was removed".to_string()
            } else {
                "a byte-order mark was added".to_string()
            },
        );
    }
    let (before, after) = (line_endings(input), line_endings(output));
    let changed = matches!(
        (before, after),
        (LineEndings::Lf, LineEndings::Crlf | LineEndings::Mixed)
            | (LineEndings::Crlf, LineEndings::Lf | LineEndings::Mixed)
    );
    if changed {
        add(
            Rule::LineEndingsChanged,
            format!("{:?} line endings became {:?}", before, after),
        );
    }

    let old = parts(input, edition);
    let new = parts(output, edition);
    if new.errors > old.errors {
        add(
            Rule::OutputBroken,
            format!(
                "the output has {} syntax error(s), the input {}",
                new.errors, old.errors
            ),
        );
    }
    let (old_header, new_header) = (by_scope(&old.header), by_scope(&new.header));
    if old_header != new_header {
        let trimmed = |h: &[String]| {
            h.iter()
                .map(|e| e.trim_end().to_string())
                .collect::<Vec<_>>()
        };
        let detail = if trimmed(&old_header) == trimmed(&new_header) {
            format!(
                "header differs only in trailing whitespace: {}",
                describe_list(&old_header)
            )
        } else {
            format!(
                "header was {}, is {}",
                describe_list(&old_header),
                describe_list(&new_header)
            )
        };
        add(Rule::HeaderChanged, detail);
    } else if old.header_first && !new.header_first {
        add(
            Rule::HeaderChanged,
            "a header element now comes after an item".to_string(),
        );
    }

    // Items: a multiset by exact text. A lost and an added item with the same core are one
    // item whose attached comments changed.
    let full =
        |items: &[(String, String)]| items.iter().map(|(f, _)| f.clone()).collect::<Vec<_>>();
    let (mut lost, mut added) = difference(&full(&old.items), &full(&new.items));
    let core_of = |text: &str, items: &[(String, String)]| {
        items
            .iter()
            .find(|(f, _)| f == text)
            .map(|(_, c)| c.clone())
            .unwrap_or_default()
    };
    let mut reattached = Vec::new();
    lost.retain(|l| {
        let core = core_of(l, &old.items);
        match added.iter().position(|a| core_of(a, &new.items) == core) {
            Some(at) => {
                added.remove(at);
                reattached.push(core);
                false
            }
            None => true,
        }
    });
    // Items are shown by their own code, not by the comments attached above them.
    let cores = |texts: &[String], items: &[(String, String)]| -> Vec<String> {
        texts.iter().map(|t| core_of(t, items)).collect()
    };
    if !lost.is_empty() {
        add(
            Rule::ItemLost,
            format!(
                "{} item(s) lost or changed: {}",
                lost.len(),
                describe_list(&cores(&lost, &old.items))
            ),
        );
    }
    if !added.is_empty() {
        add(
            Rule::ItemAdded,
            format!(
                "{} item(s) not in the input: {}",
                added.len(),
                describe_list(&cores(&added, &new.items))
            ),
        );
    }
    if !reattached.is_empty() {
        add(
            Rule::CommentReattached,
            format!(
                "{} item(s) gained or lost attached comments: {}",
                reattached.len(),
                describe_list(&reattached)
            ),
        );
    }

    let (lost, added) = difference(&old.comments, &new.comments);
    if !lost.is_empty() {
        add(
            Rule::CommentLost,
            format!("{} comment(s) lost: {}", lost.len(), describe_list(&lost)),
        );
    }
    if !added.is_empty() {
        add(
            Rule::CommentAdded,
            format!(
                "{} comment(s) added: {}",
                added.len(),
                describe_list(&added)
            ),
        );
    }
    let (lost, added) = difference(&old.others, &new.others);
    if !lost.is_empty() || !added.is_empty() {
        add(
            Rule::OtherChanged,
            format!(
                "lost {}, added {}",
                describe_list(&lost),
                describe_list(&added)
            ),
        );
    }
    let (old_macros, new_macros) = (by_scope(&old.macros), by_scope(&new.macros));
    if old_macros != new_macros {
        add(
            Rule::MacroOrderChanged,
            format!(
                "macro_rules! order was {}, is {}",
                describe_list(&old_macros),
                describe_list(&new_macros)
            ),
        );
    }
    let moved = moved_above_definitions(&old.sequence, &new.sequence);
    if !moved.is_empty() {
        add(
            Rule::MacroUserMoved,
            format!(
                "{} item(s) moved above a macro definition they came after: {}",
                moved.len(),
                describe_list(&moved)
            ),
        );
    }
    violations
}

/// A few entries, each cut to its first line, for a violation's detail.
pub(super) fn describe_list(entries: &[String]) -> String {
    if entries.is_empty() {
        return "nothing".to_string();
    }
    let mut shown: Vec<String> = entries
        .iter()
        .take(3)
        .map(|e| {
            let line = e
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim();
            let cut: String = line.chars().take(60).collect();
            if cut.chars().count() < line.chars().count() {
                format!("`{}...`", cut)
            } else {
                format!("`{}`", cut)
            }
        })
        .collect();
    if entries.len() > 3 {
        shown.push(format!("and {} more", entries.len() - 3));
    }
    shown.join(", ")
}

/// (in `a` but not `b`, in `b` but not `a`), counting repeats.
pub(super) fn difference(a: &[String], b: &[String]) -> (Vec<String>, Vec<String>) {
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    for x in a {
        *counts.entry(x.as_str()).or_insert(0) += 1;
    }
    for x in b {
        *counts.entry(x.as_str()).or_insert(0) -= 1;
    }
    let mut only_a = Vec::new();
    let mut only_b = Vec::new();
    for (text, n) in counts {
        for _ in 0..n.max(0) {
            only_a.push(text.to_string());
        }
        for _ in 0..(-n).max(0) {
            only_b.push(text.to_string());
        }
    }
    (only_a, only_b)
}

/// The entries of each scope in order, scopes sorted: what is compared when only the order
/// within a scope matters.
fn by_scope(entries: &[(String, String)]) -> Vec<String> {
    let mut sorted: Vec<&(String, String)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted.into_iter().map(|(_, e)| e.clone()).collect()
}

/// The parts of one scope: the file, or the body of an inline module. A module with a body
/// counts in its scope as its shell, `mod name {…}`; its body is a scope of its own, whose
/// entries carry the scope's name so that nothing can move between modules unnoticed.
fn collect(container: &SyntaxNode, scope: &str, source: &str, parts: &mut Parts) {
    let suffix = if scope.is_empty() {
        String::new()
    } else {
        format!("  (in {})", scope)
    };
    let mut seen_item = false;
    for element in container.children_with_tokens() {
        let (kind, text) = match &element {
            SyntaxElement::Node(node) => {
                let range = node.text_range();
                (
                    node.kind(),
                    source[range.start as usize..range.end as usize].to_string(),
                )
            }
            SyntaxElement::Token(token) => (token.kind(), token.text().to_string()),
        };
        let header = match &element {
            SyntaxElement::Token(_) => matches!(
                kind,
                SyntaxKind::Shebang
                    | SyntaxKind::Frontmatter
                    | SyntaxKind::InnerLineDoc
                    | SyntaxKind::InnerBlockDoc
            ),
            SyntaxElement::Node(node) => {
                kind == SyntaxKind::Attr && Attr::cast(node.clone()).is_some_and(|a| a.is_inner())
            }
        };
        if header {
            parts
                .header
                .push((scope.to_string(), format!("{}{}", text, suffix)));
            if seen_item {
                parts.header_first = false;
            }
            continue;
        }
        if let SyntaxElement::Node(node) = &element
            && let Some(item) = Item::cast(node.clone())
        {
            seen_item = true;
            if item.kind() == ItemKind::MacroRules {
                let name = format!("{}{}", item.name().unwrap_or_default(), suffix);
                for visible in visible_scopes(scope) {
                    parts.macros.push((visible, name.clone()));
                }
            }
            let core = core_text(node, source);
            parts.sequence.push((
                scope.to_string(),
                format!("{}{}", core, suffix),
                item.defines_macros(),
            ));
            match module_body(node) {
                Some(body) => {
                    let range = body.text_range();
                    let (from, to) = (range.start as usize + 1, range.end as usize - 1);
                    let shell = |whole: &str, start: usize| {
                        format!("{}…{}", &whole[..from - start], &whole[to - start..])
                    };
                    let item_start = node.text_range().start as usize;
                    let core_start = item_start + text.find(core.as_str()).unwrap_or(0);
                    let shell_text = shell(&text, item_start);
                    let shell_core = shell(&core, core_start);
                    let inner_scope = if scope.is_empty() {
                        shell_core.split_whitespace().collect::<Vec<_>>().join(" ")
                    } else {
                        format!(
                            "{} / {}",
                            scope,
                            shell_core.split_whitespace().collect::<Vec<_>>().join(" ")
                        )
                    };
                    parts.items.push((
                        format!("{}{}", shell_text, suffix),
                        format!("{}{}", shell_core, suffix),
                    ));
                    let macro_use = item.attrs().iter().any(|a| a.path() == "macro_use");
                    let inner_scope = if macro_use {
                        format!("{}{}", inner_scope, MACRO_USE)
                    } else {
                        inner_scope
                    };
                    collect(&body, &inner_scope, source, parts);
                }
                None => parts
                    .items
                    .push((format!("{}{}", text, suffix), format!("{}{}", core, suffix))),
            }
            continue;
        }
        if kind.is_trivia() || matches!(kind, SyntaxKind::LBrace | SyntaxKind::RBrace) {
            continue;
        }
        parts.others.push(format!("{}{}", text, suffix));
    }
}

/// The text from a node's first significant child to its last, without the comments and
/// whitespace attached around it. Attached comments are direct children, so only the
/// node's own children are looked at.
fn core_text(node: &SyntaxNode, text: &str) -> String {
    let children = node.children_with_tokens();
    let significant = |e: &&SyntaxElement| match e {
        SyntaxElement::Token(t) => !t.kind().is_trivia(),
        SyntaxElement::Node(_) => true,
    };
    let (Some(first), Some(last)) = (
        children.iter().find(significant),
        children.iter().rfind(significant),
    ) else {
        return String::new();
    };
    let from = first.text_range().start as usize;
    let to = last.text_range().end as usize;
    text[from..to].to_string()
}

fn line_endings(text: &str) -> LineEndings {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count();
    match (crlf, lf) {
        (_, 0) => LineEndings::None,
        (0, _) => LineEndings::Lf,
        (c, l) if c == l => LineEndings::Crlf,
        _ => LineEndings::Mixed,
    }
}

/// The `{ ... }` body of an inline module or an impl block: a scope of its own.
fn module_body(node: &SyntaxNode) -> Option<SyntaxNode> {
    let body = match node.kind() {
        SyntaxKind::Module => SyntaxKind::ItemList,
        SyntaxKind::Impl => SyntaxKind::AssocItemList,
        _ => return None,
    };
    node.children().into_iter().find(|c| c.kind() == body)
}

/// The items that came after a macro-defining item in the input and come before it in the
/// output, scope by scope. Items are known by their core text, numbered when it repeats.
fn moved_above_definitions(
    old: &[(String, String, bool)],
    new: &[(String, String, bool)],
) -> Vec<String> {
    // Each item as (scope, text, occurrence), with whether it defines macros.
    fn numbered(sequence: &[(String, String, bool)]) -> Vec<((&str, &str, usize), bool)> {
        let mut seen: HashMap<(&str, &str), usize> = HashMap::new();
        sequence
            .iter()
            .map(|(scope, text, definer)| {
                let n = seen.entry((scope.as_str(), text.as_str())).or_default();
                *n += 1;
                ((scope.as_str(), text.as_str(), *n), *definer)
            })
            .collect()
    }
    let (old, new) = (numbered(old), numbered(new));
    let position: HashMap<_, usize> = new.iter().enumerate().map(|(i, (k, _))| (*k, i)).collect();
    let mut moved = Vec::new();
    for (i, (definer, defines)) in old.iter().enumerate() {
        let Some(&at) = position.get(definer).filter(|_| *defines) else {
            continue;
        };
        for (later, _) in &old[i + 1..] {
            if later.0 == definer.0
                && position.get(later).is_some_and(|&p| p < at)
                && !moved.contains(&later.1.to_string())
            {
                moved.push(later.1.to_string());
            }
        }
    }
    moved
}

/// The parts of a file. Line endings are compared by their own rule, so here every
/// `\r\n` reads as `\n` and one changed line ending is not also every item changed.
fn parts(text: &str, edition: Edition) -> Parts {
    let text = text.replace("\r\n", "\n");
    let source = text.clone();
    let parsed = parse(&text, edition);
    let tree = parsed.syntax_node();
    let mut parts = Parts {
        errors: parsed.errors().len(),
        header_first: true,
        ..Parts::default()
    };
    // Comments straight from the lexer: much cheaper than walking every token of the tree.
    let mut offset = 0;
    for token in tokenize(&text, edition) {
        let end = offset + token.len as usize;
        if matches!(
            token.kind,
            SyntaxKind::LineComment | SyntaxKind::BlockComment
        ) {
            parts.comments.push(text[offset..end].to_string());
        }
        offset = end;
    }
    collect(&tree, "", &source, &mut parts);
    parts
}

/// The scopes a macro defined in `scope` is visible in: its own, and its parents' for as
/// long as the modules between are `#[macro_use]`.
fn visible_scopes(scope: &str) -> Vec<String> {
    let mut scopes = vec![scope.to_string()];
    let mut current = scope;
    while current.ends_with(MACRO_USE) {
        current = match current.rfind(" / ") {
            Some(at) => &current[..at],
            None => "",
        };
        scopes.push(current.to_string());
    }
    scopes
}

#[cfg(test)]
mod tests;
