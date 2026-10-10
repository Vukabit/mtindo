//! The configuration part of the manual (`mto --man`), generated from the settings table so
//! it cannot fall behind: every key, its file, type, legal values, defaults and description.

use crate::options::{self, Channels, File, Kind, Setting, Stability};

/// The widest a manual line is wrapped to.
pub const WIDTH: usize = 80;

/// How configuration is found and merged; kept beside the code in `config` that does it.
const LOOKUP: &str = "\
mto reads the settings for each folder it formats from, lowest precedence first:

1. The defaults below, for the style edition in effect.
2. The nearest `.rustfmt.toml` or `rustfmt.toml`, found as rustfmt finds it: in \
the folder, then each parent folder, then the home folder, then the `rustfmt` \
folder of the user's configuration folder. Only that one file is used.
3. Every `.mto.toml` or `mto.toml` from the outermost folder down to the folder \
itself; a nearer file wins key by key. The search upwards stops at a file that \
sets `root = true`. An `mto.toml` may also set rustfmt's settings, and wins over \
`rustfmt.toml`, with a warning when the two disagree.
4. The edition of the package in the nearest `Cargo.toml` (inherited from the \
workspace when it says `edition.workspace = true`), as `cargo fmt` passes it.
5. The command line: `--config key=value`, `--edition` and `--style-edition`.

`--config-path FILE` replaces steps 2 and 3 with that one file, which may hold \
both mto's and rustfmt's settings. `mto config path` lists the files in effect \
and `mto config print` shows every value with where it came from.

The style edition is resolved as rustfmt resolves it: `style_edition` if set, \
else `version` (`One` is 2015, `Two` is 2024), else `edition`, else 2015. It \
decides the defaults of the settings that change between style editions.

An unknown key, or a key in the wrong file, is a warning and is ignored. A value \
of the wrong type, or outside the legal values, is an error (exit code 2) naming \
the file, line and column. Values are matched without regard to case, as \
rustfmt matches them.";

/// The per-user file.
const USER: &str = "\
The user configuration holds how mto behaves for you, never layout or \
formatting. It is `$MTINDO_CONFIG` when set (the file must exist), else \
`$XDG_CONFIG_HOME/mtindo/config.toml`, else `~/.config/mtindo/config.toml` on \
macOS and Linux and `%APPDATA%\\mtindo\\config.toml` on Windows. Project files \
cannot set user settings, and the user configuration cannot set project ones.";

/// The configuration chapter of the manual, in the manual's Markdown.
pub fn configuration() -> String {
    let mut out = String::from("## Configuration\n");
    for paragraph in LOOKUP.split("\n\n") {
        out.push_str(&wrap_block(paragraph));
        out.push('\n');
    }
    out.push_str("### mto's settings\n");
    out.push_str(&wrap(
        "These go in `mto.toml` or `.mto.toml` only; rustfmt would not understand them.",
        "",
    ));
    for setting in options::all().filter(|s| s.file == File::Mto) {
        out.push_str(&entry(setting));
    }
    out.push_str("### rustfmt's settings\n");
    out.push_str(&wrap(
        &format!(
            "These go in `rustfmt.toml`, `.rustfmt.toml`, `mto.toml` or `.mto.toml`, \
             as rustfmt {} documents them. mto reads and checks them all, on every \
             channel, so an existing `rustfmt.toml` works unchanged; rustfmt's own \
             stability is noted for reference. Until mto formats code, only \
             `edition`, `style_edition` and `version` change what it does.",
            options::RUSTFMT_VERSION
        ),
        "",
    ));
    for setting in options::all().filter(|s| s.file == File::Format && !s.hidden) {
        out.push_str(&entry(setting));
    }
    out.push_str("### Hidden rustfmt settings\n");
    out.push_str(&wrap(
        "rustfmt leaves these out of its own listings: they are deprecated, or set \
         by rustfmt itself. mto accepts them so that existing files keep working.",
        "",
    ));
    for setting in options::all().filter(|s| s.file == File::Format && s.hidden) {
        out.push_str(&entry(setting));
    }
    out.push_str("### User settings\n");
    out.push_str(&wrap(USER, ""));
    let mut any = false;
    for setting in options::all().filter(|s| s.file == File::User) {
        out.push_str(&entry(setting));
        any = true;
    }
    if !any {
        out.push_str(&wrap(
            "There are no user settings yet, so mto does not create the file.",
            "",
        ));
    }
    out
}

/// `text` wrapped to [`WIDTH`], the first line after `lead` and the others indented to
/// match it.
pub fn wrap(text: &str, lead: &str) -> String {
    let indent = " ".repeat(lead.chars().count());
    let mut out = String::new();
    let mut line = lead.to_string();
    let mut empty = true;
    for word in unbreakable_words(text) {
        let word = word.as_str();
        if !empty && line.chars().count() + 1 + word.chars().count() > WIDTH {
            out.push_str(line.trim_end());
            out.push('\n');
            line = indent.clone();
            empty = true;
        }
        if !empty {
            line.push(' ');
        }
        line.push_str(word);
        empty = false;
    }
    out.push_str(line.trim_end());
    out.push('\n');
    out
}

fn default_line(setting: &Setting) -> String {
    if setting.default.is_empty() {
        return "Default: set by rustfmt itself; mto ignores the value.".to_string();
    }
    match setting.default_2024 {
        Some(later) => format!(
            "Default: `{}`; `{}` from style edition 2024.",
            setting.default, later
        ),
        None => format!("Default: `{}`.", setting.default),
    }
}

/// One setting: heading, description, then its facts as a list.
fn entry(setting: &Setting) -> String {
    let mut out = format!("#### `{}`\n", setting.name);
    for paragraph in setting.doc.split("\n\n") {
        for line in paragraph.lines() {
            out.push_str(&wrap_block(line));
        }
        out.push('\n');
    }
    out.push_str(&wrap(&format!("Type: {}.", type_name(setting)), "*   "));
    out.push_str(&wrap(&default_line(setting), "*   "));
    if let Some(channels) = setting.channels {
        out.push_str(&wrap(&format!("rustfmt: {}.", stability(channels)), "*   "));
    }
    if let Some(replacement) = setting.replaced_by {
        out.push_str(&wrap(
            &format!("Deprecated: use `{}`.", replacement),
            "*   ",
        ));
    }
    let choices = setting.choices();
    if !choices.is_empty() {
        out.push_str("*   Values:\n");
        for choice in choices {
            let mut text = format!("`{}`", choice.name);
            if !choice.doc.is_empty() {
                text.push_str(&format!(": {}", choice.doc));
            }
            if let Some(channels) = choice.channels
                && setting.channels.is_some_and(|c| c != channels)
            {
                text.push_str(&format!(" (rustfmt: {})", stability(channels)));
            }
            out.push_str(&wrap(&text, "    *   "));
        }
    }
    out
}

fn stability(channels: Channels) -> &'static str {
    match (channels.stable, channels.nightly) {
        (Stability::Stable, _) => "stable",
        (Stability::Unstable, _) => "unstable, nightly only",
        (Stability::Absent, Stability::Absent) => "not in the pinned releases",
        (Stability::Absent, _) => "nightly releases only",
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

/// The words of `text`, with a backticked span that holds spaces kept as one word. A
/// backtick with no partner is an ordinary character, and a span longer than half a line
/// may break.
fn unbreakable_words(text: &str) -> Vec<String> {
    let raw: Vec<&str> = text.split_whitespace().collect();
    let ticks = |word: &str| word.matches('`').count() % 2 == 1;
    let mut words = Vec::new();
    let mut idx = 0;
    while idx < raw.len() {
        let mut word = raw[idx].to_string();
        let close = (idx + 1..raw.len()).find(|&j| ticks(raw[j]));
        // A span too long for a line of its own still breaks.
        let fits =
            |close: usize| raw[idx..=close].iter().map(|w| w.len() + 1).sum::<usize>() <= WIDTH / 2;
        if ticks(raw[idx])
            && let Some(close) = close.filter(|&c| fits(c))
        {
            for part in &raw[idx + 1..=close] {
                word.push(' ');
                word.push_str(part);
            }
            idx = close;
        }
        words.push(word);
        idx += 1;
    }
    words
}

/// A paragraph, or a numbered list item, wrapped and followed by its line break.
fn wrap_block(paragraph: &str) -> String {
    let mut out = String::new();
    for item in paragraph.split('\n') {
        match item.split_once(". ") {
            Some((number, rest)) if number.chars().all(|c| c.is_ascii_digit()) => {
                let lead = format!("{}. ", number);
                let indent = " ".repeat(lead.len());
                let wrapped = wrap(rest, &indent);
                out.push_str(&lead);
                out.push_str(&wrapped[indent.len()..]);
            }
            _ => match item.strip_prefix("- ") {
                Some(rest) => out.push_str(&wrap(rest, "*   ")),
                None => out.push_str(&wrap(item, "")),
            },
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_every_key_and_value_is_documented() {
        let manual = configuration();
        for setting in options::all() {
            let heading = format!("#### `{}`\n", setting.name);
            let at = manual
                .find(&heading)
                .unwrap_or_else(|| panic!("{} has no entry", setting.name));
            let rest = &manual[at + heading.len()..];
            let body = &rest[..rest.find("\n#").unwrap_or(rest.len())];
            for choice in setting.choices() {
                assert!(
                    body.contains(&format!("`{}`", choice.name)),
                    "{} = {} is not documented",
                    setting.name,
                    choice.name
                );
            }
            assert!(body.contains("Default: "), "{}", setting.name);
        }
    }

    #[test]
    fn test_lines_fit_the_width() {
        for line in configuration().lines() {
            // A single word longer than the width cannot be broken.
            let words = line.split_whitespace().count();
            assert!(line.chars().count() <= WIDTH || words <= 2, "{}", line);
        }
    }

    #[test]
    fn test_wrap() {
        let text = "one two three";
        assert_eq!(wrap(text, "*   "), "*   one two three\n");
        let long = "word ".repeat(30);
        let wrapped = wrap(&long, "*   ");
        let lines: Vec<&str> = wrapped.lines().collect();
        assert!(lines[0].starts_with("*   word"));
        assert!(lines[1].starts_with("    word"));
        assert!(lines.iter().all(|l| l.len() <= WIDTH));
        let code = format!("{} `version = \"One\"` end", "x ".repeat(36));
        assert!(wrap(&code, "").contains("\n`version = \"One\"` end"));
        assert_eq!(
            wrap_block("1. first item\n2. second"),
            "1. first item\n2. second\n"
        );
    }
}
