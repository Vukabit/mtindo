//! `.hub.toml`: its shape (SPEC.md), read strictly, and checked as a whole.
//!
//! Reading rejects a key the engine does not know, with its line. Checking then parses
//! every expression and template, and follows every reference (a widget's source, a
//! screen's widgets, a style's colors, an action's widget or page), reporting all
//! problems at once.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::expr::Expr;
use crate::template::Template;

/// The styles the engine draws with itself.
pub const ENGINE_STYLES: &[&str] = &["text", "title", "frame", "selected", "dim"];

pub const SOURCE_KINDS: &[&str] = &[
  "toml", "tsv", "lines", "command", "files", "git", "commands", "stream", "http",
];

/// Color names a terminal knows.
pub const TERMINAL_COLORS: &[&str] = &[
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "gray",
  "white",
  "dark-gray",
  "bright-black",
  "bright-red",
  "bright-green",
  "bright-yellow",
  "bright-blue",
  "bright-magenta",
  "bright-cyan",
  "reset",
];

pub const WIDGET_KINDS: &[&str] = &[
  "table",
  "list",
  "text",
  "kv",
  "gauges",
  "bars",
  "sparklines",
  "chart",
  "log",
  "file",
  "terminal",
];

/// The tables an include adds to, and those it fills in.
const ADDED: &[&str] = &["source", "widget", "screen", "action", "alert"];

/// One key's name, or a list of them.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum KeyNames {
  One(String),
  Many(Vec<String>),
}

/// A layout: a widget's id, or a split of more layouts.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum LayoutSpec {
  Widget(String),
  Split {
    split: String,
    sizes: Vec<Size>,
    children: Vec<LayoutSpec>,
  },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Persist {
  All(bool),
  Parts(Vec<String>),
}

/// A size: columns or rows as a number, or `"30%"`, `"min:N"`, `"max:N"`, `"fill"`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Size {
  Cells(u16),
  Text(String),
}

/// A style name, or rules: the first whose `when` holds gives the style.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum StyleRef {
  Name(String),
  Rules(Vec<StyleRule>),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionSpec {
  pub id: String,
  pub label: String,
  pub key: String,
  pub on: String,
  pub when: Option<String>,
  pub confirm: Option<String>,
  pub run: Option<String>,
  pub mode: Option<String>,
  pub then: Option<String>,
  pub toast: Option<String>,
  pub open: Option<String>,
  pub copy: Option<String>,
  /// A template given to `run` on its standard input (a patch for `git apply -`).
  pub input: Option<String>,
  /// Fields asked for before it runs; their values are `ask.<name>` in its templates.
  #[serde(default)]
  pub ask: Vec<AskSpec>,
  /// How long a `wait` action may run (`"5m"`); 30 seconds when left out.
  pub timeout: Option<String>,
  /// A command run just before, whose output is `before` in `undo` (what will be lost:
  /// `git rev-parse HEAD`, `git diff -- {sel.git-files.path | quote}`).
  pub before: Option<String>,
  /// The command that takes it back, kept once it succeeds for `undo` (ctrl+z): read
  /// against the selection as it was, with `before` and `output` (what it printed).
  pub undo: Option<String>,
  /// A template given to `undo` on its standard input.
  pub undo_input: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertSpec {
  pub source: String,
  pub when: String,
  pub key: String,
  pub text: String,
  pub style: Option<String>,
  pub open: Option<String>,
}

/// A chart's vertical lines, one per row of another source: commits on a time chart.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnotateSpec {
  pub source: String,
  /// Where each goes along x (a time or a number).
  pub at: String,
  /// What the crosshair says of the nearest one.
  pub text: String,
  pub style: Option<String>,
}

/// A field an action asks for.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskSpec {
  pub name: String,
  pub label: String,
  /// A template: the value the field starts with.
  pub default: Option<String>,
  /// Lines for the field; more than one takes newlines (a commit message's body).
  pub lines: Option<u16>,
  /// Whether it may be left empty.
  pub required: Option<bool>,
}

/// `[bindings]`: the engine's keys with vim keys off (`default`) and on (`vim`).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingsSpec {
  #[serde(default)]
  pub default: BTreeMap<String, KeyNames>,
  #[serde(default)]
  pub vim: BTreeMap<String, KeyNames>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnSpec {
  pub header: String,
  pub value: String,
  pub width: Option<Size>,
  pub align: Option<String>,
  pub style: Option<StyleRef>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
  pub hub: Hub,
  #[serde(default)]
  pub theme: BTreeMap<String, String>,
  /// Colors over `[theme]`'s on a light terminal (`[hub] background`).
  #[serde(default)]
  pub light: BTreeMap<String, String>,
  #[serde(default)]
  pub styles: BTreeMap<String, StyleSpec>,
  #[serde(default, rename = "source")]
  pub sources: Vec<SourceSpec>,
  #[serde(default, rename = "widget")]
  pub widgets: Vec<WidgetSpec>,
  #[serde(default, rename = "screen")]
  pub screens: Vec<ScreenSpec>,
  #[serde(default, rename = "action")]
  pub actions: Vec<ActionSpec>,
  #[serde(default)]
  pub bindings: BindingsSpec,
  #[serde(default)]
  pub topbar: Topbar,
  #[serde(default)]
  pub footer: Footer,
  #[serde(default, rename = "alert")]
  pub alerts: Vec<AlertSpec>,
  pub palette: Option<PaletteSpec>,
  #[serde(default, rename = "variable")]
  pub variables: Vec<VariableSpec>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Footer {
  #[serde(default)]
  pub extra: Vec<(String, String)>,
}

/// An included file: a config without its own `[hub]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)] // read only to find mistakes with their lines
struct Fragment {
  #[serde(default)]
  theme: BTreeMap<String, String>,
  #[serde(default)]
  styles: BTreeMap<String, StyleSpec>,
  #[serde(default, rename = "source")]
  sources: Vec<SourceSpec>,
  #[serde(default, rename = "widget")]
  widgets: Vec<WidgetSpec>,
  #[serde(default, rename = "screen")]
  screens: Vec<ScreenSpec>,
  #[serde(default, rename = "action")]
  actions: Vec<ActionSpec>,
  #[serde(default)]
  bindings: BindingsSpec,
  #[serde(default)]
  topbar: Topbar,
  #[serde(default)]
  footer: Footer,
  #[serde(default, rename = "alert")]
  alerts: Vec<AlertSpec>,
  palette: Option<PaletteSpec>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupSpec {
  pub by: String,
  #[serde(default)]
  pub list: Vec<String>,
  #[serde(default)]
  pub last: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hub {
  pub name: String,
  pub refresh: Option<String>,
  pub start: String,
  pub min_width: Option<u16>,
  /// Below this many rows, a notice instead of the interface (8).
  pub min_height: Option<u16>,
  pub narrow_below: Option<u16>,
  pub state: Option<String>,
  pub vim: Option<bool>,
  /// Whether the hub takes the mouse (the default): clicks, the wheel.
  pub mouse: Option<bool>,
  /// Whether it draws in color (the default); off, it keeps bold and reversed only.
  pub color: Option<bool>,
  /// How long a toast is shown (`"6s"`, the default); a failure's three times as long.
  pub toast: Option<String>,
  /// Whether anything moves on its own (the default); off, the spinner stands still.
  pub motion: Option<bool>,
  /// Which colors: `"auto"` (the default: `[light]`'s over `[theme]`'s when the terminal's
  /// background is light, as it says when asked), `"light"` or `"dark"`.
  pub background: Option<String>,
  /// The terminal's own colors only, strong: bold and reversed for what stands out.
  pub contrast: Option<bool>,
  /// How a new alert or a failed action reaches the desktop: `"osc"` (the default:
  /// OSC 9 and OSC 777 notifications), `"bell"`, `"both"` or `"none"`.
  pub notify: Option<String>,
  /// Where the terminal's cursor is when nothing is being typed: hidden (the default), or
  /// `"follow"`, on the focused pane's selection, for screen readers and magnifiers.
  pub cursor: Option<String>,
  /// More `.hub.toml` files, relative to this one, whose sources, widgets, screens,
  /// actions and alerts are added to this file's, and whose theme, styles and bindings
  /// fill in what this file leaves out.
  #[serde(default)]
  #[allow(dead_code)] // read as TOML before the config is, in `with_includes`
  pub include: Vec<String>,
  /// What of the session is kept across restarts: `true` (all, the default), `false`,
  /// or some of `crate::session::PARTS`.
  pub persist: Option<Persist>,
  /// Whether the config is read again when one of its files changes (the default).
  pub reload: Option<bool>,
}

impl Hub {
  /// Whether part `part` of the session is kept.
  pub fn persists(&self, part: &str) -> bool {
    match &self.persist {
      None => true,
      Some(Persist::All(all)) => *all,
      Some(Persist::Parts(parts)) => parts.iter().any(|p| p == part),
    }
  }
}

/// A config read and checked, with every expression and template parsed once.
pub struct Loaded {
  pub config: Config,
  /// The folder paths are relative to.
  pub base: PathBuf,
  pub path: PathBuf,
  /// The config's files: its own and those it includes, watched for changes.
  pub files: Vec<PathBuf>,
  pub templates: HashMap<String, Template>,
  pub exprs: HashMap<String, Expr>,
  pub bindings: crate::bindings::Bindings,
}

impl Loaded {
  pub fn template(&self, text: &str) -> &Template {
    &self.templates[text]
  }

  pub fn expr(&self, text: &str) -> &Expr {
    &self.exprs[text]
  }

  pub fn widget(&self, id: &str) -> Option<&WidgetSpec> {
    self.config.widgets.iter().find(|w| w.id == id)
  }

  pub fn screen(&self, id: &str) -> Option<&ScreenSpec> {
    self.config.screens.iter().find(|s| s.id == id)
  }

  pub fn source(&self, name: &str) -> Option<&SourceSpec> {
    self.config.sources.iter().find(|s| s.name == name)
  }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkSpec {
  pub value: String,
  pub label: Option<String>,
  pub style: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteSpec {
  pub source: String,
  pub run: String,
  pub background: Option<String>,
  pub presets: Option<String>,
  pub recent: Option<RecentSpec>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecentSpec {
  pub source: String,
  pub command: String,
  pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenSpec {
  pub id: String,
  pub title: String,
  pub key: Option<String>,
  #[serde(default)]
  pub page: bool,
  pub layout: LayoutSpec,
  pub narrow: Option<LayoutSpec>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Segment {
  pub text: String,
  pub source: Option<String>,
  pub count: Option<String>,
  pub when: Option<String>,
  pub style: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSpec {
  pub name: String,
  pub kind: String,
  pub glob: Option<String>,
  pub header: Option<bool>,
  pub columns: Option<Vec<String>>,
  pub pattern: Option<String>,
  pub run: Option<String>,
  pub parse: Option<String>,
  pub group: Option<GroupSpec>,
  #[serde(default)]
  pub computed: BTreeMap<String, String>,
  pub refresh: Option<String>,
  /// A template whose words follow `run` (or are a git source's path, commit or log
  /// arguments): read where the source is shown, so it can follow a selection.
  pub args: Option<String>,
  /// What a `git` source reads (crate::git::OF).
  pub of: Option<String>,
  /// The most rows a `git` log gives.
  pub limit: Option<usize>,
  /// How many more rows it gives each time a selection reaches the last of them.
  pub page: Option<usize>,
  /// What an `http` source reads.
  pub url: Option<String>,
  /// Headers an `http` source sends (`"Authorization: Bearer $TOKEN"`: `$NAME` is the
  /// environment's).
  #[serde(default)]
  pub headers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleRule {
  pub when: Option<String>,
  pub style: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleSpec {
  pub fg: Option<String>,
  pub bg: Option<String>,
  pub bold: Option<bool>,
  pub italic: Option<bool>,
  pub underline: Option<bool>,
  pub reverse: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Topbar {
  #[serde(default)]
  pub segments: Vec<Segment>,
}

/// A value every template reads as `var.<name>`, chosen among `values` (or the values of
/// `field` in `source`'s rows) by its key or `:let NAME VALUE`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariableSpec {
  pub name: String,
  #[serde(default)]
  pub values: Vec<String>,
  pub source: Option<String>,
  pub field: Option<String>,
  pub default: Option<String>,
  /// Goes to its next value.
  pub key: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetSpec {
  pub id: String,
  #[serde(rename = "type")]
  pub kind: String,
  pub title: Option<String>,
  pub source: Option<String>,
  pub filter: Option<String>,
  #[serde(default)]
  pub sort: Vec<String>,
  pub limit: Option<usize>,
  pub empty: Option<String>,
  pub border: Option<bool>,
  // table
  #[serde(default)]
  pub columns: Vec<ColumnSpec>,
  pub selectable: Option<bool>,
  pub open: Option<String>,
  // list
  pub item: Option<String>,
  pub style: Option<StyleRef>,
  // text, kv
  #[serde(default)]
  pub lines: Vec<String>,
  pub row: Option<String>,
  #[serde(default)]
  pub pairs: Vec<(String, String)>,
  // gauges, bars, sparklines
  pub label: Option<String>,
  pub value: Option<String>,
  pub max: Option<String>,
  pub target: Option<String>,
  pub text: Option<String>,
  pub values: Option<String>,
  // chart
  pub x: Option<String>,
  pub y: Option<String>,
  pub series: Option<String>,
  #[serde(default)]
  pub marks: Vec<MarkSpec>,
  pub annotate: Option<AnnotateSpec>,
  pub y_label: Option<String>,
  // log, file
  pub path: Option<String>,
  pub follow: Option<bool>,
  pub level: Option<String>,
  /// A log's section starts, as a regular expression: sections fold to their first line.
  pub fold: Option<String>,
  #[serde(default)]
  pub styles: BTreeMap<String, String>,
  pub markdown: Option<bool>,
  // terminal
  pub command: Option<String>,
  pub cwd: Option<String>,
  pub escape: Option<KeyNames>,
  /// `"tmux"`: each tab's program runs in a tmux session of its own, which outlives the
  /// hub and is taken up again when it starts.
  pub keep: Option<String>,
}

impl WidgetSpec {
  /// The keys that give a terminal's keys back, when it has its own; else `[bindings]`
  /// `release` gives them.
  pub fn escape_keys(&self) -> Option<Vec<String>> {
    match &self.escape {
      Some(KeyNames::One(k)) => Some(vec![k.clone()]),
      Some(KeyNames::Many(ks)) => Some(ks.clone()),
      None => None,
    }
  }
}

/// The nearest `.hub.toml`, from `start` upwards.
pub fn find(start: &Path) -> Option<PathBuf> {
  let mut dir = Some(start);
  while let Some(here) = dir {
    let candidate = here.join(".hub.toml");
    if candidate.is_file() {
      return Some(candidate);
    }
    dir = here.parent();
  }
  None
}

/// Reads and checks `path`; every problem found, if any.
pub fn load(path: &Path) -> Result<Loaded, Vec<String>> {
  let text = std::fs::read_to_string(path)
    .map_err(|e| vec![format!("cannot read {}: {}", path.display(), e)])?;
  let base = path.parent().unwrap_or(Path::new("."));
  let config = with_includes(path, base, &text)?;
  let files = std::iter::once(path.to_path_buf())
    .chain(config.hub.include.iter().map(|i| base.join(i)))
    .collect();
  let mut loaded = Loaded {
    config,
    base: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
    path: path.to_path_buf(),
    files,
    templates: HashMap::new(),
    exprs: HashMap::new(),
    bindings: Default::default(),
  };
  default_styles(&mut loaded.config.styles);
  let problems = check(&mut loaded);
  if problems.is_empty() {
    loaded.bindings = crate::bindings::Bindings::new(&loaded.config);
    Ok(loaded)
  } else {
    Err(problems)
  }
}

/// Parses every template and expression and follows every reference.
fn check(loaded: &mut Loaded) -> Vec<String> {
  let mut problems = Vec::new();
  let mut templates: Vec<(String, String)> = Vec::new();
  let mut exprs: Vec<(String, String)> = Vec::new();
  let c = &loaded.config;
  let color = |name: &str| -> bool {
    c.theme.contains_key(name) || name.starts_with('#') || TERMINAL_COLORS.contains(&name)
  };
  for (name, value) in &c.theme {
    if !value.starts_with('#') || value.len() != 7 || u32::from_str_radix(&value[1..], 16).is_err()
    {
      problems.push(format!("theme {}: `{}` is not #rrggbb", name, value));
    }
  }
  for (name, style) in &c.styles {
    for color_name in [&style.fg, &style.bg].into_iter().flatten() {
      if !color(color_name) {
        problems.push(format!(
          "style {}: no color `{}` in [theme]",
          name, color_name
        ));
      }
    }
  }
  for name in ENGINE_STYLES {
    if !c.styles.contains_key(*name) {
      problems.push(format!(
        "[styles] has no `{}`, which the engine draws with",
        name
      ));
    }
  }
  let style_ok = |name: &str| c.styles.contains_key(name);
  let check_style_ref = |what: &str,
                         r: &Option<StyleRef>,
                         problems: &mut Vec<String>,
                         exprs: &mut Vec<(String, String)>| {
    match r {
      Some(StyleRef::Name(n)) if !style_ok(n) => {
        problems.push(format!("{}: no style `{}`", what, n))
      }
      Some(StyleRef::Rules(rules)) => {
        for rule in rules {
          if !style_ok(&rule.style) {
            problems.push(format!("{}: no style `{}`", what, rule.style));
          }
          if let Some(w) = &rule.when {
            exprs.push((format!("{} rule", what), w.clone()));
          }
        }
      }
      _ => {}
    }
  };
  let source_names: Vec<&str> = c.sources.iter().map(|s| s.name.as_str()).collect();
  for s in &c.sources {
    let what = format!("source {}", s.name);
    if !SOURCE_KINDS.contains(&s.kind.as_str()) {
      problems.push(format!(
        "{}: no kind `{}` (one of {})",
        what,
        s.kind,
        SOURCE_KINDS.join(", ")
      ));
    }
    let needs: &[(&str, bool)] = match s.kind.as_str() {
      "toml" | "tsv" | "files" => &[("glob", s.glob.is_some())],
      "lines" => &[
        ("pattern", s.pattern.is_some()),
        ("glob or run", s.glob.is_some() || s.run.is_some()),
      ],
      "command" => &[("run", s.run.is_some()), ("parse", s.parse.is_some())],
      "commands" => &[("run", s.run.is_some())],
      "stream" => &[("run", s.run.is_some())],
      "http" => &[("url", s.url.is_some()), ("parse", s.parse.is_some())],
      _ => &[],
    };
    for (key, present) in needs {
      if !present {
        problems.push(format!("{}: a `{}` source needs `{}`", what, s.kind, key));
      }
    }
    if let Some(args) = &s.args {
      templates.push((format!("{} args", what), args.clone()));
    }
    if let Some(of) = &s.of {
      if s.kind != "git" {
        problems.push(format!("{}: `of` belongs to a git source", what));
      } else if !crate::git::OF.contains(&of.as_str()) {
        problems.push(format!(
          "{}: git has no `{}` (one of {})",
          what,
          of,
          crate::git::OF.join(", ")
        ));
      }
    }
    if s.kind == "tsv" && s.header != Some(true) && s.columns.is_none() {
      problems.push(format!(
        "{}: a tsv source needs `header = true` or `columns`",
        what
      ));
    }
    if let Some(p) = &s.pattern
      && let Err(e) = regex::Regex::new(p)
    {
      problems.push(format!("{}: pattern: {}", what, e));
    }
    if let Some(parse) = &s.parse
      && !["lines", "tsv", "json", "text"].contains(&parse.as_str())
    {
      problems.push(format!(
        "{}: no parse `{}` (lines, tsv, json, text)",
        what, parse
      ));
    }
    for (field, e) in &s.computed {
      exprs.push((format!("{} computed {}", what, field), e.clone()));
    }
    for d in [&s.refresh].into_iter().flatten() {
      if crate::source::duration(d).is_none() {
        problems.push(format!("{}: refresh `{}` is not a duration", what, d));
      }
    }
  }
  if source_names.len() != c.sources.len() {
    problems.push("two sources share a name".to_string());
  }
  let widget_ids: Vec<&str> = c.widgets.iter().map(|w| w.id.as_str()).collect();
  let page_ids: Vec<&str> = c
    .screens
    .iter()
    .filter(|s| s.page)
    .map(|s| s.id.as_str())
    .collect();
  for w in &c.widgets {
    let what = format!("widget {}", w.id);
    if !WIDGET_KINDS.contains(&w.kind.as_str()) {
      problems.push(format!(
        "{}: no type `{}` (one of {})",
        what,
        w.kind,
        WIDGET_KINDS.join(", ")
      ));
    }
    if let Some(s) = &w.source
      && !source_names.contains(&s.as_str())
    {
      problems.push(format!("{}: no source `{}`", what, s));
    }
    let needs_source = !matches!(w.kind.as_str(), "text" | "log" | "file" | "terminal");
    if needs_source && w.source.is_none() {
      problems.push(format!("{}: a {} needs a `source`", what, w.kind));
    }
    if let Some(open) = &w.open
      && !page_ids.contains(&open.as_str())
    {
      problems.push(format!("{}: open: no page `{}`", what, open));
    }
    if let Some(f) = &w.filter {
      exprs.push((format!("{} filter", what), f.clone()));
    }
    for t in [&w.title, &w.empty, &w.item, &w.text, &w.label, &w.path]
      .into_iter()
      .flatten()
    {
      templates.push((what.clone(), t.clone()));
    }
    for e in [
      &w.value, &w.max, &w.target, &w.values, &w.x, &w.y, &w.series,
    ]
    .into_iter()
    .flatten()
    {
      exprs.push((what.clone(), e.clone()));
    }
    for line in &w.lines {
      templates.push((what.clone(), line.clone()));
    }
    for (_, t) in &w.pairs {
      templates.push((what.clone(), t.clone()));
    }
    for (i, col) in w.columns.iter().enumerate() {
      templates.push((format!("{} column {}", what, i + 1), col.value.clone()));
      check_style_ref(
        &format!("{} column {}", what, i + 1),
        &col.style,
        &mut problems,
        &mut exprs,
      );
    }
    check_style_ref(&what, &w.style, &mut problems, &mut exprs);
    if let Some(a) = &w.annotate {
      if !source_names.contains(&a.source.as_str()) {
        problems.push(format!("{} annotate: no source `{}`", what, a.source));
      }
      exprs.push((format!("{} annotate at", what), a.at.clone()));
      templates.push((format!("{} annotate text", what), a.text.clone()));
      if let Some(s) = &a.style
        && !style_ok(s)
      {
        problems.push(format!("{} annotate: no style `{}`", what, s));
      }
    }
    for mark in &w.marks {
      exprs.push((format!("{} mark", what), mark.value.clone()));
      if let Some(s) = &mark.style
        && !style_ok(s)
      {
        problems.push(format!("{} mark: no style `{}`", what, s));
      }
    }
    for s in w.styles.values() {
      if !style_ok(s) {
        problems.push(format!("{} styles: no style `{}`", what, s));
      }
    }
    if let Some(level) = &w.level
      && let Err(e) = regex::Regex::new(level)
    {
      problems.push(format!("{} level: {}", what, e));
    }
    if let Some(fold) = &w.fold
      && let Err(e) = regex::Regex::new(fold)
    {
      problems.push(format!("{} fold: {}", what, e));
    }
    let needs: &[(&str, bool)] = match w.kind.as_str() {
      "table" => &[("columns", !w.columns.is_empty())],
      "list" => &[("item", w.item.is_some())],
      "gauges" | "bars" => &[("label", w.label.is_some()), ("value", w.value.is_some())],
      "sparklines" => &[("label", w.label.is_some()), ("values", w.values.is_some())],
      "chart" => &[("x", w.x.is_some()), ("y", w.y.is_some())],
      "log" | "file" => &[("path", w.path.is_some())],
      _ => &[],
    };
    if let Some(keys) = w.escape_keys() {
      for key in keys {
        if crate::bindings::parse(&key).is_none() {
          problems.push(format!("{} escape: `{}` is not a key", what, key));
        }
      }
    }
    if w.kind != "terminal" && (w.command.is_some() || w.cwd.is_some() || w.escape.is_some()) {
      problems.push(format!(
        "{}: `command`, `cwd` and `escape` belong to a terminal",
        what
      ));
    }
    for (key, present) in needs {
      if !present {
        problems.push(format!("{}: a {} needs `{}`", what, w.kind, key));
      }
    }
  }
  for s in &c.screens {
    let what = format!("screen {}", s.id);
    templates.push((what.clone(), s.title.clone()));
    for layout in [Some(&s.layout), s.narrow.as_ref()].into_iter().flatten() {
      check_layout(&what, layout, &widget_ids, &mut problems);
    }
    if s.page && s.key.is_some() {
      problems.push(format!(
        "{}: a page (page = true) has no `key`; it is opened, not switched to",
        what
      ));
    }
  }
  if let Some(cursor) = &c.hub.cursor
    && cursor != "follow"
    && cursor != "hidden"
  {
    problems.push(format!(
      "[hub] cursor: `{}` is not hidden or follow",
      cursor
    ));
  }
  for w in &c.widgets {
    if let Some(keep) = &w.keep
      && keep != "tmux"
    {
      problems.push(format!("widget {} keep: `{}` is not tmux", w.id, keep));
    }
  }
  if let Some(b) = &c.hub.background
    && !["auto", "light", "dark"].contains(&b.as_str())
  {
    problems.push(format!(
      "[hub] background: `{}` is not auto, light or dark",
      b
    ));
  }
  for (name, color) in &c.light {
    if crate::widgets::parse_color(color).is_none() {
      problems.push(format!("[light] {}: `{}` is not a color", name, color));
    }
  }
  if let Some(notify) = &c.hub.notify
    && !["osc", "bell", "both", "none"].contains(&notify.as_str())
  {
    problems.push(format!(
      "[hub] notify: `{}` is not osc, bell, both or none",
      notify
    ));
  }
  if let Some(Persist::Parts(parts)) = &c.hub.persist {
    for part in parts {
      if !crate::session::PARTS.contains(&part.as_str()) {
        problems.push(format!(
          "[hub] persist: no part `{}` (one of {})",
          part,
          crate::session::PARTS.join(", ")
        ));
      }
    }
  }
  if !c.screens.iter().any(|s| s.id == c.hub.start && !s.page) {
    problems.push(format!("[hub] start: no screen `{}`", c.hub.start));
  }
  problems.extend(crate::bindings::check(c));
  for (key, _) in &c.footer.extra {
    if let Some(name) = key.strip_prefix('{').and_then(|k| k.strip_suffix('}'))
      && name != "screens"
      && !crate::bindings::ACTIONS.iter().any(|(a, _)| *a == name)
    {
      problems.push(format!(
        "[footer] extra: `{}` names no action of [bindings] (nor `{{screens}}`)",
        key
      ));
    }
  }
  let screen_ids: Vec<&str> = c.screens.iter().map(|s| s.id.as_str()).collect();
  for a in &c.actions {
    let what = format!("action {}", a.id);
    if a.on != "global"
      && !widget_ids.contains(&a.on.as_str())
      && !screen_ids.contains(&a.on.as_str())
    {
      problems.push(format!("{}: on: no widget or screen `{}`", what, a.on));
    }
    if crate::keys::parse(&a.key).is_none() {
      problems.push(format!("{}: `{}` is not a key", what, a.key));
    }
    let does = [&a.run, &a.open, &a.copy]
      .iter()
      .filter(|x| x.is_some())
      .count();
    if does != 1 {
      problems.push(format!(
        "{}: needs exactly one of `run`, `open`, `copy`",
        what
      ));
    }
    if let Some(open) = &a.open
      && !page_ids.contains(&open.as_str())
    {
      problems.push(format!("{}: open: no page `{}`", what, open));
    }
    if let Some(mode) = &a.mode
      && !["wait", "background", "terminal"].contains(&mode.as_str())
    {
      problems.push(format!(
        "{}: no mode `{}` (wait, background, terminal)",
        what, mode
      ));
    }
    if let Some(then) = &a.then {
      let ok = matches!(then.as_str(), "refresh" | "back" | "toast")
        || then
          .strip_prefix("open:")
          .is_some_and(|p| page_ids.contains(&p))
        || then
          .strip_prefix("screen:")
          .is_some_and(|p| screen_ids.contains(&p));
      if !ok {
        problems.push(format!(
          "{}: then `{}` is not refresh, back, toast, open:PAGE or screen:ID",
          what, then
        ));
      }
    }
    if let Some(w) = &a.when {
      exprs.push((what.clone(), w.clone()));
    }
    for t in [
      &a.confirm,
      &a.run,
      &a.toast,
      &a.copy,
      &a.input,
      &a.before,
      &a.undo,
      &a.undo_input,
    ]
    .into_iter()
    .flatten()
    {
      templates.push((what.clone(), t.clone()));
    }
    for f in &a.ask {
      if let Some(d) = &f.default {
        templates.push((format!("{} ask {}", what, f.name), d.clone()));
      }
    }
    if let Some(t) = &a.timeout
      && crate::source::duration(t).is_none()
    {
      problems.push(format!(
        "{}: timeout `{}` is not a duration (\"5m\")",
        what, t
      ));
    }
    let waits = a.run.is_some() && a.mode.as_deref().is_none_or(|m| m == "wait");
    if a.input.is_some() && !waits {
      problems.push(format!("{}: `input` goes with `run` in `wait` mode", what));
    }
    if (a.before.is_some() || a.undo.is_some()) && !waits {
      problems.push(format!(
        "{}: `before` and `undo` go with `run` in `wait` mode",
        what
      ));
    }
    if (a.before.is_some() || a.undo_input.is_some()) && a.undo.is_none() {
      problems.push(format!(
        "{}: `before` and `undo_input` are for `undo`, which it has not",
        what
      ));
    }
  }
  for (i, seg) in c.topbar.segments.iter().enumerate() {
    let what = format!("topbar segment {}", i + 1);
    templates.push((what.clone(), seg.text.clone()));
    for e in [&seg.count, &seg.when].into_iter().flatten() {
      exprs.push((what.clone(), e.clone()));
    }
    if let Some(s) = &seg.source
      && !source_names.contains(&s.as_str())
    {
      problems.push(format!("{}: no source `{}`", what, s));
    }
    if let Some(s) = &seg.style
      && !style_ok(s)
    {
      problems.push(format!("{}: no style `{}`", what, s));
    }
  }
  for v in &c.variables {
    let what = format!("variable {}", v.name);
    match (&v.source, &v.field, v.values.is_empty()) {
      (Some(src), Some(_), true) => {
        if !source_names.contains(&src.as_str()) {
          problems.push(format!("{}: no source `{}`", what, src));
        }
      }
      (None, None, false) => {}
      _ => problems.push(format!("{}: give `values`, or `source` and `field`", what)),
    }
    if let Some(k) = &v.key
      && crate::keys::parse(k).is_none()
    {
      problems.push(format!("{}: key `{}` is not a key", what, k));
    }
  }
  for (i, alert) in c.alerts.iter().enumerate() {
    let what = format!("alert {}", i + 1);
    if !source_names.contains(&alert.source.as_str()) {
      problems.push(format!("{}: no source `{}`", what, alert.source));
    }
    exprs.push((what.clone(), alert.when.clone()));
    templates.push((what.clone(), alert.text.clone()));
    if let Some(open) = &alert.open
      && !page_ids.contains(&open.as_str())
    {
      problems.push(format!("{}: open: no page `{}`", what, open));
    }
  }
  if let Some(p) = &c.palette {
    if !c
      .sources
      .iter()
      .any(|s| s.name == p.source && s.kind == "commands")
    {
      problems.push(format!(
        "[palette] source: no `commands` source `{}`",
        p.source
      ));
    }
    if let Some(r) = &p.recent
      && !source_names.contains(&r.source.as_str())
    {
      problems.push(format!("[palette] recent: no source `{}`", r.source));
    }
  }
  for (what, text) in templates {
    match Template::parse(&text) {
      Ok(t) => {
        loaded.templates.insert(text, t);
      }
      Err(e) => problems.push(format!("{}: {}", what, e)),
    }
  }
  for (what, text) in exprs {
    match Expr::parse(&text) {
      Ok(e) => {
        loaded.exprs.insert(text, e);
      }
      Err(e) => problems.push(format!("{}: `{}`: {}", what, text, e)),
    }
  }
  problems
}

fn check_layout(what: &str, layout: &LayoutSpec, widgets: &[&str], problems: &mut Vec<String>) {
  match layout {
    LayoutSpec::Widget(id) => {
      if !widgets.contains(&id.as_str()) {
        problems.push(format!("{}: layout: no widget `{}`", what, id));
      }
    }
    LayoutSpec::Split {
      split,
      sizes,
      children,
    } => {
      if split != "rows" && split != "columns" {
        problems.push(format!(
          "{}: split `{}` is not rows or columns",
          what, split
        ));
      }
      if sizes.len() != children.len() {
        problems.push(format!(
          "{}: {} sizes for {} children",
          what,
          sizes.len(),
          children.len()
        ));
      }
      for size in sizes {
        if crate::layout::constraint(size).is_none() {
          problems.push(format!(
            "{}: size {:?} is not N, \"N%\", \"min:N\", \"max:N\" or \"fill\"",
            what, size
          ));
        }
      }
      for child in children {
        check_layout(what, child, widgets, problems);
      }
    }
  }
}

/// The styles the engine draws with, for a config that leaves them out: the selected row
/// reversed, so it shows without color; titles and keys bold.
fn default_styles(styles: &mut BTreeMap<String, StyleSpec>) {
  let style = |fg: Option<&str>, bold: bool, reverse: bool| StyleSpec {
    fg: fg.map(str::to_string),
    bold: bold.then_some(true),
    reverse: reverse.then_some(true),
    ..StyleSpec::default()
  };
  for (name, spec) in [
    ("text", style(None, false, false)),
    ("title", style(None, true, false)),
    ("frame", style(Some("bright-black"), false, false)),
    ("selected", style(None, false, true)),
    ("dim", style(Some("bright-black"), false, false)),
    ("key", style(None, true, false)),
    ("ok", style(Some("green"), false, false)),
    ("bad", style(Some("red"), false, false)),
    ("warn", style(Some("yellow"), false, false)),
    ("info", style(Some("cyan"), false, false)),
  ] {
    styles.entry(name.to_string()).or_insert(spec);
  }
}

/// Fills `mine` with what it lacks of `theirs`, table by table.
fn fill(mine: &mut toml::Table, theirs: toml::Table) {
  for (key, value) in theirs {
    match (mine.get_mut(&key), value) {
      (Some(toml::Value::Table(m)), toml::Value::Table(t)) => fill(m, t),
      (Some(_), _) => {}
      (None, value) => {
        mine.insert(key, value);
      }
    }
  }
}

/// The config in `text` with its includes merged in: their arrays of tables after this
/// file's, their tables under this file's (this file wins).
fn with_includes(path: &Path, base: &Path, text: &str) -> Result<Config, Vec<String>> {
  // Each file on its own first, so a mistake is reported with its file and line.
  let alone: Config =
    toml::from_str(text).map_err(|e| vec![format!("{}: {}", path.display(), e)])?;
  if alone.hub.include.is_empty() {
    return Ok(alone);
  }
  let mut main: toml::Table =
    toml::from_str(text).map_err(|e| vec![format!("{}: {}", path.display(), e)])?;
  let includes: Vec<String> = main
    .get("hub")
    .and_then(|h| h.get("include"))
    .and_then(|i| i.as_array())
    .map(|a| {
      a.iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
    })
    .unwrap_or_default();
  for include in includes {
    let file = base.join(&include);
    let text = std::fs::read_to_string(&file).map_err(|e| {
      vec![format!(
        "[hub] include: cannot read {}: {}",
        file.display(),
        e
      )]
    })?;
    if let Err(e) = toml::from_str::<Fragment>(&text) {
      return Err(vec![format!(
        "{} (an included file has no [hub]): {}",
        file.display(),
        e
      )]);
    }
    let extra: toml::Table =
      toml::from_str(&text).map_err(|e| vec![format!("{}: {}", file.display(), e)])?;
    for (key, value) in extra {
      match (main.get_mut(&key), value) {
        (Some(toml::Value::Array(mine)), toml::Value::Array(theirs))
          if ADDED.contains(&key.as_str()) =>
        {
          mine.extend(theirs)
        }
        (Some(toml::Value::Table(mine)), toml::Value::Table(theirs)) => fill(mine, theirs),
        (None, value) => {
          main.insert(key, value);
        }
        (Some(_), _) => {
          return Err(vec![format!(
            "{}: `{}` is not the same kind of thing as in {}",
            file.display(),
            key,
            path.display()
          )]);
        }
      }
    }
  }
  main
    .try_into()
    .map_err(|e: toml::de::Error| vec![format!("{} (with its includes): {}", path.display(), e)])
}
