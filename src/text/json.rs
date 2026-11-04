//! JSON for mto's machine-readable outputs: a value built in code and written compact, with
//! nothing to install.
//!
//! The outputs are a contract, unlike the human views: each is an object whose first field
//! is `"version"` ([`VERSION`]). Within a version, fields are only ever added; one renamed,
//! removed or given another meaning raises the version. `--emit json` is the exception: it
//! keeps the standard formatter's own shape, so tools built for it keep working.

/// The version of every machine-readable output's shape.
pub const VERSION: u64 = 1;

/// A JSON value. Objects keep their fields in the order given.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
  Null,
  Bool(bool),
  Int(i64),
  Str(String),
  Array(Vec<Json>),
  Object(Vec<(String, Json)>),
}

impl Json {
  /// An output's top-level object: `{"version": 1, "kind": KIND}`, the fields to follow.
  pub fn output(kind: &str) -> Json {
    Json::Object(vec![
      ("version".to_string(), Json::Int(VERSION as i64)),
      ("kind".to_string(), Json::Str(kind.to_string())),
    ])
  }

  /// An empty object.
  pub fn object() -> Json {
    Json::Object(Vec::new())
  }

  /// This object with `key` set to `value` (a value that is not an object is left as is).
  pub fn with(mut self, key: &str, value: impl Into<Json>) -> Json {
    if let Json::Object(fields) = &mut self {
      fields.push((key.to_string(), value.into()));
    }
    self
  }

  /// The value written compact, on one line.
  pub fn text(&self) -> String {
    let mut out = String::new();
    self.write(&mut out);
    out
  }

  fn write(&self, out: &mut String) {
    match self {
      Json::Null => out.push_str("null"),
      Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
      Json::Int(value) => out.push_str(&value.to_string()),
      Json::Str(text) => out.push_str(&quote(text)),
      Json::Array(items) => {
        out.push('[');
        for (at, item) in items.iter().enumerate() {
          if at > 0 {
            out.push(',');
          }
          item.write(out);
        }
        out.push(']');
      }
      Json::Object(fields) => {
        out.push('{');
        for (at, (key, value)) in fields.iter().enumerate() {
          if at > 0 {
            out.push(',');
          }
          out.push_str(&quote(key));
          out.push(':');
          value.write(out);
        }
        out.push('}');
      }
    }
  }
}

impl From<bool> for Json {
  fn from(value: bool) -> Json {
    Json::Bool(value)
  }
}

impl From<i64> for Json {
  fn from(value: i64) -> Json {
    Json::Int(value)
  }
}

impl From<usize> for Json {
  fn from(value: usize) -> Json {
    Json::Int(value as i64)
  }
}

impl From<u64> for Json {
  fn from(value: u64) -> Json {
    Json::Int(value as i64)
  }
}

impl From<&str> for Json {
  fn from(value: &str) -> Json {
    Json::Str(value.to_string())
  }
}

impl From<String> for Json {
  fn from(value: String) -> Json {
    Json::Str(value)
  }
}

impl<T: Into<Json>> From<Option<T>> for Json {
  fn from(value: Option<T>) -> Json {
    value.map_or(Json::Null, Into::into)
  }
}

impl<T: Into<Json>> From<Vec<T>> for Json {
  fn from(values: Vec<T>) -> Json {
    Json::Array(values.into_iter().map(Into::into).collect())
  }
}

/// `text` as a JSON string, quotes included.
pub fn quote(text: &str) -> String {
  let mut out = String::with_capacity(text.len() + 2);
  out.push('"');
  for ch in text.chars() {
    match ch {
      '"' => out.push_str("\\\""),
      '\\' => out.push_str("\\\\"),
      '\n' => out.push_str("\\n"),
      '\r' => out.push_str("\\r"),
      '\t' => out.push_str("\\t"),
      '\u{8}' => out.push_str("\\b"),
      '\u{c}' => out.push_str("\\f"),
      c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
      c => out.push(c),
    }
  }
  out.push('"');
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_values_are_written_compact_in_field_order() {
    let value = Json::output("units")
      .with("file", "a \"b\".rs")
      .with("lines", vec![1usize, 2])
      .with("pinned", false)
      .with("caller", None::<String>)
      .with("notes", Json::object().with("tab", "\t\u{1}"));
    assert_eq!(
      value.text(),
      "{\"version\":1,\"kind\":\"units\",\"file\":\"a \\\"b\\\".rs\",\"lines\":[1,2],\
       \"pinned\":false,\"caller\":null,\"notes\":{\"tab\":\"\\t\\u0001\"}}"
    );
  }
}
