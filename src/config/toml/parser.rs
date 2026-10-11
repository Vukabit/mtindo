//! The TOML 1.0.0 grammar, parsed by recursive descent over the source text.

use super::{Array, Entry, Error, Table, TableState, Value, datetime};

/// Arrays and inline tables nested deeper than this are refused rather than risking the
/// stack.
const MAX_DEPTH: usize = 128;

type Result<T> = std::result::Result<T, Error>;

pub(super) struct Parser<'a> {
  text: &'a str,
  pos: usize,
  line_starts: Vec<usize>,
  depth: usize,
}

impl<'a> Parser<'a> {
  pub(super) fn new(text: &'a str) -> Parser<'a> {
    let mut line_starts = vec![0];
    for (idx, byte) in text.bytes().enumerate() {
      if byte == b'\n' {
        line_starts.push(idx + 1);
      }
    }
    Parser {
      text,
      pos: 0,
      line_starts,
      depth: 0,
    }
  }

  /// The whole document.
  pub(super) fn document(mut self) -> Result<Table> {
    let mut root = Table::default();
    let mut current: Vec<String> = Vec::new();
    loop {
      self.skip_ws();
      match self.peek() {
        None => return Ok(root),
        Some('#') => self.comment()?,
        Some('\n' | '\r') => self.newline()?,
        Some('[') => {
          current = self.header(&mut root)?;
          self.end_of_line()?;
        }
        Some(_) => {
          let target = resolve(&mut root, &current);
          self.keyval(target)?;
          self.end_of_line()?;
        }
      }
    }
  }

  // ---- positions and errors

  fn error<T>(&self, message: impl Into<String>) -> Result<T> {
    self.error_at(self.pos, message)
  }

  fn error_at<T>(&self, pos: usize, message: impl Into<String>) -> Result<T> {
    let (line, column) = self.location(pos);
    Err(Error {
      line,
      column,
      message: message.into(),
    })
  }

  fn location(&self, pos: usize) -> (usize, usize) {
    let line = match self.line_starts.binary_search(&pos) {
      Ok(idx) => idx,
      Err(idx) => idx - 1,
    };
    let start = self.line_starts[line];
    let column = self.text[start..pos.min(self.text.len())].chars().count() + 1;
    (line + 1, column)
  }

  // ---- characters

  fn bump(&mut self) -> Option<char> {
    let ch = self.peek()?;
    self.pos += ch.len_utf8();
    Some(ch)
  }

  fn eat(&mut self, ch: char) -> bool {
    if self.peek() == Some(ch) {
      self.pos += ch.len_utf8();
      true
    } else {
      false
    }
  }

  fn peek(&self) -> Option<char> {
    self.text[self.pos..].chars().next()
  }

  fn rest(&self) -> &'a str {
    &self.text[self.pos..]
  }

  // ---- whitespace, comments, line ends

  /// A comment through the end of its line (not including the line break).
  fn comment(&mut self) -> Result<()> {
    self.bump();
    while let Some(ch) = self.peek() {
      match ch {
        '\n' => break,
        '\r' if self.rest().starts_with("\r\n") => break,
        c if is_control(c) && c != '\t' => {
          return self.error(format!("control character {:?} in a comment", c));
        }
        _ => {
          self.bump();
        }
      }
    }
    Ok(())
  }

  /// After a key/value or a header: spaces, an optional comment, then a line end or the
  /// end of the document.
  fn end_of_line(&mut self) -> Result<()> {
    self.skip_ws();
    if self.peek() == Some('#') {
      self.comment()?;
    }
    match self.peek() {
      None => Ok(()),
      Some('\n' | '\r') => self.newline(),
      Some(c) => self.error(format!("expected the end of the line, found {:?}", c)),
    }
  }

  fn newline(&mut self) -> Result<()> {
    if self.eat('\n')
      || self.rest().starts_with("\r\n") && {
        self.pos += 2;
        true
      }
    {
      return Ok(());
    }
    self.error("a carriage return must be followed by a line feed")
  }

  fn skip_ws(&mut self) {
    while matches!(self.peek(), Some(' ' | '\t')) {
      self.pos += 1;
    }
  }

  /// Spaces, line breaks and comments, as allowed inside arrays.
  fn skip_ws_comments_newlines(&mut self) -> Result<()> {
    loop {
      self.skip_ws();
      match self.peek() {
        Some('#') => self.comment()?,
        Some('\n' | '\r') => self.newline()?,
        _ => return Ok(()),
      }
    }
  }

  // ---- keys

  /// A key: one or more simple keys separated by dots, each with where it starts.
  fn key(&mut self) -> Result<Vec<(String, usize)>> {
    let mut parts = Vec::new();
    loop {
      self.skip_ws();
      let start = self.pos;
      let part = match self.peek() {
        Some('"') => {
          if self.rest().starts_with("\"\"\"") {
            return self.error("a key cannot be a multi-line string");
          }
          self.basic_string()?
        }
        Some('\'') => {
          if self.rest().starts_with("'''") {
            return self.error("a key cannot be a multi-line string");
          }
          self.literal_string()?
        }
        Some(c) if is_bare_key_char(c) => {
          while self.peek().is_some_and(is_bare_key_char) {
            self.bump();
          }
          self.text[start..self.pos].to_string()
        }
        Some(c) => return self.error(format!("expected a key, found {:?}", c)),
        None => return self.error("expected a key, found the end of the document"),
      };
      parts.push((part, start));
      self.skip_ws();
      if !self.eat('.') {
        return Ok(parts);
      }
    }
  }

  /// `key = value`, added to `table`.
  fn keyval(&mut self, table: &mut Table) -> Result<()> {
    let keys = self.key()?;
    self.skip_ws();
    if !self.eat('=') {
      return match self.peek() {
        Some(c) => self.error(format!("expected `=` after the key, found {:?}", c)),
        None => self.error("expected `=` after the key"),
      };
    }
    self.skip_ws();
    let value = self.value()?;
    self.insert(table, &keys, value)
  }

  /// Adds `value` under a dotted key, creating the tables on the way.
  fn insert(&self, table: &mut Table, keys: &[(String, usize)], value: Value) -> Result<()> {
    let (last, path) = keys.split_last().expect("a key has at least one part");
    let mut target = table;
    for (key, pos) in path {
      if target.entry(key).is_none() {
        let entry = self.new_entry(
          key,
          *pos,
          Value::Table(Table {
            entries: Vec::new(),
            state: TableState::Dotted,
          }),
        );
        target.entries.push(entry);
      }
      let entry = target.entry_mut(key).expect("just found or added");
      target = match &mut entry.value {
        Value::Table(t) => match t.state {
          TableState::Inline => {
            return self.error_at(*pos, format!("inline table `{}` cannot be extended", key));
          }
          TableState::Header | TableState::Root => {
            return self.error_at(
              *pos,
              format!(
                "dotted keys cannot add to table `{}`, which has a [header]",
                key
              ),
            );
          }
          TableState::Implicit | TableState::Dotted => {
            t.state = TableState::Dotted;
            t
          }
        },
        _ => {
          return self.error_at(*pos, format!("`{}` is already a value, not a table", key));
        }
      };
    }
    let (key, pos) = last;
    if target.entry(key).is_some() {
      return self.error_at(*pos, format!("key `{}` is defined twice", key));
    }
    let entry = self.new_entry(key, *pos, value);
    target.entries.push(entry);
    Ok(())
  }

  fn new_entry(&self, key: &str, pos: usize, value: Value) -> Entry {
    let (line, column) = self.location(pos);
    Entry {
      key: key.to_string(),
      value,
      line,
      column,
    }
  }

  // ---- headers

  /// `[table]` or `[[array.of.tables]]`; returns the path keys then go into.
  fn header(&mut self, root: &mut Table) -> Result<Vec<String>> {
    let start = self.pos;
    self.bump();
    let array = self.eat('[');
    let keys = self.key()?;
    self.skip_ws();
    if !self.eat(']') || (array && !self.eat(']')) {
      return self.error(if array {
        "expected `]]`"
      } else {
        "expected `]`"
      });
    }
    let path: Vec<String> = keys.iter().map(|(k, _)| k.clone()).collect();
    let (last, parents) = keys.split_last().expect("a key has at least one part");
    let mut target = root;
    for (key, pos) in parents {
      if target.entry(key).is_none() {
        let entry = self.new_entry(
          key,
          *pos,
          Value::Table(Table {
            entries: Vec::new(),
            state: TableState::Implicit,
          }),
        );
        target.entries.push(entry);
      }
      let entry = target.entry_mut(key).expect("just found or added");
      target = match &mut entry.value {
        Value::Table(t) if t.state == TableState::Inline => {
          return self.error_at(*pos, format!("inline table `{}` cannot be extended", key));
        }
        Value::Table(t) => t,
        Value::Array(a) if a.of_tables => match a.items.last_mut() {
          Some(Value::Table(t)) => t,
          _ => return self.error_at(*pos, "array of tables without a table"),
        },
        _ => {
          return self.error_at(*pos, format!("`{}` is already a value, not a table", key));
        }
      };
    }
    let (key, pos) = last;
    let fresh = || Table {
      entries: Vec::new(),
      state: TableState::Header,
    };
    match target.entry_mut(key) {
      None => {
        let value = if array {
          Value::Array(Array {
            items: vec![Value::Table(fresh())],
            of_tables: true,
          })
        } else {
          Value::Table(fresh())
        };
        let entry = self.new_entry(key, *pos, value);
        target.entries.push(entry);
      }
      Some(entry) => match (&mut entry.value, array) {
        (Value::Array(a), true) if a.of_tables => a.items.push(Value::Table(fresh())),
        (Value::Table(t), false) if t.state == TableState::Implicit => {
          t.state = TableState::Header;
        }
        (Value::Table(t), false) if t.state == TableState::Dotted => {
          return self.error_at(
            start,
            format!("table `{}` was already defined by dotted keys", key),
          );
        }
        (Value::Table(_), false) => {
          return self.error_at(start, format!("table `{}` is defined twice", key));
        }
        (Value::Array(a), false) if a.of_tables => {
          return self.error_at(
            start,
            format!("`{}` is an array of tables, not a table", key),
          );
        }
        _ => {
          return self.error_at(start, format!("`{}` is already a value", key));
        }
      },
    }
    Ok(path)
  }

  // ---- values

  fn value(&mut self) -> Result<Value> {
    match self.peek() {
      Some('"') => {
        if self.rest().starts_with("\"\"\"") {
          self.multiline_basic_string().map(Value::String)
        } else {
          self.basic_string().map(Value::String)
        }
      }
      Some('\'') => {
        if self.rest().starts_with("'''") {
          self.multiline_literal_string().map(Value::String)
        } else {
          self.literal_string().map(Value::String)
        }
      }
      Some('[') => self.array(),
      Some('{') => self.inline_table(),
      Some('t') if self.rest().starts_with("true") => {
        self.pos += 4;
        self.after_scalar(Value::Boolean(true))
      }
      Some('f') if self.rest().starts_with("false") => {
        self.pos += 5;
        self.after_scalar(Value::Boolean(false))
      }
      Some(c) if c.is_ascii_digit() || matches!(c, '+' | '-' | 'i' | 'n') => {
        self.number_or_datetime()
      }
      Some(c) => self.error(format!("expected a value, found {:?}", c)),
      None => self.error("expected a value, found the end of the document"),
    }
  }

  /// A keyword or number must end at a delimiter (`true1` is not `true`).
  fn after_scalar(&self, value: Value) -> Result<Value> {
    match self.peek() {
      Some(c) if c.is_ascii_alphanumeric() || c == '_' => {
        self.error(format!("unexpected {:?} after a value", c))
      }
      _ => Ok(value),
    }
  }

  fn array(&mut self) -> Result<Value> {
    self.enter()?;
    self.bump();
    let mut items = Vec::new();
    loop {
      self.skip_ws_comments_newlines()?;
      if self.eat(']') {
        break;
      }
      items.push(self.value()?);
      self.skip_ws_comments_newlines()?;
      if self.eat(',') {
        continue;
      }
      if self.eat(']') {
        break;
      }
      return match self.peek() {
        Some(c) => self.error(format!("expected `,` or `]` in an array, found {:?}", c)),
        None => self.error("unterminated array"),
      };
    }
    self.depth -= 1;
    Ok(Value::Array(Array::new(items)))
  }

  fn inline_table(&mut self) -> Result<Value> {
    self.enter()?;
    self.bump();
    let mut table = Table {
      entries: Vec::new(),
      state: TableState::Inline,
    };
    self.skip_ws();
    if !self.eat('}') {
      loop {
        if matches!(self.peek(), Some('\n' | '\r')) {
          return self.error("an inline table must be on one line");
        }
        self.keyval(&mut table)?;
        self.skip_ws();
        if self.eat('}') {
          break;
        }
        if !self.eat(',') {
          return match self.peek() {
            Some('\n' | '\r') => self.error("an inline table must be on one line"),
            Some(c) => self.error(format!(
              "expected `,` or `}}` in an inline table, found {:?}",
              c
            )),
            None => self.error("unterminated inline table"),
          };
        }
        self.skip_ws();
        if self.peek() == Some('}') {
          return self.error("a trailing comma is not allowed in an inline table");
        }
      }
    }
    freeze(&mut table);
    self.depth -= 1;
    Ok(Value::Table(table))
  }

  fn enter(&mut self) -> Result<()> {
    self.depth += 1;
    if self.depth > MAX_DEPTH {
      return self.error(format!("values nested more than {} deep", MAX_DEPTH));
    }
    Ok(())
  }

  // ---- numbers and date-times

  fn number_or_datetime(&mut self) -> Result<Value> {
    let start = self.pos;
    let scan = |p: &mut Parser| {
      while matches!(p.peek(), Some(c) if c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '.' | ':'))
      {
        p.bump();
      }
    };
    scan(self);
    // A date and a time may be separated by a space: `1979-05-27 07:32:00`.
    let token = &self.text[start..self.pos];
    if is_date(token) && self.rest().starts_with(' ') {
      let after = &self.rest()[1..];
      if after.len() >= 2 && after.as_bytes()[..2].iter().all(u8::is_ascii_digit) {
        self.bump();
        scan(self);
      }
    }
    let token = &self.text[start..self.pos];
    let looks_like_datetime = is_date(token)
      || (token.len() >= 3
        && token.as_bytes()[..2].iter().all(u8::is_ascii_digit)
        && token.as_bytes()[2] == b':');
    if looks_like_datetime {
      return match datetime::parse(token) {
        Some(dt) => Ok(Value::Datetime(dt)),
        None => self.error_at(start, format!("invalid date-time `{}`", token)),
      };
    }
    match number(token) {
      Ok(value) => Ok(value),
      Err(message) => self.error_at(start, message),
    }
  }

  // ---- strings

  fn basic_string(&mut self) -> Result<String> {
    let start = self.pos;
    self.bump();
    let mut out = String::new();
    loop {
      match self.bump() {
        None => return self.error_at(start, "unterminated string"),
        Some('"') => return Ok(out),
        Some('\\') => self.escape(&mut out)?,
        Some('\n' | '\r') => {
          return self.error_at(
            start,
            "unterminated string: a line break before the closing `\"`",
          );
        }
        Some(c) if is_control(c) && c != '\t' => {
          return self.error(format!("control character {:?} in a string", c));
        }
        Some(c) => out.push(c),
      }
    }
  }

  /// After a backslash in a basic string.
  fn escape(&mut self, out: &mut String) -> Result<()> {
    let at = self.pos - 1;
    let ch = match self.bump() {
      Some('b') => '\u{8}',
      Some('t') => '\t',
      Some('n') => '\n',
      Some('f') => '\u{c}',
      Some('r') => '\r',
      Some('"') => '"',
      Some('\\') => '\\',
      Some(kind @ ('u' | 'U')) => {
        let len = if kind == 'u' { 4 } else { 8 };
        let hex = self.rest().get(..len).unwrap_or("");
        if hex.len() != len || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
          return self.error_at(at, format!("`\\{}` needs {} hex digits", kind, len));
        }
        self.pos += len;
        let code = u32::from_str_radix(hex, 16).expect("hex digits");
        match char::from_u32(code) {
          Some(c) => c,
          None => {
            return self.error_at(
              at,
              format!("`\\{}{}` is not a Unicode scalar value", kind, hex),
            );
          }
        }
      }
      Some(c) => return self.error_at(at, format!("unknown escape `\\{}`", c)),
      None => return self.error_at(at, "unterminated string"),
    };
    out.push(ch);
    Ok(())
  }

  fn literal_string(&mut self) -> Result<String> {
    let start = self.pos;
    self.bump();
    let mut out = String::new();
    loop {
      match self.bump() {
        None => return self.error_at(start, "unterminated string"),
        Some('\'') => return Ok(out),
        Some('\n' | '\r') => {
          return self.error_at(
            start,
            "unterminated string: a line break before the closing `'`",
          );
        }
        Some(c) if is_control(c) && c != '\t' => {
          return self.error(format!("control character {:?} in a string", c));
        }
        Some(c) => out.push(c),
      }
    }
  }

  fn multiline_basic_string(&mut self) -> Result<String> {
    let start = self.pos;
    self.pos += 3;
    self.skip_first_newline();
    let mut out = String::new();
    loop {
      if self.rest().starts_with("\"\"\"") {
        if self.closing_quotes('"', &mut out)? {
          return Ok(out);
        }
        continue;
      }
      match self.bump() {
        None => return self.error_at(start, "unterminated multi-line string"),
        Some('\\') => {
          // A backslash at the end of a line trims the line break and the
          // whitespace that follows.
          let after = self.rest();
          let blank = after
            .chars()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .count();
          let rest = &after[blank..];
          if rest.starts_with('\n') || rest.starts_with("\r\n") {
            while let Some(c) = self.peek() {
              match c {
                ' ' | '\t' => {
                  self.bump();
                }
                '\n' | '\r' => self.newline()?,
                _ => break,
              }
            }
          } else {
            self.escape(&mut out)?;
          }
        }
        Some('\r') => {
          if !self.eat('\n') {
            return self.error("a carriage return must be followed by a line feed");
          }
          out.push('\n');
        }
        Some('\n') => out.push('\n'),
        Some(c) if is_control(c) && c != '\t' => {
          return self.error(format!("control character {:?} in a string", c));
        }
        Some(c) => out.push(c),
      }
    }
  }

  fn multiline_literal_string(&mut self) -> Result<String> {
    let start = self.pos;
    self.pos += 3;
    self.skip_first_newline();
    let mut out = String::new();
    loop {
      if self.rest().starts_with("'''") {
        if self.closing_quotes('\'', &mut out)? {
          return Ok(out);
        }
        continue;
      }
      match self.bump() {
        None => return self.error_at(start, "unterminated multi-line string"),
        Some('\r') => {
          if !self.eat('\n') {
            return self.error("a carriage return must be followed by a line feed");
          }
          out.push('\n');
        }
        Some('\n') => out.push('\n'),
        Some(c) if is_control(c) && c != '\t' => {
          return self.error(format!("control character {:?} in a string", c));
        }
        Some(c) => out.push(c),
      }
    }
  }

  /// At three or more quotes: up to two of them may belong to the content before the
  /// closing three. Returns true when the string ended.
  fn closing_quotes(&mut self, quote: char, out: &mut String) -> Result<bool> {
    let run = self.rest().chars().take_while(|c| *c == quote).count();
    if run > 5 {
      return self.error(format!(
        "too many `{}` in a row in a multi-line string",
        quote
      ));
    }
    for _ in 0..run - 3 {
      out.push(quote);
    }
    self.pos += run;
    Ok(true)
  }

  fn skip_first_newline(&mut self) {
    if self.rest().starts_with('\n') {
      self.pos += 1;
    } else if self.rest().starts_with("\r\n") {
      self.pos += 2;
    }
  }
}

/// Marks a finished inline table, and the tables dotted keys made inside it, complete.
fn freeze(table: &mut Table) {
  table.state = TableState::Inline;
  for entry in table.entries.iter_mut() {
    if let Value::Table(t) = &mut entry.value {
      freeze(t);
    }
  }
}

fn is_bare_key_char(c: char) -> bool {
  c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Control characters TOML forbids in strings and comments (tab is checked separately).
fn is_control(c: char) -> bool {
  matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}')
}

/// `YYYY-MM-DD` at the start of `token`.
fn is_date(token: &str) -> bool {
  let b = token.as_bytes();
  b.len() >= 10
    && b[..4].iter().all(u8::is_ascii_digit)
    && b[4] == b'-'
    && b[5..7].iter().all(u8::is_ascii_digit)
    && b[7] == b'-'
    && b[8..10].iter().all(u8::is_ascii_digit)
}

/// An integer or float token.
fn number(token: &str) -> std::result::Result<Value, String> {
  let invalid = || format!("invalid number `{}`", token);
  let (sign, body) = match token.as_bytes().first() {
    Some(b'+') => ("", &token[1..]),
    Some(b'-') => ("-", &token[1..]),
    _ => ("", token),
  };
  match body {
    "inf" => {
      return Ok(Value::Float(if sign == "-" {
        f64::NEG_INFINITY
      } else {
        f64::INFINITY
      }));
    }
    "nan" => return Ok(Value::Float(if sign == "-" { -f64::NAN } else { f64::NAN })),
    _ => {}
  }
  for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
    if let Some(digits) = body.strip_prefix(prefix) {
      if token.len() != body.len() || !valid_digits(digits, radix) {
        return Err(invalid());
      }
      return i64::from_str_radix(&digits.replace('_', ""), radix)
        .map(Value::Integer)
        .map_err(|_| format!("`{}` does not fit in a 64-bit integer", token));
    }
  }
  // Decimal: an integer part, then optionally `.fraction` and/or `e[+-]exponent`.
  let (mantissa, exponent) = match body.find(['e', 'E']) {
    Some(at) => (&body[..at], Some(&body[at + 1..])),
    None => (body, None),
  };
  let (int_part, fraction) = match mantissa.find('.') {
    Some(at) => (&mantissa[..at], Some(&mantissa[at + 1..])),
    None => (mantissa, None),
  };
  if !valid_digits(int_part, 10) {
    return Err(invalid());
  }
  if int_part.len() > 1 && int_part.starts_with('0') {
    return Err(format!("`{}`: leading zeros are not allowed", token));
  }
  if let Some(fraction) = fraction
    && !valid_digits(fraction, 10)
  {
    return Err(invalid());
  }
  if let Some(exponent) = exponent {
    let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
    if !valid_digits(digits, 10) {
      return Err(invalid());
    }
  }
  let clean = format!("{}{}", sign, body.replace('_', ""));
  if fraction.is_none() && exponent.is_none() {
    return clean
      .parse::<i64>()
      .map(Value::Integer)
      .map_err(|_| format!("`{}` does not fit in a 64-bit integer", token));
  }
  clean
    .parse::<f64>()
    .map(Value::Float)
    .map_err(|_| invalid())
}

/// The table that `path` (a `[header]`'s keys) names, following arrays of tables to their
/// last table. The header has already created it.
fn resolve<'t>(root: &'t mut Table, path: &[String]) -> &'t mut Table {
  let mut target = root;
  for key in path {
    let entry = target.entry_mut(key).expect("headers create their path");
    target = match &mut entry.value {
      Value::Table(t) => t,
      Value::Array(a) => match a.items.last_mut() {
        Some(Value::Table(t)) => t,
        _ => unreachable!("array of tables always holds a table"),
      },
      _ => unreachable!("headers only lead through tables"),
    };
  }
  target
}

/// Digits of `radix` with single underscores between them: `1_000`, not `_1`, `1_` or `1__0`.
fn valid_digits(text: &str, radix: u32) -> bool {
  !text.is_empty()
    && !text.starts_with('_')
    && !text.ends_with('_')
    && !text.contains("__")
    && text.chars().all(|c| c == '_' || c.is_digit(radix))
}
