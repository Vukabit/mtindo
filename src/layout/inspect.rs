//! What layout sees in a file, for `mto debug units` and `mto debug moves`: each container
//! (the file, and every inline module body) with the units it was cut into, and for each
//! unit why it was cut and classed so: pinned, a paragraph, a section's start, a helper and
//! its user, or why it is not one. Nothing here changes what layout does; the notes come
//! from the same decisions the sort makes.

use std::cell::RefCell;
use std::collections::HashSet;

use super::{
  Cx, Move, Unit, helper_verdict, impls, names_used, parse_any_edition, refusal, split_container,
};
use crate::options::layout::{ImportGroup, ItemKind, LayoutConfig, Sections, Visibility};
use crate::syntax::ast::Item;
use crate::syntax::{Edition, SyntaxElement, SyntaxKind, SyntaxNode};
use crate::text::json::Json;
use crate::text::source::normalize;

/// One body layout orders on its own.
pub struct Container {
  /// `""` for the file; `mod a`, `mod a::b` for module bodies.
  pub path: String,
  pub depth: usize,
  /// The lines of the header kept first (shebang, inner attributes, licence), if any.
  pub header: Option<(usize, usize)>,
  pub units: Vec<UnitView>,
  /// Comments after the last item, kept last.
  pub tail: bool,
}

/// A file as layout cut it.
pub struct Inspection {
  pub edition: Edition,
  /// The file first, then each inline module body, depth first.
  pub containers: Vec<Container>,
  /// The units that move, as a real run would move them.
  pub moves: Vec<MoveView>,
}

impl Inspection {
  /// `mto debug units --json`.
  pub fn units_json(&self, file: &str) -> Json {
    let containers: Vec<Json> = self
      .containers
      .iter()
      .map(|c| {
        let units: Vec<Json> = c
          .units
          .iter()
          .map(|u| {
            Json::object()
              .with("lines", vec![u.lines.0, u.lines.1])
              .with("kind", u.kind)
              .with("name", u.name.as_str())
              .with("visibility", u.visibility)
              .with("notes", u.notes.clone())
          })
          .collect();
        Json::object()
          .with("path", c.path.as_str())
          .with("depth", c.depth)
          .with("header", c.header.map(|(a, b)| vec![a, b]))
          .with("tail", c.tail)
          .with("units", units)
      })
      .collect();
    Json::output("units")
      .with("file", file)
      .with("edition", self.edition.year())
      .with("containers", containers)
  }

  /// `mto debug moves --json`.
  pub fn moves_json(&self, file: &str) -> Json {
    let moves: Vec<Json> = self
      .moves
      .iter()
      .map(|m| {
        Json::object()
          .with("line", m.line)
          .with("kind", m.kind)
          .with("name", m.name.as_str())
          .with(
            "before",
            m.before.as_ref().map(|(kind, name)| {
              Json::object()
                .with("kind", *kind)
                .with("name", name.as_str())
            }),
          )
          .with("rule", m.rule.id())
          .with("says", m.rule.says())
      })
      .collect();
    Json::output("moves")
      .with("file", file)
      .with("moves", moves)
  }
}

/// A unit that moves, and why.
pub struct MoveView {
  pub line: usize,
  pub kind: &'static str,
  pub name: String,
  /// The unit it now comes right before: kind and name.
  pub before: Option<(&'static str, String)>,
  pub rule: super::Rule,
}

/// One unit: what layout moves.
pub struct UnitView {
  pub lines: (usize, usize),
  pub kind: &'static str,
  pub name: String,
  pub visibility: &'static str,
  /// Why it was cut and classed so, each a short sentence.
  pub notes: Vec<String>,
}

/// What layout sees in `source` with `config`: the cut, with reasons, and the moves.
pub fn inspect(source: &str, config: &LayoutConfig) -> Result<Inspection, String> {
  let normalized = normalize(source)?;
  let text = normalized.text.as_str();
  let (edition, parsed) = parse_any_edition(text, super::configured_edition(config));
  if let Some(reason) = refusal(&parsed) {
    return Err(reason);
  }
  let tree = parsed.syntax_node();
  let held = HashSet::new();
  let cx = Cx {
    traits: if config.impl_method_order_trait {
      impls::collect_traits(&tree)
    } else {
      impls::Traits::new()
    },
    held: &held,
    moves: RefCell::new(Vec::new()),
  };
  let mut containers = Vec::new();
  container(&tree, text, 0, "", 0, config, &cx, &mut containers)?;
  let moves = super::layout(source, config)?
    .moves
    .iter()
    .map(|m| move_view(text, m))
    .collect();
  Ok(Inspection {
    edition,
    containers,
    moves,
  })
}

/// The line `offset` is on in `text`, from 1.
pub fn line_of(text: &str, offset: usize) -> usize {
  text[..offset.min(text.len())].matches('\n').count() + 1
}

/// Cuts `node` (the file, or a module's item list starting at `base`) and adds it to `out`,
/// then each inline module body in it.
#[allow(clippy::too_many_arguments)]
fn container(
  node: &SyntaxNode,
  text: &str,
  base: usize,
  path: &str,
  depth: usize,
  config: &LayoutConfig,
  cx: &Cx,
  out: &mut Vec<Container>,
) -> Result<(), String> {
  let split = split_container(node, text, base, "\n\n", config, cx)?;
  let names = names_used(&split.units);
  let units = split
    .units
    .iter()
    .enumerate()
    .map(|(at, unit)| UnitView {
      lines: (line_of(text, unit.span.0), line_of(text, unit.span.1)),
      kind: kind_name(unit),
      name: shown_name(unit),
      visibility: visibility_name(&unit.visibility),
      notes: notes(&split.units, &names, at, config),
    })
    .collect();
  let header_text = split.header.trim();
  let header = (!header_text.is_empty()).then(|| {
    let start = base + split.header.len() - split.header.trim_start().len();
    (
      line_of(text, start),
      line_of(text, base + split.header.len()),
    )
  });
  out.push(Container {
    path: path.to_string(),
    depth,
    header,
    units,
    tail: !split.tail.trim().is_empty(),
  });
  for element in node.children_with_tokens_iter() {
    let SyntaxElement::Node(child) = element else {
      continue;
    };
    if child.kind() != SyntaxKind::Module {
      continue;
    }
    let Some(body) = child
      .children_iter()
      .find(|c| c.kind() == SyntaxKind::ItemList)
    else {
      continue;
    };
    let name = Item::cast(child.clone())
      .and_then(|item| item.name())
      .unwrap_or_default();
    let inner = if path.is_empty() {
      format!("mod {}", name)
    } else {
      format!("{}::{}", path, name)
    };
    let open_end = body.text_range().start as usize + 1;
    container(&body, text, open_end, &inner, depth + 1, config, cx, out)?;
  }
  Ok(())
}

fn kind_name(unit: &Unit) -> &'static str {
  match unit.kind {
    ItemKind::ModDecl => "mod",
    ItemKind::UseDecl => "use",
    ItemKind::MacroDecl => "macro_rules",
    ItemKind::MacroDef => "macro",
    ItemKind::ConstDecl => "const",
    ItemKind::StaticDecl => "static",
    ItemKind::TypeAlias => "type",
    ItemKind::TraitDecl => "trait",
    ItemKind::EnumDecl => "enum",
    ItemKind::StructDecl => "struct",
    ItemKind::Union => "union",
    ItemKind::ImplBlock => "impl",
    ItemKind::ExternBlock => "extern",
    ItemKind::FnDecl => "fn",
    ItemKind::Unknown if unit.comments_only => "comments",
    ItemKind::Unknown if unit.identifier == "SkippedBlock" => "region",
    ItemKind::Unknown if unit.macro_call => "macro call",
    ItemKind::Unknown => "item",
  }
}

fn move_view(text: &str, moved: &Move) -> MoveView {
  let unit_kind = |kind: &ItemKind| {
    kind_name(&Unit {
      kind: kind.clone(),
      ..super::classify::pinned_region("", 0, 0, 0)
    })
  };
  MoveView {
    line: line_of(text, moved.span.0),
    kind: unit_kind(&moved.kind),
    name: moved.identifier.clone(),
    before: moved
      .before
      .as_ref()
      .map(|(kind, name)| (unit_kind(kind), name.clone())),
    rule: moved.rule,
  }
}

/// Why the unit at `at` was cut and classed as it was.
fn notes(units: &[Unit], names: &[HashSet<&str>], at: usize, config: &LayoutConfig) -> Vec<String> {
  let unit = &units[at];
  let mut notes = Vec::new();
  if unit.boundary {
    notes.push(match config.sections {
      Sections::Markers => "starts a section: `// mto: section`".to_string(),
      Sections::Headings => "starts a section: reads as a heading".to_string(),
      _ => "starts a section: a comment with a blank line after it".to_string(),
    });
  } else if unit.comments_only {
    notes.push("kept in place: a loose comment that is not a heading".to_string());
  } else if unit.pinned {
    notes.push(format!("pinned, keeps its slot: {}", pin_reason(unit)));
  }
  if unit.paragraph {
    notes.push("a paragraph: items written with no blank line between them".to_string());
  } else if unit.cfg && unit.text.matches("#[cfg").count() > 1 {
    notes.push("#[cfg] variants written together, one unit".to_string());
  }
  if unit.test_module {
    notes.push(if config.test_module_in_place {
      "#[cfg(test)] module, kept in place".to_string()
    } else {
      "#[cfg(test)] module: goes last".to_string()
    });
  }
  if unit.macro_use {
    notes.push("#[macro_use] module: goes before the other modules".to_string());
  }
  if unit.defines_macros && !unit.pinned {
    notes.push("defines macros: nothing written after it moves above it".to_string());
  }
  if let Some(group) = &unit.import_group {
    notes.push(format!(
      "import group: {}",
      match group {
        ImportGroup::Std => "standard library",
        ImportGroup::External => "external crate",
        ImportGroup::Local => "this crate",
      }
    ));
  }
  if config.keep_helpers_near {
    match (unit.caller, helper_verdict(units, names, at)) {
      (Some(caller), _) => notes.push(format!(
        "helper: stays above `{}`, its one user",
        units[caller].identifier
      )),
      (None, Ok(_)) => notes.push("not a helper: its helpers lead back to it".to_string()),
      // Only an item that could be one is worth a word.
      (None, Err(why))
        if unit.visibility == Visibility::Private
          && !unit.comments_only
          && !why.starts_with("not a kind")
          && !unit.pinned
          && !unit.paragraph =>
      {
        notes.push(format!("not a helper: {}", why))
      }
      _ => {}
    }
  }
  notes
}

/// What pins `unit`, as best its text tells.
fn pin_reason(unit: &Unit) -> &'static str {
  if unit.identifier == "SkippedBlock" {
    "a `// mto: off` region"
  } else if unit.macro_call {
    "an item macro call"
  } else if unit.text.contains("mto: pin") {
    "`// mto: pin`"
  } else if unit.text.contains("rustfmt::skip") {
    "`#[rustfmt::skip]`"
  } else if unit.test_module {
    "`test_module = \"in_place\"`"
  } else {
    "`extern crate`"
  }
}

/// The name shown for `unit`: its own, or for comments their first line.
fn shown_name(unit: &Unit) -> String {
  if unit.comments_only || unit.identifier == "SkippedBlock" {
    return unit.text.lines().next().unwrap_or("").trim().to_string();
  }
  unit.identifier.clone()
}

fn visibility_name(visibility: &Visibility) -> &'static str {
  match visibility {
    Visibility::Pub => "pub",
    Visibility::PubCrate => "pub(crate)",
    Visibility::PubIn => "pub(in)",
    Visibility::PubSuper => "pub(super)",
    Visibility::Private => "",
  }
}
