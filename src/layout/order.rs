//! The order units are arranged in. Pinned units keep their positions; the rest are sorted
//! into the positions left, by:
//!
//! 1. `#[cfg(test)]` modules last;
//! 2. imports, when `sort_imports` is on: by group (standard library, external, this crate)
//!    when `import_grouping` is on, then `self`, `super`, `crate` paths, then by path;
//! 3. `type_order`, with an impl ranked as the type it is for when `group_impls` is on;
//! 4. with `group_impls`, a type's impls next to it, types ordered by name;
//! 5. `type_order` of the unit itself;
//! 6. `macro_rules!` and `#[macro_use]` modules in their original order;
//! 7. modules by name, ignoring visibility, as rustfmt orders them;
//! 8. visibility, then name when `sort_alphabetically` is on.
//!
//! A `macro_rules!` is only visible below its definition, so no unit that came after a
//! definition may move in front of it. If the sort would do that, it is redone with every
//! macro first; if even that moves one, the file is left alone.

use std::cmp::Ordering;
use std::collections::HashSet;

use super::Unit;
use crate::options::layout::{ItemKind, LayoutConfig};

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
        let (macro_a, macro_b) = (a.kind == ItemKind::MacroDecl, b.kind == ItemKind::MacroDecl);
        if macro_a != macro_b {
            return macro_b.cmp(&macro_a);
        }
    }
    if a.test_module != b.test_module {
        return a.test_module.cmp(&b.test_module);
    }
    if a.kind == ItemKind::UseDecl && b.kind == ItemKind::UseDecl && config.sort_imports {
        if config.import_grouping && a.import_group != b.import_group {
            return a.import_group.cmp(&b.import_group);
        }
        return (import_path_rank(&a.identifier), &a.identifier)
            .cmp(&(import_path_rank(&b.identifier), &b.identifier));
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
                    | ItemKind::TraitDecl
                    | ItemKind::ImplBlock
            )
        };
        if groupable(&a.kind) && groupable(&b.kind) && family_a != family_b {
            return family_a.cmp(&family_b);
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
            return a.identifier.cmp(&b.identifier);
        }
        return Ordering::Equal;
    }
    if a.visibility != b.visibility {
        return a.visibility.cmp(&b.visibility);
    }
    if config.sort_alphabetically {
        a.identifier.cmp(&b.identifier)
    } else {
        Ordering::Equal
    }
}

/// `self` paths first, then `super`, then `crate`, then everything else.
fn import_path_rank(path: &str) -> u8 {
    match path
        .trim_start_matches("::")
        .split("::")
        .next()
        .unwrap_or("")
    {
        "self" => 0,
        "super" => 1,
        "crate" => 2,
        _ => 3,
    }
}

/// True if some unit that came after a `macro_rules!` now comes before it.
fn macro_moved_past_user(units: &[Unit]) -> bool {
    let mut latest: Option<usize> = None;
    for unit in units {
        if unit.kind == ItemKind::MacroDecl && latest.is_some_and(|l| l > unit.index) {
            return true;
        }
        latest = latest.max(Some(unit.index));
    }
    false
}

/// Pinned units at their own positions, the others sorted into the remaining ones.
fn sorted(units: &[Unit], config: &LayoutConfig, macros_first: bool) -> Vec<Unit> {
    let families: Vec<(String, ItemKind)> = units
        .iter()
        .filter(|u| {
            matches!(
                u.kind,
                ItemKind::StructDecl | ItemKind::EnumDecl | ItemKind::TraitDecl
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
