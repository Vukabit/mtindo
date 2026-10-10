//! Layer 4: configuration files, read into settings.
//!
//! The settings for a folder come from, lowest precedence first:
//!
//! 1. the defaults in the settings table, for the style edition in effect;
//! 2. the nearest `.rustfmt.toml` or `rustfmt.toml`, found as rustfmt finds it: in the
//!    folder, then each parent, then the home folder, then the user's `rustfmt` config folder;
//! 3. every `.mto.toml` or `mto.toml` from the outermost folder down to the folder itself, a
//!    nearer file winning key by key; the search upwards stops at a file with `root = true`.
//!    An `mto.toml` may set rustfmt's keys too, and wins over `rustfmt.toml` with a warning
//!    when they disagree;
//! 4. the edition of the nearest `Cargo.toml`'s package, as `cargo fmt` passes it;
//! 5. the command line: `--config key=value`, `--edition`, `--style-edition`.
//!
//! `--config-path FILE` replaces steps 2 and 3 with that one file. The style edition is
//! resolved as rustfmt resolves it: `style_edition`, else `version` (`One` is 2015, `Two` is
//! 2024), else `edition`, else 2015.
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

use crate::options::layout::{ItemKind, LayoutConfig, NameSort, TraitImplSort, Visibility};
use crate::options::{self, File, Setting};

/// Keys mto used to accept, and the key that replaced each.
const RENAMED: &[(&str, &str)] = &[("tab_width", "tab_spaces")];

/// The key that stops the search for `mto.toml` files upwards.
const ROOT_KEY: &str = "root";

/// The kinds of configuration file, which decide what they may hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// `rustfmt.toml` or `.rustfmt.toml`: rustfmt's settings.
    Rustfmt,
    /// `mto.toml` or `.mto.toml`: mto's settings and rustfmt's.
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
            FileKind::Rustfmt => file == File::Format,
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
#[derive(Debug)]
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

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.get(name).map(|(value, _)| value)
    }

    /// The layout settings.
    pub fn layout(&self) -> LayoutConfig {
        let mut config = LayoutConfig::default();
        let flag = |name: &str, fallback: bool| {
            self.get(name).and_then(Value::as_bool).unwrap_or(fallback)
        };
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
            config.ignore_dirs = dirs.to_vec();
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

    /// rustfmt's `blank_lines_lower_bound` and `blank_lines_upper_bound`: the blank lines
    /// between items stay within them, so rustfmt leaves them alone.
    pub fn blank_line_bounds(&self) -> (usize, usize) {
        let get = |name: &str, fallback: u64| {
            self.get(name)
                .and_then(Value::as_integer)
                .unwrap_or(fallback) as usize
        };
        (
            get("blank_lines_lower_bound", 0),
            get("blank_lines_upper_bound", 1),
        )
    }

    /// Whether layout is on (`layout = true`).
    pub fn layout_on(&self) -> bool {
        self.get("layout").and_then(Value::as_bool).unwrap_or(false)
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
            self.values
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
        self.values
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
        self.user
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
            if let Some(path) = self.rustfmt_file(dir) {
                layers.push((FileKind::Rustfmt, self.load(&path, FileKind::Rustfmt)?));
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
        let mut rustfmt_values: HashMap<&str, (Value, PathBuf)> = HashMap::new();
        let mut warnings = Vec::new();
        for (kind, loaded) in &layers {
            for (setting, value, line) in &loaded.values {
                if *kind == FileKind::Rustfmt {
                    rustfmt_values.insert(setting.name, (value.clone(), loaded.path.clone()));
                } else if let Some((theirs, path)) = rustfmt_values.get(setting.name)
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
                let layout_key = setting.is_some_and(|s| s.file == File::Mto) && *name != "layout";
                if layout_key && !matches!(source, Source::Default) {
                    self.warn(format!(
                        "{}: `{}` has no effect while layout is off; add `layout = true`",
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
                    "{}: `blank_lines_between_blocks = {}` is outside rustfmt's blank-line \
                     bounds ({} to {}), which rustfmt would undo; mto uses {}",
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
            Some(manifest) => cargo::manifest_edition(&manifest, dir),
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

    /// The `rustfmt.toml` rustfmt would use for `dir`: the nearest in `dir` or a parent,
    /// else in the home folder, else in the `rustfmt` configuration folder.
    fn rustfmt_file(&mut self, dir: &Path) -> Option<PathBuf> {
        for folder in dir.ancestors() {
            if let Some(found) = self.folder(folder).rustfmt {
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
            .find_map(|folder| self.folder(&folder).rustfmt)
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
        if let Ok(real) = fs::canonicalize(base) {
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
        FileKind::Rustfmt => "rustfmt.toml",
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
                    File::Format => FileKind::Rustfmt,
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
