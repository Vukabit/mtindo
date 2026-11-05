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
//! With `flow`, the sorted units of one rank are then put in the order of their uses
//! ([`flowed`]), the sort's order deciding wherever no use does.
//!
//! A `macro_rules!` is only visible below its definition, and so are the macros of a
//! `#[macro_use]` module and of an item macro call that holds definitions (`cfg_if! {
//! macro_rules! m { ... } }`), so no unit that came after a definition may move in front of
//! it; and a unit that came before one and calls a macro of that name must not move below
//! it, where its call would reach that definition instead of the one it reached (std's
//! `vec!`, an earlier one). If the sort would do either, it is redone with every macro
//! first, then with each definition held where it is; if even that moves one, the file is
//! left alone.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap, HashSet};

use super::Unit;
use crate::options::layout::{
  CfgGroups, Flow, ItemKind, LayoutConfig, NameSort, OrderKey, TraitImplSort, Visibility,
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
  /// Above or below the items it uses (`flow`).
  Flow,
  /// An inherent impl's functions in the order of `inherent_method_order`.
  MethodOrder,
  /// `fn main` first (`order_keys`).
  Entry,
  /// `#[deprecated]` items last (`order_keys`).
  Deprecated,
  /// `#[doc(hidden)]` items last (`order_keys`).
  Hidden,
  /// At the top or the bottom of its section (`// mto: first`, `// mto: last`).
  End,
  /// After the first item under the same `#[cfg(...)]` (`cfg_groups`).
  CfgGroup,
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
      Rule::Flow => "flow",
      Rule::MethodOrder => "method_order",
      Rule::Entry => "entry",
      Rule::Deprecated => "deprecated",
      Rule::Hidden => "hidden",
      Rule::End => "end",
      Rule::CfgGroup => "cfg_group",
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
      Rule::Flow => "above or below the items it uses (flow)",
      Rule::MethodOrder => "the order of an impl's functions (inherent_method_order)",
      Rule::Entry => "fn main first (order_keys)",
      Rule::Deprecated => "#[deprecated] items last (order_keys)",
      Rule::Hidden => "#[doc(hidden)] items last (order_keys)",
      Rule::End => "at an end of its section (// mto: first, // mto: last)",
      Rule::CfgGroup => "with the items under the same #[cfg(...)] (cfg_groups)",
    }
  }
}

/// A type an impl goes with (`group_impls`), and what of it its impls take on.
pub(super) struct Family {
  name: String,
  kind: ItemKind,
  visibility: Visibility,
  deprecated: bool,
  hidden: bool,
}

/// The units in their new order.
pub(super) fn arrange(units: &[Unit], config: &LayoutConfig) -> Result<Vec<Unit>, String> {
  let kept = |arranged: &[Unit]| !macro_moved_past_user(arranged) && !macro_shadows_user(arranged);
  let arranged = sorted(units, config, false);
  if kept(&arranged) {
    return Ok(arranged);
  }
  let first = sorted(units, config, true);
  if kept(&first) {
    return Ok(first);
  }
  // Each definition where it was written: nothing crosses one, either way.
  let held = super::around_held(
    units,
    |u| u.defines_macros,
    |run| Ok(sorted(run, config, false)),
  )?;
  if kept(&held) {
    return Ok(held);
  }
  Err("cannot keep macro definitions where every call reaches the same macro".to_string())
}

pub(super) fn compare(
  a: &Unit,
  b: &Unit,
  config: &LayoutConfig,
  families: &[Family],
  macros_first: bool,
) -> Ordering {
  compare_by(a, b, config, families, macros_first).0
}

/// [`compare`], with the rule that decided it.
pub(super) fn compare_by(
  a: &Unit,
  b: &Unit,
  config: &LayoutConfig,
  families: &[Family],
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
            .then(|| {
              families
                .iter()
                .find(|f| f.name == name)
                .map(|f| &f.visibility)
            })
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
      OrderKey::Entry => {
        let later = |unit: &Unit| !(unit.kind == ItemKind::FnDecl && unit.identifier == "main");
        (later(a) != later(b)).then(|| (later(a).cmp(&later(b)), Rule::Entry))
      }
      OrderKey::Deprecated => {
        let marked = |unit: &Unit, name: &str| {
          marked(
            unit,
            name,
            config,
            families,
            |u| u.deprecated,
            |f| f.deprecated,
          )
        };
        let (da, db) = (marked(a, &family_a), marked(b, &family_b));
        (da != db).then(|| (da.cmp(&db), Rule::Deprecated))
      }
      OrderKey::Hidden => {
        let marked = |unit: &Unit, name: &str| {
          marked(unit, name, config, families, |u| u.hidden, |f| f.hidden)
        };
        let (ha, hb) = (marked(a, &family_a), marked(b, &family_b));
        (ha != hb).then(|| (ha.cmp(&hb), Rule::Hidden))
      }
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

/// True if a unit that came before a macro definition now comes after it, and calls a macro
/// that definition may define: there its call would reach the definition, not the one it
/// reached before (std's, an earlier module's). Names are read from the units' tokens; a
/// definition whose names cannot be read (a `#[macro_use] mod name;`, its file out of
/// sight) may define any macro.
pub(super) fn macro_shadows_user(units: &[Unit]) -> bool {
  if !units.iter().any(defines_macros) {
    return false;
  }
  let calls: Vec<HashSet<String>> = units.iter().map(macros_called).collect();
  units.iter().enumerate().any(|(at, definer)| {
    if !defines_macros(definer) {
      return false;
    }
    let names = macros_defined(definer);
    units[at + 1..]
      .iter()
      .zip(&calls[at + 1..])
      .any(|(unit, called)| {
        unit.index < definer.index
          && match &names {
            Some(names) => called.iter().any(|name| names.contains(name)),
            None => !called.is_empty(),
          }
      })
  })
}

/// The rule that put `unit` where it is in `arranged`, at `at`: above its user, by a pin,
/// or by the comparison with the unit after it (before it, for the last).
pub(super) fn reason(arranged: &[Unit], at: usize, config: &LayoutConfig) -> Rule {
  let unit = &arranged[at];
  if unit.pinned {
    return Rule::Slot;
  }
  if unit.end.is_some() {
    return Rule::End;
  }
  // Right after an item under the same predicate it was gathered to.
  if config.cfg_groups == CfgGroups::Gather
    && super::in_cfg_group(unit)
    && at > 0
    && super::in_cfg_group(&arranged[at - 1])
    && arranged[at - 1].cfg_predicate == unit.cfg_predicate
  {
    return Rule::CfgGroup;
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
    // The plain order would not have put it here: the uses did.
    Ordering::Greater if config.flow != Flow::Off => Rule::Flow,
    // The plain order would not have put it here: the macro rule did.
    Ordering::Greater if defines_macros(unit) || arranged.iter().any(defines_macros) => {
      Rule::MacrosFirst
    }
    _ => Rule::Written,
  }
}

/// The strongly connected components of the graph `after` (Tarjan's, without recursion):
/// each node's component number.
fn components(after: &[Vec<usize>]) -> Vec<usize> {
  let unseen = usize::MAX;
  let mut index = vec![unseen; after.len()];
  let mut low = vec![0; after.len()];
  let mut on_stack = vec![false; after.len()];
  let mut stack = Vec::new();
  let mut component = vec![0; after.len()];
  let (mut next, mut count) = (0, 0);
  for root in 0..after.len() {
    if index[root] != unseen {
      continue;
    }
    index[root] = next;
    low[root] = next;
    next += 1;
    stack.push(root);
    on_stack[root] = true;
    let mut calls = vec![(root, 0)];
    while let Some(&(node, edge)) = calls.last() {
      if let Some(&to) = after[node].get(edge) {
        calls.last_mut().unwrap().1 += 1;
        if index[to] == unseen {
          index[to] = next;
          low[to] = next;
          next += 1;
          stack.push(to);
          on_stack[to] = true;
          calls.push((to, 0));
        } else if on_stack[to] {
          low[node] = low[node].min(index[to]);
        }
        continue;
      }
      calls.pop();
      if let Some(&(parent, _)) = calls.last() {
        low[parent] = low[parent].min(low[node]);
      }
      if low[node] == index[node] {
        while let Some(member) = stack.pop() {
          on_stack[member] = false;
          component[member] = count;
          if member == node {
            break;
          }
        }
        count += 1;
      }
    }
  }
  component
}

/// A `macro_rules!`, or a `#[macro_use]` module, whose macros are visible after it.
fn defines_macros(unit: &Unit) -> bool {
  unit.defines_macros
}

/// The types among `units`, by name, the one placed first first: what an impl of a name
/// two types share joins.
fn families(units: &[Unit], config: &LayoutConfig) -> Vec<Family> {
  let mut families: Vec<Family> = units
    .iter()
    .filter(|u| {
      matches!(
        u.kind,
        ItemKind::StructDecl | ItemKind::EnumDecl | ItemKind::Union | ItemKind::TraitDecl
      )
    })
    .map(|u| Family {
      name: u.identifier.clone(),
      kind: u.kind.clone(),
      visibility: u.visibility.clone(),
      deprecated: u.deprecated,
      hidden: u.hidden,
    })
    .collect();
  // Of types sharing a name (in different modules, or under `cfg`), an impl goes with the
  // one placed first, whatever the order they were written in.
  families.sort_by_key(|f| kind_rank(&f.kind, config));
  families
}

/// The positions in `range` of `units` in the order of their uses (see [`flowed`]).
fn flow_run(
  units: &[Unit],
  uses: &[HashSet<&str>],
  range: std::ops::Range<usize>,
  config: &LayoutConfig,
) -> Vec<usize> {
  let grouped = |unit: &Unit| {
    config.group_impls
      && matches!(
        unit.kind,
        ItemKind::StructDecl
          | ItemKind::EnumDecl
          | ItemKind::Union
          | ItemKind::TraitDecl
          | ItemKind::ImplBlock
      )
  };
  let node_key = |at: usize| -> (bool, String) {
    let unit = &units[at];
    if grouped(unit) {
      let family = if unit.kind == ItemKind::ImplBlock {
        &unit.parent_target
      } else {
        &unit.identifier
      };
      (true, family.clone())
    } else {
      (false, super::first_variant(units, at).to_string())
    }
  };
  // Nodes: units of one key, wherever they stand in the run (a type and its impls written
  // apart, as the written order leaves them), each placed where its first unit is. Only
  // next to each other they would be one node in a second run and two in the first.
  let mut nodes: Vec<Vec<usize>> = Vec::new();
  let mut node_of: HashMap<(bool, String), usize> = HashMap::new();
  for at in range {
    match node_of.entry(node_key(at)) {
      std::collections::hash_map::Entry::Occupied(found) => nodes[*found.get()].push(at),
      std::collections::hash_map::Entry::Vacant(free) => {
        free.insert(nodes.len());
        nodes.push(vec![at]);
      }
    }
  }
  // The names each node defines; a name two nodes define decides nothing. A helper kept
  // next to its user (`keep_helpers_near`) goes with it, and takes no part.
  let helper = |node: &[usize]| node.iter().all(|&at| units[at].caller.is_some());
  let mut defined: HashMap<&str, Option<usize>> = HashMap::new();
  for (n, node) in nodes.iter().enumerate() {
    if helper(node) {
      continue;
    }
    let names: HashSet<&str> = node
      .iter()
      .map(|&at| {
        let unit = &units[at];
        if unit.kind == ItemKind::ImplBlock {
          unit.parent_target.as_str()
        } else {
          unit.identifier.as_str()
        }
      })
      .filter(|name| !name.is_empty())
      .collect();
    for name in names {
      defined
        .entry(name)
        .and_modify(|by| *by = None)
        .or_insert(Some(n));
    }
  }
  // `after[x]` holds the nodes that must come after `x`.
  let mut after: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
  for (n, node) in nodes.iter().enumerate() {
    if helper(node) {
      continue;
    }
    for &at in node {
      for name in &uses[at] {
        let Some(Some(used)) = defined.get(name) else {
          continue;
        };
        if *used == n {
          continue;
        }
        match config.flow {
          Flow::TopDown => after[n].push(*used),
          Flow::BottomUp => after[*used].push(n),
          Flow::Off => {}
        }
      }
    }
  }
  for next in after.iter_mut() {
    next.sort_unstable();
    next.dedup();
  }
  let component = components(&after);
  let mut waiting = vec![0usize; nodes.len()];
  for (n, next) in after.iter_mut().enumerate() {
    // Inside a circle, the sorted order decides.
    next.retain(|&m| component[m] != component[n]);
    for &m in next.iter() {
      waiting[m] += 1;
    }
  }
  let mut free: BinaryHeap<Reverse<usize>> = (0..nodes.len())
    .filter(|&n| waiting[n] == 0)
    .map(Reverse)
    .collect();
  let mut order = Vec::new();
  while let Some(Reverse(n)) = free.pop() {
    order.extend(&nodes[n]);
    for &m in &after[n] {
      waiting[m] -= 1;
      if waiting[m] == 0 {
        free.push(Reverse(m));
      }
    }
  }
  order
}

/// `free` (sorted) with `flow` applied: in each run of units of one rank (one kind, an impl
/// at its type's; every unit but imports, modules and macros when `kind` is not a key), a
/// unit above the units it uses (`top_down`) or below them (`bottom_up`). A type and its
/// impls are one node, and so are `#[cfg]` variants, written together or apart, and come
/// out together; nodes that use each other in a circle
/// keep the sorted order among themselves, and wherever no use decides, the sorted order
/// does: of the nodes free to go next, the one sorted first goes. So an order that already
/// follows the uses is kept as it is, and a second run changes nothing.
fn flowed(free: Vec<Unit>, config: &LayoutConfig, families: &[Family]) -> Vec<Unit> {
  let family_kind = |unit: &Unit| -> ItemKind {
    if config.group_impls
      && unit.kind == ItemKind::ImplBlock
      && let Some(family) = families.iter().find(|f| f.name == unit.parent_target)
    {
      return family.kind.clone();
    }
    unit.kind.clone()
  };
  let rank = |unit: &Unit| -> Option<usize> {
    let fixed = matches!(
      unit.kind,
      ItemKind::ModDecl | ItemKind::UseDecl | ItemKind::MacroDecl | ItemKind::MacroDef
    ) || unit.test_module
      || unit.defines_macros
      || unit.comments_only;
    if fixed {
      None
    } else if config.order_keys.contains(&OrderKey::Kind) {
      Some(kind_rank(&family_kind(unit), config))
    } else {
      Some(0)
    }
  };
  let uses = super::names_used(&free);
  let mut order = Vec::with_capacity(free.len());
  let mut at = 0;
  while at < free.len() {
    let Some(this) = rank(&free[at]) else {
      order.push(at);
      at += 1;
      continue;
    };
    let mut end = at + 1;
    while end < free.len() && rank(&free[end]) == Some(this) {
      end += 1;
    }
    order.extend(flow_run(&free, &uses, at..end, config));
    at = end;
  }
  let mut free: Vec<Option<Unit>> = free.into_iter().map(Some).collect();
  order.into_iter().filter_map(|at| free[at].take()).collect()
}

/// `free`, in its order, with each unit under a `#[cfg(...)]` (`cfg_groups = "gather"`)
/// moved up to follow the first under the same predicate. It depends on the order alone,
/// not on where the units were written, so a second run gathers the same.
fn gathered(free: Vec<Unit>) -> Vec<Unit> {
  let mut groups: Vec<Vec<Unit>> = Vec::new();
  let mut at: HashMap<String, usize> = HashMap::new();
  for unit in free {
    let key = unit
      .cfg_predicate
      .clone()
      .filter(|_| super::in_cfg_group(&unit));
    match key.as_ref().and_then(|k| at.get(k)) {
      Some(&group) => groups[group].push(unit),
      None => {
        if let Some(key) = key {
          at.insert(key, groups.len());
        }
        groups.push(vec![unit]);
      }
    }
  }
  groups.into_iter().flatten().collect()
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

/// Whether `word` is an identifier.
fn is_name(word: &str) -> bool {
  let word = word.trim_start_matches("r#");
  word.starts_with(|c: char| c.is_alphabetic() || c == '_')
    && word.chars().all(|c| c.is_alphanumeric() || c == '_')
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
  families: &[Family],
  family_a: &str,
  family_b: &str,
) -> Option<(Ordering, Rule)> {
  let family_kind = |unit: &Unit, name: &str| -> ItemKind {
    if config.group_impls
      && unit.kind == ItemKind::ImplBlock
      && let Some(family) = families.iter().find(|f| f.name == name)
    {
      return family.kind.clone();
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

/// The macros `unit` calls: each name followed by `!`, but `macro_rules` itself.
fn macros_called(unit: &Unit) -> HashSet<String> {
  let words = words(&unit.text);
  words
    .windows(2)
    .filter(|pair| pair[1] == "!" && pair[0] != "macro_rules" && is_name(&pair[0]))
    .map(|pair| pair[0].trim_start_matches("r#").to_string())
    .collect()
}

/// The macros `unit` defines (`macro_rules! name`, `macro name`), or `None` when they cannot
/// be read: a module declared without a body, or a definer whose names are not in sight.
fn macros_defined(unit: &Unit) -> Option<HashSet<String>> {
  if unit.module_declaration {
    return None;
  }
  let words = words(&unit.text);
  let names: HashSet<String> = words
    .windows(3)
    .filter_map(|w| match (w[0].as_str(), w[1].as_str()) {
      ("macro_rules", "!") if is_name(&w[2]) => Some(w[2].trim_start_matches("r#").to_string()),
      ("macro", name) if is_name(name) => Some(name.trim_start_matches("r#").to_string()),
      _ => None,
    })
    .collect();
  (!names.is_empty()).then_some(names)
}

/// Whether `unit` is marked so, by `own`; an impl going with its type (`group_impls`) is as
/// its type is, by `of_type`.
fn marked(
  unit: &Unit,
  name: &str,
  config: &LayoutConfig,
  families: &[Family],
  own: fn(&Unit) -> bool,
  of_type: fn(&Family) -> bool,
) -> bool {
  if config.group_impls
    && unit.kind == ItemKind::ImplBlock
    && let Some(family) = families.iter().find(|f| f.name == name)
  {
    return of_type(family);
  }
  own(unit)
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
fn sort_runs(units: &mut [Unit], config: &LayoutConfig, families: &[Family]) {
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
  if config.flow != Flow::Off {
    free = flowed(free, config, &families);
  }
  if config.cfg_groups == CfgGroups::Gather {
    free = gathered(free);
  }
  free = super::to_ends(free);
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

/// The tokens of `text` that are not whitespace or comments.
fn words(text: &str) -> Vec<String> {
  let mut at = 0;
  let mut out = Vec::new();
  for token in crate::syntax::lexer::tokenize(text, crate::syntax::Edition::E2021) {
    let end = at + token.len as usize;
    if !token.kind.is_trivia() {
      out.push(text[at..end].to_string());
    }
    at = end;
  }
  out
}
