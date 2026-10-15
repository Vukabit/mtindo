//! `docs-config [--check]`: the settings reference in docs/config/, a dictionary of every key
//! mto reads from a configuration file. It is written from the settings table
//! (`mtindo::options`, which `mto --man` and `mto config print` read too), each key's status
//! in compat/capabilities.txt, and the rustfmt releases pinned in
//! compat/rustfmt-surface.txt, so it cannot disagree with them; `--check` only compares, for
//! the `config-docs` gate.
//!
//! The reference is public: its Markdown keeps within the docs gate's 80 columns, and it
//! names rustfmt only by its file names, as the naming gate asks, so the table's text and the
//! capability notes pass through [`public`] on the way in.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use mtindo::cli::manual::{self, wrap, wrap_block};
use mtindo::options::{self, Channels, Choice, File, Kind, Setting, Stability};

/// The command that writes the reference, named in it and in the stale message.
pub const COMMAND: &str = "cargo xtask docs config";

/// Where the reference is written, from the repository root.
pub const DIR: &str = "docs/config";

/// A rustfmt release the settings table follows.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Release {
  version: String,
  released: String,
}

/// What the reference reads besides the table.
struct Sources {
  /// By item: `option:max_width`, `value:brace_style=AlwaysNextLine`, `config:layout`.
  support: BTreeMap<String, Support>,
  stable: Release,
  nightly: Release,
}

impl Sources {
  fn of_setting(&self, setting: &Setting) -> Option<&Support> {
    match setting.file {
      File::Format => self.support.get(&format!("option:{}", setting.name)),
      File::Mto => self.support.get(&format!("config:{}", setting.name)),
      File::User => None,
    }
  }

  fn of_value(&self, setting: &Setting, choice: &Choice) -> Option<&Support> {
    self
      .support
      .get(&format!("value:{}={}", setting.name, choice.name))
  }
}

/// What mto does with a key or value: its status and note in compat/capabilities.txt.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Support {
  status: String,
  /// Made fit for the reference by [`public_note`]; may be empty.
  note: String,
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
  let check = match args {
    [] => false,
    [flag] if flag == "--check" => true,
    _ => {
      eprintln!("docs-config: takes only --check");
      return ExitCode::from(2);
    }
  };
  let files = match generate(root) {
    Ok(files) => files,
    Err(e) => {
      eprintln!("{}", e);
      return ExitCode::from(1);
    }
  };
  let dir = root.join(DIR);
  if check {
    let stale: Vec<&str> = files
      .iter()
      .filter(|(name, text)| fs::read_to_string(dir.join(name)).ok().as_ref() != Some(text))
      .map(|(name, _)| *name)
      .collect();
    if stale.is_empty() {
      println!("{} matches the settings table", DIR);
      return ExitCode::SUCCESS;
    }
    for name in stale {
      println!("{}/{} is stale; run `{}`", DIR, name, COMMAND);
    }
    return ExitCode::from(1);
  }
  if let Err(e) = fs::create_dir_all(&dir) {
    eprintln!("cannot create {}: {}", DIR, e);
    return ExitCode::from(1);
  }
  for (name, text) in &files {
    if let Err(e) = fs::write(dir.join(name), text) {
      eprintln!("cannot write {}/{}: {}", DIR, name, e);
      return ExitCode::from(1);
    }
    println!("wrote {}/{}", DIR, name);
  }
  ExitCode::SUCCESS
}

/// The anchor a Markdown renderer gives a key's heading: `history.enabled` becomes
/// `historyenabled`.
fn anchor(name: &str) -> String {
  name
    .chars()
    .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
    .map(|c| c.to_ascii_lowercase())
    .collect()
}

fn default_line(setting: &Setting) -> String {
  if setting.default.is_empty() {
    return "Default: none; the standard formatter sets it itself, and mto ignores the \
            value."
      .to_string();
  }
  match setting.file {
    File::Format => {}
    File::Mto => return format!("Default: `{}`, in every style edition.", setting.default),
    File::User => return format!("Default: `{}`.", setting.default),
  }
  format!(
    "Default: `{}`; from style edition 2024: `{}`{}.",
    setting.default,
    setting.default_for(2024),
    if setting.default_2024.is_some() {
      ""
    } else {
      ", the same"
    }
  )
}

/// The schema's description of a key: its text, then what the reference's facts add.
fn description(setting: &Setting, sources: &Sources) -> String {
  let mut text = public(setting.doc);
  if let Some(later) = setting.default_2024 {
    text.push_str(&format!(
      "\n\nFrom style edition 2024 the default is {}.",
      later
    ));
  }
  if let Some(replacement) = setting.replaced_by {
    text.push_str(&format!("\n\nDeprecated: use `{}`.", replacement));
  }
  if let Some(support) = sources.of_setting(setting) {
    text.push_str(&format!("\n\nIn mto: {}", support_text(support)));
  }
  text
}

/// One key: its heading, its description, then its facts as a list.
fn entry(setting: &Setting, sources: &Sources) -> String {
  let mut out = format!("### `{}`\n\n", setting.name);
  for part in setting.doc.split("\n\n") {
    for line in part.lines() {
      out.push_str(&wrap_block(&public(line)));
    }
    out.push('\n');
  }
  let fact = |text: &str| wrap(text, "*   ");
  out.push_str(&fact(&format!("Type: {}.", type_name(setting))));
  out.push_str(&fact(&default_line(setting)));
  if let Some(channels) = setting.channels {
    out.push_str(&fact(&format!(
      "In the standard formatter: {}.",
      stability(channels, sources)
    )));
  }
  let support = sources.of_setting(setting);
  if let Some(support) = support {
    out.push_str(&fact(&format!("In mto: {}", support_text(support))));
  }
  if let Some(replacement) = setting.replaced_by {
    out.push_str(&fact(&format!("Deprecated: use `{}`.", replacement)));
  }
  let choices = setting.choices();
  if !choices.is_empty() {
    out.push_str("*   Values:\n");
    for choice in choices {
      let mut text = format!("`{}`", choice.name);
      if !choice.doc.is_empty() {
        text.push_str(&format!(": {}", public(choice.doc)));
      }
      if let Some(channels) = choice.channels
        && setting.channels.is_some_and(|c| c != channels)
      {
        text.push_str(&format!(
          " In the standard formatter: {}.",
          stability(channels, sources)
        ));
      }
      if let Some(value) = sources.of_value(setting, choice)
        && support.is_none_or(|s| s.status != value.status)
      {
        text.push_str(&format!(" In mto: {}", support_text(value)));
      }
      out.push_str(&wrap(&text, "    *   "));
    }
  }
  out.push('\n');
  out
}

fn file_name(file: File) -> &'static str {
  match file {
    File::Mto => "mto.toml",
    File::Format => "rustfmt.toml",
    File::User => "config.toml",
  }
}

/// The front matter: what wrote the page, the releases the table follows, and the counts.
fn front_matter(
  title: &str,
  counts: &[(&str, usize)],
  statuses: &[(&str, usize)],
  sources: &Sources,
) -> String {
  let mut out = format!("---\ntitle: {}\ngenerated_by: {}\n", title, COMMAND);
  out.push_str("standard_formatter:\n");
  for (channel, release) in [("stable", &sources.stable), ("nightly", &sources.nightly)] {
    out.push_str(&format!(
      "  {}:\n    version: \"{}\"\n    released: \"{}\"\n",
      channel, release.version, release.released
    ));
  }
  out.push_str("keys:\n");
  for (name, count) in counts {
    out.push_str(&format!("  {}: {}\n", name, count));
  }
  if !statuses.is_empty() {
    out.push_str("in_mto:\n");
    for (status, count) in statuses {
      out.push_str(&format!("  {}: {}\n", status.replace('-', "_"), count));
    }
  }
  out.push_str("---\n\n");
  out
}

/// Every file of the reference, by name, from the table and the files in compat/.
fn generate(root: &Path) -> Result<Vec<(&'static str, String)>, String> {
  let read = |path: &str| {
    fs::read_to_string(root.join(path)).map_err(|e| format!("cannot read {}: {}", path, e))
  };
  let sources = sources(
    &read("compat/capabilities.txt")?,
    &read("compat/rustfmt-surface.txt")?,
  )?;
  if sources.stable.version != options::RUSTFMT_VERSION {
    return Err(format!(
      "compat/rustfmt-surface.txt pins rustfmt {}, but src/options/rustfmt.rs follows {}; \
       run `cargo xtask settings`",
      sources.stable.version,
      options::RUSTFMT_VERSION
    ));
  }
  let settings: Vec<&Setting> = options::all().collect();
  Ok(render(&settings, &sources))
}

/// `text` as HTML: special characters escaped and backticked spans as `<code>`.
fn html(text: &str) -> String {
  let escaped = text
    .replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;");
  let mut out = String::new();
  for (idx, part) in escaped.split('`').enumerate() {
    if idx % 2 == 1 {
      out.push_str(&format!("<code>{}</code>", part));
    } else {
      out.push_str(part);
    }
  }
  out
}

fn json_string(text: &str) -> String {
  let mut out = String::from("\"");
  for c in text.chars() {
    match c {
      '"' => out.push_str("\\\""),
      '\\' => out.push_str("\\\\"),
      '\n' => out.push_str("\\n"),
      c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
      c => out.push(c),
    }
  }
  out.push('"');
  out
}

/// The page for one file's keys.
fn page(file: File, settings: &[&Setting], sources: &Sources) -> String {
  let mine: Vec<&Setting> = settings
    .iter()
    .copied()
    .filter(|s| s.file == file)
    .collect();
  let hidden = mine.iter().filter(|s| s.hidden).count();
  let (title, intro) = match file {
    File::Mto => (
      "mto.toml",
      "mto's own settings, which only `mto.toml` and `.mto.toml` may hold; the \
       standard formatter would not understand them. Besides these, an `mto.toml` may \
       set `root = true`, so that the search upwards for more `mto.toml` files stops at \
       it, and any of the formatting settings in [rustfmt.toml.md](rustfmt.toml.md)."
        .to_string(),
    ),
    File::Format => (
      "rustfmt.toml",
      format!(
        "The formatting settings, which `rustfmt.toml`, `.rustfmt.toml`, `mto.toml` and \
         `.mto.toml` may hold, as the standard formatter {} documents them. mto reads \
         and checks them all, on every channel, so an existing `rustfmt.toml` works \
         unchanged; each entry says how far mto honors the setting, and the standard \
         formatter's own stability in the releases pinned above.",
        sources.stable.version
      ),
    ),
    File::User => (
      "config.toml",
      format!(
        "{} Its keys are written in sections: `history.enabled` is `enabled` under \
         `[history]`.",
        manual::USER
      ),
    ),
  };
  let mut counts = vec![("total", mine.len())];
  if file == File::Format {
    counts.push(("hidden", hidden));
  }
  let mut out = front_matter(
    &format!("{} settings", title),
    &counts,
    &statuses(&mine, sources),
    sources,
  );
  out.push_str(&format!("# `{}` settings\n\n", title));
  out.push_str(&paragraph(&intro));
  out.push_str(&paragraph(&format!(
    "Generated by `{}` from mto's settings table; [README.md](README.md) explains how \
     mto finds its settings.",
    COMMAND
  )));
  if file == File::User {
    out.push_str("```toml\n[history]\nenabled = true\nkeep_days = 30\n```\n\n");
  }
  out.push_str("## Settings\n\n");
  for setting in mine.iter().filter(|s| !s.hidden) {
    out.push_str(&entry(setting, sources));
  }
  if hidden > 0 {
    out.push_str("## Hidden settings\n\n");
    out.push_str(&paragraph(
      "The standard formatter leaves these out of its own listings: they are \
       deprecated, or set by the formatter itself. mto accepts them so that existing \
       files keep working.",
    ));
    for setting in mine.iter().filter(|s| s.hidden) {
      out.push_str(&entry(setting, sources));
    }
  }
  // One line break at the end, as the docs keep it.
  while out.ends_with("\n\n") {
    out.pop();
  }
  out
}

fn page_name(file: File) -> String {
  format!("{}.md", file_name(file))
}

/// A paragraph wrapped to the width, then a blank line.
fn paragraph(text: &str) -> String {
  format!("{}\n", wrap(text, ""))
}

/// `text` naming rustfmt as "the standard formatter", but in the spellings that do work:
/// its file names and the attributes formatters skip items by.
fn public(text: &str) -> String {
  const KEPT: &[&str] = &["rustfmt.toml", "rustfmt::", "rustfmt_skip"];
  let lower = text.to_ascii_lowercase();
  let mut out = String::new();
  let mut from = 0;
  while let Some(at) = lower[from..].find("rustfmt").map(|i| from + i) {
    let end = at + "rustfmt".len();
    out.push_str(&text[from..at]);
    let kept = KEPT.iter().any(|k| lower[at..].starts_with(k)) || text[..at].ends_with("cfg_attr(");
    if kept {
      out.push_str(&text[at..end]);
    } else if text[at..].starts_with('R') {
      out.push_str("The standard formatter");
    } else {
      out.push_str("the standard formatter");
    }
    from = end;
  }
  out.push_str(&text[from..]);
  out
}

/// A capabilities note for the reference's readers: without the tests it cites as
/// evidence or the plan's milestone ids, and naming rustfmt as the standard formatter.
fn public_note(note: &str) -> String {
  // A parenthesis that cites tests: `(src/config/tests.rs; corpus check agreement)`.
  let mut kept = String::new();
  let mut rest = note.trim();
  while let Some(open) = rest.find('(') {
    let Some(close) = rest[open..].find(')').map(|c| open + c) else {
      break;
    };
    if rest[open..close].contains(".rs") {
      kept.push_str(rest[..open].trim_end());
    } else {
      kept.push_str(&rest[..=close]);
    }
    rest = &rest[close + 1..];
  }
  kept.push_str(rest);
  let milestone = |word: &str| {
    let word = word.trim_end_matches([',', ';', '.', ':']);
    let mut chars = word.chars();
    matches!(chars.next(), Some('F' | 'M' | 'N'))
      && (1..=2).contains(&chars.as_str().len())
      && chars.all(|c| c.is_ascii_digit())
  };
  let mut words: Vec<&str> = Vec::new();
  for word in kept.split_whitespace() {
    if word.contains(".rs") {
      continue;
    }
    if milestone(word) {
      // "follows it from N3": the milestone, and the word that led to it.
      if words
        .last()
        .is_some_and(|w| matches!(*w, "from" | "in" | "since"))
      {
        words.pop();
      }
      continue;
    }
    words.push(word);
  }
  let text = words.join(" ");
  public(text.trim_end_matches([',', ';']))
}

/// The index: how mto finds settings, then every key.
fn readme(settings: &[&Setting], sources: &Sources) -> String {
  let count = |file: File| settings.iter().filter(|s| s.file == file).count();
  let mut out = front_matter(
    "mto settings reference",
    &[
      ("total", settings.len()),
      ("mto", count(File::Mto)),
      ("formatting", count(File::Format)),
      ("user", count(File::User)),
    ],
    &statuses(settings, sources),
    sources,
  );
  out.push_str("# mto settings reference\n\n");
  out.push_str(&paragraph(&format!(
    "Every key mto reads from a configuration file: its type, legal values, defaults, \
     and how far mto honors it. These pages are generated from mto's settings table, \
     the one `mto --man` and `mto config print` read, by `{}`; change the table, not \
     the pages.",
    COMMAND
  )));
  out.push_str("## The files\n\n");
  for (link, rest) in [
    (
      "[`mto.toml`](mto.toml.md)",
      " or `.mto.toml`: mto's own settings, which only these files may hold. They \
       may hold the formatting settings too.",
    ),
    (
      "[`rustfmt.toml`](rustfmt.toml.md)",
      " or `.rustfmt.toml`: the formatting settings, as the standard formatter reads them.",
    ),
    (
      "[`config.toml`](config.toml.md)",
      ": the per-user settings, how mto behaves for one person on every project.",
    ),
    (
      "[`mto.schema.json`](mto.schema.json)",
      ": a JSON Schema of `mto.toml`, for editors.",
    ),
  ] {
    out.push_str(&wrap(&format!("{}{}", link, rest), "*   "));
  }
  out.push('\n');
  out.push_str("## How mto finds settings\n\n");
  let lookup = manual::LOOKUP.replace("The defaults below", "The defaults on these pages");
  for part in lookup.split("\n\n") {
    out.push_str(&wrap_block(part));
    out.push('\n');
  }
  out.push_str("## Turning mto on\n\n");
  let summary = |name: &str| {
    settings
      .iter()
      .find(|s| s.name == name)
      .map(|s| public(s.summary))
      .unwrap_or_default()
  };
  out.push_str(&paragraph(
    "Layout and formatting are off by default, so mto changes nothing in a project \
     until an `mto.toml` turns them on; `mto init` writes one with layout on. Three \
     keys of `mto.toml` decide where and what mto does:",
  ));
  out.push_str(&wrap(
    &format!(
      "`layout = true`: {}. While it is off, mto's other settings but `format` and \
       `ignore_dirs` have no effect, and setting one is a warning.",
      summary("layout")
    ),
    "*   ",
  ));
  out.push_str(&wrap(
    &format!(
      "`format = true`: {}. It does not apply where `disable_all_formatting` is set.",
      summary("format")
    ),
    "*   ",
  ));
  out.push_str(&wrap(
    "`root = true`: this file is the outermost `mto.toml` read; the search upwards for \
     more stops here. Only `mto.toml` and `.mto.toml` may hold it, as `true` or `false`.",
    "*   ",
  ));
  out.push('\n');
  out.push_str("## The per-user file\n\n");
  out.push_str(&paragraph(manual::USER));
  out.push_str(&paragraph(
    "Its keys are written in sections: `history.enabled` is `enabled` under \
     `[history]`.",
  ));
  out.push_str("## Editor support\n\n");
  out.push_str(&paragraph(
    "`mto.schema.json` describes `mto.toml` in JSON Schema (draft 2020-12): each key's \
     type, legal values, default and description, and `root`. TOML editor tooling that \
     reads JSON Schema can check and complete `mto.toml` with it. Some tools read \
     a comment on the first line naming the schema by its path or URL; a project \
     keeping a copy beside its `mto.toml` would start the file:",
  ));
  out.push_str("```toml\n#:schema ./mto.schema.json\nlayout = true\n```\n\n");
  out.push_str(&paragraph(
    "mto matches values without regard to case. The schema lists each value as \
     `mto config print` writes it, so an editor may flag another spelling that mto \
     accepts.",
  ));
  out.push_str("## Every key\n\n");
  out.push_str(&paragraph(
    "The last column says how far mto honors the key: supported, partial, missing or \
     not planned; the key's entry says which part. The per-user keys are mto's own \
     behavior, so the column does not apply to them.",
  ));
  let mut sorted = settings.to_vec();
  sorted.sort_by_key(|s| s.name);
  out.push_str("<table>\n<tr><th>Key</th><th>File</th><th>Summary</th><th>In mto</th></tr>\n");
  for setting in sorted {
    let status = sources
      .of_setting(setting)
      .map(|s| status_name(&s.status))
      .unwrap_or("n/a");
    out.push_str(&format!(
      "<tr>\n<td><a href=\"{}#{}\">\n<code>{}</code></a></td>\n<td>{}</td>\n",
      page_name(setting.file),
      anchor(setting.name),
      setting.name,
      file_name(setting.file)
    ));
    out.push_str(&wrap(
      &format!("{}</td>", html(&public(setting.summary))),
      "<td>",
    ));
    out.push_str(&format!("<td>{}</td>\n</tr>\n", status));
  }
  out.push_str("</table>\n");
  out
}

/// The reference for `settings`: the index, a page per file, and the schema.
fn render(settings: &[&Setting], sources: &Sources) -> Vec<(&'static str, String)> {
  vec![
    ("README.md", readme(settings, sources)),
    ("mto.toml.md", page(File::Mto, settings, sources)),
    ("rustfmt.toml.md", page(File::Format, settings, sources)),
    ("config.toml.md", page(File::User, settings, sources)),
    ("mto.schema.json", schema(settings, sources)),
  ]
}

/// The JSON Schema of `mto.toml`: `root`, mto's own keys, then the formatting keys.
fn schema(settings: &[&Setting], sources: &Sources) -> String {
  let mut properties = vec![(
    "root".to_string(),
    vec![
      (
        "description",
        json_string(
          "Stop the search upwards for mto.toml files at this file: it is the outermost \
           one read.",
        ),
      ),
      ("type", json_string("boolean")),
      ("default", "false".to_string()),
    ],
  )];
  let order = |file: File| match file {
    File::Mto => 0,
    File::Format => 1,
    File::User => 2,
  };
  let mut keys: Vec<&Setting> = settings
    .iter()
    .copied()
    .filter(|s| s.file != File::User)
    .collect();
  keys.sort_by_key(|s| order(s.file));
  for setting in keys {
    let mut fields = vec![("description", json_string(&description(setting, sources)))];
    let enumeration = |choices: &[Choice]| {
      let names: Vec<String> = choices.iter().map(|c| json_string(c.name)).collect();
      format!("[{}]", names.join(", "))
    };
    match setting.kind {
      Kind::Bool => fields.push(("type", json_string("boolean"))),
      Kind::Integer => {
        fields.push(("type", json_string("integer")));
        fields.push(("minimum", "0".to_string()));
      }
      Kind::Text | Kind::Size => fields.push(("type", json_string("string"))),
      Kind::Choice(choices) => {
        fields.push(("type", json_string("string")));
        fields.push(("enum", enumeration(choices)));
      }
      Kind::List => {
        fields.push(("type", json_string("array")));
        fields.push(("items", "{\"type\": \"string\"}".to_string()));
      }
      Kind::Sequence(choices) => {
        fields.push(("type", json_string("array")));
        fields.push((
          "items",
          format!(
            "{{\"type\": \"string\", \"enum\": {}}}",
            enumeration(choices)
          ),
        ));
        fields.push(("uniqueItems", "true".to_string()));
      }
      Kind::Internal => {}
    }
    // A default is TOML, and the defaults the table holds (numbers, booleans, strings and
    // arrays of strings) are written the same in JSON.
    if !setting.default.is_empty() {
      fields.push(("default", setting.default.to_string()));
    }
    if setting.replaced_by.is_some() {
      fields.push(("deprecated", "true".to_string()));
    }
    properties.push((setting.name.to_string(), fields));
  }
  let mut out = String::from("{\n");
  out.push_str("  \"$schema\": \"https://json-schema.org/draft/2020-12/schema\",\n");
  out.push_str(&format!(
    "  \"$comment\": {},\n",
    json_string(&format!(
      "Generated by `{}` from mto's settings table; do not edit.",
      COMMAND
    ))
  ));
  out.push_str("  \"title\": \"mto.toml\",\n");
  out.push_str(&format!(
    "  \"description\": {},\n",
    json_string(
      "mto's configuration file: mto's own settings and the formatting settings. mto \
       matches values without regard to case; the values here are spelled as \
       `mto config print` writes them."
    )
  ));
  out.push_str("  \"type\": \"object\",\n");
  out.push_str("  \"additionalProperties\": false,\n");
  out.push_str("  \"properties\": {\n");
  for (idx, (name, fields)) in properties.iter().enumerate() {
    out.push_str(&format!("    {}: {{\n", json_string(name)));
    for (at, (field, value)) in fields.iter().enumerate() {
      let comma = if at + 1 < fields.len() { "," } else { "" };
      out.push_str(&format!("      \"{}\": {}{}\n", field, value, comma));
    }
    let comma = if idx + 1 < properties.len() { "," } else { "" };
    out.push_str(&format!("    }}{}\n", comma));
  }
  out.push_str("  }\n}\n");
  out
}

/// Reads the statuses in capabilities.txt and the pinned releases in rustfmt-surface.txt.
fn sources(capabilities: &str, surface: &str) -> Result<Sources, String> {
  let mut support = BTreeMap::new();
  for line in capabilities.lines() {
    let (head, note) = line.split_once('#').unwrap_or((line, ""));
    let words: Vec<&str> = head.split_whitespace().collect();
    let [kind, name, status] = words[..] else {
      continue;
    };
    let item = match kind {
      "option" | "value" => format!("{}:{}", kind, name),
      "mto" if name.starts_with("config:") => name.to_string(),
      _ => continue,
    };
    let note = public_note(note);
    support.insert(
      item,
      Support {
        status: status.to_string(),
        note,
      },
    );
  }
  let release = |channel: &str| -> Result<Release, String> {
    let words: Vec<&str> = surface
      .lines()
      .map(|line| line.split_whitespace().collect::<Vec<_>>())
      .find(|words| words.len() > 3 && words[0] == "source" && words[1] == channel)
      .ok_or_else(|| format!("compat/rustfmt-surface.txt pins no {} release", channel))?;
    let released = words
      .iter()
      .position(|w| *w == "released")
      .and_then(|at| words.get(at + 1))
      .ok_or_else(|| {
        format!(
          "compat/rustfmt-surface.txt: the {} release has no date",
          channel
        )
      })?;
    Ok(Release {
      version: words[3].trim_end_matches(',').to_string(),
      released: released.to_string(),
    })
  };
  Ok(Sources {
    support,
    stable: release("stable")?,
    nightly: release("nightly")?,
  })
}

fn stability(channels: Channels, sources: &Sources) -> String {
  let word = |stability: Stability| match stability {
    Stability::Stable => "stable",
    Stability::Unstable => "unstable",
    Stability::Absent => "absent",
  };
  format!(
    "{} in {} (stable channel), {} in {} (nightly channel)",
    word(channels.stable),
    sources.stable.version,
    word(channels.nightly),
    sources.nightly.version
  )
}

fn status_name(status: &str) -> &str {
  match status {
    "not-planned" => "not planned",
    other => other,
  }
}

/// How many of `settings` have each status, in the order capabilities.txt explains them.
fn statuses(settings: &[&Setting], sources: &Sources) -> Vec<(&'static str, usize)> {
  let found: Vec<&str> = settings
    .iter()
    .filter_map(|s| sources.of_setting(s))
    .map(|s| s.status.as_str())
    .collect();
  if found.is_empty() {
    return Vec::new();
  }
  ["supported", "partial", "missing", "not-planned"]
    .into_iter()
    .map(|status| (status, found.iter().filter(|s| **s == status).count()))
    .collect()
}

/// "partial. With format = true, for attributes." from a status and its note.
fn support_text(support: &Support) -> String {
  let mut chars = support.note.chars();
  match chars.next() {
    Some(first) => format!(
      "{}. {}{}.",
      status_name(&support.status),
      first.to_uppercase(),
      chars.as_str().trim_end_matches('.')
    ),
    None => format!("{}.", status_name(&support.status)),
  }
}

fn type_name(setting: &Setting) -> &'static str {
  match setting.kind {
    Kind::Bool => "`true` or `false`",
    Kind::Integer => "a whole number, zero or more",
    Kind::Text => "a string",
    Kind::Size => "a size such as `\"256MB\"`",
    Kind::Choice(_) => "one of the values below, as a string",
    Kind::List => "an array of strings",
    Kind::Sequence(_) => "an array of the values below, each at most once",
    Kind::Internal => "any value",
  }
}

#[cfg(test)]
mod tests {
  use mtindo::cli::manual::WIDTH;

  use super::*;

  const CAPABILITIES: &str = "\
# a comment
option  max_width        partial      # with format = true (src/format/tests.rs); from N3
option  brace_style      missing
value   brace_style=Next supported    # tests/domain_8_cli.rs
mto     config:layout    supported    # tests/domain_8_cli.rs
mto     flag:--check     supported
flag    rustfmt:--emit   supported    # as rustfmt writes them
";

  const SURFACE: &str = "\
source  stable   rustfmt 1.10.0, rust-lang/rust b94, released 2026-10-01
source  nightly  rustfmt 1.11.0-nightly, rust-lang/rust 32d, released 2026-10-10
";

  static BRACES: &[Choice] = &[
    Choice {
      name: "Same",
      doc: "On the same line, as rustfmt does.",
      channels: None,
    },
    Choice {
      name: "Next",
      doc: "On the next line.",
      channels: Some(Channels {
        stable: Stability::Absent,
        nightly: Stability::Unstable,
      }),
    },
  ];

  static TABLE: &[Setting] = &[
    Setting {
      name: "max_width",
      file: File::Format,
      kind: Kind::Integer,
      default: "100",
      default_2024: None,
      summary: "Maximum width of each line.",
      doc: "Maximum width of each line.\n\nSee rustfmt.toml and `#[rustfmt::skip]`.",
      channels: Some(Channels {
        stable: Stability::Stable,
        nightly: Stability::Stable,
      }),
      hidden: false,
      replaced_by: None,
    },
    Setting {
      name: "brace_style",
      file: File::Format,
      kind: Kind::Choice(BRACES),
      default: "\"Same\"",
      default_2024: Some("\"Next\""),
      summary: "Where an item's brace goes.",
      doc: "Where an item's brace goes.",
      channels: Some(Channels {
        stable: Stability::Unstable,
        nightly: Stability::Unstable,
      }),
      hidden: false,
      replaced_by: None,
    },
    Setting {
      name: "old_width",
      file: File::Format,
      kind: Kind::Internal,
      default: "",
      default_2024: None,
      summary: "Rustfmt sets it.",
      doc: "Rustfmt sets it.",
      channels: Some(Channels {
        stable: Stability::Unstable,
        nightly: Stability::Unstable,
      }),
      hidden: true,
      replaced_by: Some("max_width"),
    },
    Setting {
      name: "layout",
      file: File::Mto,
      kind: Kind::Bool,
      default: "false",
      default_2024: None,
      summary: "Rearrange the items of each file",
      doc: "Turns layout on.",
      channels: None,
      hidden: false,
      replaced_by: None,
    },
    Setting {
      name: "type_order",
      file: File::Mto,
      kind: Kind::Sequence(BRACES),
      default: "[\"Same\", \"Next\"]",
      default_2024: None,
      summary: "The order of kinds",
      doc: "The order of kinds.",
      channels: None,
      hidden: false,
      replaced_by: None,
    },
    Setting {
      name: "history.enabled",
      file: File::User,
      kind: Kind::Bool,
      default: "true",
      default_2024: None,
      summary: "Record every run",
      doc: "Every run is recorded.",
      channels: None,
      hidden: false,
      replaced_by: None,
    },
  ];

  fn file<'a>(files: &'a [(&str, String)], name: &str) -> &'a str {
    &files.iter().find(|(n, _)| *n == name).unwrap().1
  }

  fn fixture() -> (Vec<&'static Setting>, Sources) {
    (
      TABLE.iter().collect(),
      sources(CAPABILITIES, SURFACE).unwrap(),
    )
  }

  #[test]
  fn test_an_entry_has_every_fact() {
    let (settings, sources) = fixture();
    let page = page(File::Format, &settings, &sources);
    let brace = &page[page.find("### `brace_style`").unwrap()..];
    let brace = &brace[..brace.find("\n\n#").unwrap() + 1];
    assert_eq!(
      brace,
      "### `brace_style`\n\n\
       Where an item's brace goes.\n\n\
       *   Type: one of the values below, as a string.\n\
       *   Default: `\"Same\"`; from style edition 2024: `\"Next\"`.\n\
       *   In the standard formatter: unstable in 1.10.0 (stable channel), unstable in\n    \
       1.11.0-nightly (nightly channel).\n\
       *   In mto: missing.\n\
       *   Values:\n    \
       *   `Same`: On the same line, as the standard formatter does.\n    \
       *   `Next`: On the next line. In the standard formatter: absent in 1.10.0\n        \
       (stable channel), unstable in 1.11.0-nightly (nightly channel). In mto:\n        \
       supported.\n"
    );
    assert!(page.contains("*   Default: `100`; from style edition 2024: `100`, the same.\n"));
    assert!(page.contains("*   In mto: partial. With format = true.\n"));
    // The hidden key has its own section, with what replaced it.
    let hidden = &page[page.find("## Hidden settings").unwrap()..];
    assert!(hidden.contains("### `old_width`"));
    assert!(hidden.contains("*   Deprecated: use `max_width`.\n"));
    assert!(hidden.contains("Default: none;"));
    assert!(page.ends_with(".\n") && !page.ends_with("\n\n"));
  }

  #[test]
  fn test_front_matter_holds_the_releases_and_counts() {
    let (settings, sources) = fixture();
    let page = page(File::Format, &settings, &sources);
    assert!(page.starts_with(
      "---\ntitle: rustfmt.toml settings\ngenerated_by: cargo xtask docs config\n\
       standard_formatter:\n  stable:\n    version: \"1.10.0\"\n    released: \
       \"2026-10-01\"\n  nightly:\n    version: \"1.11.0-nightly\"\n    released: \
       \"2026-10-10\"\nkeys:\n  total: 3\n  hidden: 1\nin_mto:\n  supported: 0\n  \
       partial: 1\n  missing: 1\n  not_planned: 0\n---\n\n# `rustfmt.toml` settings\n"
    ));
    // No statuses are tracked for the per-user keys.
    let user = super::page(File::User, &settings, &sources);
    assert!(user.contains("keys:\n  total: 1\n---\n"));
  }

  #[test]
  fn test_public_names_the_standard_formatter_but_keeps_file_names() {
    assert_eq!(
      public("Rustfmt reads rustfmt.toml and .rustfmt.toml; rustfmt's own way."),
      "The standard formatter reads rustfmt.toml and .rustfmt.toml; the standard \
       formatter's own way."
    );
    assert_eq!(
      public("`#[rustfmt::skip]`, `rustfmt_skip` and `cfg_attr(rustfmt, skip)`"),
      "`#[rustfmt::skip]`, `rustfmt_skip` and `cfg_attr(rustfmt, skip)`"
    );
  }

  #[test]
  fn test_public_note_drops_evidence_and_milestones() {
    assert_eq!(
      public_note(
        " resolved as rustfmt resolves it (src/config/tests.rs); order follows it from N3"
      ),
      "resolved as the standard formatter resolves it; order follows it"
    );
    assert_eq!(public_note(" tests/domain_8_cli.rs"), "");
    assert_eq!(
      public_note(" for calls (calls, chains) and attributes (src/a/tests.rs, tests/b.rs)"),
      "for calls (calls, chains) and attributes"
    );
  }

  #[test]
  fn test_sources_read_statuses_and_releases() {
    assert!(sources(CAPABILITIES, "").is_err());
    let sources = sources(CAPABILITIES, SURFACE).unwrap();
    let items: Vec<&str> = sources.support.keys().map(String::as_str).collect();
    assert_eq!(
      items,
      [
        "config:layout",
        "option:brace_style",
        "option:max_width",
        "value:brace_style=Next"
      ]
    );
    assert_eq!(
      sources.support["option:max_width"],
      Support {
        status: "partial".to_string(),
        note: "with format = true".to_string(),
      }
    );
    assert_eq!(sources.support["config:layout"].note, "");
    assert_eq!(
      sources.stable,
      Release {
        version: "1.10.0".to_string(),
        released: "2026-10-01".to_string(),
      }
    );
    assert_eq!(sources.nightly.version, "1.11.0-nightly");
  }

  #[test]
  fn test_the_index_lists_every_key_with_its_file_and_status() {
    let (settings, sources) = fixture();
    let readme = readme(&settings, &sources);
    assert!(readme.contains("keys:\n  total: 6\n  mto: 2\n  formatting: 3\n  user: 1\n"));
    for setting in &settings {
      assert!(
        readme.contains(&format!("<code>{}</code></a></td>", setting.name)),
        "{}",
        setting.name
      );
    }
    assert!(readme.contains(
      "<tr>\n<td><a href=\"config.toml.md#historyenabled\">\n<code>history.enabled</code></a></td>\n\
       <td>config.toml</td>\n<td>Record every run</td>\n<td>n/a</td>\n</tr>\n"
    ));
    assert!(readme.contains("<td>Where an item's brace goes.</td>\n<td>missing</td>"));
    assert!(readme.contains("<td>The standard formatter sets it.</td>"));
    // Keys are in name order, as a dictionary.
    let at = |name: &str| readme.find(&format!("<code>{}</code></a>", name)).unwrap();
    assert!(at("brace_style") < at("history.enabled") && at("layout") < at("max_width"));
  }

  #[test]
  fn test_the_real_reference_is_public_and_complete() {
    let root = crate::project_root();
    let files = generate(&root).unwrap();
    let banned = ['\u{2014}', '\u{2013}'];
    for (name, text) in &files {
      for (idx, line) in text.lines().enumerate() {
        assert!(
          line.chars().count() <= WIDTH || name.ends_with(".json"),
          "{}:{}: {} columns",
          name,
          idx + 1,
          line.chars().count()
        );
        assert!(!line.contains(banned), "{}:{}: a dash", name, idx + 1);
        // Outside code, where `mto doctor on -- COMMAND` is a command line.
        let prose: String = line.split('`').step_by(2).collect();
        assert!(!prose.contains(" -- "), "{}:{}: a dash", name, idx + 1);
        assert_eq!(
          public(line),
          *line,
          "{}:{}: names rustfmt outside its file names",
          name,
          idx + 1
        );
      }
    }
    let readme = file(&files, "README.md");
    for setting in options::all() {
      let page = file(&files, &page_name(setting.file));
      assert!(
        page.contains(&format!("### `{}`\n", setting.name)),
        "{}",
        setting.name
      );
      assert!(
        readme.contains(&format!("<code>{}</code></a></td>", setting.name)),
        "{}",
        setting.name
      );
    }
  }

  #[test]
  fn test_the_schema_describes_mto_toml() {
    let (settings, sources) = fixture();
    let schema = schema(&settings, &sources);
    assert!(
      schema.starts_with("{\n  \"$schema\": \"https://json-schema.org/draft/2020-12/schema\",\n")
    );
    assert!(schema.contains(
      "    \"max_width\": {\n      \"description\": \"Maximum width of each line.\\n\\nSee \
       rustfmt.toml and `#[rustfmt::skip]`.\\n\\nIn mto: partial. With format = \
       true.\",\n      \"type\": \"integer\",\n      \"minimum\": 0,\n      \"default\": \
       100\n    },\n"
    ));
    assert!(schema.contains(
      "      \"type\": \"string\",\n      \"enum\": [\"Same\", \"Next\"],\n      \
       \"default\": \"Same\"\n"
    ));
    assert!(schema.contains(
      "      \"items\": {\"type\": \"string\", \"enum\": [\"Same\", \"Next\"]},\n      \
       \"uniqueItems\": true,\n      \"default\": [\"Same\", \"Next\"]\n"
    ));
    assert!(schema.contains("\"deprecated\": true"));
    assert!(schema.contains("    \"root\": {\n"));
    // mto.toml cannot hold the per-user keys.
    assert!(!schema.contains("history.enabled"));
    // mto's own keys come first, after `root`.
    assert!(schema.find("\"layout\"").unwrap() < schema.find("\"max_width\"").unwrap());
  }
}
