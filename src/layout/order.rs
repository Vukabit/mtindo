//! The order units are arranged in. Pinned units keep their positions; the rest are sorted
//! into the positions left, by:
//!
//! 1. `#[cfg(test)]` modules last;
//! 2. imports, when `sort_imports` is on: by group (standard library, external, this crate)
//!    when `import_grouping` is on, then as the standard formatter sorts them
//!    ([`super::imports`]); when it is off, in the order they were written;
//! 3. `type_order`, with an impl ranked as the type it is for when `group_impls` is on;
//! 4. with `group_impls`, a type's impls next to it, types ordered by name; impls of one
//!    type: its own first (as written), then trait impls by `trait_impl_order` and
//!    `trait_impl_sort`;
//! 5. `type_order` of the unit itself;
//! 6. `macro_rules!` and `#[macro_use]` modules in their original order;
//! 7. modules by name, ignoring visibility, as the standard formatter orders them (by bytes
//!    through style edition 2024, by version after it);
//! 8. visibility in `visibility_order`, then name in `name_sort` order when
//!    `sort_alphabetically` is on.
//!
//! A `macro_rules!` is only visible below its definition, and so are the macros of a
//! `#[macro_use]` module and of an item macro call that holds definitions (`cfg_if! {
//! macro_rules! m { ... } }`), so no unit that came after a definition may move in front of
//! it. If the sort would do that, it is redone with every
//! macro first; if even that moves one, the file is left alone.

use std::cmp::Ordering;
use std::collections::HashSet;

use super::Unit;
use super::imports::version_sort;
use crate::options::layout::{ItemKind, LayoutConfig, NameSort, TraitImplSort, Visibility};

/// The units in their new order.
pub(super) fn arrange(units: &[Unit], config: &LayoutConfig) -> Result<Vec<Unit>, String> {
  let mut arranged = sorted(units, config, false);
  if macro_moved_past_user(&arranged) {
    arranged = sorted(units, config, true);
  }
  if macro_moved_past_user(&arranged) {
    return Err("cannot keep macro definitions ahead of the items after them".to_string());
  }
  Ok(arranged)
}

fn compare(
  a: &Unit,
  b: &Unit,
  config: &LayoutConfig,
  families: &[(String, ItemKind)],
  macros_first: bool,
) -> Ordering {
  if macros_first {
    let (macro_a, macro_b) = (defines_macros(a), defines_macros(b));
    if macro_a != macro_b {
      return macro_b.cmp(&macro_a);
    }
    if macro_a {
      // Macro definitions keep their written order among themselves.
      return a.index.cmp(&b.index);
    }
  }
  if a.test_module != b.test_module {
    return a.test_module.cmp(&b.test_module);
  }
  if a.kind == ItemKind::UseDecl && b.kind == ItemKind::UseDecl && !config.sort_imports {
    // Unsorted imports keep their written order; sorting them by name would read
    // names from text that the standard formatter rewrites, so the order would not settle.
    return a.index.cmp(&b.index);
  }
  if a.kind == ItemKind::UseDecl && b.kind == ItemKind::UseDecl && config.sort_imports {
    if config.import_grouping && a.import_group != b.import_group {
      return a.import_group.cmp(&b.import_group);
    }
    let version = config.version_sorted_imports();
    return match (&a.use_tree, &b.use_tree) {
      (Some(x), Some(y)) => x.compare(y, version),
      _ => a.identifier.cmp(&b.identifier),
    };
  }
  let family = |unit: &Unit| -> String {
    if config.group_impls && unit.kind == ItemKind::ImplBlock {
      unit.parent_target.clone()
    } else {
      unit.identifier.clone()
    }
  };
  let (family_a, family_b) = (family(a), family(b));
  let family_kind = |unit: &Unit, name: &str| -> ItemKind {
    if config.group_impls
      && unit.kind == ItemKind::ImplBlock
      && let Some((_, kind)) = families.iter().find(|(n, _)| n == name)
    {
      return kind.clone();
    }
    unit.kind.clone()
  };
  let rank = |kind: &ItemKind| {
    config
      .type_order
      .iter()
      .position(|k| k == kind)
      .unwrap_or(99)
  };
  let (rank_a, rank_b) = (
    rank(&family_kind(a, &family_a)),
    rank(&family_kind(b, &family_b)),
  );
  if rank_a != rank_b {
    return rank_a.cmp(&rank_b);
  }
  if config.group_impls {
    let groupable = |k: &ItemKind| {
      matches!(
        k,
        ItemKind::StructDecl
          | ItemKind::EnumDecl
          | ItemKind::Union
          | ItemKind::TraitDecl
          | ItemKind::ImplBlock
      )
    };
    if groupable(&a.kind) && groupable(&b.kind) && family_a != family_b {
      return names(&family_a, &family_b, config);
    }
  }
  let (own_a, own_b) = (rank(&a.kind), rank(&b.kind));
  if own_a != own_b {
    return own_a.cmp(&own_b);
  }
  // Macros are scoped by textual order, so they never change places with each other.
  if a.kind == ItemKind::MacroDecl && b.kind == ItemKind::MacroDecl {
    return a.index.cmp(&b.index);
  }
  if a.kind == ItemKind::ModDecl && b.kind == ItemKind::ModDecl {
    // A #[macro_use] module stays ahead of the modules that rely on its macros.
    if a.macro_use != b.macro_use {
      return b.macro_use.cmp(&a.macro_use);
    }
    if a.macro_use {
      return a.index.cmp(&b.index);
    }
    if config.sort_alphabetically {
      // As the standard formatter orders `mod` declarations: by bytes through style
      // edition 2024. The standard formatter compares the names without `r#`.
      let (x, y) = (
        a.identifier.trim_start_matches("r#"),
        b.identifier.trim_start_matches("r#"),
      );
      return if config.style_edition > 2024 {
        version_sort(x, y)
      } else {
        x.cmp(y)
      };
    }
    return Ordering::Equal;
  }
  if a.kind == ItemKind::ImplBlock && b.kind == ItemKind::ImplBlock {
    return impls(a, b, config);
  }
  if a.visibility != b.visibility {
    let rank = |v: &Visibility| {
      config
        .visibility_order
        .iter()
        .position(|o| o == v)
        .unwrap_or(usize::MAX)
    };
    return rank(&a.visibility).cmp(&rank(&b.visibility));
  }
  if config.sort_alphabetically {
    names(&a.identifier, &b.identifier, config)
  } else {
    Ordering::Equal
  }
}

/// A `macro_rules!`, or a `#[macro_use]` module, whose macros are visible after it.
fn defines_macros(unit: &Unit) -> bool {
  unit.defines_macros
}

/// Pinned units at their own positions, the others sorted into the remaining ones.
/// Two impl blocks: by the type they are for, then the type's own impls (as written), then
/// trait impls: those in `trait_impl_order` first, in its order, then by trait name or as
/// written (`trait_impl_sort`).
fn impls(a: &Unit, b: &Unit, config: &LayoutConfig) -> Ordering {
  if a.parent_target != b.parent_target {
    return names(&a.parent_target, &b.parent_target, config);
  }
  let (trait_a, trait_b) = match (&a.trait_name, &b.trait_name) {
    (None, None) => return a.index.cmp(&b.index),
    (None, Some(_)) => return Ordering::Less,
    (Some(_), None) => return Ordering::Greater,
    (Some(x), Some(y)) => (x, y),
  };
  let listed = |name: &str| {
    config
      .trait_impl_order
      .iter()
      .position(|t| t == name)
      .unwrap_or(usize::MAX)
  };
  listed(trait_a)
    .cmp(&listed(trait_b))
    .then_with(|| match config.trait_impl_sort {
      TraitImplSort::Name if config.sort_alphabetically => {
        names(trait_a, trait_b, config).then_with(|| names(&a.identifier, &b.identifier, config))
      }
      _ => Ordering::Equal,
    })
    .then_with(|| a.index.cmp(&b.index))
}

/// True if some unit that came after a `macro_rules!` now comes before it.
fn macro_moved_past_user(units: &[Unit]) -> bool {
  let mut latest: Option<usize> = None;
  for unit in units {
    if defines_macros(unit) && latest.is_some_and(|l| l > unit.index) {
      return true;
    }
    latest = latest.max(Some(unit.index));
  }
  false
}

/// Two names in the order of `name_sort`.
fn names(a: &str, b: &str, config: &LayoutConfig) -> Ordering {
  match config.name_sort {
    NameSort::Version => version_sort(a, b),
    NameSort::Bytes => a.cmp(b),
    NameSort::CaseInsensitive => {
      version_sort(&a.to_lowercase(), &b.to_lowercase()).then_with(|| version_sort(a, b))
    }
  }
}

fn sorted(units: &[Unit], config: &LayoutConfig, macros_first: bool) -> Vec<Unit> {
  let families: Vec<(String, ItemKind)> = units
    .iter()
    .filter(|u| {
      matches!(
        u.kind,
        ItemKind::StructDecl | ItemKind::EnumDecl | ItemKind::Union | ItemKind::TraitDecl
      )
    })
    .map(|u| (u.identifier.clone(), u.kind.clone()))
    .collect();
  let mut free: Vec<Unit> = units.iter().filter(|u| !u.pinned).cloned().collect();
  free.sort_by(|a, b| compare(a, b, config, &families, macros_first));
  let pinned: HashSet<usize> = units.iter().filter(|u| u.pinned).map(|u| u.index).collect();
  let mut free = free.into_iter();
  units
    .iter()
    .enumerate()
    .filter_map(|(position, unit)| {
      if pinned.contains(&position) {
        Some(unit.clone())
      } else {
        free.next()
      }
    })
    .collect()
}
