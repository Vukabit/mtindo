//! A TOML 1.0.0 parser with no dependencies, for `mto.toml`, `rustfmt.toml`, the per-user
//! `config.toml` and `Cargo.toml`.
//!
//! [`parse`] turns a document into a [`Table`]; every key remembers the line and column it
//! was written at, so configuration errors can point at it. Errors also carry a line and
//! column ([`Error`]). The whole of TOML 1.0.0 is accepted, and everything it forbids is
//! rejected: tables defined twice, keys redefined, inline tables or arrays extended after
//! the fact, control characters, out-of-range numbers and dates. The shared `toml-test`
//! suite checks this (`cargo xtask toml-test`).
//!
//! Parsing never panics on any input; the `toml` fuzz target checks it.

mod datetime;
mod parser;

pub use datetime::{Date, Datetime, Offset, Time};

/// Where a table came from. The rules about extending tables depend on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TableState {
  /// The document root.
  #[default]
  Root,
  /// Defined by a `[header]`.
  Header,
  /// Created as the parent of a `[deeper.header]`; may still get its own header once.
  Implicit,
  /// Created by a dotted key (`a.b = 1`); may never get a header.
  Dotted,
  /// Written inline (`{ ... }`); complete as written.
  Inline,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
  String(String),
  Integer(i64),
  Float(f64),
  Boolean(bool),
  Datetime(Datetime),
  Array(Array),
  Table(Table),
}

impl Value {
  pub fn as_array(&self) -> Option<&[Value]> {
    match self {
      Value::Array(array) => Some(&array.items),
      _ => None,
    }
  }

  pub fn as_bool(&self) -> Option<bool> {
    match self {
      Value::Boolean(b) => Some(*b),
      _ => None,
    }
  }

  pub fn as_integer(&self) -> Option<i64> {
    match self {
      Value::Integer(i) => Some(*i),
      _ => None,
    }
  }

  pub fn as_str(&self) -> Option<&str> {
    match self {
      Value::String(s) => Some(s),
      _ => None,
    }
  }

  pub fn as_table(&self) -> Option<&Table> {
    match self {
      Value::Table(t) => Some(t),
      _ => None,
    }
  }

  /// The name of the value's type, for error messages: `string`, `integer`...
  pub fn type_name(&self) -> &'static str {
    match self {
      Value::String(_) => "string",
      Value::Integer(_) => "integer",
      Value::Float(_) => "float",
      Value::Boolean(_) => "boolean",
      Value::Datetime(_) => "date-time",
      Value::Array(_) => "array",
      Value::Table(_) => "table",
    }
  }
}

/// An array, and whether it was written as `[[header]]` tables (which later headers may
/// extend) or as a value (which nothing may extend).
#[derive(Clone, Debug, PartialEq)]
pub struct Array {
  pub items: Vec<Value>,
  of_tables: bool,
}

impl Array {
  pub fn new(items: Vec<Value>) -> Array {
    Array {
      items,
      of_tables: false,
    }
  }
}

/// A key and its value, with where the key was written.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
  pub key: String,
  pub value: Value,
  pub line: usize,
  pub column: usize,
}

/// A parse error, at a 1-based line and column (counted in characters).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
  pub line: usize,
  pub column: usize,
  pub message: String,
}

impl std::fmt::Display for Error {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "line {}, column {}: {}",
      self.line, self.column, self.message
    )
  }
}

/// A table: its entries in the order they were written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
  entries: Vec<Entry>,
  /// How the table came to exist, which decides what may still add to it.
  state: TableState,
}

impl Table {
  pub fn entries(&self) -> &[Entry] {
    &self.entries
  }

  pub fn get(&self, key: &str) -> Option<&Value> {
    self.entry(key).map(|e| &e.value)
  }

  pub fn entry(&self, key: &str) -> Option<&Entry> {
    self.entries.iter().find(|e| e.key == key)
  }

  pub fn is_empty(&self) -> bool {
    self.entries.is_empty()
  }

  pub fn len(&self) -> usize {
    self.entries.len()
  }

  fn entry_mut(&mut self, key: &str) -> Option<&mut Entry> {
    self.entries.iter_mut().find(|e| e.key == key)
  }
}

/// Parses a TOML document.
pub fn parse(text: &str) -> Result<Table, Error> {
  parser::Parser::new(text).document()
}

#[cfg(test)]
mod tests;
