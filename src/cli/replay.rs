//! What runs a report's bug again, and running it (`mto report recheck`).
//!
//! The settings are kept as a configuration file holds them, `key = value`, so that
//! `--config-path` reads them: written from the settings the run used, field by field
//! against the defaults (as `{:#?}` writes them), and kept only when reading the file back
//! gives the same settings. A report kept before replays were shows only those fields as
//! lines (`layout: sections: Comments`); the same mapping reads them, and they are used only
//! when every line maps to a key and the file read back shows the very same lines. The
//! edition, which those reports do not show, is the command line's, or layout's when layout
//! was on; else each edition is tried, so that a report is found fixed only when it passes
//! under all of them.

use crate::Settings;
use crate::config::{self, Value};
use crate::options::format::FormatConfig;
use crate::options::layout::LayoutConfig;
use crate::options::{self, File, Kind};
use crate::report::{Entry, Failure, Replay};
use crate::syntax::Edition;

/// What a replay file starts with.
const HEADER: &str = "# The settings mto found this bug with, as an mto.toml holds them. With the \
                      smallest\n# failing part beside it, `mto --config-path replay.toml --emit \
                      stdout input.rs` runs it\n# again; `mto report recheck` does so for every \
                      report.\n";

/// A field a configuration key sets under another name, with the value it takes the key
/// to: the same (`None`), or, for a flag, the key's value when it is true.
const RENAMED: &[(&str, &str, Option<&str>)] = &[
  ("binop_back", "binop_separator", Some("Back")),
  ("blank_lines_tight", "blank_lines_lower_bound", None),
  ("brace_next_line", "brace_style", Some("AlwaysNextLine")),
  (
    "brace_prefer_same_line",
    "brace_style",
    Some("PreferSameLine"),
  ),
  (
    "compressed_type_punctuation",
    "type_punctuation_density",
    Some("Compressed"),
  ),
  ("condense_wildcards", "condense_wildcard_suffixes", None),
  ("field_init_shorthand", "use_field_init_shorthand", None),
  ("float_trailing_zero", "float_literal_trailing_zero", None),
  (
    "impl_method_order_trait",
    "impl_method_order",
    Some("trait"),
  ),
  ("leading_pipes", "match_arm_leading_pipes", None),
  ("small_heuristics", "use_small_heuristics", None),
  ("tab_width", "tab_spaces", None),
  ("test_module_in_place", "test_module", Some("in_place")),
  ("trailing_comma_never", "trailing_comma", Some("Never")),
  ("try_shorthand", "use_try_shorthand", None),
];

/// What a replay file says when the report did not give the edition.
const TRIED: &str = "# The report did not say the edition: this is the first one the bug shows \
                     under.\n";

/// Fields only the walk reads, which a replay neither writes nor compares.
const WALK_ONLY: &[&str] = &["ignore_dirs", "ignore_paths"];

/// How a bug fails, as two of its sightings compare: a panic by what it said, not where
/// (the line moves from build to build); a verifier stop by the rules broken.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Signature {
  Panic(String),
  Rules(Vec<String>),
}

impl Signature {
  fn of(failure: &Failure) -> Signature {
    match failure {
      Failure::Panicked { said } => Signature::Panic(unplaced(said).to_string()),
      Failure::Verifier { violations } => Signature::Rules(
        crate::report::rules(violations)
          .into_iter()
          .map(String::from)
          .collect(),
      ),
    }
  }

  /// How the report `text` says its bug failed.
  fn shown(text: &str) -> Option<Signature> {
    if let Some(said) = block_after(text, "mto failed (panicked) while laying out a file:") {
      return Some(Signature::Panic(unplaced(said).to_string()));
    }
    let at = text.find("The verifier stopped the write")?;
    let mut rules: Vec<String> = text[at..]
      .lines()
      .skip(1)
      .skip_while(|line| line.trim().is_empty())
      .map_while(|line| line.strip_prefix("- `"))
      .filter_map(|rest| rest.split_once('`').map(|(rule, _)| rule.to_string()))
      .collect();
    rules.sort_unstable();
    rules.dedup();
    (!rules.is_empty()).then_some(Signature::Rules(rules))
  }
}

/// What `mto report recheck` found of one report.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
  /// It fails as it did: kept.
  Reproduces,
  /// It fails, but not as it did: kept, with how it fails now.
  Differs(String),
  /// It passes: removed.
  Fixed,
  /// It cannot be run again with certainty: kept as it is, with why.
  CannotReplay(String),
}

/// A report rechecked: the verdict, and the replay to keep with it, for a report kept
/// before replays were that still fails.
#[derive(Debug)]
pub struct Rechecked {
  pub verdict: Verdict,
  pub replay: Option<Replay>,
}

/// A setting as a report shows it: its part (`layout` or `format`), and its field and value
/// as `{:#?}` writes them, on one line.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
  part: &'static str,
  field: String,
  value: String,
}

/// Runs `entry`'s bug again under this build, with its replay, or, for a report kept before
/// replays were, with what its text shows when that is certain.
pub fn recheck(entry: &Entry) -> Rechecked {
  let cannot = |why: String| Rechecked {
    verdict: Verdict::CannotReplay(why),
    replay: None,
  };
  let text = match entry.read() {
    Ok(text) => text,
    Err(e) => return cannot(format!("cannot read its report: {}", e)),
  };
  let Some(was) = Signature::shown(&text) else {
    return cannot("the report does not say how it failed".to_string());
  };
  if let Some(replay) = entry.replay() {
    return match read_back(&replay.settings) {
      Ok(settings) => Rechecked {
        verdict: verdict(&was, &[(replay.input, settings, String::new())]).0,
        replay: None,
      },
      Err(why) => cannot(format!("its replay.toml no longer reads: {}", why)),
    };
  }
  let (inputs, whole) = match inputs(entry, &text) {
    Ok(found) => found,
    Err(why) => return cannot(why),
  };
  let tries = match settings_shown(&text) {
    Ok(tries) => tries,
    Err(why) => return cannot(why),
  };
  let mut trials = Vec::new();
  for input in &inputs {
    for (settings, file) in &tries {
      trials.push((input.clone(), settings.clone(), file.clone()));
    }
  }
  let (verdict, failing) = verdict(&was, &trials);
  // Kept with what runs it again: the smallest part, found now for a report that kept none.
  let replay = match (&verdict, failing) {
    (Verdict::Reproduces, Some((input, settings, file))) if whole => {
      let minimal = crate::with_stack(|| {
        crate::verify::report::minimize(&input, settings.edition, &|candidate| {
          crate::replay(candidate, &settings).is_some_and(|now| Signature::of(&now) == was)
        })
      });
      Some(Replay {
        input: minimal,
        settings: file,
      })
    }
    (Verdict::Reproduces | Verdict::Differs(_), Some((input, _, file))) if !whole => Some(Replay {
      input,
      settings: file,
    }),
    _ => None,
  };
  Rechecked { verdict, replay }
}

/// The settings `settings` were made from, as a replay file: `None` when no file reads back
/// the same (a setting no key gives, such as the macros a crate root skips).
pub fn settings_file(settings: &Settings) -> Option<String> {
  let mut shown = Vec::new();
  if let Some(config) = &settings.layout {
    shown.extend(differing(
      "layout",
      &format!("{:#?}", config),
      &format!("{:#?}", LayoutConfig::default()),
    ));
  }
  if let Some(config) = &settings.format {
    shown.extend(differing(
      "format",
      &format!("{:#?}", config),
      &format!("{:#?}", FormatConfig::default()),
    ));
  }
  let text = file_text(
    settings.layout.is_some(),
    settings.format.is_some(),
    &shown,
    Some(settings.edition),
  )
  .ok()?;
  let back = read_back(&text).ok()?;
  let walked = |config: &Option<LayoutConfig>| {
    config.clone().map(|mut config| {
      config.ignore_dirs.clear();
      config.ignore_paths.clear();
      config
    })
  };
  let same = walked(&back.layout) == walked(&settings.layout)
    && back.format == settings.format
    && back.edition == settings.edition;
  same.then_some(text)
}

/// The text of the fenced block that follows `heading` in `text`.
fn block_after<'a>(text: &'a str, heading: &str) -> Option<&'a str> {
  let rest = &text[text.find(heading)? + heading.len()..];
  let rest = rest.trim_start_matches('\n').strip_prefix("```")?;
  let rest = &rest[rest.find('\n')? + 1..];
  let end = rest.find("\n```").unwrap_or(rest.len());
  Some(&rest[..end])
}

/// The fields of `debug` (a struct as `{:#?}` writes it) whose values differ from those of
/// `default`, each value on one line: `[a, b]` written `[ a, b, ]`.
fn differing(part: &'static str, debug: &str, default: &str) -> Vec<Shown> {
  let defaults = fields(default);
  fields(debug)
    .into_iter()
    .filter(|(field, value)| {
      !WALK_ONLY.contains(&field.as_str()) && defaults.iter().all(|(f, v)| f != field || v != value)
    })
    .map(|(field, value)| Shown { part, field, value })
    .collect()
}

/// The edition `command` gives, the last of `--edition` and `--config edition=`.
fn edition_given(command: &str) -> Option<Edition> {
  let words: Vec<&str> = command.split_whitespace().collect();
  let mut found = None;
  for (i, word) in words.iter().enumerate() {
    let (flag, inline) = match word.split_once('=') {
      Some((flag, value)) if flag.starts_with("--") => (flag, Some(value)),
      _ => (*word, None),
    };
    let value = || inline.or_else(|| words.get(i + 1).copied());
    match flag {
      "--edition" => found = value().and_then(Edition::from_year).or(found),
      "--config" => {
        for pair in value().unwrap_or("").split(',') {
          if let Some(("edition", year)) = pair.split_once('=') {
            found = Edition::from_year(year.trim_matches('"')).or(found);
          }
        }
      }
      _ => {}
    }
  }
  found
}

/// The fields of a struct as `{:#?}` writes it, each value on one line.
fn fields(debug: &str) -> Vec<(String, String)> {
  let mut out: Vec<(String, String)> = Vec::new();
  for line in debug.lines().skip(1) {
    let field = line
      .strip_prefix("    ")
      .filter(|rest| !rest.starts_with(' '))
      .and_then(|rest| rest.split_once(": "))
      .filter(|(name, _)| name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    match (field, out.last_mut()) {
      (Some((name, value)), _) => out.push((name.to_string(), value.to_string())),
      (None, Some((_, value))) if line != "}" => {
        value.push(' ');
        value.push_str(line.trim());
      }
      _ => {}
    }
  }
  for (_, value) in &mut out {
    let trimmed = value.trim_end_matches(',').to_string();
    *value = trimmed;
  }
  out
}

/// The replay file for settings shown as `shown`, with `layout` and formatting on or off,
/// and `edition` when known: every field as the key that sets it, with the style edition
/// always given, so that the edition does not change it. An error names a field no key
/// gives, or two fields that ask one key for different values.
fn file_text(
  layout: bool,
  format: bool,
  shown: &[Shown],
  edition: Option<Edition>,
) -> Result<String, String> {
  let mut keys: Vec<(&'static str, String)> = vec![
    ("layout", layout.to_string()),
    ("format", format.to_string()),
  ];
  fn add(
    keys: &mut Vec<(&'static str, String)>,
    key: &'static str,
    value: String,
  ) -> Result<(), String> {
    match keys.iter().find(|(k, _)| *k == key) {
      Some((_, was)) if *was != value => {
        Err(format!("`{}` would be both {} and {}", key, was, value))
      }
      Some(_) => Ok(()),
      None => {
        keys.push((key, value));
        Ok(())
      }
    }
  }
  if let Some(edition) = edition {
    add(&mut keys, "edition", quoted(edition.year()))?;
  }
  for one in shown {
    let line = || format!("{}: {}: {}", one.part, one.field, one.value);
    let found = key_of(&one.field, &one.value).map_err(|why| format!("`{}` {}", line(), why))?;
    if let Some((key, value)) = found {
      add(&mut keys, key, value).map_err(|why| format!("`{}`: {}", line(), why))?;
    }
  }
  if !keys.iter().any(|(key, _)| *key == "style_edition") {
    add(&mut keys, "style_edition", quoted("2015"))?;
  }
  let mut text = HEADER.to_string();
  for (key, value) in keys {
    text.push_str(&format!("{} = {}\n", key, value));
  }
  Ok(text)
}

/// The inputs to run a report kept before replays were with: its smallest part, with each
/// end the report's code block may have cut off (a line end, none, a blank line), else the
/// whole file it was seen in when that is unchanged; and whether it is the whole file.
fn inputs(entry: &Entry, text: &str) -> Result<(Vec<String>, bool), String> {
  if let Some(minimal) = minimal_shown(text) {
    let ends = ["\n", "", "\n\n"];
    let inputs = ends
      .iter()
      .map(|end| format!("{}{}", minimal, end))
      .collect();
    return Ok((inputs, false));
  }
  for seen in &entry.files {
    let whole = seen
      .path
      .as_ref()
      .and_then(|path| std::fs::read_to_string(path).ok());
    if let Some(whole) = whole
      && crate::history::sha256::hex(whole.as_bytes()) == seen.hash
    {
      return Ok((vec![whole], true));
    }
  }
  Err("it kept no smallest part, and the file it was seen in has changed or gone".to_string())
}

/// The key that sets `field` to `value` (as `{:#?}` writes it), with the value as TOML;
/// `None` for a width too wide for any file to give, which only `use_small_heuristics =
/// "Off"` makes (so it is left to that).
fn key_of(field: &str, value: &str) -> Result<Option<(&'static str, String)>, String> {
  let renamed = RENAMED.iter().find(|(f, _, _)| *f == field);
  let name = renamed.map_or(field, |(_, key, _)| key);
  let setting = options::find(name)
    .filter(|s| s.file != File::User)
    .ok_or("is set by no key")?;
  if let Some((_, _, Some(when_true))) = renamed {
    return match value {
      "true" => Ok(Some((setting.name, quoted(when_true)))),
      _ => Err("is set by no key at that value".to_string()),
    };
  }
  let unparsed = || "has a value that cannot be read".to_string();
  let choice = |given: &str| {
    let plain = |text: &str| text.replace('_', "").to_ascii_lowercase();
    setting
      .choices()
      .iter()
      .find(|c| plain(c.name) == plain(given.trim_matches('"')))
      .map(|c| c.name.to_string())
  };
  let value = match value.strip_prefix("Some(") {
    Some(inner) => inner
      .strip_suffix(')')
      .ok_or_else(unparsed)?
      .trim()
      .trim_end_matches(','),
    None => value,
  };
  let toml = match &setting.kind {
    Kind::Bool if matches!(value, "true" | "false") => value.to_string(),
    Kind::Integer if value.parse::<u64>().is_ok() => match value.parse::<i64>() {
      Ok(_) => value.to_string(),
      Err(_) => return Ok(None),
    },
    Kind::Choice(_) => quoted(&choice(value).ok_or_else(unparsed)?),
    Kind::Text => quoted(&unescape(value).ok_or_else(unparsed)?),
    Kind::List => {
      let items: Option<Vec<String>> = list(value)
        .ok_or_else(unparsed)?
        .iter()
        .map(|item| unescape(item))
        .collect();
      Value::List(items.ok_or_else(unparsed)?).to_toml()
    }
    Kind::Sequence(_) => {
      let items: Option<Vec<String>> = list(value)
        .ok_or_else(unparsed)?
        .iter()
        .map(|item| choice(item))
        .collect();
      Value::List(items.ok_or_else(unparsed)?).to_toml()
    }
    _ => return Err(unparsed()),
  };
  Ok(Some((setting.name, toml)))
}

/// The items of a list as `{:#?}` writes it on one line (`[ "a", "b", ]`), each as written.
fn list(value: &str) -> Option<Vec<String>> {
  let inner = value.strip_prefix('[')?.strip_suffix(']')?;
  let mut items = Vec::new();
  let mut item = String::new();
  let mut quoted = false;
  let mut escaped = false;
  for c in inner.chars() {
    match c {
      _ if escaped => escaped = false,
      '\\' if quoted => escaped = true,
      '"' => quoted = !quoted,
      ',' if !quoted => {
        items.push(std::mem::take(&mut item));
        continue;
      }
      _ => {}
    }
    item.push(c);
  }
  items.push(item);
  Some(
    items
      .into_iter()
      .map(|item| item.trim().to_string())
      .filter(|item| !item.is_empty())
      .collect(),
  )
}

/// The smallest failing part a report's text shows, without its last line end.
fn minimal_shown(text: &str) -> Option<&str> {
  let rest = &text[text.find("The smallest part of the file that still fails")?..];
  let start = rest.find("```rust\n")? + "```rust\n".len();
  let rest = &rest[start..];
  // It ends where the report goes on: the drop-down's end, or (in older reports) the command.
  let end = ["\n```\n\n</details>", "\n```\n\nCommand:"]
    .iter()
    .filter_map(|end| rest.find(end))
    .min()?;
  Some(&rest[..end])
}

/// `text` as a TOML string.
fn quoted(text: &str) -> String {
  Value::Text(text.to_string()).to_toml()
}

/// The settings a replay file gives, as a run would use them.
fn read_back(text: &str) -> Result<Settings, String> {
  let config = config::replayed(text).map_err(|e| e.message)?;
  Settings::of(&config).ok_or_else(|| "neither layout nor formatting is on".to_string())
}

/// The settings a report kept before replays were shows, each as a replay file and what it
/// reads back as, one for each edition it may have had: an error when a line maps to no key,
/// or the file does not read back as the very same lines.
fn settings_shown(text: &str) -> Result<Vec<(Settings, String)>, String> {
  let block = block_after(text, "Settings that differ from the defaults:")
    .ok_or("the report shows no settings")?;
  let lines: Vec<&str> = block
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty() && *line != "(none)")
    .collect();
  let (mut layout, mut format) = (true, true);
  let mut shown = Vec::new();
  for line in &lines {
    let unmapped = || format!("the setting line `{}` cannot be mapped", line);
    let (part, rest) = line.split_once(": ").ok_or_else(unmapped)?;
    let part = match part {
      "layout" => "layout",
      "format" => "format",
      _ => return Err(unmapped()),
    };
    if rest == "off" {
      match part {
        "layout" => layout = false,
        _ => format = false,
      }
      continue;
    }
    let (field, value) = rest.split_once(": ").ok_or_else(unmapped)?;
    if !field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
      return Err(unmapped());
    }
    shown.push(Shown {
      part,
      field: field.to_string(),
      value: value.to_string(),
    });
  }
  let command = block_after(text, "Command:").unwrap_or("");
  // Layout's edition is the run's; without layout, only the command line says it.
  let edition_shown = shown
    .iter()
    .find(|s| s.part == "layout" && s.field == "edition")
    .and_then(|s| Edition::from_year(&s.value));
  let editions = match (edition_given(command), layout) {
    (Some(edition), _) => vec![edition],
    (None, true) => vec![edition_shown.unwrap_or(Edition::DEFAULT)],
    (None, false) => vec![
      Edition::E2015,
      Edition::E2018,
      Edition::E2021,
      Edition::E2024,
    ],
  };
  let tried = editions.len() > 1;
  let mut tries = Vec::new();
  for edition in editions {
    let mut file = file_text(layout, format, &shown, Some(edition))?;
    if tried {
      file.insert_str(HEADER.len(), TRIED);
    }
    let settings = read_back(&file)?;
    let mut back: Vec<String> = crate::settings_text(&settings)
      .lines()
      .map(|line| line.trim().to_string())
      .collect();
    let mut was: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
    back.sort();
    was.sort();
    if back != was || settings.edition != edition {
      return Err(format!(
        "its settings do not read back the same (now: {})",
        back.join("; ")
      ));
    }
    tries.push((settings, file));
  }
  Ok(tries)
}

/// A string as `{:#?}` writes it (`"a\"b"`), read back.
fn unescape(written: &str) -> Option<String> {
  let inner = written.strip_prefix('"')?.strip_suffix('"')?;
  let mut out = String::new();
  let mut chars = inner.chars();
  while let Some(c) = chars.next() {
    if c != '\\' {
      out.push(c);
      continue;
    }
    match chars.next()? {
      'n' => out.push('\n'),
      't' => out.push('\t'),
      'r' => out.push('\r'),
      '0' => out.push('\0'),
      'u' => {
        let code: String = chars.by_ref().skip(1).take_while(|&c| c != '}').collect();
        out.push(char::from_u32(u32::from_str_radix(&code, 16).ok()?)?);
      }
      other => out.push(other),
    }
  }
  Some(out)
}

/// `said` without where it was said: a panic's ` (at src/x.rs:1)`.
fn unplaced(said: &str) -> &str {
  match said.rfind(" (at ") {
    Some(at) if said.ends_with(')') => &said[..at],
    _ => said,
  }
}

/// What running `trials` (an input, its settings and their file) finds of a bug that
/// failed as `was`: it reproduces when any fails the same way, differs when any fails
/// otherwise, and is fixed only when all pass. Gives the trial it fails with.
fn verdict(
  was: &Signature,
  trials: &[(String, Settings, String)],
) -> (Verdict, Option<(String, Settings, String)>) {
  let mut differs = None;
  for (input, settings, file) in trials {
    let now = crate::with_stack(|| crate::replay(input, settings));
    match now {
      Some(now) if Signature::of(&now) == *was => {
        return (
          Verdict::Reproduces,
          Some((input.clone(), settings.clone(), file.clone())),
        );
      }
      Some(now) if differs.is_none() => {
        let trial = (input.clone(), settings.clone(), file.clone());
        differs = Some((now.title(), trial));
      }
      _ => {}
    }
  }
  match differs {
    Some((what, trial)) => (Verdict::Differs(what), Some(trial)),
    None => (Verdict::Fixed, None),
  }
}

#[cfg(test)]
mod tests;
