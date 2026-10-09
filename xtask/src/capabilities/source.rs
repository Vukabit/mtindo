//! Reads rustfmt's surface from its source tree (`src/tools/rustfmt` in rust-lang/rust).
//!
//! - Options and their stability: the top-level `create_config!` in `src/config/mod.rs`
//!   (`name: Type, stable, "description";`).
//! - Hidden options: `HIDE_OPTIONS` in `src/config/config_type.rs`, plus the options
//!   `PartialConfig::to_toml` strips because they cannot be set in `rustfmt.toml`
//!   (`cloned.NAME = None;` in `src/config/mod.rs`).
//! - Values: the `Possible values` line of each option in `Configurations.md`, when it is a
//!   list of at least two names (booleans and free-form values are not listed), including
//!   the `Stable values` / `Unstable values` sub-lists.
//! - rustfmt flags: the `opts.opt*` calls in `make_opts` (`src/bin/main.rs`); calls inside an
//!   `if is_nightly { ... }` block are unstable.
//! - cargo-fmt flags: the `long = "..."` arguments in `src/cargo-fmt/main.rs`, plus clap's
//!   `--help`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::surface::{Found, Kind, Reading};

/// (flag) from cargo-fmt's clap arguments.
pub fn parse_cargo_fmt_flags(text: &str) -> Vec<String> {
    let mut flags = Vec::new();
    let mut rest = text;
    while let Some(idx) = rest.find("long = \"") {
        rest = &rest[idx + 8..];
        if let Some(end) = rest.find('"') {
            flags.push(format!("--{}", &rest[..end]));
        }
    }
    if !flags.iter().any(|f| f == "--help") {
        flags.push("--help".to_string());
    }
    flags
}

/// (option, stable) from the top-level `create_config!` invocation.
pub fn parse_create_config(text: &str) -> Result<Vec<(String, bool)>, String> {
    let start = text
        .find("\ncreate_config! {")
        .ok_or("no top-level `create_config! {` in src/config/mod.rs")?;
    let open = start + text[start..].find('{').unwrap_or(0);
    let mut options = Vec::new();
    for line in balanced(text, open).lines() {
        let trimmed = line.trim();
        let Some((name, rest)) = trimmed.split_once(':') else {
            continue;
        };
        if trimmed.starts_with("//") || !is_option_name(name) {
            continue;
        }
        let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
        if parts.len() >= 2 && (parts[1] == "true" || parts[1] == "false") {
            options.push((name.to_string(), parts[1] == "true"));
        }
    }
    if options.is_empty() {
        return Err("`create_config!` lists no options".to_string());
    }
    Ok(options)
}

/// (option, [(value, stable)]) from Configurations.md.
pub fn parse_documented_values(text: &str) -> BTreeMap<String, Vec<(String, bool)>> {
    let mut values = BTreeMap::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut current: Option<String> = None;
    for (idx, line) in lines.iter().enumerate() {
        if let Some(heading) = line.strip_prefix("## `") {
            current = heading.strip_suffix('`').map(str::to_string);
            continue;
        }
        let Some(rest) = line.strip_prefix("- **Possible values**:") else {
            continue;
        };
        let Some(option) = current.take() else {
            continue;
        };
        let mut found: Vec<(String, bool)> = Vec::new();
        if rest.trim().is_empty() {
            for sub in lines[idx + 1..].iter() {
                let sub = sub.trim();
                if let Some(list) = sub.strip_prefix("- Stable values:") {
                    found.extend(quoted_names(list).into_iter().map(|v| (v, true)));
                } else if let Some(list) = sub.strip_prefix("- Unstable values:") {
                    found.extend(quoted_names(list).into_iter().map(|v| (v, false)));
                } else {
                    break;
                }
            }
        } else {
            found = quoted_names(rest).into_iter().map(|v| (v, true)).collect();
        }
        let names_only = found.iter().all(|(v, _)| {
            !v.is_empty()
                && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && v != "true"
                && v != "false"
        });
        if found.len() >= 2 && names_only {
            values.insert(option, found);
        }
    }
    values
}

/// The names in `HIDE_OPTIONS: [&str; N] = [ ... ]`.
pub fn parse_hidden_options(text: &str) -> Vec<String> {
    let Some(start) = text.find("HIDE_OPTIONS") else {
        return Vec::new();
    };
    let rest = &text[start..];
    let Some(list_start) = rest.find("= [") else {
        return Vec::new();
    };
    let list = &rest[list_start + 3..];
    let list = &list[..list.find(']').unwrap_or(list.len())];
    quoted_names(list)
}

/// (flag, stable) from the `opts.opt*` calls in `make_opts`.
pub fn parse_rustfmt_flags(text: &str) -> Result<Vec<(String, bool)>, String> {
    let start = text
        .find("fn make_opts")
        .ok_or("no `fn make_opts` in src/bin/main.rs")?;
    let open = start + text[start..].find('{').unwrap_or(0);
    let body = balanced(text, open);
    let mut nightly_ranges = Vec::new();
    let mut search = 0;
    while let Some(idx) = body[search..].find("if is_nightly {") {
        let block_open = search + idx + "if is_nightly ".len();
        let inner = balanced(body, block_open);
        nightly_ranges.push((block_open, block_open + inner.len() + 1));
        search = block_open + 1;
    }
    let mut flags = Vec::new();
    let mut search = 0;
    while let Some(idx) = body[search..].find("opts.opt") {
        let call = search + idx;
        search = call + 1;
        let end = body[call..].find(");").map_or(body.len(), |e| call + e);
        let strings = string_literals(&body[call..end]);
        let Some(long) = strings.get(1) else {
            continue;
        };
        let stable = !nightly_ranges
            .iter()
            .any(|(from, to)| *from < call && call < *to);
        flags.push((format!("--{}", long), stable));
    }
    if flags.is_empty() {
        return Err("`make_opts` declares no flags".to_string());
    }
    Ok(flags)
}

/// The options `to_toml` sets to `None` before printing a config.
pub fn parse_toml_stripped(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("cloned."))
        .filter_map(|rest| rest.strip_suffix(" = None;"))
        .filter(|name| is_option_name(name))
        .map(str::to_string)
        .collect()
}

pub fn read(label: &str, description: &str, dir: &Path) -> Result<Reading, String> {
    let load = |path: &str| {
        fs::read_to_string(dir.join(path))
            .map_err(|e| format!("cannot read {}: {}", dir.join(path).display(), e))
    };
    let config = load("src/config/mod.rs")?;
    let options = parse_create_config(&config)?;
    let mut hidden = parse_hidden_options(&load("src/config/config_type.rs")?);
    hidden.extend(parse_toml_stripped(&config));
    let values = parse_documented_values(&load("Configurations.md")?);
    let mut items = Vec::new();
    for (name, stable) in options.iter() {
        let mut found = Found::new(Kind::Option, name.clone(), *stable);
        found.hidden = hidden.contains(name);
        items.push(found);
        for (value, value_stable) in values.get(name).into_iter().flatten() {
            items.push(Found::new(
                Kind::Value,
                format!("{}={}", name, value),
                *stable && *value_stable,
            ));
        }
    }
    for (flag, stable) in parse_rustfmt_flags(&load("src/bin/main.rs")?)? {
        items.push(Found::new(Kind::Flag, format!("rustfmt:{}", flag), stable));
    }
    for flag in parse_cargo_fmt_flags(&load("src/cargo-fmt/main.rs")?) {
        items.push(Found::new(Kind::Flag, format!("cargo-fmt:{}", flag), true));
    }
    Ok(Reading {
        label: label.to_string(),
        description: description.to_string(),
        items,
    })
}

/// The text between the `{` at `open` and its matching `}`.
fn balanced(text: &str, open: usize) -> &str {
    let mut depth = 0;
    for (idx, ch) in text[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[open + 1..open + idx];
                }
            }
            _ => {}
        }
    }
    &text[open + 1..]
}

fn is_option_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Names quoted with backticks, or else with double quotes, minus the quotes inside.
fn quoted_names(text: &str) -> Vec<String> {
    let delimiter = if text.contains('`') { '`' } else { '"' };
    text.split(delimiter)
        .skip(1)
        .step_by(2)
        .map(|name| name.trim_matches('"').to_string())
        .collect()
}

fn string_literals(text: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '"' {
            continue;
        }
        let mut literal = String::new();
        let mut escaped = false;
        for c in chars.by_ref() {
            if escaped {
                literal.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                break;
            } else {
                literal.push(c);
            }
        }
        literals.push(literal);
    }
    literals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_config_reads_the_top_level_block_only() {
        let text = "use x;\n\ncreate_config! {\n    // Fundamental stuff\n    max_width: MaxWidth, true, \"Maximum width of each line\";\n    use_small_heuristics: UseSmallHeuristics, true, \"Whether to use different \\\n        formatting: for items\";\n    indent_style: IndentStyleConfig, false,\n        \"How do we indent\";\n}\n\nfn test() {\n        create_config! {\n            verbose: Verbose, false, \"x\";\n        }\n}\n";
        assert_eq!(
            parse_create_config(text).unwrap(),
            vec![
                ("max_width".to_string(), true),
                ("use_small_heuristics".to_string(), true),
                ("indent_style".to_string(), false),
            ]
        );
        assert!(parse_create_config("fn main() {}").is_err());
    }

    #[test]
    fn test_documented_values_in_every_layout() {
        let text = "## `newline_style`\n\n- **Default value**: `\"Auto\"`\n- **Possible values**: `\"Auto\"`, `\"Native\"`, `\"Unix\"`\n- **Stable**: Yes\n\n## `imports_layout`\n\n- **Possible values**: \"Horizontal\", \"Mixed\"\n\n## `group_imports`\n\n- **Possible values**: `Preserve`, `One`\n\n## `style_edition`\n\n- **Possible values**:\n  - Stable values: `\"2015\"`, `\"2024\"`\n  - Unstable values: `\"2027\"`\n- **Stable**: Yes\n\n## `hard_tabs`\n\n- **Possible values**: `true`, `false`\n\n## `required_version`\n\n- **Possible values**: `semver` compliant values\n\n## `skip_macro_invocations`\n\n- **Possible values**: a list of macro name idents, `[\"name_0\", \"*\"]`\n";
        let values = parse_documented_values(text);
        let names = |option: &str| {
            values[option]
                .iter()
                .map(|(v, _)| v.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(names("newline_style"), vec!["Auto", "Native", "Unix"]);
        assert_eq!(names("imports_layout"), vec!["Horizontal", "Mixed"]);
        assert_eq!(names("group_imports"), vec!["Preserve", "One"]);
        assert_eq!(
            values["style_edition"],
            vec![
                ("2015".to_string(), true),
                ("2024".to_string(), true),
                ("2027".to_string(), false)
            ]
        );
        assert_eq!(values.len(), 4, "{:?}", values.keys());
    }

    #[test]
    fn test_hidden_options() {
        let text = "const HIDE_OPTIONS: [&str; 2] = [\n    \"verbose\",\n    \"file_lines\",\n];";
        assert_eq!(parse_hidden_options(text), vec!["verbose", "file_lines"]);
    }

    #[test]
    fn test_toml_stripped_options() {
        let text = "    pub fn to_toml(&self) {\n        let mut cloned = self.clone();\n        cloned.file_lines = None;\n        cloned.print_misformatted_file_names = None;\n        cloned.x.y = None;\n";
        assert_eq!(
            parse_toml_stripped(text),
            vec!["file_lines", "print_misformatted_file_names"]
        );
    }

    #[test]
    fn test_rustfmt_flags_and_nightly_blocks() {
        let text = "fn make_opts() -> Options {\n    let mut opts = Options::new();\n    opts.optflag(\n        \"\",\n        \"check\",\n        \"Run in 'check' mode.\",\n    );\n    let emit_opts = if is_nightly {\n        \"[a|b]\"\n    } else {\n        \"[a]\"\n    };\n    opts.optopt(\"\", \"emit\", \"What data\", emit_opts);\n    if is_nightly {\n        opts.optflag(\"\", \"unstable-features\", \"Enables {unstable} features\");\n    }\n    opts.optflag(\"v\", \"verbose\", \"Print \\\"verbose\\\" output\");\n    opts\n}\n";
        assert_eq!(
            parse_rustfmt_flags(text).unwrap(),
            vec![
                ("--check".to_string(), true),
                ("--emit".to_string(), true),
                ("--unstable-features".to_string(), false),
                ("--verbose".to_string(), true),
            ]
        );
    }

    #[test]
    fn test_cargo_fmt_flags() {
        let text = "#[arg(short = 'q', long = \"quiet\")]\nquiet: bool,\n#[arg(long = \"manifest-path\", value_name = \"manifest-path\")]\n";
        assert_eq!(
            parse_cargo_fmt_flags(text),
            vec!["--quiet", "--manifest-path", "--help"]
        );
    }
}
