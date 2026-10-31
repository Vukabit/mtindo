//! Layer 4: configuration files, read into settings.
//!
//! The settings for a folder come from, lowest precedence first:
//!
//! 1. the defaults in the settings table, for the style edition in effect;
//! 2. the nearest `.rustfmt.toml` or `rustfmt.toml`, found as the standard formatter finds
//!    it: in the folder, then each parent, then the home folder, then the standard
//!    formatter's folder in the user's configuration folder;
//! 3. every `.mto.toml` or `mto.toml` from the outermost folder down to the folder itself, a
//!    nearer file winning key by key; the search upwards stops at a file with `root = true`.
//!    An `mto.toml` may set the formatting keys too, and wins over `rustfmt.toml` with a warning
//!    when they disagree;
//! 4. the edition of the nearest `Cargo.toml`'s package, as `cargo fmt` passes it;
//! 5. the command line: `--config key=value`, `--edition`, `--style-edition`.
//!
//! `--config-path FILE` replaces steps 2 and 3 with that one file. The style edition is
//! resolved as the standard formatter resolves it: `style_edition`, else `version` (`One`
//! is 2015, `Two` is 2024), else `edition`, else 2015.
//!
//! Unknown keys and keys in the wrong file are warnings; a value of the wrong type or outside
//! the legal values is an error, naming the file, line and column.

mod cargo;
mod files;
pub mod toml;
pub mod user;
mod value;
pub mod workspace;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub use files::Environment;
pub use value::Value;

use crate::options::format::{FormatConfig, ImportsLayout, NewlineStyle};
use crate::options::layout::{ItemKind, LayoutConfig, NameSort, TraitImplSort, Visibility};
use crate::options::{self, Channels, File, Setting, Stability};
use crate::syntax::Edition;

/// Keys mto used to accept, and the key that replaced each.
const RENAMED: &[(&str, &str)] = &[("tab_width", "tab_spaces")];

/// The key that stops the search for `mto.toml` files upwards.
const ROOT_KEY: &str = "root";

/// The kinds of configuration file, which decide what they may hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
  /// `rustfmt.toml` or `.rustfmt.toml`: the standard formatter's settings.
  Standard,
  /// `mto.toml` or `.mto.toml`: mto's settings and the formatting settings.
  Mto,
  /// The file named by `--config-path`: like an `mto.toml`.
  Explicit,
  /// The per-user `config.toml`: tool behavior only.
  User,
  /// `Cargo.toml`, read for the edition.
  Cargo,
}

impl FileKind {
  fn may_hold(self, file: File) -> bool {
    match self {
      FileKind::Standard => file == File::Format,
      FileKind::Mto | FileKind::Explicit => file != File::User,
      FileKind::User => file == File::User,
      FileKind::Cargo => false,
    }
  }
}

/// Where a value came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
  Default,
  File {
    path: PathBuf,
    line: usize,
  },
  CommandLine,
  /// Derived from another setting (`style_edition` from `edition`).
  Derived(&'static str),
}

impl fmt::Display for Source {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Source::Default => write!(f, "default"),
      Source::File { path, line } => write!(f, "{}:{}", path.display(), line),
      Source::CommandLine => write!(f, "command line"),
      Source::Derived(from) => write!(f, "from {}", from),
    }
  }
}

/// The settings in effect for one folder.
#[derive(Clone, Debug)]
pub struct Config {
  values: BTreeMap<&'static str, (Value, Source)>,
  style_edition: u16,
  files: Vec<(FileKind, PathBuf)>,
}

impl Config {
  /// The configuration files that apply, lowest precedence first.
  pub fn files(&self) -> &[(FileKind, PathBuf)] {
    &self.files
  }

  /// These settings as the standard formatter's stable release reads them: a setting or a
  /// value only its nightly release knows is dropped, so its default applies.
  pub fn stable_only(&self) -> Config {
    let stable =
      |channels: Option<Channels>| channels.is_none_or(|c| c.stable == Stability::Stable);
    let mut config = self.clone();
    config.values.retain(|name, (value, _)| {
      let Some(setting) = options::all().find(|s| s.name == *name) else {
        return true;
      };
      let value_stable = value.as_text().is_none_or(|text| {
        setting
          .choices()
          .iter()
          .find(|c| c.name.eq_ignore_ascii_case(text))
          .is_none_or(|c| stable(c.channels))
      });
      stable(setting.channels) && value_stable
    });
    config
  }

  pub fn get(&self, name: &str) -> Option<&Value> {
    self.values.get(name).map(|(value, _)| value)
  }

  /// The layout settings.
  pub fn layout(&self) -> LayoutConfig {
    let mut config = LayoutConfig::default();
    let flag =
      |name: &str, fallback: bool| self.get(name).and_then(Value::as_bool).unwrap_or(fallback);
    config.sort_alphabetically = flag("sort_alphabetically", config.sort_alphabetically);
    config.group_impls = flag("group_impls", config.group_impls);
    config.sort_imports = flag("sort_imports", config.sort_imports);
    config.import_grouping = flag("import_grouping", config.import_grouping);
    let (lower, upper) = self.blank_line_bounds();
    if let Some(lines) = self
      .get("blank_lines_between_blocks")
      .and_then(Value::as_integer)
    {
      config.blank_lines_between_blocks = (lines as usize).clamp(lower, upper.max(lower));
    }
    config.blank_lines_tight = lower;
    if let Some(dirs) = self.get("ignore_dirs").and_then(Value::as_list) {
      // A relative path is read from the folder of the file that set it (the current
      // folder for the command line).
      let base = match self.source("ignore_dirs") {
        Some(Source::File { path, .. }) => path.parent().map(Path::to_path_buf),
        _ => std::env::current_dir().ok(),
      }
      .unwrap_or_default();
      let (paths, names): (Vec<&String>, Vec<&String>) = dirs
        .iter()
        .partition(|d| d.contains('/') || d.contains('\\'));
      config.ignore_dirs = names.into_iter().cloned().collect();
      config.ignore_paths = paths
        .into_iter()
        .map(|d| {
          let joined = base.join(d.trim_end_matches(['/', '\\']));
          crate::real_path(&joined).unwrap_or_else(|_| lexical(&joined))
        })
        .collect();
    }
    config.layout_generated_files = flag("layout_generated_files", config.layout_generated_files);
    if let Some(markers) = self.get("generated_markers").and_then(Value::as_list) {
      config.generated_markers = markers.to_vec();
    }
    if let Some(lines) = self
      .get("generated_marker_line_search_limit")
      .and_then(Value::as_integer)
    {
      config.generated_marker_line_search_limit = lines as usize;
    }
    if let Some(kinds) = self.get("type_order").and_then(Value::as_list) {
      config.type_order = kinds.iter().map(|k| ItemKind::from_name(k)).collect();
    }
    if let Some(width) = self.get("max_width").and_then(Value::as_integer) {
      config.max_width = width as usize;
    }
    if let Some(spaces) = self.get("tab_spaces").and_then(Value::as_integer) {
      config.tab_width = spaces as usize;
    }
    config.style_edition = self.style_edition;
    let year = |name: &str| self.get(name).and_then(Value::as_text)?.parse::<u16>().ok();
    config.edition = year("edition").unwrap_or(2015);
    config.import_sort = year("import_sort");
    config.name_sort = match self.get("name_sort").and_then(Value::as_text) {
      Some("bytes") => NameSort::Bytes,
      Some("case_insensitive") => NameSort::CaseInsensitive,
      _ => NameSort::Version,
    };
    config.reorder_impl_items = flag("reorder_impl_items", false);
    config.impl_method_order_trait =
      self.get("impl_method_order").and_then(Value::as_text) == Some("trait");
    config.test_module_in_place =
      self.get("test_module").and_then(Value::as_text) == Some("in_place");
    if self.get("trait_impl_sort").and_then(Value::as_text) == Some("original") {
      config.trait_impl_sort = TraitImplSort::Original;
    }
    if let Some(traits) = self.get("trait_impl_order").and_then(Value::as_list) {
      config.trait_impl_order = traits.to_vec();
    }
    if let Some(order) = self.get("visibility_order").and_then(Value::as_list) {
      config.visibility_order = order
        .iter()
        .filter_map(|v| Visibility::from_name(v))
        .collect();
    }
    config
  }

  /// `blank_lines_lower_bound` and `blank_lines_upper_bound`: the blank lines between
  /// items stay within them, so the standard formatter leaves them alone.
  pub fn blank_line_bounds(&self) -> (usize, usize) {
    let get = |name: &str, fallback: u64| {
      self
        .get(name)
        .and_then(Value::as_integer)
        .unwrap_or(fallback) as usize
    };
    (
      get("blank_lines_lower_bound", 0),
      get("blank_lines_upper_bound", 1),
    )
  }

  /// The edition the code is written in: `edition`, from the configuration or the crate's
  /// `Cargo.toml`; 2015 when neither gives one, as the standard formatter assumes.
  pub fn edition(&self) -> Edition {
    self
      .get("edition")
      .and_then(Value::as_text)
      .and_then(Edition::from_year)
      .unwrap_or(Edition::DEFAULT)
  }

  /// Whether layout is on (`layout = true`).
  pub fn layout_on(&self) -> bool {
    self.get("layout").and_then(Value::as_bool).unwrap_or(false)
  }

  /// Whether the files a path's modules reach are left out (`skip_children`).
  pub fn skip_children(&self) -> bool {
    self
      .get("skip_children")
      .and_then(Value::as_bool)
      .unwrap_or(false)
  }

  /// Whether `format = true` applies: not where the standard formatter's
  /// `disable_all_formatting` is set.
  pub fn format_on(&self) -> bool {
    let disabled = self
      .get("disable_all_formatting")
      .and_then(Value::as_bool)
      .unwrap_or(false);
    !disabled && self.get("format").and_then(Value::as_bool).unwrap_or(true)
  }

  /// The formatting settings.
  pub fn format(&self) -> FormatConfig {
    let mut config = FormatConfig::default();
    let number = |name: &str, fallback: usize| {
      self
        .get(name)
        .and_then(Value::as_integer)
        .map_or(fallback, |n| n as usize)
    };
    config.max_width = number("max_width", config.max_width);
    config.tab_spaces = number("tab_spaces", config.tab_spaces);
    config.blank_lines_upper_bound =
      number("blank_lines_upper_bound", config.blank_lines_upper_bound);
    config.blank_lines_lower_bound =
      number("blank_lines_lower_bound", config.blank_lines_lower_bound);
    config.hard_tabs = self
      .get("hard_tabs")
      .and_then(Value::as_bool)
      .unwrap_or(config.hard_tabs);
    let flag =
      |name: &str, fallback: bool| self.get(name).and_then(Value::as_bool).unwrap_or(fallback);
    config.spaces_around_ranges = flag("spaces_around_ranges", config.spaces_around_ranges);
    config.space_before_colon = flag("space_before_colon", config.space_before_colon);
    config.space_after_colon = flag("space_after_colon", config.space_after_colon);
    config.disable_all_formatting = flag("disable_all_formatting", false);
    // The widths: `use_small_heuristics` first (Default scales the standard formatter's
    // percentages up past a `max_width` of 100, in steps of a tenth; Max makes every
    // width the line width; Off lifts them), then any width set explicitly.
    let max_width = config.max_width;
    let scaled = |percent: usize| match self.get("use_small_heuristics").and_then(Value::as_text) {
      Some("Max") => max_width,
      Some("Off") => usize::MAX,
      _ => {
        let tenths = if max_width > 100 {
          (max_width * 10 + 50) / 100
        } else {
          10
        };
        (percent * tenths + 5) / 10
      }
    };
    let width = |name: &str, percent: usize| match self.source(name) {
      Some(source) if *source != Source::Default => number(name, scaled(percent)),
      _ => scaled(percent),
    };
    config.fn_call_width = width("fn_call_width", 60);
    config.chain_width = width("chain_width", 60);
    config.struct_lit_width = width("struct_lit_width", 18);
    config.struct_variant_width = width("struct_variant_width", 35);
    config.trailing_comma_never =
      self.get("trailing_comma").and_then(Value::as_text) == Some("Never");
    config.array_width = width("array_width", 60);
    config.attr_fn_like_width = width("attr_fn_like_width", 70);
    config.single_line_if_else_max_width = width("single_line_if_else_max_width", 50);
    config.single_line_let_else_max_width = width("single_line_let_else_max_width", 50);
    config.style_edition = self.style_edition();
    config.reorder_imports = flag("reorder_imports", true);
    config.reorder_modules = flag("reorder_modules", true);
    config.force_explicit_abi = flag("force_explicit_abi", true);
    config.merge_derives = flag("merge_derives", true);
    config.remove_nested_parens = flag("remove_nested_parens", true);
    if let Some(names) = self.get("skip_macro_invocations").and_then(Value::as_list) {
      config.skip_macro_invocations = names.to_vec();
    }
    config.brace_next_line =
      self.get("brace_style").and_then(Value::as_text) == Some("AlwaysNextLine");
    config.brace_prefer_same_line =
      self.get("brace_style").and_then(Value::as_text) == Some("PreferSameLine");
    config.short_array_element_width_threshold = number(
      "short_array_element_width_threshold",
      config.short_array_element_width_threshold,
    );
    config.match_arm_indent = flag("match_arm_indent", true);
    config.trailing_semicolon = flag("trailing_semicolon", true);
    config.combine_control_expr = flag("combine_control_expr", true);
    config.format_generated_files = flag("format_generated_files", true);
    config.format_macro_bodies = flag("format_macro_bodies", true);
    config.match_block_trailing_comma = flag("match_block_trailing_comma", false);
    config.field_init_shorthand = flag("use_field_init_shorthand", false);
    config.struct_lit_single_line = flag("struct_lit_single_line", true);
    config.try_shorthand = flag("use_try_shorthand", false);
    config.match_arm_blocks = flag("match_arm_blocks", true);
    config.force_multiline_blocks = flag("force_multiline_blocks", false);
    config.overflow_delimited_expr = flag("overflow_delimited_expr", false);
    config.empty_item_single_line = flag("empty_item_single_line", true);
    config.fn_single_line = flag("fn_single_line", false);
    config.where_single_line = flag("where_single_line", false);
    config.condense_wildcards = flag("condense_wildcard_suffixes", false);
    config.normalize_comments = flag("normalize_comments", false);
    config.wrap_comments = flag("wrap_comments", false);
    config.comment_width = number("comment_width", 80);
    config.normalize_doc_attributes = flag("normalize_doc_attributes", false);
    config.leading_pipes = match self.get("match_arm_leading_pipes").and_then(Value::as_text) {
      Some("Always") => crate::options::format::LeadingPipes::Always,
      Some("Preserve") => crate::options::format::LeadingPipes::Preserve,
      _ => crate::options::format::LeadingPipes::Never,
    };
    let choice = |name: &str| {
      self
        .get(name)
        .and_then(Value::as_text)
        .map(str::to_ascii_lowercase)
    };
    config.hex_literal_case = match choice("hex_literal_case").as_deref() {
      Some("upper") => crate::options::format::HexLiteralCase::Upper,
      Some("lower") => crate::options::format::HexLiteralCase::Lower,
      _ => crate::options::format::HexLiteralCase::Preserve,
    };
    config.binop_back = choice("binop_separator").as_deref() == Some("back");
    config.imports_granularity = match choice("imports_granularity").as_deref() {
      Some("crate") => crate::options::format::ImportsGranularity::Crate,
      Some("module") => crate::options::format::ImportsGranularity::Module,
      Some("item") => crate::options::format::ImportsGranularity::Item,
      Some("one") => crate::options::format::ImportsGranularity::One,
      _ => crate::options::format::ImportsGranularity::Preserve,
    };
    config.control_brace_style = match choice("control_brace_style").as_deref() {
      Some("closingnextline") => crate::options::format::ControlBraceStyle::ClosingNextLine,
      Some("alwaysnextline") => crate::options::format::ControlBraceStyle::AlwaysNextLine,
      _ => crate::options::format::ControlBraceStyle::AlwaysSameLine,
    };
    config.fn_params_layout = match choice("fn_params_layout").as_deref() {
      Some("compressed") => crate::options::format::FnParamsLayout::Compressed,
      Some("vertical") => crate::options::format::FnParamsLayout::Vertical,
      _ => crate::options::format::FnParamsLayout::Tall,
    };
    config.float_trailing_zero = match choice("float_literal_trailing_zero").as_deref() {
      Some("always") => crate::options::format::FloatTrailingZero::Always,
      Some("ifnopostfix") => crate::options::format::FloatTrailingZero::IfNoPostfix,
      Some("never") => crate::options::format::FloatTrailingZero::Never,
      _ => crate::options::format::FloatTrailingZero::Preserve,
    };
    if let Some(markers) = self.get("generated_markers").and_then(Value::as_list) {
      config.generated_markers = markers.to_vec();
    }
    config.inline_attribute_width = number("inline_attribute_width", 0);
    config.struct_field_align_threshold = number("struct_field_align_threshold", 0);
    config.enum_discrim_align_threshold = number("enum_discrim_align_threshold", 0);
    config.generated_marker_line_search_limit = number(
      "generated_marker_line_search_limit",
      config.generated_marker_line_search_limit,
    );
    config.compressed_type_punctuation = self
      .get("type_punctuation_density")
      .and_then(Value::as_text)
      == Some("Compressed");
    config.imports_layout = match self.get("imports_layout").and_then(Value::as_text) {
      Some("HorizontalVertical") => ImportsLayout::HorizontalVertical,
      Some("Vertical") => ImportsLayout::Vertical,
      Some("Horizontal") => ImportsLayout::Horizontal,
      _ => ImportsLayout::Mixed,
    };
    config.newline_style = match self.get("newline_style").and_then(Value::as_text) {
      Some("Native") => NewlineStyle::Native,
      Some("Unix") => NewlineStyle::Unix,
      Some("Windows") => NewlineStyle::Windows,
      _ => NewlineStyle::Auto,
    };
    config
  }

  pub fn source(&self, name: &str) -> Option<&Source> {
    self.values.get(name).map(|(_, source)| source)
  }

  pub fn style_edition(&self) -> u16 {
    self.style_edition
  }

  /// Every setting with a value, in table order, with its source.
  pub fn values(&self) -> impl Iterator<Item = (&'static Setting, &Value, &Source)> {
    options::all().filter_map(|setting| {
      self
        .values
        .get(setting.name)
        .map(|(value, source)| (setting, value, source))
    })
  }
}

/// A configuration error: a file that cannot be read or parsed, or an invalid value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
  pub message: String,
}

impl Error {
  fn new(message: impl Into<String>) -> Error {
    Error {
      message: message.into(),
    }
  }
}

impl fmt::Display for Error {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}", self.message)
  }
}

/// One configuration file, read and checked.
#[derive(Debug)]
struct Loaded {
  path: PathBuf,
  values: Vec<(&'static Setting, Value, usize)>,
  /// `root = true` in an `mto.toml`.
  root: bool,
  warnings: Vec<String>,
}

impl Loaded {
  fn get(&self, name: &str) -> Option<&Value> {
    self
      .values
      .iter()
      .find(|(setting, _, _)| setting.name == name)
      .map(|(_, value, _)| value)
  }
}

/// The settings given on the command line, checked once for the whole run.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
  /// `--config-path FILE`.
  pub config_path: Option<PathBuf>,
  /// `--config key=value[,key=value]`, in order.
  pub settings: Vec<(String, String)>,
  /// `--edition`.
  pub edition: Option<String>,
  /// `--style-edition`.
  pub style_edition: Option<String>,
}

/// Finds and merges configuration for folders, reading each file once and printing each
/// warning once.
pub struct Resolver {
  env: Environment,
  command_line: Vec<(&'static Setting, Value)>,
  explicit: Option<Rc<Loaded>>,
  user: Option<Rc<Loaded>>,
  loaded: HashMap<PathBuf, Rc<Loaded>>,
  configs: HashMap<PathBuf, Rc<Config>>,
  /// What each folder holds, and each folder's package edition, found once.
  folders: HashMap<PathBuf, files::Folder>,
  editions: HashMap<PathBuf, Result<Option<cargo::Edition>, String>>,
  manifests: cargo::Manifests,
  warnings: Vec<String>,
  warned: HashSet<String>,
}

impl Resolver {
  /// Checks the command-line settings and reads `--config-path` and the user config.
  pub fn new(overrides: &Overrides, env: Environment) -> Result<Resolver, Error> {
    let mut resolver = Resolver {
      env,
      command_line: Vec::new(),
      explicit: None,
      user: None,
      loaded: HashMap::new(),
      configs: HashMap::new(),
      folders: HashMap::new(),
      editions: HashMap::new(),
      manifests: cargo::Manifests::default(),
      warnings: Vec::new(),
      warned: HashSet::new(),
    };
    let mut settings: Vec<(&str, &str, &str)> = overrides
      .settings
      .iter()
      .map(|(key, text)| ("--config", key.as_str(), text.as_str()))
      .collect();
    if let Some(edition) = &overrides.edition {
      settings.push(("--edition", "edition", edition));
    }
    if let Some(style_edition) = &overrides.style_edition {
      settings.push(("--style-edition", "style_edition", style_edition));
    }
    for (flag, key, text) in settings {
      let setting = options::find(key).ok_or_else(|| {
        Error::new(format!(
          "{}: unknown key `{}`{}",
          flag,
          key,
          suggestion(key)
        ))
      })?;
      if setting.file == File::User {
        return Err(Error::new(format!(
          "{}: `{}` belongs in the user config, not on the command line",
          flag, key
        )));
      }
      match value::from_command_line(setting, text) {
        Ok(Some(value)) => resolver.command_line.push((setting, value)),
        Ok(None) => {}
        Err(message) => return Err(Error::new(format!("{}: {}", flag, message))),
      }
    }
    if let Some(path) = &overrides.config_path {
      if !path.is_file() {
        return Err(Error::new(format!(
          "--config-path: {} is not a file",
          path.display()
        )));
      }
      resolver.explicit = Some(resolver.load(path, FileKind::Explicit)?);
    }
    if let Some(path) = user::path(&resolver.env)?
      && path.is_file()
    {
      resolver.user = Some(resolver.load(&path, FileKind::User)?);
    }
    Ok(resolver)
  }

  /// The settings for files in `dir`.
  pub fn config_for(&mut self, dir: &Path) -> Result<Rc<Config>, Error> {
    self.config_for_absolute(absolute(dir))
  }

  /// [`Resolver::config_for`] for a folder already absolute, with symlinks resolved: what
  /// a walk from an absolute root that skips symlinks reaches.
  pub fn config_for_absolute(&mut self, dir: PathBuf) -> Result<Rc<Config>, Error> {
    if let Some(config) = self.configs.get(&dir) {
      return Ok(Rc::clone(config));
    }
    let config = Rc::new(self.resolve(&dir)?);
    self.configs.insert(dir, Rc::clone(&config));
    Ok(config)
  }

  /// The environment the resolver reads.
  pub fn env(&self) -> &Environment {
    &self.env
  }

  /// A per-user setting: the user config's value, else the default.
  pub fn user_value(&self, name: &str) -> Option<Value> {
    let setting = options::find(name).filter(|s| s.file == File::User)?;
    self
      .user
      .as_ref()
      .and_then(|user| user.values.iter().find(|(s, _, _)| s.name == name))
      .map(|(_, value, _)| value.clone())
      .or_else(|| value::default_of(setting, 2015))
  }

  /// Warnings found since the last call, each given once per run.
  pub fn take_warnings(&mut self) -> Vec<String> {
    std::mem::take(&mut self.warnings)
  }

  fn load(&mut self, path: &Path, kind: FileKind) -> Result<Rc<Loaded>, Error> {
    if let Some(loaded) = self.loaded.get(path) {
      return Ok(Rc::clone(loaded));
    }
    let text = fs::read_to_string(path)
      .map_err(|e| Error::new(format!("cannot read {}: {}", path.display(), e)))?;
    let loaded = Rc::new(read(path, &text, kind)?);
    for warning in &loaded.warnings {
      self.warn(warning.clone());
    }
    self.loaded.insert(path.to_path_buf(), Rc::clone(&loaded));
    Ok(loaded)
  }

  fn resolve(&mut self, dir: &Path) -> Result<Config, Error> {
    let mut layers: Vec<(FileKind, Rc<Loaded>)> = Vec::new();
    if let Some(explicit) = &self.explicit {
      layers.push((FileKind::Explicit, Rc::clone(explicit)));
    } else {
      if let Some(path) = self.standard_file(dir) {
        layers.push((FileKind::Standard, self.load(&path, FileKind::Standard)?));
      }
      let mut mto_files = Vec::new();
      for folder in dir.ancestors() {
        let found = self.folder(folder);
        let Some(path) = found.mto else {
          continue;
        };
        if found.both_mto {
          self.warn(format!(
            "both {} and {} exist; using {}",
            path.display(),
            folder.join("mto.toml").display(),
            path.display()
          ));
        }
        let loaded = self.load(&path, FileKind::Mto)?;
        let root = loaded.root;
        mto_files.push(loaded);
        if root {
          break;
        }
      }
      for loaded in mto_files.into_iter().rev() {
        layers.push((FileKind::Mto, loaded));
      }
    }
    let cargo = match self.cargo_edition(dir) {
      Ok(Some(found)) => match options::find("edition")
        .map(|setting| value::from_command_line(setting, &found.edition))
      {
        Some(Ok(_)) => Some(found),
        _ => {
          self.warn(format!(
            "{}:{}: unknown edition `{}`; ignored",
            found.path.display(),
            found.line,
            found.edition
          ));
          None
        }
      },
      Ok(None) => None,
      Err(warning) => {
        self.warn(format!("{}; its edition is ignored", warning));
        None
      }
    };

    // The style edition decides the defaults, so it is resolved first.
    let latest = |name: &str| -> Option<Value> {
      let from_command_line = self
        .command_line
        .iter()
        .rev()
        .find(|(s, _)| s.name == name)
        .map(|(_, v)| v.clone());
      let from_cargo = match name {
        "edition" => cargo
          .as_ref()
          .map(|found| Value::Text(found.edition.clone())),
        _ => None,
      };
      let from_files = layers.iter().rev().find_map(|(_, l)| l.get(name).cloned());
      from_command_line.or(from_cargo).or(from_files)
    };
    let year = |value: Option<Value>| -> Option<u16> { value?.as_text()?.parse().ok() };
    let (style_edition, derived) = match (
      year(latest("style_edition")),
      latest("version"),
      year(latest("edition")),
    ) {
      (Some(se), _, _) => (se, None),
      (None, Some(Value::Text(v)), _) if v == "Two" => (2024, Some("version")),
      (None, Some(Value::Text(v)), _) if v == "One" => (2015, Some("version")),
      (None, _, Some(edition)) => (edition, Some("edition")),
      _ => (2015, None),
    };

    let mut values: BTreeMap<&'static str, (Value, Source)> = BTreeMap::new();
    for setting in options::all() {
      if let Some(value) = value::default_of(setting, style_edition) {
        values.insert(setting.name, (value, Source::Default));
      }
    }
    if let Some(user) = &self.user {
      for (setting, value, line) in &user.values {
        let source = Source::File {
          path: user.path.clone(),
          line: *line,
        };
        values.insert(setting.name, (value.clone(), source));
      }
    }
    let mut standard_values: HashMap<&str, (Value, PathBuf)> = HashMap::new();
    let mut warnings = Vec::new();
    for (kind, loaded) in &layers {
      for (setting, value, line) in &loaded.values {
        if *kind == FileKind::Standard {
          standard_values.insert(setting.name, (value.clone(), loaded.path.clone()));
        } else if let Some((theirs, path)) = standard_values.get(setting.name)
          && theirs != value
        {
          warnings.push(format!(
            "`{}` is {} in {} but {} in {}; mto uses {}",
            setting.name,
            value.to_toml(),
            loaded.path.display(),
            theirs.to_toml(),
            path.display(),
            value.to_toml()
          ));
        }
        let source = Source::File {
          path: loaded.path.clone(),
          line: *line,
        };
        values.insert(setting.name, (value.clone(), source));
      }
    }
    for warning in warnings {
      self.warn(warning);
    }
    if let Some(found) = &cargo {
      let source = Source::File {
        path: found.path.clone(),
        line: found.line,
      };
      values.insert("edition", (Value::Text(found.edition.clone()), source));
    }
    for (setting, value) in &self.command_line {
      values.insert(setting.name, (value.clone(), Source::CommandLine));
    }
    if let Some(from) = derived {
      values.insert(
        "style_edition",
        (
          Value::Text(style_edition.to_string()),
          Source::Derived(from),
        ),
      );
    }

    let layout_on = values
      .get("layout")
      .and_then(|(value, _)| value.as_bool())
      .unwrap_or(false);
    if !layout_on {
      for (name, (_, source)) in &values {
        let setting = options::find(name);
        // `format`, `ignore_dirs` and `generated_markers` matter without layout too.
        let layout_key = setting.is_some_and(|s| s.file == File::Mto)
          && !matches!(
            *name,
            "layout" | "format" | "ignore_dirs" | "generated_markers"
          );
        if layout_key && !matches!(source, Source::Default) {
          self.warn(format!(
            "{}: `{}` has no effect while layout is off; add `layout = true`",
            source, name
          ));
        }
      }
    }

    // A setting of the standard formatter's that mto does not honor yet is ignored: said
    // once, when it is set to anything but its default and formatting is on.
    let format_on = values
      .get("format")
      .and_then(|(value, _)| value.as_bool())
      .unwrap_or(true);
    if format_on {
      for (name, (value, source)) in &values {
        let Some(setting) = options::find(name) else {
          continue;
        };
        let honored = !matches!(
          options::support::OPTIONS
            .binary_search_by_key(name, |&(n, _)| n)
            .map(|i| options::support::OPTIONS[i].1),
          Ok("missing" | "not-planned")
        );
        let default = value.to_toml() == setting.default
          || setting.default_2024 == Some(value.to_toml().as_str());
        // A deprecated name has its own warning, naming the one to use.
        if setting.file == File::Format
          && setting.replaced_by.is_none()
          && !honored
          && !default
          && !matches!(source, Source::Default | Source::Derived(_))
        {
          self.warn(format!(
            "{}: `{}` is not honored by mto yet, so it is ignored (`mto config options` \
             lists what is)",
            source, name
          ));
        }
      }
    }

    if layout_on
      && let Some((Value::Integer(lines), source)) = values.get("blank_lines_between_blocks")
      && *source != Source::Default
    {
      let bound = |name: &str, fallback: u64| {
        values
          .get(name)
          .and_then(|(v, _)| v.as_integer())
          .unwrap_or(fallback)
      };
      let (lower, upper) = (
        bound("blank_lines_lower_bound", 0),
        bound("blank_lines_upper_bound", 1),
      );
      let used = (*lines).clamp(lower, upper.max(lower));
      if used != *lines {
        self.warn(format!(
          "{}: `blank_lines_between_blocks = {}` is outside `blank_lines_lower_bound` \
                     and `blank_lines_upper_bound` ({} to {}), which formatting keeps to; mto \
                     uses {}",
          source, lines, lower, upper, used
        ));
      }
    }

    let mut files: Vec<(FileKind, PathBuf)> = Vec::new();
    if let Some(user) = &self.user {
      files.push((FileKind::User, user.path.clone()));
    }
    files.extend(layers.iter().map(|(kind, l)| (*kind, l.path.clone())));
    if let Some(found) = cargo {
      files.push((FileKind::Cargo, found.path));
    }
    Ok(Config {
      values,
      style_edition,
      files,
    })
  }

  /// What `folder` holds, looked up once.
  /// What `folder` holds, from its listing (`is_file` says whether it holds a file of that
  /// name), so that resolving it asks the file system nothing more. A walk reads every
  /// folder's listing anyway.
  pub fn note_folder(&mut self, folder: &Path, is_file: impl Fn(&str) -> bool) {
    if !self.folders.contains_key(folder) {
      let found = files::Folder::with(folder, is_file);
      self.folders.insert(folder.to_path_buf(), found);
    }
  }

  fn folder(&mut self, folder: &Path) -> files::Folder {
    if let Some(found) = self.folders.get(folder) {
      return found.clone();
    }
    let found = files::Folder::scan(folder);
    self.folders.insert(folder.to_path_buf(), found.clone());
    found
  }

  /// The package edition for `dir`, each manifest read once.
  fn cargo_edition(&mut self, dir: &Path) -> Result<Option<cargo::Edition>, String> {
    if let Some(found) = self.editions.get(dir) {
      return found.clone();
    }
    let here = match self.folder(dir).cargo {
      Some(manifest) => cargo::manifest_edition(&mut self.manifests, &manifest, dir),
      None => Ok(None),
    };
    let found = match here {
      Ok(None) => match dir.parent() {
        Some(parent) => self.cargo_edition(parent),
        None => Ok(None),
      },
      other => other,
    };
    self.editions.insert(dir.to_path_buf(), found.clone());
    found
  }

  /// The `rustfmt.toml` the standard formatter would use for `dir`: the nearest in `dir`
  /// or a parent, else in the home folder, else in its folder in the user's configuration
  /// folder.
  fn standard_file(&mut self, dir: &Path) -> Option<PathBuf> {
    for folder in dir.ancestors() {
      if let Some(found) = self.folder(folder).standard {
        return Some(found);
      }
    }
    let fallbacks = [
      self.env.home.clone(),
      self.env.os_config_dir.as_ref().map(|d| d.join("rustfmt")),
    ];
    fallbacks
      .into_iter()
      .flatten()
      .find_map(|folder| self.folder(&folder).standard)
  }

  fn warn(&mut self, warning: String) {
    if self.warned.insert(warning.clone()) {
      self.warnings.push(warning);
    }
  }
}

/// Layout settings from `key = value` pairs over the defaults, with no files read: for
/// tests and embedding. Values are read as `--config` reads them.
pub fn layout_from_settings(pairs: &[(&str, &str)]) -> Result<LayoutConfig, Error> {
  let mut values = BTreeMap::new();
  for setting in options::all() {
    if let Some(value) = value::default_of(setting, 2015) {
      values.insert(setting.name, (value, Source::Default));
    }
  }
  for (key, text) in pairs {
    let setting = options::find(key)
      .ok_or_else(|| Error::new(format!("unknown key `{}`{}", key, suggestion(key))))?;
    if let Some(value) = value::from_command_line(setting, text).map_err(Error::new)? {
      values.insert(setting.name, (value, Source::CommandLine));
    }
  }
  let config = Config {
    values,
    style_edition: 2015,
    files: Vec::new(),
  };
  Ok(config.layout())
}

/// `dir` made absolute with symlinks resolved, so every spelling of a folder shares one
/// cache entry; a part that does not exist yet is kept as written.
fn absolute(dir: &Path) -> PathBuf {
  let dir = if dir.as_os_str().is_empty() {
    Path::new(".")
  } else {
    dir
  };
  for base in dir.ancestors() {
    if let Ok(real) = crate::real_path(base) {
      let rest = dir.strip_prefix(base).unwrap_or(Path::new(""));
      return real.join(rest);
    }
  }
  dir.to_path_buf()
}

/// Levenshtein distance.
fn distance(a: &str, b: &str) -> usize {
  let b: Vec<char> = b.chars().collect();
  let mut row: Vec<usize> = (0..=b.len()).collect();
  for (i, ca) in a.chars().enumerate() {
    let mut previous = row[0];
    row[0] = i + 1;
    for (j, cb) in b.iter().enumerate() {
      let substitution = previous + usize::from(ca != *cb);
      previous = row[j + 1];
      row[j + 1] = substitution.min(row[j] + 1).min(previous + 1);
    }
  }
  row[b.len()]
}

fn file_label(kind: FileKind) -> &'static str {
  match kind {
    FileKind::Standard => "rustfmt.toml",
    FileKind::Mto | FileKind::Explicit => "mto.toml",
    FileKind::User => "the user config",
    FileKind::Cargo => "Cargo.toml",
  }
}

/// The keys of a table, with sub-tables of the user config flattened to `section.key`.
fn flatten<'a>(
  prefix: &str,
  table: &'a toml::Table,
  kind: FileKind,
  out: &mut Vec<(String, &'a toml::Entry)>,
) {
  for entry in table.entries() {
    let key = format!("{}{}", prefix, entry.key);
    match &entry.value {
      toml::Value::Table(inner) if kind == FileKind::User => {
        flatten(&format!("{}.", key), inner, kind, out)
      }
      _ => out.push((key, entry)),
    }
  }
}

/// `path` with its `.` and `..` parts worked out by name, for a folder that does not exist.
fn lexical(path: &Path) -> PathBuf {
  let mut out = PathBuf::new();
  for part in path.components() {
    match part {
      std::path::Component::CurDir => {}
      std::path::Component::ParentDir => {
        out.pop();
      }
      other => out.push(other),
    }
  }
  out
}

/// Reads and checks one configuration file.
fn read(path: &Path, text: &str, kind: FileKind) -> Result<Loaded, Error> {
  let at = |line: usize, column: usize| format!("{}:{}:{}", path.display(), line, column);
  let table = toml::parse(text)
    .map_err(|e| Error::new(format!("{}: {}", at(e.line, e.column), e.message)))?;
  let mut loaded = Loaded {
    path: path.to_path_buf(),
    values: Vec::new(),
    root: false,
    warnings: Vec::new(),
  };
  let mut entries: Vec<(String, &toml::Entry)> = Vec::new();
  flatten("", &table, kind, &mut entries);
  for (key, entry) in entries {
    let place = at(entry.line, entry.column);
    if key == ROOT_KEY && matches!(kind, FileKind::Mto | FileKind::Explicit) {
      loaded.root = entry
        .value
        .as_bool()
        .ok_or_else(|| Error::new(format!("{}: `root` must be true or false", place)))?;
      continue;
    }
    let Some(setting) = options::find(&key) else {
      loaded.warnings.push(format!(
        "{}: unknown key `{}`{}; ignored",
        place,
        key,
        suggestion(&key)
      ));
      continue;
    };
    if !kind.may_hold(setting.file) {
      loaded.warnings.push(format!(
        "{}: `{}` cannot be set in {}; put it in {}; ignored",
        place,
        key,
        file_label(kind),
        file_label(match setting.file {
          File::Format => FileKind::Standard,
          File::Mto => FileKind::Mto,
          File::User => FileKind::User,
        })
      ));
      continue;
    }
    if let Some(replacement) = setting.replaced_by {
      loaded.warnings.push(format!(
        "{}: `{}` is deprecated; use `{}`",
        place, key, replacement
      ));
    }
    match value::check(setting, &entry.value) {
      Ok(Some(value)) => loaded.values.push((setting, value, entry.line)),
      Ok(None) => {}
      Err(message) => return Err(Error::new(format!("{}: {}", place, message))),
    }
  }
  Ok(loaded)
}

/// The closest key to `key`, as ` (did you mean `...`?)`, or nothing.
fn suggestion(key: &str) -> String {
  if let Some((_, new)) = RENAMED.iter().find(|(old, _)| *old == key) {
    return format!(" (renamed `{}`)", new);
  }
  let normalized = key.replace('-', "_").to_ascii_lowercase();
  let best = options::all()
    .map(|s| s.name)
    .chain([ROOT_KEY])
    .map(|name| (distance(&normalized, name), name))
    .min();
  match best {
    Some((d, name)) if d <= 2 || (d <= 3 && key.len() > 8) => {
      format!(" (did you mean `{}`?)", name)
    }
    _ => String::new(),
  }
}

#[cfg(test)]
mod tests;
