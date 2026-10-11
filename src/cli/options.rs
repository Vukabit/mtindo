//! `mto config options`: every setting a configuration file may hold, by file, with its legal
//! values and default, and for the standard formatter's settings how far mto honors each
//! (from `compat/capabilities.txt`, generated into [`crate::options::support`]).

use crate::options::{self, File, Kind, Setting, support};

/// The longest a VALUES cell runs before it is cut.
const VALUES_WIDTH: usize = 30;

/// The listing: a table per file, or with `toml`, the same as a commented file to copy from.
pub fn render(toml: bool) -> String {
    let sections = [
        (
            File::Mto,
            "mto.toml (mto's own settings)",
            "mto's own settings, for mto.toml or .mto.toml.",
        ),
        (
            File::Format,
            "rustfmt.toml (the standard formatter's settings; mto.toml accepts them too)",
            "The standard formatter's settings, for rustfmt.toml or mto.toml.",
        ),
        (
            File::User,
            "config.toml (per user; `mto config path` shows where)",
            "Per-user settings, for the user config.toml (`mto config path` shows where).",
        ),
    ];
    let mut out = String::new();
    for (file, heading, comment) in sections {
        let mut settings: Vec<&Setting> = options::all()
            .filter(|s| s.file == file && !s.hidden && s.kind != Kind::Internal)
            .collect();
        settings.sort_by_key(|s| s.name);
        if !out.is_empty() {
            out.push('\n');
        }
        if toml {
            out.push_str(&commented(comment, &settings));
        } else {
            out.push_str(&table(heading, file, &settings));
        }
    }
    out
}

/// A section as TOML: each setting commented out at its default, after its summary and
/// legal values.
fn commented(comment: &str, settings: &[&Setting]) -> String {
    let mut out = format!("# {}\n", comment);
    for setting in settings {
        out.push_str(&format!("\n# {}\n", setting.summary.trim_end_matches('.')));
        out.push_str(&format!("# values: {}", values(setting)));
        if setting.file == File::Format {
            out.push_str(&format!("; mto: {}", status(setting.name)));
        }
        out.push('\n');
        if let Some(default_2024) = setting.default_2024 {
            out.push_str(&format!("# under style edition 2024: {}\n", default_2024));
        }
        out.push_str(&format!("# {} = {}\n", setting.name, setting.default));
    }
    out
}

/// `text` cut to `width` characters, with `...` when cut.
fn cut(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let kept: String = text.chars().take(width.saturating_sub(3)).collect();
    format!("{}...", kept)
}

/// The default, with the style edition 2024 one when it differs.
fn default(setting: &Setting) -> String {
    let short = |text: &str| cut(text, 16);
    match setting.default_2024 {
        Some(default_2024) => format!("{} (2024: {})", short(setting.default), short(default_2024)),
        None => short(setting.default),
    }
}

/// How far mto honors one of the standard formatter's settings.
fn status(name: &str) -> &'static str {
    match support::OPTIONS
        .binary_search_by_key(&name, |&(n, _)| n)
        .map(|i| support::OPTIONS[i].1)
    {
        Ok("supported") => "yes",
        Ok("partial") => "partly",
        Ok("not-planned") => "never",
        _ => "not yet",
    }
}

/// A table section: NAME, VALUES, DEFAULT, and for the standard formatter's settings MTO.
fn table(heading: &str, file: File, settings: &[&Setting]) -> String {
    let mut rows: Vec<[String; 4]> = vec![[
        "NAME".to_string(),
        "VALUES".to_string(),
        "DEFAULT".to_string(),
        if file == File::Format { "MTO" } else { "" }.to_string(),
    ]];
    for setting in settings {
        rows.push([
            setting.name.to_string(),
            cut(&values(setting), VALUES_WIDTH),
            default(setting),
            if file == File::Format {
                status(setting.name).to_string()
            } else {
                String::new()
            },
        ]);
    }
    let width = |column: usize| {
        rows.iter()
            .map(|r| r[column].chars().count())
            .max()
            .unwrap_or(0)
    };
    let widths = [width(0), width(1), width(2)];
    let mut out = format!("{}\n", heading);
    for [name, values, default, status] in &rows {
        let line = format!(
            "  {:<w0$}  {:<w1$}  {:<w2$}  {}",
            name,
            values,
            default,
            status,
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2]
        );
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// A setting's legal values, in a word or a list.
fn values(setting: &Setting) -> String {
    let names = || {
        setting
            .choices()
            .iter()
            .map(|c| c.name)
            .collect::<Vec<_>>()
            .join(" | ")
    };
    match setting.kind {
        Kind::Bool => "true | false".to_string(),
        Kind::Integer => "integer".to_string(),
        Kind::Text => "text".to_string(),
        Kind::Size => "size, such as \"256MB\"".to_string(),
        Kind::Choice(_) => names(),
        Kind::List => "list of text".to_string(),
        Kind::Sequence(_) => format!("list of {}", names()),
        Kind::Internal => String::new(),
    }
}

#[cfg(test)]
mod tests;
