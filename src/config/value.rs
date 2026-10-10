//! Checking a value against its setting's type and legal values.

use crate::config::toml;
use crate::options::{self, Kind, Setting};

/// A checked setting value. Choices hold the table's spelling, whatever case was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Integer(u64),
    Text(String),
    List(Vec<String>),
}

impl Value {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<u64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[String]> {
        match self {
            Value::List(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The value as TOML: `100`, `true`, `"Unix"`, `["a", "b"]`.
    pub fn to_toml(&self) -> String {
        match self {
            Value::Bool(b) => b.to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Text(text) => quote(text),
            Value::List(items) => {
                let quoted: Vec<String> = items.iter().map(|item| quote(item)).collect();
                format!("[{}]", quoted.join(", "))
            }
        }
    }
}

/// Checks a value from a file. `None` for [`Kind::Internal`] settings, which accept anything
/// and are not kept.
pub(super) fn check(setting: &Setting, value: &toml::Value) -> Result<Option<Value>, String> {
    let wrong_type = |expected: &str| {
        format!(
            "`{}` must be {}, not {}",
            setting.name,
            expected,
            article(value.type_name())
        )
    };
    let checked =
        match &setting.kind {
            Kind::Internal => return Ok(None),
            Kind::Bool => Value::Bool(value.as_bool().ok_or_else(|| wrong_type("true or false"))?),
            Kind::Integer => {
                let number = value
                    .as_integer()
                    .ok_or_else(|| wrong_type("a whole number"))?;
                Value::Integer(u64::try_from(number).map_err(|_| {
                    format!("`{}` must be zero or more, not {}", setting.name, number)
                })?)
            }
            Kind::Text => Value::Text(
                value
                    .as_str()
                    .ok_or_else(|| wrong_type("a string"))?
                    .to_string(),
            ),
            Kind::Choice(_) => {
                let text = value.as_str().ok_or_else(|| {
                    format!(
                        "`{}` must be one of {}, not {}",
                        setting.name,
                        legal_values(setting),
                        article(value.type_name())
                    )
                })?;
                Value::Text(choice(setting, text)?)
            }
            Kind::List | Kind::Sequence(_) => {
                let items = value
                    .as_array()
                    .ok_or_else(|| wrong_type("an array of strings"))?;
                let mut strings = Vec::new();
                for item in items {
                    let text = item
                        .as_str()
                        .ok_or_else(|| wrong_type("an array of strings"))?;
                    if matches!(setting.kind, Kind::Sequence(_)) {
                        let name = choice(setting, text)?;
                        if strings.contains(&name) {
                            return Err(format!("`{}` lists `{}` twice", setting.name, name));
                        }
                        strings.push(name);
                    } else {
                        strings.push(text.to_string());
                    }
                }
                Value::List(strings)
            }
        };
    Ok(Some(checked))
}

/// A setting's default under a style edition.
pub(super) fn default_of(setting: &Setting, style_edition: u16) -> Option<Value> {
    let text = setting.default_for(style_edition);
    if text.is_empty() {
        return None;
    }
    let table = toml::parse(&format!("v = {}", text)).ok()?;
    check(setting, table.get("v")?).ok().flatten()
}

/// Checks a value given on the command line (`--config key=value`). Booleans, numbers and
/// arrays are read as TOML; anything else is taken as a string, so `newline_style=Unix`
/// needs no quotes.
pub(super) fn from_command_line(setting: &Setting, text: &str) -> Result<Option<Value>, String> {
    let parsed = toml::parse(&format!("v = {}", text))
        .ok()
        .and_then(|table| table.get("v").cloned())
        .unwrap_or_else(|| toml::Value::String(text.to_string()));
    let parsed = match (&setting.kind, &parsed) {
        (Kind::Text | Kind::Choice(_), value) if value.as_str().is_none() => {
            toml::Value::String(text.to_string())
        }
        _ => parsed,
    };
    check(setting, &parsed)
}

/// `a, b or c`, quoted.
pub(super) fn legal_values(setting: &Setting) -> String {
    let names: Vec<String> = setting
        .choices()
        .iter()
        .map(|c| format!("`{}`", c.name))
        .collect();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {}", rest.join(", "), last),
        Some((last, _)) => last.clone(),
        None => String::new(),
    }
}

fn article(type_name: &str) -> String {
    match type_name {
        "integer" | "array" => format!("an {}", type_name),
        _ => format!("a {}", type_name),
    }
}

/// The table's spelling of `text` among the setting's choices, matched without regard to
/// case as rustfmt does.
fn choice(setting: &Setting, text: &str) -> Result<String, String> {
    setting
        .choices()
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(text))
        .map(|c| c.name.to_string())
        .ok_or_else(|| {
            if let Some(reason) = options::refusal(setting, text) {
                return reason;
            }
            format!(
                "`{}` cannot be `{}`; legal values: {}",
                setting.name,
                text,
                legal_values(setting)
            )
        })
}

fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
