//! Templates: text with `{expression | format:arg}` pieces (SPEC.md section 4).

use crate::expr::{Expr, Scope, Value, now_secs, parse_time};

pub const FORMATS: &[&str] = &[
  "num", "int", "short", "pct", "bytes", "duration", "ago", "time", "date", "spark", "bar", "pad",
  "rpad", "trim", "upper", "lower", "or", "quote",
];

#[derive(Clone, Debug, PartialEq)]
enum Piece {
  Text(String),
  Field(Expr, Vec<(String, Vec<String>)>),
}

/// A parsed template.
#[derive(Clone, Debug, PartialEq)]
pub struct Template(Vec<Piece>);

impl Template {
  /// The field a template shows when it is that field alone (`{took | duration}`):
  /// what a column of it sorts by.
  pub fn sole_field(&self) -> Option<&str> {
    match self.0.as_slice() {
      [Piece::Field(Expr::Name(name), _)] => Some(name),
      _ => None,
    }
  }

  pub fn parse(text: &str) -> Result<Template, String> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
      match c {
        '{' if chars.peek() == Some(&'{') => {
          chars.next();
          literal.push('{');
        }
        '}' if chars.peek() == Some(&'}') => {
          chars.next();
          literal.push('}');
        }
        '{' => {
          let mut inner = String::new();
          let mut depth = 0;
          let mut in_string = false;
          loop {
            let Some(c) = chars.next() else {
              return Err(format!("a `{{` is not closed in `{}`", text));
            };
            match c {
              '"' => in_string = !in_string,
              '(' | '[' if !in_string => depth += 1,
              ')' | ']' if !in_string => depth -= 1,
              '}' if !in_string && depth == 0 => break,
              _ => {}
            }
            inner.push(c);
          }
          if !literal.is_empty() {
            pieces.push(Piece::Text(std::mem::take(&mut literal)));
          }
          pieces.push(field(&inner).map_err(|e| format!("{} in `{}`", e, text))?);
        }
        '}' => return Err(format!("a lone `}}` in `{}` (write `}}}}`)", text)),
        c => literal.push(c),
      }
    }
    if !literal.is_empty() {
      pieces.push(Piece::Text(literal));
    }
    Ok(Template(pieces))
  }

  pub fn render(&self, scope: &Scope) -> String {
    let mut out = String::new();
    for piece in &self.0 {
      match piece {
        Piece::Text(t) => out.push_str(t),
        Piece::Field(expr, formats) => {
          let mut value = expr.eval(scope);
          for (name, args) in formats {
            value = format_value(&value, name, args);
          }
          out.push_str(&value.text());
        }
      }
    }
    out
  }
}

pub fn bar(value: f64, max: f64, width: usize) -> String {
  const EIGHTHS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
  let filled = (value / max.max(f64::EPSILON)).clamp(0.0, 1.0) * width as f64;
  let full = filled.floor() as usize;
  let mut out = "█".repeat(full);
  if full < width {
    let part = ((filled - full as f64) * 8.0).floor() as usize;
    if part > 0 {
      out.push(EIGHTHS[part]);
    }
  }
  out
}

pub fn bytes(x: f64) -> String {
  if x >= 1e9 {
    format!("{:.2} GB", x / 1e9)
  } else if x >= 1e6 {
    format!("{:.0} MB", x / 1e6)
  } else if x >= 1e3 {
    format!("{:.0} KB", x / 1e3)
  } else {
    format!("{} B", x)
  }
}

pub fn duration(seconds: f64) -> String {
  if seconds < 0.0 || !seconds.is_finite() {
    return "-".to_string();
  }
  if seconds < 10.0 {
    return format!("{:.1}s", seconds);
  }
  let s = seconds.round() as u64;
  match (s / 86400, s / 3600 % 24, s / 60 % 60, s % 60) {
    (0, 0, 0, sec) => format!("{}s", sec),
    (0, 0, m, sec) => format!("{}m{:02}s", m, sec),
    (0, h, m, _) => format!("{}h{:02}m", h, m),
    (d, h, _, _) => format!("{}d{}h", d, h),
  }
}

/// Applies format `name` with `args` to a value.
pub fn format_value(value: &Value, name: &str, args: &[String]) -> Value {
  let n = |i: usize, default: usize| -> usize {
    args.get(i).and_then(|a| a.parse().ok()).unwrap_or(default)
  };
  let text = value.text();
  let number = value.num();
  let s = |t: String| Value::Str(t);
  match name {
    "num" => number.map_or(Value::Null, |x| s(format!("{:.*}", n(0, 2), x))),
    "int" => number.map_or(Value::Null, |x| s(thousands(x.round() as i64))),
    "short" => number.map_or(Value::Null, |x| s(short(x))),
    "pct" => number.map_or(Value::Null, |x| s(format!("{:.*}%", n(0, 1), x))),
    "bytes" => number.map_or(Value::Null, |x| s(bytes(x))),
    "duration" => number.map_or(Value::Null, |x| s(duration(x))),
    "ago" => parse_time(&text).map_or(Value::Null, |t| {
      s(format!("{} ago", duration(now_secs() - t)))
    }),
    "time" => s(text.get(11..19).unwrap_or("").to_string()),
    "date" => s(text.get(..10).unwrap_or("").to_string()),
    "spark" => {
      let values: Vec<f64> = match value {
        Value::List(l) => l.iter().filter_map(Value::num).collect(),
        other => other.num().into_iter().collect(),
      };
      let width = n(0, values.len().max(1));
      s(spark(&values[values.len().saturating_sub(width)..]))
    }
    "bar" => {
      let width = n(0, 10);
      let max = args
        .get(1)
        .and_then(|a| a.parse::<f64>().ok())
        .unwrap_or(100.0);
      s(bar(number.unwrap_or(0.0), max, width))
    }
    "pad" => s(pad(&text, n(0, 0), true)),
    "rpad" => s(pad(&text, n(0, 0), false)),
    "trim" => s(trim(&text, n(0, 20))),
    "upper" => s(text.to_uppercase()),
    // One word for a command line, whatever it holds: 'it'"'"'s'.
    "quote" => s(format!("'{}'", text.replace('\'', "'\"'\"'"))),
    "lower" => s(text.to_lowercase()),
    "or" => {
      if value.is_empty() {
        s(args.join(":"))
      } else {
        value.clone()
      }
    }
    _ => value.clone(),
  }
}

/// Now, as `2026-10-11T15:17:30Z`.
pub fn now_stamp() -> String {
  utc(crate::expr::now_secs() as i64)
}

/// Padded with spaces to `width` columns as drawn, at the right (`left` true) or the left.
pub fn pad(text: &str, width: usize, left: bool) -> String {
  let fill = " ".repeat(width.saturating_sub(unicode_width::UnicodeWidthStr::width(text)));
  if left {
    format!("{}{}", text, fill)
  } else {
    format!("{}{}", fill, text)
  }
}

pub fn short(x: f64) -> String {
  let a = x.abs();
  if a >= 1e9 {
    format!("{:.2}G", x / 1e9)
  } else if a >= 1e6 {
    format!("{:.2}M", x / 1e6)
  } else if a >= 1e3 {
    format!("{:.1}k", x / 1e3)
  } else if x.fract() == 0.0 {
    format!("{}", x)
  } else {
    format!("{:.2}", x)
  }
}

pub fn spark(values: &[f64]) -> String {
  const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
  let (min, max) = values
    .iter()
    .fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
  values
    .iter()
    .map(|v| {
      if (max - min).abs() < f64::EPSILON {
        BARS[3]
      } else {
        BARS[((v - min) / (max - min) * 7.0).round() as usize]
      }
    })
    .collect()
}

pub fn thousands(n: i64) -> String {
  let digits = n.unsigned_abs().to_string();
  let mut out = String::new();
  for (i, c) in digits.chars().enumerate() {
    if i > 0 && (digits.len() - i).is_multiple_of(3) {
      out.push(',');
    }
    out.push(c);
  }
  if n < 0 { format!("-{}", out) } else { out }
}

/// At most `width` columns, as drawn (a wide character takes two), ending in `…` when cut.
pub fn trim(text: &str, width: usize) -> String {
  crate::widgets::fit(text, width)
}

/// Seconds since the epoch as `2026-10-11T15:17:30Z`.
pub fn utc(secs: i64) -> String {
  let days = secs.div_euclid(86_400);
  let rem = secs.rem_euclid(86_400);
  // Civil from days (Howard Hinnant's algorithm).
  let z = days + 719_468;
  let era = z.div_euclid(146_097);
  let doe = z - era * 146_097;
  let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let day = doy - (153 * mp + 2) / 5 + 1;
  let month = if mp < 10 { mp + 3 } else { mp - 9 };
  let year = yoe + era * 400 + i64::from(month <= 2);
  format!(
    "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
    year,
    month,
    day,
    rem / 3600,
    rem / 60 % 60,
    rem % 60
  )
}

/// One `expression | format:arg | ...` piece. `|` inside a string or brackets belongs to
/// the expression.
fn field(inner: &str) -> Result<Piece, String> {
  let mut parts = Vec::new();
  let mut current = String::new();
  let (mut depth, mut in_string) = (0, false);
  for c in inner.chars() {
    match c {
      '"' => in_string = !in_string,
      '(' | '[' if !in_string => depth += 1,
      ')' | ']' if !in_string => depth -= 1,
      '|' if !in_string && depth == 0 => {
        parts.push(std::mem::take(&mut current));
        continue;
      }
      _ => {}
    }
    current.push(c);
  }
  parts.push(current);
  let expr = Expr::parse(parts[0].trim())?;
  let mut formats = Vec::new();
  for part in &parts[1..] {
    let mut words = part.trim().split(':');
    let name = words.next().unwrap_or("").trim().to_string();
    if !FORMATS.contains(&name.as_str()) {
      return Err(format!("no format `{}`", name));
    }
    formats.push((name, words.map(|w| w.to_string()).collect()));
  }
  Ok(Piece::Field(expr, formats))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::expr::Row;

  fn render(template: &str, row: &Row) -> String {
    Template::parse(template).unwrap().render(&Scope {
      row: Some(row),
      ..Scope::default()
    })
  }

  #[test]
  fn test_errors_name_the_template() {
    assert!(
      Template::parse("{a | nope}")
        .unwrap_err()
        .contains("no format `nope`")
    );
    assert!(Template::parse("{a").unwrap_err().contains("not closed"));
    assert!(Template::parse("a } b").unwrap_err().contains("lone"));
  }

  #[test]
  fn test_numbers_read_well() {
    assert_eq!(duration(90.0), "1m30s");
    assert_eq!(duration(3.25), "3.2s");
    assert_eq!(duration(7200.0), "2h00m");
    assert_eq!(short(3_200_302.0), "3.20M");
    assert_eq!(spark(&[1.0, 2.0, 3.0]), "▁▅█");
    assert_eq!(bar(50.0, 100.0, 4), "██");
    assert_eq!(bar(93.6, 100.0, 10), "█████████▎");
  }

  #[test]
  fn test_templates_render_fields_and_formats() {
    let mut row = Row::new();
    row.insert("mf".into(), Value::Num(9771.0));
    row.insert("f".into(), Value::Num(10436.0));
    row.insert("name".into(), Value::Str("rewrite_list".into()));
    row.insert("peak".into(), Value::Num(1_490_000_000.0));
    assert_eq!(render("{100 * mf / f | num:1}%", &row), "93.6%");
    assert_eq!(render("{mf | int} of {f | int}", &row), "9,771 of 10,436");
    assert_eq!(render("{name | trim:8 | pad:9}|", &row), "rewrite… |");
    assert_eq!(render("{peak | bytes}", &row), "1.49 GB");
    assert_eq!(render("{missing | or:none}", &row), "none");
    assert_eq!(render("{{literal}}", &row), "{literal}");
    row.insert("path".into(), Value::Str("a b's.txt".into()));
    let quoted = render("{path | quote}", &row);
    assert_eq!(
      crate::source::words(&format!("git add -- {}", quoted))[3],
      "a b's.txt"
    );
    assert_eq!(render("{if(mf > 1, \"a|b\", \"c\")}", &row), "a|b");
    // Wide characters count as the two columns they take.
    row.insert("jp".into(), Value::Str("日本語テキスト".into()));
    assert_eq!(render("{jp | trim:7}|", &row), "日本語…|");
    assert_eq!(
      render("{jp | trim:20 | pad:16}|", &row),
      "日本語テキスト  |"
    );
  }
}
