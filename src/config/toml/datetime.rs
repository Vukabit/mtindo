//! TOML date-times (RFC 3339): offset date-times, local date-times, local dates and local
//! times. Values are validated: months 1-12, days within the month (leap years counted),
//! hours 0-23, minutes 0-59, seconds 0-60 (a leap second), offsets within a day.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offset {
  /// `Z`: UTC.
  Utc,
  /// `+hh:mm` or `-hh:mm`, in minutes.
  Minutes(i16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
  pub year: u16,
  pub month: u8,
  pub day: u8,
}

/// One of TOML's four date-time forms, by which parts are present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Datetime {
  pub date: Option<Date>,
  pub time: Option<Time>,
  pub offset: Option<Offset>,
}

impl fmt::Display for Datetime {
  /// RFC 3339 with `T` and `Z`: `1979-05-27T07:32:00.5-07:00`.
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if let Some(date) = &self.date {
      write!(f, "{:04}-{:02}-{:02}", date.year, date.month, date.day)?;
      if self.time.is_some() {
        write!(f, "T")?;
      }
    }
    if let Some(time) = &self.time {
      write!(f, "{:02}:{:02}:{:02}", time.hour, time.minute, time.second)?;
      if !time.fraction.is_empty() {
        write!(f, ".{}", time.fraction)?;
      }
    }
    match self.offset {
      Some(Offset::Utc) => write!(f, "Z"),
      Some(Offset::Minutes(m)) => {
        let sign = if m < 0 { '-' } else { '+' };
        write!(f, "{}{:02}:{:02}", sign, m.abs() / 60, m.abs() % 60)
      }
      None => Ok(()),
    }
  }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Time {
  pub hour: u8,
  pub minute: u8,
  pub second: u8,
  /// Fractional-second digits as written, without the dot (`"5"` for `.5`).
  pub fraction: String,
}

/// Parses a whole date-time candidate; `None` if it is not one, or is out of range.
pub(super) fn parse(text: &str) -> Option<Datetime> {
  let (date, rest) = match date(text) {
    Some((date, rest)) => (Some(date), rest),
    None => (None, text),
  };
  if date.is_some() && rest.is_empty() {
    return Some(Datetime {
      date,
      time: None,
      offset: None,
    });
  }
  let rest = if date.is_some() {
    let mut chars = rest.chars();
    match chars.next() {
      Some('T' | 't' | ' ') => chars.as_str(),
      _ => return None,
    }
  } else {
    rest
  };
  let (time, rest) = time(rest)?;
  let offset = if rest.is_empty() {
    None
  } else {
    // An offset needs a date: a local time cannot have one.
    date?;
    Some(offset(rest)?)
  };
  Some(Datetime {
    date,
    time: Some(time),
    offset,
  })
}

fn date(text: &str) -> Option<(Date, &str)> {
  let (year, rest) = digits(text, 4)?;
  let rest = rest.strip_prefix('-')?;
  let (month, rest) = digits(rest, 2)?;
  let rest = rest.strip_prefix('-')?;
  let (day, rest) = digits(rest, 2)?;
  let (year, month, day) = (year as u16, month as u8, day as u8);
  if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
    return None;
  }
  Some((Date { year, month, day }, rest))
}

fn days_in_month(year: u16, month: u8) -> u8 {
  match month {
    1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
    4 | 6 | 9 | 11 => 30,
    _ => {
      let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
      if leap { 29 } else { 28 }
    }
  }
}

/// Exactly `count` ASCII digits at the start of `text`, as a number, and the rest.
fn digits(text: &str, count: usize) -> Option<(u32, &str)> {
  let head = text.get(..count)?;
  if !head.bytes().all(|b| b.is_ascii_digit()) {
    return None;
  }
  Some((head.parse().ok()?, &text[count..]))
}

fn offset(text: &str) -> Option<Offset> {
  if text == "Z" || text == "z" {
    return Some(Offset::Utc);
  }
  let (sign, rest) = match text.chars().next()? {
    '+' => (1, &text[1..]),
    '-' => (-1, &text[1..]),
    _ => return None,
  };
  let (hours, rest) = digits(rest, 2)?;
  let rest = rest.strip_prefix(':')?;
  let (minutes, rest) = digits(rest, 2)?;
  if !rest.is_empty() || hours > 23 || minutes > 59 {
    return None;
  }
  Some(Offset::Minutes(sign * (hours * 60 + minutes) as i16))
}

fn time(text: &str) -> Option<(Time, &str)> {
  let (hour, rest) = digits(text, 2)?;
  let rest = rest.strip_prefix(':')?;
  let (minute, rest) = digits(rest, 2)?;
  // TOML 1.0 requires seconds.
  let rest = rest.strip_prefix(':')?;
  let (second, mut rest) = digits(rest, 2)?;
  if hour > 23 || minute > 59 || second > 60 {
    return None;
  }
  let mut fraction = String::new();
  if let Some(after) = rest.strip_prefix('.') {
    let len = after.bytes().take_while(u8::is_ascii_digit).count();
    if len == 0 {
      return None;
    }
    fraction = after[..len].to_string();
    rest = &after[len..];
  }
  Some((
    Time {
      hour: hour as u8,
      minute: minute as u8,
      second: second as u8,
      fraction,
    },
    rest,
  ))
}
