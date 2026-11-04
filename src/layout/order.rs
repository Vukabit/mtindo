//! The order units are arranged in. Pinned units keep their positions; the rest are sorted
//! into the positions left, by:
//!
//! 1. `#[cfg(test)]` modules last;
//! 2. imports, when `sort_imports` is on: by group (standard library, external, this crate)
//!    when `import_grouping` is on, then as the standard formatter sorts them
//!    ([`super::imports`]); when it is off, in the order they were written;
//! 3. `type_order`, with an impl ranked as the type it is for when `group_impls` is on;
//! 4. with `group_impls`, a type's impls next to it (of types sharing a name, the one placed
//!    first), types ordered by name; impls of one type: its own first (as written), then
//!    trait impls by `trait_impl_order` and `trait_impl_sort`;
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

use super::Unit;
use crate::options::layout::{
  ItemKind, LayoutConfig, NameSort, OrderKey, TraitImplSort, Visibility,
};
use crate::order::version_sort;

/// Why a unit moved: the rule that put it before the unit it now precedes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
  /// `#[cfg(test)]` modules go last.
  TestModuleLast,
  /// Macro definitions go first, so nothing ends up above a definition it uses.
  MacrosFirst,
  /// Imports in their groups (`import_grouping`).
  ImportGroups,
  /// Imports in the Rust style's order (`sort_imports`).
  Imports,
  /// The order of kinds (`type_order`).
  TypeOrder,
  /// An impl beside the type it is for, types by name (`group_impls`).
  GroupImpls,
  /// `#[macro_use]` modules first.
  MacroUseModules,
  /// Modules by name.
  Modules,
  /// Impls of one type: its own first, then trait impls (`trait_impl_order`,
  /// `trait_impl_sort`).
  ImplsOfOneType,
  /// The order of visibilities (`visibility_order`).
  Visibility,
  /// Names (`name_sort`).
  Name,
  /// Kept in the order written; others moved past it.
  Written,
  /// A pinned unit keeps its slot; others moved past it.
  Slot,
  /// Above the one item that uses it (`keep_helpers_near`).
  Helper,
  /// Items in an impl in the standard formatter's order (`reorder_impl_items`).
  ImplItems,
  /// An impl's functions in its trait's order (`impl_method_order`).
  TraitOrder,
}

impl Rule {
  /// The rule's name in JSON outputs, which does not change.
  pub fn id(self) -> &'static str {
    match self {
      Rule::TestModuleLast => "test_module_last",
      Rule::MacrosFirst => "macros_first",
      Rule::ImportGroups => "import_groups",
      Rule::Imports => "imports",
      Rule::TypeOrder => "type_order",
      Rule::GroupImpls => "group_impls",
      Rule::MacroUseModules => "macro_use_modules",
      Rule::Modules => "modules",
      Rule::ImplsOfOneType => "impls_of_one_type",
      Rule::Visibility => "visibility",
      Rule::Name => "name",
      Rule::Written => "written",
      Rule::Slot => "slot",
      Rule::Helper => "helper",
      Rule::ImplItems => "impl_items",
      Rule::TraitOrder => "trait_order",
    }
  }

  /// What the rule says, for `mto debug moves`.
  pub fn says(self) -> &'static str {
    match self {
      Rule::TestModuleLast => "a #[cfg(test)] module goes last",
      Rule::MacrosFirst => "macro definitions go first, so nothing ends up above one it uses",
      Rule::ImportGroups => "imports in groups: std, external, this crate (import_grouping)",
      Rule::Imports => "imports in the Rust style's order (sort_imports)",
      Rule::TypeOrder => "the order of kinds (type_order)",
      Rule::GroupImpls => "an impl beside its type, types by name (group_impls)",
      Rule::MacroUseModules => "#[macro_use] modules first",
      Rule::Modules => "modules by name",
      Rule::ImplsOfOneType => {
        "a type's own impls first, then trait impls (trait_impl_order, trait_impl_sort)"
      }
      Rule::Visibility => "the order of visibilities (visibility_order)",
      Rule::Name => "by name (name_sort)",
      Rule::Written => "kept as written; others moved past it",
      Rule::Slot => "pinned: keeps its slot while others move past it",
      Rule::Helper => "above the one item that uses it (keep_helpers_near)",
      Rule::ImplItems => "the standard formatter's order of impl items (reorder_impl_items)",
      Rule::TraitOrder => "the trait's order of functions (impl_method_order)",
    }
  }
}

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

pub(super) fn compare(
  a: &Unit,
  b: &Unit,
  config: &LayoutConfig,
  families: &[(String, ItemKind, Visibility)],
  macros_first: bool,
) -> Ordering {
  compare_by(a, b, config, families, macros_first).0
}

/// [`compare`], with the rule that decided it.
pub(super) fn compare_by(
  a: &Unit,
  b: &Unit,
  config: &LayoutConfig,
  families: &[(String, ItemKind, Visibility)],
  macros_first: bool,
) -> (Ordering, Rule) {
  if macros_first {
    let (macro_a, macro_b) = (defines_macros(a), defines_macros(b));
    if macro_a != macro_b {
      return (macro_b.cmp(&macro_a), Rule::MacrosFirst);
    }
    if macro_a {
      // Macro definitions keep their written order among themselves.
      return (a.index.cmp(&b.index), Rule::MacrosFirst);
    }
  }
  if a.test_module != b.test_module {
    return (a.test_module.cmp(&b.test_module), Rule::TestModuleLast);
  }
  if a.kind == ItemKind::UseDecl && b.kind == ItemKind::UseDecl && !config.sort_imports {
    // Unsorted imports keep their written order; sorting them by name would read
    // names from text that the standard formatter rewrites, so the order would not settle.
    return (a.index.cmp(&b.index), Rule::Written);
  }
  if a.kind == ItemKind::UseDecl && b.kind == ItemKind::UseDecl && config.sort_imports {
    if config.import_grouping && a.import_group != b.import_group {
      return (a.import_group.cmp(&b.import_group), Rule::ImportGroups);
    }
    let version = config.version_sorted_imports();
    return (
      match (&a.use_tree, &b.use_tree) {
        (Some(x), Some(y)) => x.compare(y, version),
        _ => a.identifier.cmp(&b.identifier),
      },
      Rule::Imports,
    );
  }
  let family = |unit: &Unit| -> String {
    if config.group_impls && unit.kind == ItemKind::ImplBlock {
      unit.parent_target.clone()
    } else {
      unit.identifier.clone()
    }
  };
  let (family_a, family_b) = (family(a), family(b));
  let rank = |kind: &ItemKind| kind_rank(kind, config);
  if config.order_keys.first() != Some(&OrderKey::Kind) {
    // Unless `kind` comes first, imports, declarations and macros still come first, each
    // kind with its own rules; everything else is one group the keys order. The order
    // stays one order: no pair is compared by rules another pair is not.
    let special = |unit: &Unit| {
      matches!(
        unit.kind,
        ItemKind::ModDecl | ItemKind::UseDecl | ItemKind::MacroDecl | ItemKind::MacroDef
      )
    };
    let group = |unit: &Unit| {
      if special(unit) {
        rank(&unit.kind)
      } else {
        usize::MAX
      }
    };
    if group(a) != group(b) {
      return (group(a).cmp(&group(b)), Rule::TypeOrder);
    }
    if special(a) {
      return kind_rules(a, b, config).unwrap_or((a.index.cmp(&b.index), Rule::Written));
    }
  }
  for key in &config.order_keys {
    let decided = match key {
      OrderKey::Kind => kind_step(a, b, config, families, &family_a, &family_b),
      OrderKey::Visibility => {
        let visibility = |unit: &Unit, name: &str| {
          // An impl goes with its type, at the type's visibility.
          let of_type = (config.group_impls && unit.kind == ItemKind::ImplBlock)
            .then(|| families.iter().find(|(n, _, _)| n == name).map(|f| &f.2))
            .flatten();
          let shown = of_type.unwrap_or(&unit.visibility);
          config
            .visibility_order
            .iter()
            .position(|o| o == shown)
            .unwrap_or(usize::MAX)
        };
        let (va, vb) = (visibility(a, &family_a), visibility(b, &family_b));
        (va != vb).then(|| (va.cmp(&vb), Rule::Visibility))
      }
      OrderKey::Name if config.sort_alphabetically => {
        let order = names(&family_a, &family_b, config);
        (order != Ordering::Equal).then_some((order, Rule::Name))
      }
      OrderKey::Name => None,
    };
    if let Some(decided) = decided {
      return decided;
    }
  }
  // Ties the keys leave: a type before its impls, impls of one type in their own order.
  if family_a == family_b {
    let (own_a, own_b) = (rank(&a.kind), rank(&b.kind));
    if own_a != own_b {
      return (own_a.cmp(&own_b), Rule::GroupImpls);
    }
    if a.kind == ItemKind::ImplBlock && b.kind == ItemKind::ImplBlock {
      return (impls(a, b, config), Rule::ImplsOfOneType);
    }
  }
  (Ordering::Equal, Rule::Written)
}

/// True if some unit that came after a `macro_rules!` now comes before it.
pub(super) fn macro_moved_past_user(units: &[Unit]) -> bool {
  let mut latest: Option<usize> = None;
  for unit in units {
    if defines_macros(unit) && latest.is_some_and(|l| l > unit.index) {
      return true;
    }
    latest = latest.max(Some(unit.index));
  }
  false
}

/// The rule that put `unit` where it is in `arranged`, at `at`: above its user, by a pin,
/// or by the comparison with the unit after it (before it, for the last).
pub(super) fn reason(arranged: &[Unit], at: usize, config: &LayoutConfig) -> Rule {
  let unit = &arranged[at];
  if unit.pinned {
    return Rule::Slot;
  }
  // Next to its user: the first unit after it (before it, for one kept below) that is
  // not another helper of that user.
  if let Some(caller) = unit.caller {
    let other = |next: &usize| arranged[*next].caller != Some(caller) && !arranged[*next].pinned;
    let user = if unit.below {
      (0..at).rev().find(other)
    } else {
      (at + 1..arranged.len()).find(other)
    };
    if user.is_some_and(|next| super::first_variant(arranged, next) == caller) {
      return Rule::Helper;
    }
  }
  let families = families(arranged, config);
  let free = |u: &&Unit| !u.pinned && !u.comments_only;
  let (rule, ordering, neighbour) = match arranged[at + 1..].iter().find(free) {
    Some(next) => {
      let (ordering, rule) = compare_by(unit, next, config, &families, false);
      (rule, ordering, next)
    }
    None => match arranged[..at].iter().rev().find(free) {
      Some(previous) => {
        let (ordering, rule) = compare_by(previous, unit, config, &families, false);
        (rule, ordering, previous)
      }
      None => return Rule::Written,
    },
  };
  match ordering {
    Ordering::Less => rule,
    // Out of the plain order beside a helper: the helper's place decided it.
    _ if unit.caller.is_some() || neighbour.caller.is_some() => Rule::Helper,
    // The plain order would not have put it here: the macro rule did.
    Ordering::Greater if defines_macros(unit) || arranged.iter().any(defines_macros) => {
      Rule::MacrosFirst
    }
    _ => Rule::Written,
  }
}

/// A `macro_rules!`, or a `#[macro_use]` module, whose macros are visible after it.
fn defines_macros(unit: &Unit) -> bool {
  unit.defines_macros
}

/// The types among `units`, by name, the one placed first first: what an impl of a name
/// two types share joins.
fn families(units: &[Unit], config: &LayoutConfig) -> Vec<(String, ItemKind, Visibility)> {
  let mut families: Vec<(String, ItemKind, Visibility)> = units
    .iter()
    .filter(|u| {
      matches!(
        u.kind,
        ItemKind::StructDecl | ItemKind::EnumDecl | ItemKind::Union | ItemKind::TraitDecl
      )
    })
    .map(|u| (u.identifier.clone(), u.kind.clone(), u.visibility.clone()))
    .collect();
  // Of types sharing a name (in different modules, or under `cfg`), an impl goes with the
  // one placed first, whatever the order they were written in.
  families.sort_by_key(|(_, kind, _)| kind_rank(kind, config));
  families
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

/// A kind's place in `type_order`. Kinds left out of the list go after the listed ones, in
/// their default order: no two kinds share a rank, as the rules within a kind (imports or
/// macros as written) would not be one order with a rule across kinds.
fn kind_rank(kind: &ItemKind, config: &LayoutConfig) -> usize {
  match config.type_order.iter().position(|k| k == kind) {
    Some(at) => at,
    None => config.type_order.len() + kind.clone() as usize,
  }
}

/// The rules some kinds keep whatever the keys: macros in their written order (a macro is
/// visible only below its definition), modules as the standard formatter orders them.
fn kind_rules(a: &Unit, b: &Unit, config: &LayoutConfig) -> Option<(Ordering, Rule)> {
  if a.kind != b.kind {
    return None;
  }
  match a.kind {
    // Macros are scoped by textual order, so they never change places with each other.
    ItemKind::MacroDecl => Some((a.index.cmp(&b.index), Rule::Written)),
    ItemKind::ModDecl => {
      // A #[macro_use] module stays ahead of the modules that rely on its macros.
      if a.macro_use != b.macro_use {
        return Some((b.macro_use.cmp(&a.macro_use), Rule::MacroUseModules));
      }
      if a.macro_use {
        return Some((a.index.cmp(&b.index), Rule::MacroUseModules));
      }
      if !config.sort_alphabetically {
        return Some((Ordering::Equal, Rule::Written));
      }
      // As the standard formatter orders `mod` declarations: by bytes through style
      // edition 2024. The standard formatter compares the names without `r#`.
      let (x, y) = (
        a.identifier.trim_start_matches("r#"),
        b.identifier.trim_start_matches("r#"),
      );
      let order = if config.style_edition > 2024 {
        version_sort(x, y)
      } else {
        x.cmp(y)
      };
      Some((order, Rule::Modules))
    }
    _ => None,
  }
}

/// The `kind` key: the kind's place in `type_order` (an impl at its type's with
/// `group_impls`), types by name with their impls, then each kind's own rules. `None`
/// when it leaves the two equal.
fn kind_step(
  a: &Unit,
  b: &Unit,
  config: &LayoutConfig,
  families: &[(String, ItemKind, Visibility)],
  family_a: &str,
  family_b: &str,
) -> Option<(Ordering, Rule)> {
  let family_kind = |unit: &Unit, name: &str| -> ItemKind {
    if config.group_impls
      && unit.kind == ItemKind::ImplBlock
      && let Some((_, kind, _)) = families.iter().find(|(n, _, _)| n == name)
    {
      return kind.clone();
    }
    unit.kind.clone()
  };
  let rank = |kind: &ItemKind| kind_rank(kind, config);
  let (rank_a, rank_b) = (
    rank(&family_kind(a, family_a)),
    rank(&family_kind(b, family_b)),
  );
  if rank_a != rank_b {
    return Some((rank_a.cmp(&rank_b), Rule::TypeOrder));
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
      return Some((names(family_a, family_b, config), Rule::GroupImpls));
    }
  }
  let (own_a, own_b) = (rank(&a.kind), rank(&b.kind));
  if own_a != own_b {
    return Some((own_a.cmp(&own_b), Rule::GroupImpls));
  }
  if a.kind == ItemKind::ImplBlock && b.kind == ItemKind::ImplBlock {
    return Some((impls(a, b, config), Rule::ImplsOfOneType));
  }
  kind_rules(a, b, config)
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

/// `free` (sorted) with each helper lifted out and put directly next to the unit that uses
/// it (`keep_helpers_near`): above it, or below it when written below it; several helpers
/// of one unit in the order they sort in, so `#[cfg]` variants of one helper come out
/// together. A helper whose user is not among `free` (pinned, or in another section) stays
/// where it sorted.
fn near_callers(free: Vec<Unit>) -> Vec<Unit> {
  // A helper of an item with `#[cfg]` variants goes above whichever variant sorts first,
  // or below the last.
  let keys: Vec<usize> = (0..free.len())
    .map(|at| super::first_variant(&free, at))
    .collect();
  let present: std::collections::HashSet<usize> = keys.iter().copied().collect();
  type Waiting = std::collections::HashMap<usize, Vec<(Unit, usize)>>;
  let (mut above, mut below): (Waiting, Waiting) = Default::default();
  let mut rest = Vec::with_capacity(free.len());
  for (unit, key) in free.into_iter().zip(keys) {
    match unit.caller {
      Some(caller) if present.contains(&caller) => {
        let side = if unit.below { &mut below } else { &mut above };
        side.entry(caller).or_default().push((unit, key))
      }
      _ => rest.push((unit, key)),
    }
  }
  fn place(
    unit: Unit,
    key: usize,
    last: bool,
    sides: &mut (Waiting, Waiting),
    out: &mut Vec<Unit>,
  ) {
    for (helper, key) in sides.0.remove(&key).unwrap_or_default() {
      place(helper, key, true, sides, out);
    }
    out.push(unit);
    if last {
      for (helper, key) in sides.1.remove(&key).unwrap_or_default() {
        place(helper, key, true, sides, out);
      }
    }
  }
  let mut sides = (above, below);
  let mut out = Vec::with_capacity(rest.len());
  let mut rest = rest.into_iter().peekable();
  while let Some((unit, key)) = rest.next() {
    let last = rest.peek().is_none_or(|(_, next)| *next != key);
    place(unit, key, last, &mut sides, &mut out);
  }
  out
}

/// With `order_keys = []`, the one sorting left: each run of imports that stand
/// together in `units` (the order as it will be written) sorted where it stands, as the
/// standard formatter sorts them, and so each run of `mod name;` declarations. Runs are
/// found in the order the output has, so the next run finds the same ones.
fn sort_runs(
  units: &mut [Unit],
  config: &LayoutConfig,
  families: &[(String, ItemKind, Visibility)],
) {
  let class = |unit: &Unit| match unit.kind {
    _ if unit.pinned || unit.test_module => 0,
    ItemKind::UseDecl if config.sort_imports => 1,
    ItemKind::ModDecl if unit.module_declaration => 2,
    _ => 0,
  };
  let mut at = 0;
  while at < units.len() {
    let mut end = at + 1;
    while class(&units[at]) != 0 && end < units.len() && class(&units[end]) == class(&units[at]) {
      end += 1;
    }
    units[at..end].sort_by(|a, b| compare(a, b, config, families, false));
    at = end;
  }
}

fn sorted(units: &[Unit], config: &LayoutConfig, macros_first: bool) -> Vec<Unit> {
  let families = families(units, config);
  let mut free: Vec<Unit> = units.iter().filter(|u| !u.pinned).cloned().collect();
  let written = config.order_keys.is_empty();
  if written {
    // Test modules last, all else as written.
    let (rest, tests): (Vec<Unit>, Vec<Unit>) = free.into_iter().partition(|u| !u.test_module);
    free = rest;
    free.extend(tests);
  } else {
    free.sort_by(|a, b| compare(a, b, config, &families, macros_first));
  }
  if free.iter().any(|u| u.caller.is_some()) {
    free = near_callers(free);
  }
  // By position in `units`, which may be a run between held units: a unit's index is its
  // position in the whole container.
  let mut free = free.into_iter();
  let mut filled: Vec<Unit> = units
    .iter()
    .filter_map(|unit| {
      if unit.pinned {
        Some(unit.clone())
      } else {
        free.next()
      }
    })
    .collect();
  if written {
    sort_runs(&mut filled, config, &families);
  }
  filled
}
