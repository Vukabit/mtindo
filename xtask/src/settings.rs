//! Generates `src/options/rustfmt.rs`, rustfmt's settings as mto's settings table, from
//! rustfmt's own source at the stable and nightly releases pinned in
//! `compat/rustfmt-surface.txt` (`cargo xtask capabilities refresh` moves the pins).
//!
//! - Options, their type and stability: the top-level `create_config!` in
//!   `src/config/mod.rs` (`name: ConfigType, stable, "summary";`).
//! - Defaults: `config_option_with_style_edition_default!` in `src/config/options.rs`
//!   (`ConfigType, value type, [Edition2024 => default,] _ => default;`).
//! - Legal values: the `#[config_type]` enums in `options.rs` and `lists.rs`, with their doc
//!   comments, `#[value = "..."]` spellings and `#[unstable_variant]` markers.
//! - Hidden options: `HIDE_OPTIONS` and the options `to_toml` strips, as the capabilities
//!   reader finds them.
//! - Descriptions: the paragraphs under each option's heading in `Configurations.md`, or the
//!   `create_config!` summary when there is no heading.
//!
//! Data comes from the stable release; options only nightly has come from nightly. If the
//! two disagree on an option's type or defaults, generation fails: the table would need to
//! learn about channel-specific defaults first.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use crate::capabilities::{fetch, load_surface, source};
use crate::common::log::Logger;
use crate::unicode::rustfmt;

/// Value types rustfmt sets itself, with no file representation of their own.
const INTERNAL_TYPES: &[&str] = &["FileLines", "WidthHeuristics"];

/// Value types that are arrays of strings in a configuration file.
const LIST_TYPES: &[&str] = &["IgnoreList", "MacroSelectors"];

/// Default expressions that are not literals or enum variants, as TOML.
const SPECIAL_DEFAULTS: &[(&str, &str)] = &[
  ("IgnoreList::default()", "[]"),
  ("MacroSelectors::default()", "[]"),
  ("FileLines::all()", ""),
  ("WidthHeuristics::scaled(100)", ""),
];

const TABLE_PATH: &str = "src/options/rustfmt.rs";

/// The default of `required_version`; replaced by the release's version.
const VERSION_DEFAULT: &str = "env!(\"CARGO_PKG_VERSION\").to_owned()";

/// ConfigType -> (value type, default, 2024 default) from
/// `config_option_with_style_edition_default!`.
type Defaults = BTreeMap<String, (String, String, Option<String>)>;

#[derive(Clone, Debug, PartialEq)]
enum Kind {
  Bool,
  Integer,
  Text,
  Choice(String),
  List,
  Internal,
}

struct ConfigEntry {
  name: String,
  config_type: String,
  stable: bool,
  summary: String,
}

#[derive(Clone, Debug)]
struct Opt {
  name: String,
  kind: Kind,
  default: String,
  default_2024: Option<String>,
  /// The default expressions as written, to compare releases by.
  source_defaults: (String, Option<String>),
  summary: String,
  doc: String,
  stable: bool,
  hidden: bool,
  replaced_by: Option<String>,
}

/// One release's reading.
struct Release {
  version: String,
  commit: String,
  options: Vec<Opt>,
  enums: BTreeMap<String, Vec<Variant>>,
}

#[derive(Clone, Debug, PartialEq)]
struct Variant {
  /// The Rust identifier.
  ident: String,
  /// The value as written in a file.
  name: String,
  doc: String,
  stable: bool,
}

pub fn run(root: &Path, m: &clap::ArgMatches) -> ExitCode {
  let log = Logger::new(root, "settings");
  let check = crate::cli::on(m, "check");
  let generated = match generate(root, &log) {
    Ok(text) => text,
    Err(e) => {
      log.error(&e);
      eprintln!("{}", e);
      return ExitCode::from(1);
    }
  };
  let path = root.join(TABLE_PATH);
  if check {
    if fs::read_to_string(&path).unwrap_or_default() == generated {
      println!("{} matches the pinned rustfmt releases", TABLE_PATH);
      return ExitCode::SUCCESS;
    }
    println!("{} is stale; run `cargo xtask settings`", TABLE_PATH);
    return ExitCode::from(1);
  }
  if let Err(e) = fs::write(&path, generated) {
    eprintln!("cannot write {}: {}", path.display(), e);
    return ExitCode::from(1);
  }
  log.info(&format!("wrote {}", TABLE_PATH));
  println!("wrote {}", TABLE_PATH);
  ExitCode::SUCCESS
}

/// The text between the delimiter at `open` and its match, skipping strings and comments.
fn delimited(text: &str, open: usize) -> Option<&str> {
  let bytes = text.as_bytes();
  let (left, right) = match bytes[open] {
    b'{' => (b'{', b'}'),
    b'(' => (b'(', b')'),
    _ => return None,
  };
  let mut depth = 0;
  let mut idx = open;
  while idx < bytes.len() {
    match bytes[idx] {
      b'"' => idx = skip_string(bytes, idx),
      b'/' if bytes.get(idx + 1) == Some(&b'/') => {
        idx += bytes[idx..].iter().position(|b| *b == b'\n')?;
      }
      b if b == left => depth += 1,
      b if b == right => {
        depth -= 1;
        if depth == 0 {
          return Some(&text[open + 1..idx]);
        }
      }
      _ => {}
    }
    idx += 1;
  }
  None
}

fn generate(root: &Path, log: &Logger) -> Result<String, String> {
  let surface = load_surface(root)?;
  let mut releases = Vec::new();
  for label in ["stable", "nightly"] {
    let (_, description) = surface
            .sources
            .iter()
            .find(|(l, _)| l == label)
            .ok_or_else(|| {
                format!(
                    "compat/rustfmt-surface.txt has no {} release; run `cargo xtask capabilities refresh` online",
                    label
                )
            })?;
    let (version, commit) = release_of(description)
      .ok_or_else(|| format!("cannot read the {} release from `{}`", label, description))?;
    let dir = fetch::checkout(root, &commit, log)?;
    let release = read_release(&dir, version, commit)
      .map_err(|e| format!("rustfmt {} ({}): {}", label, dir.display(), e))?;
    releases.push(release);
  }
  let nightly = releases.pop().unwrap();
  let stable = releases.pop().unwrap();
  render(&stable, &nightly)
}

fn kind_of(value_type: &str, enums: &BTreeMap<String, Vec<Variant>>) -> Option<Kind> {
  Some(match value_type {
    "bool" => Kind::Bool,
    "usize" => Kind::Integer,
    "String" => Kind::Text,
    t if enums.contains_key(t) => Kind::Choice(t.to_string()),
    t if LIST_TYPES.contains(&t) => Kind::List,
    t if INTERNAL_TYPES.contains(&t) => Kind::Internal,
    _ => return None,
  })
}

/// The formatter's own name, outside code spans and where it is not a file or path
/// (`rustfmt.toml`, `rustfmt::skip`), said as "the formatter": the manual names the Rust
/// style, not the tool (S3-9).
fn neutral_name(text: &str) -> String {
  let mut out = String::new();
  let mut in_code = false;
  let mut rest = text;
  while let Some(ch) = rest.chars().next() {
    if ch == '`' {
      in_code = !in_code;
    }
    let word = rest.get(..7).filter(|w| w.eq_ignore_ascii_case("rustfmt"));
    let before_ok = !out.ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '-');
    let after = rest.get(7..).unwrap_or("");
    let after_ok = !after.starts_with(|c: char| c.is_alphanumeric() || c == '_')
      && !after.starts_with(".toml")
      && !after.starts_with("::")
      && !after.starts_with('-');
    if word.is_some() && !in_code && before_ok && after_ok {
      let starts_sentence =
        out.trim_end().is_empty() || out.trim_end().ends_with(['.', ':', '!', '?']);
      out.push_str(if starts_sentence {
        "The formatter"
      } else {
        "the formatter"
      });
      rest = after;
      continue;
    }
    out.push(ch);
    rest = &rest[ch.len_utf8()..];
  }
  out
}

/// The entries of the top-level `create_config!`.
fn parse_create_config(text: &str) -> Result<Vec<ConfigEntry>, String> {
  let start = text
    .find("\ncreate_config! {")
    .ok_or("no top-level `create_config! {`")?;
  let body = delimited(text, start + text[start..].find('{').unwrap_or(0))
    .ok_or("unbalanced `create_config!`")?;
  let mut entries = Vec::new();
  for statement in split_top_level(body, ';') {
    let Some((name, rest)) = statement.split_once(':') else {
      continue;
    };
    let parts = split_top_level(rest, ',');
    let [config_type, stable, summary] = parts[..] else {
      return Err(format!("unreadable `create_config!` entry `{}`", statement));
    };
    entries.push(ConfigEntry {
      name: name.trim().to_string(),
      config_type: config_type.trim().to_string(),
      stable: match stable.trim() {
        "true" => true,
        "false" => false,
        other => return Err(format!("`{}` has stability `{}`", name.trim(), other)),
      },
      summary: string_literal(summary.trim())
        .ok_or_else(|| format!("`{}` has no summary string", name.trim()))?,
    });
  }
  if entries.is_empty() {
    return Err("`create_config!` lists no options".to_string());
  }
  Ok(entries)
}

fn parse_defaults(text: &str) -> Result<Defaults, String> {
  let start = text
    .find("\nconfig_option_with_style_edition_default!(")
    .ok_or("no `config_option_with_style_edition_default!(` invocation")?;
  let body = delimited(text, start + text[start..].find('(').unwrap_or(0))
    .ok_or("unbalanced `config_option_with_style_edition_default!`")?;
  let mut defaults = BTreeMap::new();
  for statement in split_top_level(body, ';') {
    let parts: Vec<&str> = split_top_level(statement, ',')
      .into_iter()
      .map(str::trim)
      .collect();
    let unreadable = || format!("unreadable default entry `{}`", statement);
    let (name, value_type, rest) = match parts[..] {
      [name, value_type, ref rest @ ..] if !rest.is_empty() => (name, value_type, rest),
      _ => return Err(unreadable()),
    };
    let mut default = None;
    let mut default_2024 = None;
    for arm in rest {
      match arm.split_once("=>").map(|(k, v)| (k.trim(), v.trim())) {
        Some(("_", expr)) => default = Some(expr.to_string()),
        Some(("Edition2024", expr)) => default_2024 = Some(expr.to_string()),
        _ => return Err(unreadable()),
      }
    }
    let default = default.ok_or_else(unreadable)?;
    defaults.insert(
      name.to_string(),
      (value_type.to_string(), default, default_2024),
    );
  }
  Ok(defaults)
}

/// Option name -> its description paragraphs from Configurations.md.
fn parse_docs(text: &str) -> BTreeMap<String, String> {
  let mut docs = BTreeMap::new();
  let mut lines = text.lines().peekable();
  while let Some(line) = lines.next() {
    let Some(name) = line
      .strip_prefix("## `")
      .and_then(|rest| rest.strip_suffix('`'))
    else {
      continue;
    };
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    while let Some(next) = lines.peek() {
      if next.starts_with("- **") || next.starts_with('#') || next.starts_with("```") {
        break;
      }
      let next = lines.next().unwrap().trim();
      if next.is_empty() {
        if !current.is_empty() {
          paragraphs.push(std::mem::take(&mut current));
        }
      } else {
        // A list item starts its own line, so the manual can show it as one.
        let item = next.starts_with("- ")
          || next
            .split_once(". ")
            .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        if !current.is_empty() {
          current.push(if item { '\n' } else { ' ' });
        }
        current.push_str(next);
      }
    }
    if !current.is_empty() {
      paragraphs.push(current);
    }
    let doc = paragraphs
      .iter()
      .map(|p| neutral_name(&strip_links(p)))
      .collect::<Vec<_>>()
      .join("\n\n");
    if !doc.is_empty() {
      docs.insert(name.to_string(), doc);
    }
  }
  docs
}

/// Every `#[config_type]` enum: name -> variants.
fn parse_enums(text: &str) -> Result<BTreeMap<String, Vec<Variant>>, String> {
  let mut enums = BTreeMap::new();
  let mut search = 0;
  while let Some(idx) = text[search..].find("#[config_type]") {
    let at = search + idx;
    search = at + 1;
    let Some(enum_at) = text[at..].find("pub enum ").map(|i| at + i) else {
      break;
    };
    let name: String = text[enum_at + 9..]
      .chars()
      .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
      .collect();
    let open = enum_at + text[enum_at..].find('{').ok_or("enum without a body")?;
    let body = delimited(text, open).ok_or_else(|| format!("unbalanced enum `{}`", name))?;
    let mut variants = Vec::new();
    let mut doc: Vec<String> = Vec::new();
    let mut value = None;
    let mut stable = true;
    for line in body.lines().map(str::trim) {
      if let Some(comment) = line.strip_prefix("///") {
        doc.push(comment.trim().to_string());
      } else if let Some(attr) = line.strip_prefix("#[value = ") {
        value = string_literal(attr.trim_end_matches(']'));
      } else if line == "#[unstable_variant]" {
        stable = false;
      } else if !(line.is_empty() || line.starts_with("#[") || line.starts_with("//")) {
        let ident = line.trim_end_matches(',');
        if ident.ends_with(')') {
          // A variant with data (`LimitedHorizontalVertical(usize)`) cannot be
          // written in a file; rustfmt's documentation leaves it out too.
          doc.clear();
          stable = true;
          continue;
        }
        if !ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
          return Err(format!(
            "enum `{}` has an unreadable variant `{}`",
            name, line
          ));
        }
        variants.push(Variant {
          ident: ident.to_string(),
          name: value.take().unwrap_or_else(|| ident.to_string()),
          doc: sentence(&neutral_name(&strip_links(&doc.join(" ")))),
          stable,
        });
        doc.clear();
        stable = true;
      }
    }
    enums.insert(name, variants);
  }
  Ok(enums)
}

fn read_release(dir: &Path, version: String, commit: String) -> Result<Release, String> {
  let load = |path: &str| {
    fs::read_to_string(dir.join(path)).map_err(|e| format!("cannot read {}: {}", path, e))
  };
  let config = load("src/config/mod.rs")?;
  let options_rs = load("src/config/options.rs")?;
  let mut enums = parse_enums(&options_rs)?;
  enums.extend(parse_enums(&load("src/config/lists.rs")?)?);
  let defaults = parse_defaults(&options_rs)?;
  let docs = parse_docs(&load("Configurations.md")?);
  let mut hidden = source::parse_hidden_options(&load("src/config/config_type.rs")?);
  hidden.extend(source::parse_toml_stripped(&config));
  let mut options = Vec::new();
  for entry in parse_create_config(&config)? {
    let (value_type, default_expr, default_2024_expr) = defaults
      .get(&entry.config_type)
      .ok_or_else(|| format!("no default for `{}`", entry.config_type))?;
    let kind = kind_of(value_type, &enums)
      .ok_or_else(|| format!("`{}` has unknown type `{}`", entry.name, value_type))?;
    let render = |expr: &str| render_default(expr, &kind, &enums, &version);
    let default = render(default_expr).ok_or_else(|| {
      format!(
        "`{}` has an unreadable default `{}`",
        entry.name, default_expr
      )
    })?;
    let default_2024 = match default_2024_expr {
      Some(expr) => Some(
        render(expr)
          .ok_or_else(|| format!("`{}` has an unreadable 2024 default `{}`", entry.name, expr))?,
      ),
      None => None,
    };
    let replaced_by = entry
      .summary
      .strip_prefix("(deprecated: use ")
      .and_then(|rest| rest.split_whitespace().next())
      .map(str::to_string);
    options.push(Opt {
      doc: docs
        .get(&entry.name)
        .cloned()
        .unwrap_or_else(|| entry.summary.clone()),
      hidden: hidden.contains(&entry.name),
      name: entry.name,
      kind,
      source_defaults: (default_expr.clone(), default_2024_expr.clone()),
      default,
      default_2024,
      summary: entry.summary,
      stable: entry.stable,
      replaced_by,
    });
  }
  Ok(Release {
    version,
    commit,
    options,
    enums,
  })
}

/// (version, commit) from `rustfmt 1.10.0, rust-lang/rust COMMIT, released DATE`.
fn release_of(description: &str) -> Option<(String, String)> {
  let version = description
    .strip_prefix("rustfmt ")?
    .split(',')
    .next()?
    .to_string();
  let commit = description
    .split([' ', ','])
    .find(|w| w.len() == 40 && w.chars().all(|c| c.is_ascii_hexdigit()))?
    .to_string();
  Some((version, commit))
}

fn render(stable: &Release, nightly: &Release) -> Result<String, String> {
  let mut names: Vec<&str> = stable
    .options
    .iter()
    .chain(nightly.options.iter())
    .map(|o| o.name.as_str())
    .collect();
  names.sort_unstable();
  names.dedup();
  let mut out = format!(
    "//! rustfmt's settings, generated by `cargo xtask settings` from rustfmt's source at the\n\
         //! stable and nightly releases pinned in `compat/rustfmt-surface.txt`; do not edit.\n\
         //!\n\
         //! - stable: rustfmt {}, rust-lang/rust {}\n\
         //! - nightly: rustfmt {}, rust-lang/rust {}\n\n\
         use super::{{Channels, Choice, File, Kind, Setting, Stability}};\n\n\
         /// The rustfmt version mto's defaults and documentation follow.\n\
         pub const RUSTFMT_VERSION: &str = {:?};\n\n\
         pub(super) static SETTINGS: &[Setting] = &[\n",
    stable.version, stable.commit, nightly.version, nightly.commit, stable.version
  );
  for name in names {
    let in_stable = stable.options.iter().find(|o| o.name == name);
    let in_nightly = nightly.options.iter().find(|o| o.name == name);
    let option = in_stable.or(in_nightly).unwrap();
    if let (Some(s), Some(n)) = (in_stable, in_nightly) {
      let enum_of = |kind: &Kind, release: &Release| match kind {
        Kind::Choice(e) => release.enums.get(e).map(|v| v.len()),
        _ => None,
      };
      if s.kind != n.kind
        || s.source_defaults != n.source_defaults
        || enum_of(&s.kind, stable) != enum_of(&n.kind, nightly)
      {
        return Err(format!(
          "`{}` differs between stable and nightly (type or default); mto's settings \
                     table cannot express that yet",
          name
        ));
      }
    }
    let stability = |found: Option<&Opt>| match found {
      None => "Absent",
      Some(o) if o.stable => "Stable",
      Some(_) => "Unstable",
    };
    let kind = match &option.kind {
      Kind::Bool => "Kind::Bool".to_string(),
      Kind::Integer => "Kind::Integer".to_string(),
      Kind::Text => "Kind::Text".to_string(),
      Kind::List => "Kind::List".to_string(),
      Kind::Internal => "Kind::Internal".to_string(),
      Kind::Choice(enum_name) => {
        let source = if in_stable.is_some() { stable } else { nightly };
        let mut choices = String::from("Kind::Choice(&[");
        for variant in &source.enums[enum_name] {
          let variant_stability = |found: Option<&Opt>, release: &Release| {
            let present = release
              .enums
              .get(enum_name)
              .and_then(|vs| vs.iter().find(|v| v.name == variant.name));
            match (found, present) {
              (Some(o), Some(v)) if o.stable && v.stable => "Stable",
              (Some(_), Some(_)) => "Unstable",
              _ => "Absent",
            }
          };
          choices.push_str(&format!(
            "Choice {{ name: {:?}, doc: {:?}, channels: Some(Channels {{ stable: \
                         Stability::{}, nightly: Stability::{} }}) }},",
            variant.name,
            variant.doc,
            variant_stability(in_stable, stable),
            variant_stability(in_nightly, nightly),
          ));
        }
        choices.push_str("])");
        choices
      }
    };
    out.push_str(&format!(
      "Setting {{ name: {:?}, file: File::Format, kind: {}, default: {:?}, \
             default_2024: {:?}, summary: {:?}, doc: {:?}, channels: Some(Channels {{ stable: \
             Stability::{}, nightly: Stability::{} }}), hidden: {}, replaced_by: {:?} }},\n",
      option.name,
      kind,
      option.default,
      option.default_2024,
      sentence(&option.summary),
      option.doc,
      stability(in_stable),
      stability(in_nightly),
      option.hidden,
      option.replaced_by,
    ));
  }
  out.push_str("];\n");
  rustfmt(&out)
}

/// A default expression as TOML.
fn render_default(
  expr: &str,
  kind: &Kind,
  enums: &BTreeMap<String, Vec<Variant>>,
  version: &str,
) -> Option<String> {
  let expr: String = expr.split_whitespace().collect::<Vec<_>>().join(" ");
  if expr == VERSION_DEFAULT {
    return Some(format!("{:?}", version));
  }
  if let Some((_, toml)) = SPECIAL_DEFAULTS.iter().find(|(e, _)| *e == expr) {
    return Some(toml.to_string());
  }
  match kind {
    Kind::Bool if expr == "true" || expr == "false" => Some(expr),
    Kind::Integer if expr.chars().all(|c| c.is_ascii_digit()) => Some(expr),
    Kind::Choice(name) => {
      let (enum_name, variant) = expr.split_once("::")?;
      if enum_name != name {
        return None;
      }
      let found = enums[name].iter().find(|v| v.ident == variant)?;
      Some(format!("{:?}", found.name))
    }
    _ => None,
  }
}

/// `text` with a capital first letter and a final full stop.
fn sentence(text: &str) -> String {
  let text = text.trim();
  if text.is_empty() {
    return String::new();
  }
  let mut chars = text.chars();
  let first = chars.next().unwrap().to_uppercase().collect::<String>();
  let mut out = first + chars.as_str();
  if !out.ends_with(['.', '!', '?', ')']) {
    out.push('.');
  }
  out
}

/// The index of the closing quote of the string starting at `at`.
fn skip_string(bytes: &[u8], at: usize) -> usize {
  let mut idx = at + 1;
  while idx < bytes.len() {
    match bytes[idx] {
      b'\\' => idx += 1,
      b'"' => return idx,
      _ => {}
    }
    idx += 1;
  }
  idx
}

/// `text` split at `separator` outside strings, comments and brackets; comment lines are
/// dropped and empty pieces left out.
fn split_top_level(text: &str, separator: char) -> Vec<&str> {
  let bytes = text.as_bytes();
  let mut pieces = Vec::new();
  let mut depth = 0i32;
  let mut start = 0;
  let mut idx = 0;
  while idx < bytes.len() {
    match bytes[idx] {
      b'"' => idx = skip_string(bytes, idx),
      b'/' if bytes.get(idx + 1) == Some(&b'/') => {
        let end = bytes[idx..]
          .iter()
          .position(|b| *b == b'\n')
          .map_or(bytes.len(), |p| idx + p);
        if text[start..idx].trim().is_empty() {
          start = end;
        }
        idx = end;
        continue;
      }
      b'(' | b'[' | b'{' => depth += 1,
      b')' | b']' | b'}' => depth -= 1,
      b if depth == 0 && b == separator as u8 => {
        pieces.push(&text[start..idx]);
        start = idx + 1;
      }
      _ => {}
    }
    idx += 1;
  }
  pieces.push(&text[start..]);
  pieces
    .into_iter()
    .map(strip_comment_lines)
    .filter(|piece| !piece.trim().is_empty())
    .collect()
}

/// The value of a Rust string literal, with `\` line continuations joined.
fn string_literal(text: &str) -> Option<String> {
  let inner = text.trim().strip_prefix('"')?.strip_suffix('"')?;
  let mut out = String::new();
  let mut chars = inner.chars().peekable();
  while let Some(ch) = chars.next() {
    if ch != '\\' {
      out.push(ch);
      continue;
    }
    match chars.next()? {
      '\n' => {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
          chars.next();
        }
      }
      'n' => out.push('\n'),
      other => out.push(other),
    }
  }
  Some(out)
}

fn strip_comment_lines(piece: &str) -> &str {
  let mut rest = piece;
  loop {
    let trimmed = rest.trim_start();
    if !trimmed.starts_with("//") {
      return rest;
    }
    rest = trimmed.find('\n').map_or("", |n| &trimmed[n..]);
  }
}

/// Markdown links reduced to their text, outside code spans: `[text](url)`, `[text]()` and
/// reference links `[text]`.
fn strip_links(text: &str) -> String {
  let mut out = String::new();
  let mut in_code = false;
  let mut chars = text.chars().peekable();
  while let Some(ch) = chars.next() {
    match ch {
      '`' => {
        in_code = !in_code;
        out.push(ch);
      }
      '[' if !in_code => {
        let mut label = String::new();
        let mut closed = false;
        for c in chars.by_ref() {
          if c == ']' {
            closed = true;
            break;
          }
          label.push(c);
        }
        out.push_str(&label);
        if closed && chars.peek() == Some(&'(') {
          for c in chars.by_ref() {
            if c == ')' {
              break;
            }
          }
        }
      }
      _ => out.push(ch),
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_create_config_entries() {
    let text = "\ncreate_config! {\n    // Fundamental stuff\n    max_width: MaxWidth, true, \"Maximum width\";\n    hard_tabs: HardTabs, false, \"Use tab \\\n        characters; really\";\n}\n";
    let entries = parse_create_config(text).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "max_width");
    assert_eq!(entries[0].config_type, "MaxWidth");
    assert!(entries[0].stable);
    assert_eq!(entries[1].summary, "Use tab characters; really");
    assert!(!entries[1].stable);
  }

  #[test]
  fn test_defaults_with_style_edition_arms() {
    let text = "\nconfig_option_with_style_edition_default!(\n    // Misc.\n    MaxWidth, usize, _ => 100;\n    VersionConfig, Version, Edition2024 => Version::Two, _ => Version::One;\n    Heur, WidthHeuristics, _ => WidthHeuristics::scaled(100);\n);\n";
    let defaults = parse_defaults(text).unwrap();
    assert_eq!(
      defaults["MaxWidth"],
      ("usize".to_string(), "100".to_string(), None)
    );
    assert_eq!(defaults["VersionConfig"].2.as_deref(), Some("Version::Two"));
    assert_eq!(defaults["Heur"].1, "WidthHeuristics::scaled(100)");
  }

  #[test]
  fn test_docs_stop_at_the_details() {
    let text = "## `indent_style`\n\nIndent on [items](#x).\nSecond line.\n\nMore.\n\n- **Default value**: `\"Block\"`\n\n## `bare`\n\n- **Default value**: `1`\n";
    let docs = parse_docs(text);
    assert_eq!(
      docs["indent_style"],
      "Indent on items. Second line.\n\nMore."
    );
    assert!(!docs.contains_key("bare"));
    assert_eq!(
      strip_links("See [RFC 3338] and [Edition 2015]() but not `#[cfg(test)]` or `[]`."),
      "See RFC 3338 and Edition 2015 but not `#[cfg(test)]` or `[]`."
    );
  }

  #[test]
  fn test_enums_and_default_rendering() {
    let text = "#[config_type]\n/// Doc\npub enum Edition {\n    #[value = \"2015\"]\n    #[doc_hint = \"2015\"]\n    /// Edition 2015\n    Edition2015,\n    #[value = \"2027\"]\n    #[unstable_variant]\n    Edition2027,\n}\n#[config_type]\npub enum Color {\n    /// always use color\n    Always,\n    Never,\n}\n";
    let enums = parse_enums(text).unwrap();
    assert_eq!(
      enums["Edition"],
      vec![
        Variant {
          ident: "Edition2015".into(),
          name: "2015".into(),
          doc: "Edition 2015.".into(),
          stable: true
        },
        Variant {
          ident: "Edition2027".into(),
          name: "2027".into(),
          doc: String::new(),
          stable: false
        },
      ]
    );
    assert_eq!(enums["Color"][0].doc, "Always use color.");
    let edition = Kind::Choice("Edition".into());
    let color = Kind::Choice("Color".into());
    let render = |e: &str, k: &Kind| render_default(e, k, &enums, "1.10.0");
    assert_eq!(
      render("Edition::Edition2015", &edition).as_deref(),
      Some("\"2015\"")
    );
    assert_eq!(render("Color::Never", &color).as_deref(), Some("\"Never\""));
    assert_eq!(render("Color::Sometimes", &color), None);
    assert_eq!(
      render(VERSION_DEFAULT, &Kind::Text).as_deref(),
      Some("\"1.10.0\"")
    );
    assert_eq!(render("100", &Kind::Bool), None);
  }

  #[test]
  fn test_release_of() {
    assert_eq!(
      release_of(
        "rustfmt 1.10.0, rust-lang/rust b940084d7eb6a299eb4bfeb8e34901bc051e7ac4, released 2026-10-01"
      ),
      Some((
        "1.10.0".to_string(),
        "b940084d7eb6a299eb4bfeb8e34901bc051e7ac4".to_string()
      ))
    );
  }

  #[test]
  fn test_the_formatter_is_not_named_in_its_docs() {
    assert_eq!(
      neutral_name(
        "rustfmt will not touch it. It is a bug in Rustfmt; see `rustfmt --help`, \
                 rustfmt.toml, rustfmt::skip and rustfmt-config."
      ),
      "The formatter will not touch it. It is a bug in the formatter; see \
             `rustfmt --help`, rustfmt.toml, rustfmt::skip and rustfmt-config."
    );
  }
}
