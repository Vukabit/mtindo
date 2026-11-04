//! `mto config derive`: the mto.toml that moves the fewest items in a project as it is
//! written. Every Rust file a run would find is laid out in memory (nothing is written)
//! under candidate settings, starting from what `mto init` writes; each setting in turn
//! takes the value that moves the fewest items (then changes the fewest files), and the
//! passes repeat while one finds less to move. A tie keeps the value already chosen, so
//! the defaults stand unless the project's own order beats them.
//!
//! Derive finds an order, never "no order": `order_keys = []` (every file as written) and
//! `sort_alphabetically = false` are not tried, as they would win everywhere.
//!
//! The project's formatting settings (its rustfmt.toml) are read, never copied: they apply
//! as they are, and the layout settings they decide stay as they ask (imports sorted while
//! the formatter sorts them, modules by name while it reorders them), so the layout
//! derived never fights the formatting.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config::{Config, Resolver, Value};
use crate::options::layout::{ItemKind, LayoutConfig};
use crate::{RunOptions, options};

/// Settings that change how a project's imports are written.
const IMPORT_STYLE: &[&str] = &["import_grouping", "sort_imports", "import_sort"];

/// Cutting a section at every loose comment keeps much of any order where it is, so it
/// saves moves almost everywhere: it is tried after the rest, and kept only when it takes
/// away at least this share of the moves left.
const SECTIONS_SHARE: usize = 2;

/// The settings derive tries, each with its values, the first the one `mto init` writes.
struct Candidate {
  setting: &'static str,
  values: Vec<String>,
}

/// A setting derive changed from what `mto init` writes.
#[derive(Clone, Debug)]
pub struct Changed {
  pub setting: &'static str,
  pub value: String,
  /// Moves it saved when it was chosen.
  pub saved: usize,
  /// It changes how imports are written: a style shift to look at, whatever it saves.
  pub imports: bool,
}

/// What derive found.
#[derive(Clone, Debug)]
pub struct Derived {
  /// The mto.toml: `mto init`'s, with the derived values in place.
  pub toml: String,
  /// Rust files looked at.
  pub files: usize,
  /// What `mto init`'s settings do, and what the derived ones do.
  pub start: Score,
  pub end: Score,
  pub changed: Vec<Changed>,
  /// Settings kept as they are, as the project's formatting decides them: why.
  pub held: Vec<(&'static str, String)>,
  /// The standard formatter's file that applies in the folder, if any.
  pub standard: Option<PathBuf>,
}

/// One file, with the settings that apply in its folder (an index into the folders').
struct File {
  source: String,
  folder: usize,
}

/// What a set of settings does to the project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Score {
  /// Items layout moves, at every level.
  pub moves: usize,
  /// Files whose text changes.
  pub files: usize,
}

/// Derives the mto.toml for the project in `dir`.
pub fn derive(dir: &Path, resolver: &mut Resolver) -> Result<Derived, String> {
  let jobs = crate::collect_jobs(&[dir.to_path_buf()], resolver, &RunOptions::default())
    .map_err(|e| e.to_string())?
    .jobs;
  let mut folders: Vec<Rc<Config>> = Vec::new();
  let mut files = Vec::new();
  for (path, _) in jobs {
    let Ok(source) = std::fs::read_to_string(&path) else {
      continue;
    };
    let folder = path
      .parent()
      .map(Path::to_path_buf)
      .unwrap_or_else(|| PathBuf::from("."));
    let config = resolver.config_for(&folder).map_err(|e| e.to_string())?;
    let at = match folders.iter().position(|f| Rc::ptr_eq(f, &config)) {
      Some(at) => at,
      None => {
        folders.push(config);
        folders.len() - 1
      }
    };
    files.push(File { source, folder: at });
  }
  let here = resolver.config_for(dir).map_err(|e| e.to_string())?;
  let standard = here
    .files()
    .iter()
    .find(|(kind, _)| *kind == crate::config::FileKind::Standard)
    .map(|(_, path)| path.clone());
  let (table, held) = table(&here, &consensus(&files, &folders)?);
  let score = |chosen: &[(&'static str, String)]| -> Result<Score, String> {
    let pairs: Vec<(&str, &str)> = chosen
      .iter()
      .map(|(key, value)| (*key, value.as_str()))
      .collect();
    let configs = folders
      .iter()
      .map(|folder| folder.with_settings(&pairs).map(|c| c.layout()))
      .collect::<Result<Vec<LayoutConfig>, _>>()
      .map_err(|e| e.to_string())?;
    Ok(laid_out(&files, &configs))
  };
  let mut chosen: Vec<(&'static str, String)> = table
    .iter()
    .map(|c| (c.setting, c.values[0].clone()))
    .collect();
  let start = score(&chosen)?;
  let mut best = start;
  let mut saved: HashMap<&'static str, usize> = HashMap::new();
  for _pass in 0..3 {
    let mut better = false;
    for (at, candidate) in table.iter().enumerate() {
      for value in &candidate.values {
        if *value == chosen[at].1 {
          continue;
        }
        let mut trial = chosen.clone();
        trial[at].1 = value.clone();
        let found = score(&trial)?;
        if found < best {
          *saved.entry(candidate.setting).or_default() += best.moves.saturating_sub(found.moves);
          best = found;
          chosen = trial;
          better = true;
        }
      }
    }
    if !better {
      break;
    }
  }
  // Last, and only for a clear win: sections at every loose comment.
  if let Some(at) = table.iter().position(|c| c.setting == "sections") {
    let mut trial = chosen.clone();
    trial[at].1 = "\"comments\"".to_string();
    let found = score(&trial)?;
    if found < best && found.moves * SECTIONS_SHARE <= best.moves {
      *saved.entry("sections").or_default() += best.moves - found.moves;
      best = found;
      chosen = trial;
    }
  }
  let changed: Vec<Changed> = table
    .iter()
    .zip(&chosen)
    .filter(|(candidate, (_, value))| candidate.values[0] != *value)
    .map(|(candidate, (_, value))| Changed {
      setting: candidate.setting,
      value: value.clone(),
      saved: saved.get(candidate.setting).copied().unwrap_or(0),
      imports: IMPORT_STYLE.contains(&candidate.setting),
    })
    .collect();
  let mut toml = crate::init_template();
  for change in &changed {
    toml = with_line(&toml, change.setting, &change.value);
  }
  Ok(Derived {
    toml,
    files: files.len(),
    start,
    end: best,
    changed,
    held,
    standard,
  })
}

/// The kind order `files` (each its items' kinds as written) agree on: each pair of kinds
/// a file holds is a vote for the one it mostly writes first; kinds are ranked by the share
/// of their votes they won, `default`'s order breaking ties, and each kind no file holds
/// goes after the kinds `default` puts before it.
pub fn written_order(files: &[Vec<ItemKind>], default: &[ItemKind]) -> Vec<ItemKind> {
  let at = |kind: &ItemKind| {
    default
      .iter()
      .position(|k| k == kind)
      .unwrap_or(default.len())
  };
  // Votes each kind won and lost.
  let mut score: HashMap<usize, (u64, u64)> = HashMap::new();
  for kinds in files {
    // How often each kind comes before each other kind in this file.
    let mut before: HashMap<(usize, usize), usize> = HashMap::new();
    let mut seen: HashMap<usize, usize> = HashMap::new();
    for kind in kinds {
      let k = at(kind);
      for (&other, &count) in &seen {
        if other != k {
          *before.entry((other, k)).or_default() += count;
        }
      }
      *seen.entry(k).or_default() += 1;
      score.entry(k).or_default();
    }
    let present: Vec<usize> = seen.keys().copied().collect();
    for &a in &present {
      for &b in &present {
        if a < b {
          let (ab, ba) = (
            before.get(&(a, b)).copied().unwrap_or(0),
            before.get(&(b, a)).copied().unwrap_or(0),
          );
          let (won, lost) = match ab.cmp(&ba) {
            std::cmp::Ordering::Greater => (a, b),
            std::cmp::Ordering::Less => (b, a),
            std::cmp::Ordering::Equal => continue,
          };
          score.entry(won).or_default().0 += 1;
          score.entry(lost).or_default().1 += 1;
        }
      }
    }
  }
  let mut used: Vec<usize> = score
    .keys()
    .copied()
    .filter(|&k| k < default.len())
    .collect();
  // By the share of its votes a kind won, so a kind many files hold is not ranked down
  // for meeting more others; one that never met another counts as even.
  let share = |k: &usize| match score[k] {
    (0, 0) => (1, 2),
    (won, lost) => (won, won + lost),
  };
  used.sort_by(|a, b| {
    let ((wa, ta), (wb, tb)) = (share(a), share(b));
    (wb * ta).cmp(&(wa * tb)).then(a.cmp(b))
  });
  let mut order: Vec<ItemKind> = used.iter().map(|&k| default[k].clone()).collect();
  // A kind no file holds goes right after the kinds `default` puts before it, so a
  // project written in the default order gets it back whole.
  for (at, kind) in default.iter().enumerate() {
    if order.contains(kind) {
      continue;
    }
    let after = order
      .iter()
      .rposition(|placed| default[..at].contains(placed))
      .map_or(0, |found| found + 1);
    order.insert(after, kind.clone());
  }
  order
}

/// The project's own `type_order`, as a TOML array ([`written_order`] of its files' items
/// as written).
fn consensus(files: &[File], folders: &[Rc<Config>]) -> Result<String, String> {
  let configs: Vec<LayoutConfig> = folders.iter().map(|f| f.layout()).collect();
  let written: Vec<Vec<ItemKind>> = files
    .iter()
    .filter_map(|file| {
      let laid = crate::layout::layout(&file.source, &configs[file.folder]).ok()?;
      Some(
        laid
          .before
          .iter()
          .filter(|u| !u.comments_only && !u.pinned)
          .map(|u| u.kind.clone())
          .collect(),
      )
    })
    .collect();
  let order = written_order(&written, &LayoutConfig::default().type_order);
  let names: Vec<String> = order.iter().map(|k| format!("{:?}", k)).collect();
  Ok(list(&names.iter().map(String::as_str).collect::<Vec<_>>()))
}

/// The score of laying out each file with the settings of its folder, in memory, the
/// files spread over the machine's cores.
fn laid_out(files: &[File], configs: &[LayoutConfig]) -> Score {
  let next = AtomicUsize::new(0);
  let moves = AtomicUsize::new(0);
  let changed = AtomicUsize::new(0);
  let workers = std::thread::available_parallelism()
    .map(|n| n.get())
    .unwrap_or(1)
    .min(files.len());
  std::thread::scope(|scope| {
    for _ in 0..workers {
      let worker = || {
        while let Some(file) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
          let laid = crate::quietly(false, || {
            crate::layout::layout(&file.source, &configs[file.folder])
          });
          if let Ok(Ok(laid)) = laid
            && laid.text != file.source
          {
            moves.fetch_add(laid.moves.len(), Ordering::Relaxed);
            changed.fetch_add(1, Ordering::Relaxed);
          }
        }
      };
      let _ = std::thread::Builder::new()
        .stack_size(crate::STACK)
        .spawn_scoped(scope, worker);
    }
  });
  Score {
    moves: moves.into_inner(),
    files: changed.into_inner(),
  }
}

/// `items` as a TOML array of strings.
fn list(items: &[&str]) -> String {
  let quoted: Vec<String> = items.iter().map(|i| format!("\"{}\"", i)).collect();
  format!("[{}]", quoted.join(", "))
}

/// The strings a TOML array of them holds, as a setting's default writes it.
fn names_in(array: &str) -> Vec<String> {
  array
    .trim()
    .trim_start_matches('[')
    .trim_end_matches(']')
    .split(',')
    .map(|item| item.trim().trim_matches('"').to_string())
    .filter(|item| !item.is_empty())
    .collect()
}

/// The settings derive tries in a folder whose settings are `here`, and those it holds as
/// the project's formatting decides them. `consensus` is the project's own `type_order`.
fn table(here: &Config, consensus: &str) -> (Vec<Candidate>, Vec<(&'static str, String)>) {
  let flag =
    |name: &str, fallback: bool| here.get(name).and_then(Value::as_bool).unwrap_or(fallback);
  let formats = here.format_on();
  let mut held = Vec::new();
  let mut free = |name: &'static str, why: Option<&str>| -> bool {
    match why {
      Some(why) => {
        held.push((name, why.to_string()));
        false
      }
      None => true,
    }
  };
  let default = |name: &str| {
    options::find(name)
      .map(|s| s.default.to_string())
      .unwrap_or_default()
  };
  let mut table = Vec::new();
  let boolean = |name: &'static str, start: &str| Candidate {
    setting: name,
    values: vec![
      start.to_string(),
      if start == "true" { "false" } else { "true" }.to_string(),
    ],
  };
  // On from `mto init`, unlike its default.
  table.push(boolean("keep_paragraphs", "true"));
  table.push(boolean("group_impls", &default("group_impls")));
  table.push(boolean("keep_helpers_near", &default("keep_helpers_near")));
  if free(
    "sort_imports",
    (formats && flag("reorder_imports", true))
      .then_some("the formatter sorts imports (reorder_imports)"),
  ) {
    table.push(boolean("sort_imports", &default("sort_imports")));
  }
  let groups = here.get("group_imports").and_then(Value::as_text);
  if free(
    "import_grouping",
    (formats && groups == Some("StdExternalCrate"))
      .then_some("the formatter groups imports (group_imports = \"StdExternalCrate\")"),
  ) {
    table.push(boolean("import_grouping", &default("import_grouping")));
  }
  for name in [
    "sections",
    "flow",
    "inherent_method_order",
    "impl_method_order",
    "trait_impl_sort",
    "test_module",
    "name_sort",
  ] {
    let Some(setting) = options::find(name) else {
      continue;
    };
    let mut values = vec![setting.default.to_string()];
    for choice in setting.choices() {
      let quoted = format!("\"{}\"", choice.name);
      // Tried apart, after the rest (`SECTIONS_SHARE`).
      if name == "sections" && choice.name == "comments" {
        continue;
      }
      if !values.contains(&quoted) {
        values.push(quoted);
      }
    }
    table.push(Candidate {
      setting: setting.name,
      values,
    });
  }
  // Not `[]`, the written order: it moves next to nothing anywhere, so it would win in
  // every project, and it is a choice to sort nothing, not an order derive can find.
  table.push(Candidate {
    setting: "order_keys",
    values: vec![
      default("order_keys"),
      list(&["visibility", "kind", "name"]),
      list(&["kind", "name"]),
    ],
  });
  let visibility = default("visibility_order");
  let reversed: Vec<String> = names_in(&visibility).into_iter().rev().collect();
  table.push(Candidate {
    setting: "visibility_order",
    values: vec![
      visibility,
      list(&reversed.iter().map(String::as_str).collect::<Vec<_>>()),
    ],
  });
  let mut types = vec![default("type_order")];
  if consensus != types[0] {
    types.push(consensus.to_string());
  }
  table.push(Candidate {
    setting: "type_order",
    values: types,
  });
  (table, held)
}

/// `toml` with the line setting `name` set to `value` instead.
fn with_line(toml: &str, name: &str, value: &str) -> String {
  let prefix = format!("{} = ", name);
  toml
    .lines()
    .map(|line| {
      if line.starts_with(&prefix) {
        format!("{}{}", prefix, value)
      } else {
        line.to_string()
      }
    })
    .collect::<Vec<_>>()
    .join("\n")
    + "\n"
}

#[cfg(test)]
mod tests {
  use ItemKind::*;

  use super::*;

  #[test]
  fn test_a_setting_line_is_replaced_in_place() {
    let toml = "# a\nflow = \"off\"\n\n# b\nflow_x = 1\n";
    assert_eq!(
      with_line(toml, "flow", "\"top_down\""),
      "# a\nflow = \"top_down\"\n\n# b\nflow_x = 1\n"
    );
  }

  /// The kinds a project writes first lead, by the files' votes; kinds it never writes
  /// follow in the default order.
  #[test]
  fn test_the_written_order_is_the_files_consensus() {
    let default = LayoutConfig::default().type_order;
    let files = vec![
      vec![UseDecl, FnDecl, FnDecl, StructDecl, ImplBlock],
      vec![UseDecl, FnDecl, StructDecl],
      vec![FnDecl, StructDecl, ConstDecl],
    ];
    let order = written_order(&files, &default);
    let used: Vec<ItemKind> = order
      .iter()
      .filter(|k| files.iter().any(|f| f.contains(k)))
      .cloned()
      .collect();
    // `struct` meets more kinds than `const` does, and still comes first: every file
    // that holds both writes it first.
    assert_eq!(used, [UseDecl, FnDecl, StructDecl, ConstDecl, ImplBlock]);
    // Every kind is there once.
    assert_eq!(order.len(), default.len());
    // A project that writes the default order gets it back.
    let plain = vec![vec![UseDecl, ConstDecl, StructDecl, ImplBlock, FnDecl]];
    assert_eq!(written_order(&plain, &default), default);
    // A file with mixed order votes only where it leans one way.
    let tied = vec![vec![FnDecl, StructDecl, StructDecl, FnDecl]];
    assert_eq!(written_order(&tied, &default), default);
  }
}
