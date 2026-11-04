//! Version requirements, for `required_version` (the standard formatter's version mto
//! reproduces) and `mto_version` (mto's own): comparators separated by commas, each `=`,
//! `>`, `>=`, `<`, `<=`, `~` or `^` before a version of one to three numbers; a version
//! alone means `^`, and `*` any version, as Cargo reads them.

/// Whether `version` (`1.10.0`) meets `requirement`, or why `requirement` cannot be read.
pub fn meets(requirement: &str, version: &str) -> Result<bool, String> {
  let have = parse(version).ok_or_else(|| format!("`{}` is not a version", version))?;
  for part in requirement.split(',') {
    let part = part.trim();
    if part == "*" {
      continue;
    }
    let (op, rest) = ["<=", ">=", "=", ">", "<", "~", "^"]
      .iter()
      .find_map(|op| part.strip_prefix(op).map(|rest| (*op, rest.trim())))
      .unwrap_or(("^", part));
    let want = parse_partial(rest)
      .ok_or_else(|| format!("`{}` is not a version requirement", requirement))?;
    let given = want.1;
    let low = want.0;
    let ok = match op {
      "=" => have[..given] == low[..given],
      ">" => have > low && !(given < 3 && have[..given] == low[..given]),
      ">=" => have >= low,
      "<" => have < low,
      "<=" => have <= low || have[..given] == low[..given],
      "~" => have >= low && have[..given.min(2)] == low[..given.min(2)],
      _ => {
        // `^`: the same leftmost number that is not zero (or the same given numbers).
        let fixed = low[..given]
          .iter()
          .position(|&n| n != 0)
          .map_or(given, |at| at + 1);
        have >= low && have[..fixed] == low[..fixed]
      }
    };
    if !ok {
      return Ok(false);
    }
  }
  Ok(true)
}

fn parse(text: &str) -> Option<[u64; 3]> {
  let (numbers, given) = parse_partial(text)?;
  (given == 3).then_some(numbers)
}

/// The numbers of `text` (missing ones zero) and how many were given.
fn parse_partial(text: &str) -> Option<([u64; 3], usize)> {
  let core = text.split(['-', '+']).next()?;
  let mut numbers = [0; 3];
  let mut given = 0;
  for part in core.split('.') {
    if given == 3 {
      return None;
    }
    numbers[given] = part.parse().ok()?;
    given += 1;
  }
  Some((numbers, given))
}

#[cfg(test)]
mod tests {
  use super::meets;

  #[test]
  fn test_requirements_read_as_cargo_reads_them() {
    for (requirement, version, expected) in [
      ("1.10.0", "1.10.0", true),
      ("1.10.0", "1.10.3", true),
      ("1.10.0", "1.11.0", true),
      ("1.10.0", "2.0.0", false),
      ("1.11", "1.10.0", false),
      ("=1.10.0", "1.10.1", false),
      ("=1.10", "1.10.4", true),
      (">=1.9, <1.11", "1.10.0", true),
      (">=1.9, <1.10", "1.10.0", false),
      ("~1.10", "1.10.9", true),
      ("~1.10", "1.11.0", false),
      ("^0.1", "0.1.5", true),
      ("^0.1", "0.2.0", false),
      ("*", "0.1.0", true),
    ] {
      assert_eq!(
        meets(requirement, version),
        Ok(expected),
        "{} against {}",
        requirement,
        version
      );
    }
    assert!(meets("one", "1.0.0").is_err());
  }
}
