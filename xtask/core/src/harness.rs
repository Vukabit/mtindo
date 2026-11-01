//! rustfmt's test harness, as its own test runner reads a test pair: the settings a
//! `// rustfmt-KEY: VALUE` header asks for (`config` loading a file from `tests/config/`
//! first), and those settings as the `rustfmt.toml` that makes the released binary run
//! the same way.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Keys in rustfmt's test headers that steer its test runner rather than set options.
pub const DIRECTIVES: &[&str] = &["target", "stable", "unstable"];

/// How a header line starts: rustfmt's own repository (and its stable releases' suites)
/// write a comment, the suite in rust-lang/rust a directive.
pub const HEADER_PREFIXES: &[&str] = &["// rustfmt-", "//@ rustfmt-"];

/// `settings` as a `rustfmt.toml`.
pub fn config_text(settings: &[(String, String)], string_keys: &BTreeSet<String>) -> String {
  settings
    .iter()
    .map(|(k, v)| toml_line(k, v, string_keys))
    .collect()
}

/// What follows a header line's prefix (`KEY: VALUE`), when `line` is one.
pub fn header(line: &str) -> Option<&str> {
  HEADER_PREFIXES.iter().find_map(|p| line.strip_prefix(p))
}

/// The settings a test input asks for, from its `// rustfmt-KEY: VALUE` lines.
pub fn headers(text: &str) -> Vec<(String, String)> {
  text
    .lines()
    .filter_map(header)
    .filter_map(|rest| rest.split_once(':'))
    .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
    .filter(|(key, _)| !key.is_empty() && key.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
    .collect()
}

/// `text` with its header lines spelled `// rustfmt-`: the suite in rust-lang/rust writes
/// them `//@ rustfmt-`, a directive, where rustfmt's own repository (and its stable
/// releases' suites) write a comment; the spelling is not formatting.
pub fn older_headers(text: &str) -> String {
  text
    .split_inclusive('\n')
    .map(|line| match line.strip_prefix("//@ rustfmt-") {
      Some(rest) => format!("// rustfmt-{}", rest),
      None => line.to_string(),
    })
    .collect()
}

/// Whether `line` is the test runner's rather than the pair's: a header in any spelling a
/// suite has written one (`//rustfmt-` too), or another `//@` directive. A suite may spell
/// these anew without changing the formatting.
pub fn runner_line(line: &str) -> bool {
  let line = line.trim_start();
  header(line).is_some() || line.starts_with("//@") || line.starts_with("//rustfmt-")
}

/// The settings an input runs with, following rustfmt's test harness: `config` loads a
/// file from `tests/config/` first (header lines then override it), and the runner-only
/// keys are dropped. `None` when a named config file is missing.
pub fn settings(text: &str, config_dir: &Path) -> Option<Vec<(String, String)>> {
  let headers = headers(text);
  let mut settings: Vec<(String, String)> = Vec::new();
  for (key, value) in headers.iter() {
    if key == "config" {
      let file = fs::read_to_string(config_dir.join(value)).ok()?;
      for (k, v) in toml_settings(&file) {
        settings.retain(|(existing, _)| *existing != k);
        settings.push((k, v));
      }
    }
  }
  for (key, value) in headers {
    if key == "config" || DIRECTIVES.contains(&key.as_str()) {
      continue;
    }
    settings.retain(|(existing, _)| *existing != key);
    settings.push((key, value));
  }
  Some(settings)
}

/// The options whose values are a fixed list, from rustfmt's recorded surface
/// (`records/upstream/rustfmt-surface.txt`): their values are strings in its config.
pub fn string_keys(surface: &str) -> BTreeSet<String> {
  surface
    .lines()
    .filter_map(|line| line.strip_prefix("value"))
    .filter_map(|rest| rest.split_whitespace().next())
    .filter_map(|item| item.split_once('=').map(|(k, _)| k.to_string()))
    .collect()
}

/// A setting as a TOML line. Options whose values are a fixed list (`edition`,
/// `style_edition`, `imports_granularity`...) are strings in rustfmt's config, so their
/// values are always quoted, even `2021`; otherwise numbers, booleans, arrays and
/// already-quoted strings are written as they are and words are quoted.
pub fn toml_line(key: &str, value: &str, string_keys: &BTreeSet<String>) -> String {
  let bare = !string_keys.contains(key)
    && (value.parse::<f64>().is_ok()
      || value == "true"
      || value == "false"
      || value.starts_with('[')
      || value.starts_with('{'))
    || value.starts_with('"');
  if bare {
    format!("{} = {}\n", key, value)
  } else {
    format!(
      "{} = \"{}\"\n",
      key,
      value.replace('\\', "\\\\").replace('"', "\\\"")
    )
  }
}

/// `key = value` lines of a small TOML file, values unquoted; tables and comments skipped.
/// An array may span several lines; it is joined onto one.
pub fn toml_settings(text: &str) -> Vec<(String, String)> {
  let mut settings = Vec::new();
  let mut lines = text.lines().map(str::trim);
  while let Some(line) = lines.next() {
    if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
      continue;
    }
    let Some((key, value)) = line.split_once('=') else {
      continue;
    };
    let mut value = value.trim().to_string();
    let open = |v: &str| v.matches('[').count() > v.matches(']').count();
    while open(&value) {
      let Some(more) = lines.next() else {
        break;
      };
      if !more.is_empty() && !more.starts_with('#') {
        value.push(' ');
        value.push_str(more);
      }
    }
    let unquoted = value
      .strip_prefix('"')
      .and_then(|v| v.strip_suffix('"'))
      .map(str::to_string)
      .unwrap_or(value);
    settings.push((key.trim().to_string(), unquoted));
  }
  settings
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_both_spellings_of_a_header_are_read() {
    let text = "//@ rustfmt-max_width: 50\n// rustfmt-hard_tabs: true\nfn f() {}\n";
    assert_eq!(headers(text).len(), 2);
    assert_eq!(
      older_headers(text),
      "// rustfmt-max_width: 50\n// rustfmt-hard_tabs: true\nfn f() {}\n"
    );
  }

  #[test]
  fn test_harness_settings_follow_rustfmts_test_runner() {
    let dir = std::env::temp_dir().join(format!("xtask-race-config-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
      dir.join("small_tabs.toml"),
      "# comment\nmax_width = 80\nhard_tabs = true\nimports_granularity = \"Crate\"\n",
    )
    .unwrap();
    let text = "// rustfmt-config: small_tabs.toml\n// rustfmt-hard_tabs: false\n// rustfmt-target: hello.rs\n// rustfmt-stable: true\n// rustfmt-unstable: true\nfn f() {}\n";
    let read = settings(text, &dir).unwrap();
    assert_eq!(
      read,
      vec![
        ("max_width".to_string(), "80".to_string()),
        ("imports_granularity".to_string(), "Crate".to_string()),
        ("hard_tabs".to_string(), "false".to_string()),
      ]
    );
    assert!(settings("// rustfmt-config: gone.toml\n", &dir).is_none());
    fs::remove_dir_all(&dir).unwrap();
  }

  #[test]
  fn test_settings_of_reads_header_lines() {
    let text = "// rustfmt-max_width: 80\n// rustfmt-imports_granularity: Crate\n// plain comment\nfn f() {}\n// rustfmt-Bad: x\n";
    assert_eq!(
      headers(text),
      vec![
        ("max_width".to_string(), "80".to_string()),
        ("imports_granularity".to_string(), "Crate".to_string())
      ]
    );
  }

  #[test]
  fn test_string_keys_come_from_the_surfaces_values() {
    let surface = "option  edition  in=all\nvalue   edition=2021  in=all\nvalue   brace_style=AlwaysNextLine  in=all\n";
    let keys = string_keys(surface);
    assert_eq!(
      keys.into_iter().collect::<Vec<_>>(),
      vec!["brace_style", "edition"]
    );
  }

  #[test]
  fn test_toml_lines() {
    let strings: BTreeSet<String> = ["edition", "style_edition", "imports_granularity"]
      .iter()
      .map(|s| s.to_string())
      .collect();
    assert_eq!(toml_line("max_width", "80", &strings), "max_width = 80\n");
    assert_eq!(
      toml_line("hard_tabs", "true", &strings),
      "hard_tabs = true\n"
    );
    assert_eq!(
      toml_line("imports_granularity", "Crate", &strings),
      "imports_granularity = \"Crate\"\n"
    );
    // Fixed-list options are strings, even when the value looks like a number.
    assert_eq!(
      toml_line("edition", "2021", &strings),
      "edition = \"2021\"\n"
    );
    assert_eq!(
      toml_line("style_edition", "2024", &strings),
      "style_edition = \"2024\"\n"
    );
    assert_eq!(
      toml_line("ignore", "[\"a\"]", &strings),
      "ignore = [\"a\"]\n"
    );
    assert_eq!(
      toml_line("edition", "\"2018\"", &strings),
      "edition = \"2018\"\n"
    );
  }

  #[test]
  fn test_toml_settings_join_multi_line_arrays() {
    let text =
      "# comment\nignore = [\n  \"tests/**/ice.rs\",\n  \"b.rs\"\n]\nmax_width = 80\n[table]\n";
    assert_eq!(
      toml_settings(text),
      vec![
        (
          "ignore".to_string(),
          "[ \"tests/**/ice.rs\", \"b.rs\" ]".to_string()
        ),
        ("max_width".to_string(), "80".to_string()),
      ]
    );
  }
}
