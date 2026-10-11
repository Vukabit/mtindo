//! What layout needs to know about an item, read from its syntax tree: kind, visibility,
//! the name it sorts by, the type an impl belongs to, and the attributes that pin modules.

use super::Unit;
use super::imports::UseTree;
use crate::options::layout::{ImportGroup, ItemKind, LayoutConfig, Visibility};
use crate::syntax::ast::{self, Attr, Item};
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode, parse};

/// The import group of the first `use` in `text` (`Std`, `External` or `Local`), for a
/// snippet such as `pub use std::fmt;`. Text without a `use` counts as `External`.
pub fn import_group_of_text(text: &str) -> ImportGroup {
  let tree = parse(text, Edition::E2021).syntax_node();
  tree
    .children()
    .into_iter()
    .filter_map(Item::cast)
    .find_map(|item| item.as_use())
    .map(|view| group_of_root(view.root().as_deref()))
    .unwrap_or(ImportGroup::External)
}

/// A `// mto: off` ... `// mto: on` region, `text[start..end]`: never moved, never looked
/// into.
pub(super) fn pinned_region(text: &str, start: usize, end: usize, index: usize) -> Unit {
  Unit {
    kind: ItemKind::Unknown,
    visibility: Visibility::Private,
    identifier: "SkippedBlock".to_string(),
    parent_target: String::new(),
    trait_name: None,
    text: text[start..end].to_string(),
    span: (start, end),
    pinned: true,
    macro_call: false,
    index,
    cfg: false,
    test_module: false,
    macro_use: false,
    // A region left as written may hold definitions too.
    defines_macros: text[start..end].contains("macro_rules!"),
    module_declaration: false,
    import_group: None,
    use_tree: None,
  }
}

/// The unit for `item`, whose exact text (with the loose comments above it) is `text`.
pub(super) fn unit(item: &Item, text: String, index: usize, config: &LayoutConfig) -> Unit {
  let kind = match item.kind() {
    ast::ItemKind::Const => ItemKind::ConstDecl,
    ast::ItemKind::Enum => ItemKind::EnumDecl,
    ast::ItemKind::Fn => ItemKind::FnDecl,
    ast::ItemKind::Impl => ItemKind::ImplBlock,
    ast::ItemKind::MacroRules => ItemKind::MacroDecl,
    ast::ItemKind::Module => ItemKind::ModDecl,
    ast::ItemKind::Static => ItemKind::StaticDecl,
    ast::ItemKind::Struct => ItemKind::StructDecl,
    ast::ItemKind::Trait => ItemKind::TraitDecl,
    ast::ItemKind::Use => ItemKind::UseDecl,
    ast::ItemKind::MacroDef => ItemKind::MacroDef,
    ast::ItemKind::TypeAlias => ItemKind::TypeAlias,
    ast::ItemKind::Union => ItemKind::Union,
    ast::ItemKind::ExternBlock => ItemKind::ExternBlock,
    _ => ItemKind::Unknown,
  };
  let attrs = item.attrs();
  let pinned = matches!(
    item.kind(),
    ast::ItemKind::ExternCrate | ast::ItemKind::MacroCall
  ) || attrs.iter().any(|a| a.is_rustfmt_skip());
  let visibility = match item.visibility() {
    ast::Visibility::Pub => Visibility::Pub,
    ast::Visibility::PubCrate => Visibility::PubCrate,
    ast::Visibility::PubSuper => Visibility::PubSuper,
    // The standard formatter writes `pub(in crate)` as `pub(crate)` and `pub(in super)` as
    // `pub(super)`, so they must sort alike.
    ast::Visibility::PubIn(path) if path == "crate" => Visibility::PubCrate,
    ast::Visibility::PubIn(path) if path == "super" => Visibility::PubSuper,
    ast::Visibility::PubIn(path) if path == "self" => Visibility::Private,
    ast::Visibility::PubIn(_) => Visibility::PubIn,
    ast::Visibility::Private | ast::Visibility::PubSelf => Visibility::Private,
  };
  let (identifier, parent_target) = match (item.as_impl(), item.as_use()) {
    (Some(view), _) => {
      let target = view.self_ty_name().unwrap_or_default();
      match view.trait_name() {
        Some(name) => (format!("{} for {}", name, target), target),
        None => (target.clone(), target),
      }
    }
    (_, Some(view)) => (
      view
        .tree()
        .map(|tree| significant_text(&tree))
        .unwrap_or_default(),
      String::new(),
    ),
    _ => (item.name().unwrap_or_default(), String::new()),
  };
  let is_module = kind == ItemKind::ModDecl;
  Unit {
    kind,
    visibility,
    identifier,
    parent_target,
    trait_name: item.as_impl().and_then(|view| view.trait_name()),
    text,
    span: (0, 0),
    pinned,
    macro_call: item.kind() == ast::ItemKind::MacroCall,
    index,
    cfg: attrs.iter().any(|a| a.path() == "cfg"),
    test_module: is_module && attrs.iter().any(|a| attr_body(a) == "cfg(test)"),
    macro_use: is_module && attrs.iter().any(|a| a.path() == "macro_use"),
    defines_macros: item.defines_macros(),
    module_declaration: is_module && ends_with_semicolon(item.syntax()),
    import_group: item
      .as_use()
      .map(|view| group_of_root(view.root().as_deref())),
    use_tree: item.as_use().and_then(|view| view.tree()).map(|tree| {
      UseTree::from_syntax(
        &tree,
        config.edition >= 2018,
        config.version_sorted_imports(),
      )
    }),
  }
}

/// An attribute without `#[`, `]` and spaces: `cfg(test)`.
fn attr_body(attr: &Attr) -> String {
  let text = significant_text(attr.syntax());
  text
    .trim_start_matches('#')
    .trim_start_matches('[')
    .trim_end_matches(']')
    .to_string()
}

fn ends_with_semicolon(node: &SyntaxNode) -> bool {
  node
    .descendants_with_tokens()
    .into_iter()
    .rev()
    .find_map(|e| match e {
      SyntaxElement::Token(t) if !t.kind().is_trivia() => Some(t.kind()),
      _ => None,
    })
    == Some(SyntaxKind::Semi)
}

fn group_of_root(root: Option<&str>) -> ImportGroup {
  match root {
    Some("std" | "core" | "alloc") => ImportGroup::Std,
    Some("crate" | "super" | "self") => ImportGroup::Local,
    _ => ImportGroup::External,
  }
}

/// A node's text without whitespace and comments: `std::{fs,io}`.
fn significant_text(node: &SyntaxNode) -> String {
  node
    .descendants_with_tokens()
    .into_iter()
    .filter_map(|e| match e {
      SyntaxElement::Token(t) if !t.kind().is_trivia() => Some(t.text().to_string()),
      _ => None,
    })
    .collect()
}
