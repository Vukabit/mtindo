//! Values, rows and the expression language of SPEC.md section 4.

use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const FUNCTIONS: &[&str] = &[
  "len",
  "max",
  "min",
  "abs",
  "now",
  "age",
  "secs",
  "last",
  "first",
  "pluck",
  "pct_change",
  "exists",
  "lower",
  "upper",
  "if",
];

const OPS: &[&str] = &[
  "==", "!=", "<=", ">=", "<", ">", "+", "-", "*", "/", "%", "(", ")", "[", "]", ",",
];

pub type Row = BTreeMap<String, Value>;

/// A parsed expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
  Lit(Value),
  Name(String),
  List(Vec<Expr>),
  Not(Box<Expr>),
  Neg(Box<Expr>),
  Bin(&'static str, Box<Expr>, Box<Expr>),
  Call(String, Vec<Expr>),
}

impl Expr {
  pub fn parse(text: &str) -> Result<Expr, String> {
    let mut parser = Parser {
      tokens: lex(text)?,
      at: 0,
    };
    let expr = parser.or()?;
    if parser.at != parser.tokens.len() {
      return Err(format!(
        "unexpected text after the expression in `{}`",
        text
      ));
    }
    Ok(expr)
  }

  pub fn eval(&self, scope: &Scope) -> Value {
    match self {
      Expr::Lit(v) => v.clone(),
      Expr::Name(n) => scope.lookup(n),
      Expr::List(items) => Value::List(items.iter().map(|e| e.eval(scope)).collect()),
      Expr::Not(e) => Value::Bool(!e.eval(scope).truthy()),
      Expr::Neg(e) => e.eval(scope).num().map_or(Value::Null, |n| Value::Num(-n)),
      Expr::Bin(op, a, b) => binary(op, a, b, scope),
      Expr::Call(name, args) => {
        let args: Vec<Value> = args.iter().map(|a| a.eval(scope)).collect();
        call(name, &args)
      }
    }
  }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
  Num(f64),
  Str(String),
  Name(String),
  Op(&'static str),
}

/// A field's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
  Null,
  Bool(bool),
  Num(f64),
  Str(String),
  List(Vec<Value>),
  /// An array of tables in a TOML file: a list of rows.
  Rows(Vec<Row>),
}

impl Value {
  pub fn truthy(&self) -> bool {
    match self {
      Value::Null => false,
      Value::Bool(b) => *b,
      Value::Num(n) => *n != 0.0,
      Value::Str(s) => !s.is_empty(),
      Value::List(l) => !l.is_empty(),
      Value::Rows(r) => !r.is_empty(),
    }
  }

  /// The value as a number: numbers as they are, strings parsed, booleans 0 or 1.
  pub fn num(&self) -> Option<f64> {
    match self {
      Value::Num(n) => Some(*n),
      Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
      Value::Str(s) => s.trim().parse().ok(),
      _ => None,
    }
  }

  pub fn text(&self) -> String {
    self.to_string()
  }

  pub fn is_empty(&self) -> bool {
    matches!(self, Value::Null) || matches!(self, Value::Str(s) if s.is_empty())
  }
}

impl fmt::Display for Value {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Value::Null => Ok(()),
      Value::Bool(b) => write!(f, "{}", b),
      Value::Num(n) => {
        if n.fract() == 0.0 && n.abs() < 1e15 {
          write!(f, "{}", *n as i64)
        } else {
          write!(f, "{}", n)
        }
      }
      Value::Str(s) => f.write_str(s),
      Value::List(l) => {
        let items: Vec<String> = l.iter().map(Value::to_string).collect();
        f.write_str(&items.join(" "))
      }
      Value::Rows(r) => write!(f, "{} rows", r.len()),
    }
  }
}

struct Parser {
  tokens: Vec<Token>,
  at: usize,
}

impl Parser {
  fn peek(&self) -> Option<&Token> {
    self.tokens.get(self.at)
  }

  fn word(&self) -> Option<&str> {
    match self.peek() {
      Some(Token::Name(n)) => Some(n),
      Some(Token::Op(o)) => Some(o),
      _ => None,
    }
  }

  fn expect(&mut self, op: &str) -> Result<(), String> {
    if self.word() == Some(op) {
      self.at += 1;
      Ok(())
    } else {
      Err(format!("expected `{}`", op))
    }
  }

  fn or(&mut self) -> Result<Expr, String> {
    let mut left = self.and()?;
    while self.word() == Some("or") {
      self.at += 1;
      left = Expr::Bin("or", Box::new(left), Box::new(self.and()?));
    }
    Ok(left)
  }

  fn and(&mut self) -> Result<Expr, String> {
    let mut left = self.not()?;
    while self.word() == Some("and") {
      self.at += 1;
      left = Expr::Bin("and", Box::new(left), Box::new(self.not()?));
    }
    Ok(left)
  }

  fn not(&mut self) -> Result<Expr, String> {
    if self.word() == Some("not") {
      self.at += 1;
      return Ok(Expr::Not(Box::new(self.not()?)));
    }
    self.compare()
  }

  fn compare(&mut self) -> Result<Expr, String> {
    let left = self.sum()?;
    const COMPARE: &[&str] = &[
      "==",
      "!=",
      "<=",
      ">=",
      "<",
      ">",
      "in",
      "contains",
      "starts_with",
    ];
    if let Some(op) = self.word().and_then(|w| COMPARE.iter().find(|c| **c == w)) {
      self.at += 1;
      return Ok(Expr::Bin(op, Box::new(left), Box::new(self.sum()?)));
    }
    Ok(left)
  }

  fn sum(&mut self) -> Result<Expr, String> {
    let mut left = self.product()?;
    while let Some(op) = self
      .word()
      .and_then(|w| ["+", "-"].into_iter().find(|o| *o == w))
    {
      self.at += 1;
      left = Expr::Bin(op, Box::new(left), Box::new(self.product()?));
    }
    Ok(left)
  }

  fn product(&mut self) -> Result<Expr, String> {
    let mut left = self.unary()?;
    while let Some(op) = self
      .word()
      .and_then(|w| ["*", "/", "%"].into_iter().find(|o| *o == w))
    {
      self.at += 1;
      left = Expr::Bin(op, Box::new(left), Box::new(self.unary()?));
    }
    Ok(left)
  }

  fn unary(&mut self) -> Result<Expr, String> {
    if self.word() == Some("-") {
      self.at += 1;
      return Ok(Expr::Neg(Box::new(self.unary()?)));
    }
    self.atom()
  }

  fn atom(&mut self) -> Result<Expr, String> {
    let token = self.peek().cloned().ok_or("the expression ends early")?;
    self.at += 1;
    match token {
      Token::Num(n) => Ok(Expr::Lit(Value::Num(n))),
      Token::Str(s) => Ok(Expr::Lit(Value::Str(s))),
      Token::Op("(") => {
        let inner = self.or()?;
        self.expect(")")?;
        Ok(inner)
      }
      Token::Op("[") => {
        let mut items = Vec::new();
        while self.word() != Some("]") {
          items.push(self.or()?);
          if self.word() == Some(",") {
            self.at += 1;
          }
        }
        self.expect("]")?;
        Ok(Expr::List(items))
      }
      Token::Name(name) => match name.as_str() {
        "true" => Ok(Expr::Lit(Value::Bool(true))),
        "false" => Ok(Expr::Lit(Value::Bool(false))),
        "null" => Ok(Expr::Lit(Value::Null)),
        _ if self.word() == Some("(") => {
          self.at += 1;
          let mut args = Vec::new();
          while self.word() != Some(")") {
            args.push(self.or()?);
            if self.word() == Some(",") {
              self.at += 1;
            }
          }
          self.expect(")")?;
          if !FUNCTIONS.contains(&name.as_str()) {
            return Err(format!("no function `{}`", name));
          }
          Ok(Expr::Call(name, args))
        }
        _ => Ok(Expr::Name(name)),
      },
      Token::Op(op) => Err(format!("unexpected `{}`", op)),
    }
  }
}

/// Where names are looked up: the row's own fields, then named scopes (`row`, `sel.<id>`,
/// `hub`, `count`).
#[derive(Default)]
pub struct Scope<'a> {
  pub row: Option<&'a Row>,
  pub page: Option<&'a Row>,
  pub selected: Option<&'a BTreeMap<String, Row>>,
  pub vars: Option<&'a Row>,
}

impl Scope<'_> {
  pub fn lookup(&self, name: &str) -> Value {
    if let Some(rest) = name.strip_prefix("row.") {
      return self
        .page
        .and_then(|r| r.get(rest))
        .cloned()
        .unwrap_or(Value::Null);
    }
    if let Some(rest) = name.strip_prefix("sel.") {
      let Some((widget, field)) = rest.split_once('.') else {
        return Value::Null;
      };
      return self
        .selected
        .and_then(|s| s.get(widget))
        .and_then(|r| r.get(field))
        .cloned()
        .unwrap_or(Value::Null);
    }
    if let Some(v) = self.row.and_then(|r| r.get(name)) {
      return v.clone();
    }
    self
      .vars
      .and_then(|v| v.get(name))
      .cloned()
      .unwrap_or(Value::Null)
  }
}

pub fn now_secs() -> f64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or(Duration::ZERO)
    .as_secs_f64()
}

/// A UTC time `2026-10-11T15:17:30.123Z` (or without the fraction) as seconds since the
/// epoch.
pub fn parse_time(stamp: &str) -> Option<f64> {
  let digits = |range: std::ops::Range<usize>| stamp.get(range)?.parse::<i64>().ok();
  let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
  let (hour, minute, second) = (digits(11..13)?, digits(14..16)?, digits(17..19)?);
  if stamp.as_bytes().get(10) != Some(&b'T') {
    return None;
  }
  let millis = digits(20..23).unwrap_or(0);
  let y = if month <= 2 { year - 1 } else { year };
  let era = y.div_euclid(400);
  let yoe = y - era * 400;
  let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
  let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
  let days = era * 146097 + doe - 719468;
  Some((days * 86400 + hour * 3600 + minute * 60 + second) as f64 + millis as f64 / 1000.0)
}

fn binary(op: &str, a: &Expr, b: &Expr, scope: &Scope) -> Value {
  match op {
    "and" => return Value::Bool(a.eval(scope).truthy() && b.eval(scope).truthy()),
    "or" => return Value::Bool(a.eval(scope).truthy() || b.eval(scope).truthy()),
    _ => {}
  }
  let (x, y) = (a.eval(scope), b.eval(scope));
  let numbers = x.num().zip(y.num());
  match op {
    "==" => Value::Bool(equal(&x, &y)),
    "!=" => Value::Bool(!equal(&x, &y)),
    "<" | "<=" | ">" | ">=" => {
      let ordering = match numbers {
        Some((p, q)) => p.partial_cmp(&q),
        None => Some(x.text().cmp(&y.text())),
      };
      Value::Bool(ordering.is_some_and(|o| match op {
        "<" => o.is_lt(),
        "<=" => o.is_le(),
        ">" => o.is_gt(),
        _ => o.is_ge(),
      }))
    }
    "in" => Value::Bool(match &y {
      Value::List(items) => items.iter().any(|i| equal(&x, i)),
      other => other.text().contains(&x.text()),
    }),
    "contains" => Value::Bool(match &x {
      Value::List(items) => items.iter().any(|i| equal(i, &y)),
      other => other.text().contains(&y.text()),
    }),
    "starts_with" => Value::Bool(x.text().starts_with(&y.text())),
    "+" => match numbers {
      Some((p, q)) => Value::Num(p + q),
      None => Value::Str(x.text() + &y.text()),
    },
    _ => match numbers {
      Some((p, q)) => Value::Num(match op {
        "-" => p - q,
        "*" => p * q,
        "/" if q != 0.0 => p / q,
        "%" if q != 0.0 => p % q,
        _ => return Value::Null,
      }),
      None => Value::Null,
    },
  }
}

fn call(name: &str, args: &[Value]) -> Value {
  let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Null);
  let numbers = |v: &Value| -> Vec<f64> {
    match v {
      Value::List(items) => items.iter().filter_map(Value::num).collect(),
      other => other.num().into_iter().collect(),
    }
  };
  match name {
    "len" => Value::Num(match arg(0) {
      Value::List(l) => l.len(),
      Value::Rows(r) => r.len(),
      Value::Str(s) => s.chars().count(),
      Value::Null => 0,
      _ => 1,
    } as f64),
    "max" | "min" => {
      let all: Vec<f64> = args.iter().flat_map(numbers).collect();
      let pick = if name == "max" {
        all.iter().copied().reduce(f64::max)
      } else {
        all.iter().copied().reduce(f64::min)
      };
      pick.map_or(Value::Null, Value::Num)
    }
    "abs" => arg(0).num().map_or(Value::Null, |n| Value::Num(n.abs())),
    "now" => Value::Num(now_secs()),
    "age" => parse_time(&arg(0).text()).map_or(Value::Null, |t| Value::Num(now_secs() - t)),
    "secs" => {
      let from = parse_time(&arg(0).text());
      let to = parse_time(&arg(1).text()).unwrap_or_else(now_secs);
      from.map_or(Value::Null, |f| Value::Num(to - f))
    }
    // A field of each row of a list of rows: `pluck(occurrence, "when")`.
    "pluck" => match arg(0) {
      Value::Rows(rows) => {
        let field = arg(1).text();
        Value::List(
          rows
            .iter()
            .map(|r| r.get(&field).cloned().unwrap_or(Value::Null))
            .collect(),
        )
      }
      _ => Value::List(Vec::new()),
    },
    "last" | "first" => match arg(0) {
      Value::List(l) => (if name == "last" { l.last() } else { l.first() })
        .cloned()
        .unwrap_or(Value::Null),
      other => other,
    },
    "pct_change" => {
      let all = numbers(&arg(0));
      match all.as_slice() {
        [.., before, last] if *before != 0.0 => Value::Num(100.0 * (last - before) / before.abs()),
        _ => Value::Null,
      }
    }
    "exists" => Value::Bool(std::path::Path::new(&arg(0).text()).exists()),
    "lower" => Value::Str(arg(0).text().to_lowercase()),
    "upper" => Value::Str(arg(0).text().to_uppercase()),
    "if" => {
      if arg(0).truthy() {
        arg(1)
      } else {
        arg(2)
      }
    }
    _ => Value::Null,
  }
}

fn equal(x: &Value, y: &Value) -> bool {
  match (x.num(), y.num()) {
    (Some(p), Some(q)) if !matches!(x, Value::Str(_)) || !matches!(y, Value::Str(_)) => p == q,
    _ => x.text() == y.text(),
  }
}

fn lex(text: &str) -> Result<Vec<Token>, String> {
  let chars: Vec<char> = text.chars().collect();
  let mut tokens = Vec::new();
  let mut i = 0;
  while i < chars.len() {
    let c = chars[i];
    if c.is_whitespace() {
      i += 1;
    } else if c.is_ascii_digit()
      || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
    {
      let start = i;
      while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
        i += 1;
      }
      let s: String = chars[start..i].iter().collect();
      tokens.push(Token::Num(
        s.parse().map_err(|_| format!("bad number `{}`", s))?,
      ));
    } else if c == '"' {
      let mut s = String::new();
      i += 1;
      loop {
        match chars.get(i) {
          None => return Err("a string is not closed".into()),
          Some('"') => break,
          Some('\\') => {
            s.push(*chars.get(i + 1).ok_or("a string is not closed")?);
            i += 2;
          }
          Some(c) => {
            s.push(*c);
            i += 1;
          }
        }
      }
      i += 1;
      tokens.push(Token::Str(s));
    } else if c.is_alphabetic() || c == '_' {
      let start = i;
      while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '.' | '-')) {
        // A dash is subtraction, except inside the widget id of `sel.<id>.<field>`
        // (widget ids like `series-list` are common; field names use underscores).
        if chars[i] == '-' {
          let so_far: String = chars[start..i].iter().collect();
          let in_widget_id = so_far.starts_with("sel.") && so_far.matches('.').count() == 1;
          if !(in_widget_id && chars.get(i + 1).is_some_and(|c| c.is_alphanumeric())) {
            break;
          }
        }
        i += 1;
      }
      tokens.push(Token::Name(chars[start..i].iter().collect()));
    } else {
      let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
      let op = OPS
        .iter()
        .find(|op| rest.starts_with(**op))
        .ok_or_else(|| format!("unexpected `{}`", c))?;
      tokens.push(Token::Op(op));
      i += op.len();
    }
  }
  Ok(tokens)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn eval(text: &str, row: &Row) -> Value {
    Expr::parse(text).unwrap().eval(&Scope {
      row: Some(row),
      ..Scope::default()
    })
  }

  fn row(pairs: &[(&str, Value)]) -> Row {
    pairs
      .iter()
      .map(|(k, v)| (k.to_string(), v.clone()))
      .collect()
  }

  #[test]
  fn test_errors_say_what() {
    assert!(Expr::parse("1 +").unwrap_err().contains("ends early"));
    assert!(Expr::parse("nope(1)").unwrap_err().contains("no function"));
    assert!(Expr::parse("\"open").unwrap_err().contains("not closed"));
    assert!(
      Expr::parse("a b")
        .unwrap_err()
        .contains("after the expression")
    );
  }

  #[test]
  fn test_functions_on_lists_and_times() {
    let r = row(&[(
      "values",
      Value::List(vec![Value::Num(10.0), Value::Num(12.5)]),
    )]);
    assert_eq!(eval("pct_change(values)", &r), Value::Num(25.0));
    assert_eq!(eval("last(values)", &r), Value::Num(12.5));
    assert_eq!(eval("len(values)", &r), Value::Num(2.0));
    let seen = row(&[(
      "occurrence",
      Value::Rows(vec![
        row(&[("when", Value::Str("first".into()))]),
        row(&[("when", Value::Str("last".into()))]),
      ]),
    )]);
    assert_eq!(
      eval("first(pluck(occurrence, \"when\"))", &seen),
      Value::Str("first".into())
    );
    assert_eq!(
      eval("last(pluck(occurrence, \"when\"))", &seen),
      Value::Str("last".into())
    );
    assert_eq!(parse_time("1970-01-01T00:00:01.500Z"), Some(1.5));
    assert_eq!(
      eval(
        "secs(\"2026-10-11T00:00:00Z\", \"2026-10-11T00:01:30Z\")",
        &r
      ),
      Value::Num(90.0)
    );
  }

  #[test]
  fn test_operators_and_precedence() {
    let r = row(&[
      ("state", Value::Str("running".into())),
      ("mf", Value::Num(3.0)),
      ("f", Value::Num(4.0)),
    ]);
    assert_eq!(eval("100 * mf / max(f, 1)", &r), Value::Num(75.0));
    assert_eq!(eval("1 + 2 * 3", &r), Value::Num(7.0));
    assert_eq!(eval("-(1 + 2) * 2", &r), Value::Num(-6.0));
    assert_eq!(
      eval("state == \"running\" and mf > 2", &r),
      Value::Bool(true)
    );
    assert_eq!(
      eval("not state in [\"passed\", \"failed\"]", &r),
      Value::Bool(true)
    );
    assert_eq!(
      eval("state starts_with \"run\" or false", &r),
      Value::Bool(true)
    );
    assert_eq!(eval("missing == null", &r), Value::Bool(true));
    assert_eq!(
      eval("if(mf > 5, \"big\", \"small\")", &r),
      Value::Str("small".into())
    );
    assert_eq!(eval("\"a\" + \"b\"", &r), Value::Str("ab".into()));
  }

  #[test]
  fn test_scopes() {
    let page = row(&[("id", Value::Str("run-1".into()))]);
    let mut selected = BTreeMap::new();
    selected.insert(
      "jobs".to_string(),
      row(&[("state", Value::Str("failed".into()))]),
    );
    let scope = Scope {
      page: Some(&page),
      selected: Some(&selected),
      ..Scope::default()
    };
    assert_eq!(
      Expr::parse("row.id").unwrap().eval(&scope),
      Value::Str("run-1".into())
    );
    assert_eq!(
      Expr::parse("sel.jobs.state == \"failed\"")
        .unwrap()
        .eval(&scope),
      Value::Bool(true)
    );
    let mut selected = BTreeMap::new();
    selected.insert("series-list".to_string(), row(&[("n", Value::Num(3.0))]));
    let scope = Scope {
      selected: Some(&selected),
      row: Some(&page),
      ..Scope::default()
    };
    assert_eq!(
      Expr::parse("sel.series-list.n - 1").unwrap().eval(&scope),
      Value::Num(2.0)
    );
    let numbers = row(&[("a", Value::Num(5.0)), ("b", Value::Num(2.0))]);
    assert_eq!(eval("a-b", &numbers), Value::Num(3.0));
  }
}
