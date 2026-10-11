//! Ordering inside impl blocks, both optional:
//!
//! - **`reorder_impl_items`** (the standard formatter's setting, honored as the standard
//!   formatter applies it): associated types first (plain ones, then `impl Trait` ones,
//!   each by name), then consts by name, then macro calls, then functions in their order;
//!   items of different kinds, and functions, are a blank line apart, while consecutive
//!   types or consts are not;
//! - **`impl_method_order = "trait"`**: in an impl of a trait declared in the same file
//!   (under one name only), the functions follow the trait's order. Only the functions swap
//!   places; everything else, blank lines included, stays where it was.
//!
//! Pinned items (`// mto: pin`, `#[rustfmt::skip]`, `// mto: off` regions) keep their
//! places, as everywhere.

use std::cmp::Ordering;
use std::collections::HashMap;

use super::{Unit, split_container};
use crate::options::layout::{ItemKind, LayoutConfig};
use crate::syntax::ast::Item;
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

/// Each trait declared in the file, by name, with its functions in order. A name declared
/// twice (in two modules) is left out: which one an impl means is not known.
pub(super) type Traits = HashMap<String, Option<Vec<String>>>;

pub(super) fn collect_traits(tree: &SyntaxNode) -> Traits {
  let mut traits = Traits::new();
  for node in tree.descendants() {
    if node.kind() != SyntaxKind::Trait {
      continue;
    }
    let Some(item) = Item::cast(node.clone()) else {
      continue;
    };
    let Some(name) = item.name() else {
      continue;
    };
    let functions: Vec<String> = assoc_list(&node)
      .map(|list| {
        list
          .children()
          .into_iter()
          .filter(|c| c.kind() == SyntaxKind::Fn)
          .filter_map(|f| Item::cast(f).and_then(|i| i.name()))
          .collect()
      })
      .unwrap_or_default();
    traits
      .entry(name)
      .and_modify(|seen| *seen = None)
      .or_insert(Some(functions));
  }
  traits
}

/// The text of an impl block with its items reordered, or `None` when nothing applies or
/// nothing would change.
pub(super) fn impl_with_items_ordered(
  item: &Item,
  text: &str,
  apart: &str,
  config: &LayoutConfig,
  traits: &Traits,
) -> Option<String> {
  let node = item.syntax();
  let view = item.as_impl()?;
  let trait_order: Option<&Vec<String>> = if config.impl_method_order_trait {
    view
      .trait_name()
      .and_then(|name| traits.get(&name))
      .and_then(Option::as_ref)
  } else {
    None
  };
  if !config.reorder_impl_items && trait_order.is_none() {
    return None;
  }
  let body = assoc_list(node)?;
  let range = body.text_range();
  let (open_end, close_start) = (range.start as usize + 1, range.end as usize - 1);
  let elements = body.children_with_tokens();
  let significant = |e: &SyntaxElement| {
    !matches!(
      e.kind(),
      SyntaxKind::Whitespace | SyntaxKind::LBrace | SyntaxKind::RBrace
    )
  };
  let first = elements.iter().find(|e| significant(e))?.text_range().start as usize;
  let leading = &text[open_end..first];
  if !leading.contains('\n') {
    return None;
  }
  let indent = &leading[leading.rfind('\n')? + 1..];
  let inner_apart = format!("{}{}", apart.trim_end_matches(|c| c != '\n'), indent);
  let split = split_container(&body, text, open_end, &inner_apart, config, traits).ok()?;
  let mut units = split.units;
  if units.len() < 2 {
    return None;
  }
  if config.reorder_impl_items {
    // The standard formatter moves macro calls in impls like any other item.
    for unit in units.iter_mut() {
      if unit.macro_call {
        unit.pinned = false;
      }
    }
  }
  let rank = |name: &str| {
    trait_order
      .and_then(|order| order.iter().position(|n| n == name))
      .unwrap_or(usize::MAX)
  };
  let fn_order = |a: &Unit, b: &Unit| {
    rank(&a.identifier)
      .cmp(&rank(&b.identifier))
      .then(a.index.cmp(&b.index))
  };
  let arranged = if config.reorder_impl_items {
    let key = |u: &Unit| impl_item_rank(u, text);
    place(&units, |a, b| {
      let (ka, kb) = (key(a), key(b));
      ka.cmp(&kb).then_with(|| match ka {
        // Types and consts by name, as the standard formatter sorts them; functions
        // by the trait.
        0..=2 => a.identifier.cmp(&b.identifier),
        5 => fn_order(a, b),
        _ => Ordering::Equal,
      })
    })
  } else {
    // Functions trade places among the functions' slots only.
    let is_fn = |u: &Unit| u.kind == ItemKind::FnDecl && !u.pinned;
    let mut functions: Vec<Unit> = units.iter().filter(|u| is_fn(u)).cloned().collect();
    functions.sort_by(fn_order);
    let mut functions = functions.into_iter();
    units
      .iter()
      .map(|u| {
        if is_fn(u) {
          functions.next().unwrap_or_else(|| u.clone())
        } else {
          u.clone()
        }
      })
      .collect()
  };
  if arranged
    .iter()
    .zip(units.iter())
    .all(|(a, b)| a.index == b.index)
  {
    return None;
  }
  let mut out = String::new();
  let node_range = node.text_range();
  out.push_str(&text[node_range.start as usize..units[0].span.0]);
  for (slot, unit) in arranged.iter().enumerate() {
    if slot > 0 {
      if config.reorder_impl_items {
        let prev = &arranged[slot - 1];
        let (kp, ku) = (impl_item_rank(prev, text), impl_item_rank(unit, text));
        let tight = kp == ku && kp <= 2;
        out.push('\n');
        if !tight {
          out.push('\n');
        }
        out.push_str(indent);
      } else {
        // The gap that was before this slot.
        out.push_str(&text[units[slot - 1].span.1..units[slot].span.0]);
      }
    }
    out.push_str(&unit.text);
  }
  let last_end = units.last()?.span.1;
  out.push_str(&text[last_end..close_start]);
  out.push_str(&text[close_start..node_range.end as usize]);
  Some(out)
}

fn assoc_list(node: &SyntaxNode) -> Option<SyntaxNode> {
  node
    .children()
    .into_iter()
    .find(|c| c.kind() == SyntaxKind::AssocItemList)
}

/// The unit's place in the standard formatter's `reorder_impl_items` order: a plain
/// associated type 0, an `impl Trait` one 1, a const 2, a macro call 3, anything else 4, a
/// function 5.
fn impl_item_rank(unit: &Unit, text: &str) -> u8 {
  match unit.kind {
    ItemKind::TypeAlias => {
      let item_text = &text[unit.span.0..unit.span.1];
      let opaque = item_text
        .split_once('=')
        .is_some_and(|(_, ty)| ty.trim_start().starts_with("impl "));
      u8::from(opaque)
    }
    ItemKind::ConstDecl => 2,
    _ if unit.macro_call => 3,
    ItemKind::FnDecl => 5,
    _ => 4,
  }
}

/// The units with the pinned ones in their places and the rest sorted by `compare` into
/// the places left.
fn place(units: &[Unit], compare: impl Fn(&Unit, &Unit) -> Ordering) -> Vec<Unit> {
  let mut free: Vec<Unit> = units.iter().filter(|u| !u.pinned).cloned().collect();
  free.sort_by(|a, b| compare(a, b));
  let mut free = free.into_iter();
  units
    .iter()
    .map(|u| {
      if u.pinned {
        u.clone()
      } else {
        free.next().unwrap_or_else(|| u.clone())
      }
    })
    .collect()
}
