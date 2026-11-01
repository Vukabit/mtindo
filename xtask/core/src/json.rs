//! JSON for what the commands print with `--json`: a value built in code and written out
//! compact, its keys in the order given, so that the same answer is the same text. Every
//! document a command prints names its schema (`"schema": "mtindo.xtask.NAME/VERSION"`),
//! whose version moves when a field changes meaning or goes.

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
  Null,
  Bool(bool),
  Number(f64),
  Text(String),
  List(Vec<Json>),
  Object(Vec<(String, Json)>),
}

impl Json {
  /// An object of `fields`, in their order.
  pub fn object(fields: Vec<(&str, Json)>) -> Json {
    Json::Object(
      fields
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
    )
  }

  /// A command's document: its schema first, then `fields`.
  pub fn document(schema: &str, fields: Vec<(&str, Json)>) -> Json {
    let mut all = vec![("schema", Json::from(schema))];
    all.extend(fields);
    Json::object(all)
  }

  /// The value as one line of JSON.
  pub fn render(&self) -> String {
    let mut out = String::new();
    self.write(&mut out);
    out
  }

  fn write(&self, out: &mut String) {
    match self {
      Json::Null => out.push_str("null"),
      Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
      // Whole numbers as integers; others as Rust writes them, which JSON reads.
      Json::Number(n) if n.fract() == 0.0 && n.abs() < 1e15 => {
        out.push_str(&format!("{}", *n as i64))
      }
      Json::Number(n) if n.is_finite() => out.push_str(&format!("{}", n)),
      Json::Number(_) => out.push_str("null"),
      Json::Text(text) => out.push_str(&string(text)),
      Json::List(items) => {
        out.push('[');
        for (i, item) in items.iter().enumerate() {
          if i > 0 {
            out.push(',');
          }
          item.write(out);
        }
        out.push(']');
      }
      Json::Object(fields) => {
        out.push('{');
        for (i, (key, value)) in fields.iter().enumerate() {
          if i > 0 {
            out.push(',');
          }
          out.push_str(&string(key));
          out.push(':');
          value.write(out);
        }
        out.push('}');
      }
    }
  }
}

impl From<&str> for Json {
  fn from(text: &str) -> Json {
    Json::Text(text.to_string())
  }
}

impl From<String> for Json {
  fn from(text: String) -> Json {
    Json::Text(text)
  }
}

impl From<&String> for Json {
  fn from(text: &String) -> Json {
    Json::Text(text.clone())
  }
}

impl From<bool> for Json {
  fn from(b: bool) -> Json {
    Json::Bool(b)
  }
}

impl From<usize> for Json {
  fn from(n: usize) -> Json {
    Json::Number(n as f64)
  }
}

impl From<u64> for Json {
  fn from(n: u64) -> Json {
    Json::Number(n as f64)
  }
}

impl From<f64> for Json {
  fn from(n: f64) -> Json {
    Json::Number(n)
  }
}

impl<T: Into<Json>> From<Option<T>> for Json {
  fn from(value: Option<T>) -> Json {
    value.map_or(Json::Null, Into::into)
  }
}

impl<T: Into<Json>> From<Vec<T>> for Json {
  fn from(items: Vec<T>) -> Json {
    Json::List(items.into_iter().map(Into::into).collect())
  }
}

/// `text` as a JSON string, quoted and escaped.
pub fn string(text: &str) -> String {
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_values_render_as_json() {
    let doc = Json::document(
      "mtindo.xtask.test/1",
      vec![
        ("name", "a \"b\"\n".into()),
        ("count", 3usize.into()),
        ("share", 0.5f64.into()),
        ("none", Option::<usize>::None.into()),
        ("list", vec!["x", "y"].into()),
        ("ok", true.into()),
      ],
    );
    assert_eq!(
      doc.render(),
      r#"{"schema":"mtindo.xtask.test/1","name":"a \"b\"\n","count":3,"share":0.5,"none":null,"list":["x","y"],"ok":true}"#
    );
    assert_eq!(Json::Number(f64::NAN).render(), "null");
  }
}
