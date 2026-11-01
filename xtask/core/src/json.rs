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

impl Json {
  /// The value of `key`, when this is an object holding it.
  pub fn get(&self, key: &str) -> Option<&Json> {
    match self {
      Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
      _ => None,
    }
  }

  /// The text, when this is a string.
  pub fn as_str(&self) -> Option<&str> {
    match self {
      Json::Text(text) => Some(text),
      _ => None,
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

struct Reader {
  chars: Vec<char>,
  at: usize,
}

impl Reader {
  fn space(&mut self) {
    while self.chars.get(self.at).is_some_and(|c| c.is_whitespace()) {
      self.at += 1;
    }
  }

  fn expect(&mut self, word: &str) -> Result<(), String> {
    for c in word.chars() {
      if self.chars.get(self.at) != Some(&c) {
        return Err(format!("expected `{}` at {}", word, self.at));
      }
      self.at += 1;
    }
    Ok(())
  }

  fn value(&mut self) -> Result<Json, String> {
    self.space();
    match self.chars.get(self.at) {
      Some('{') => self.object(),
      Some('[') => self.list(),
      Some('"') => self.string().map(Json::Text),
      Some('t') => self.expect("true").map(|_| Json::Bool(true)),
      Some('f') => self.expect("false").map(|_| Json::Bool(false)),
      Some('n') => self.expect("null").map(|_| Json::Null),
      Some(c) if *c == '-' || c.is_ascii_digit() => self.number(),
      _ => Err(format!("expected a value at {}", self.at)),
    }
  }

  fn object(&mut self) -> Result<Json, String> {
    self.at += 1;
    let mut fields = Vec::new();
    self.space();
    if self.chars.get(self.at) == Some(&'}') {
      self.at += 1;
      return Ok(Json::Object(fields));
    }
    loop {
      self.space();
      let key = self.string()?;
      self.space();
      self.expect(":")?;
      fields.push((key, self.value()?));
      self.space();
      match self.chars.get(self.at) {
        Some(',') => self.at += 1,
        Some('}') => {
          self.at += 1;
          return Ok(Json::Object(fields));
        }
        _ => return Err(format!("expected `,` or `}}` at {}", self.at)),
      }
    }
  }

  fn list(&mut self) -> Result<Json, String> {
    self.at += 1;
    let mut items = Vec::new();
    self.space();
    if self.chars.get(self.at) == Some(&']') {
      self.at += 1;
      return Ok(Json::List(items));
    }
    loop {
      items.push(self.value()?);
      self.space();
      match self.chars.get(self.at) {
        Some(',') => self.at += 1,
        Some(']') => {
          self.at += 1;
          return Ok(Json::List(items));
        }
        _ => return Err(format!("expected `,` or `]` at {}", self.at)),
      }
    }
  }

  fn string(&mut self) -> Result<String, String> {
    self.expect("\"")?;
    let mut out = String::new();
    loop {
      let Some(&c) = self.chars.get(self.at) else {
        return Err("a string does not end".to_string());
      };
      self.at += 1;
      match c {
        '"' => return Ok(out),
        '\\' => {
          let Some(&e) = self.chars.get(self.at) else {
            return Err("a string does not end".to_string());
          };
          self.at += 1;
          match e {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'u' => {
              let hex: String = self.chars.iter().skip(self.at).take(4).collect();
              let code = u32::from_str_radix(&hex, 16).map_err(|_| "a bad \\u escape")?;
              self.at += 4;
              out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
            }
            other => out.push(other),
          }
        }
        c => out.push(c),
      }
    }
  }

  fn number(&mut self) -> Result<Json, String> {
    let start = self.at;
    while self
      .chars
      .get(self.at)
      .is_some_and(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
    {
      self.at += 1;
    }
    let text: String = self.chars[start..self.at].iter().collect();
    text
      .parse::<f64>()
      .map(Json::Number)
      .map_err(|_| format!("a bad number `{}`", text))
  }
}

/// Reads one JSON value, the whole of `text` but whitespace around it.
pub fn parse(text: &str) -> Result<Json, String> {
  let mut reader = Reader {
    chars: text.chars().collect(),
    at: 0,
  };
  let value = reader.value()?;
  reader.space();
  if reader.at < reader.chars.len() {
    return Err(format!("unexpected text at {}", reader.at));
  }
  Ok(value)
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

  #[test]
  fn test_what_is_written_is_read_back() {
    let text = r#"{"jsonrpc":"2.0","id":1,"params":{"name":"gaps","arguments":{"setting":"a \"b\"\n\u00e9"},"list":[true,null,-150,0.25]}}"#;
    let value = parse(text).unwrap();
    assert_eq!(value.render(), text.replace("\\u00e9", "é"));
    let params = value.get("params").unwrap();
    assert_eq!(params.get("name").and_then(Json::as_str), Some("gaps"));
    assert!(parse("{").is_err());
    assert!(parse("[1,]").is_err());
    assert!(parse("1 2").is_err());
  }
}
