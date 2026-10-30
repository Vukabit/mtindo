//! The path patterns `flow.toml`'s sections are written in: `/`-separated from the
//! repository root, `*` for any characters within one name, `**` for any number of
//! folders. A pattern without a `/` matches at the root only.

/// Whether `path` matches `pattern`.
pub fn matches(pattern: &str, path: &str) -> bool {
  let pattern: Vec<&str> = pattern.split('/').collect();
  let path: Vec<&str> = path.split('/').collect();
  segments(&pattern, &path)
}

/// `*` within one name.
fn name_matches(pattern: &str, name: &str) -> bool {
  match pattern.split_once('*') {
    None => pattern == name,
    Some((head, tail)) => {
      let Some(after) = name.strip_prefix(head) else {
        return false;
      };
      (0..=after.len())
        .filter(|&i| after.is_char_boundary(i))
        .any(|i| name_matches(tail, &after[i..]))
    }
  }
}

fn segments(pattern: &[&str], path: &[&str]) -> bool {
  match pattern.split_first() {
    None => path.is_empty(),
    Some((&"**", rest)) => (0..=path.len()).any(|skip| segments(rest, &path[skip..])),
    Some((first, rest)) => path
      .split_first()
      .is_some_and(|(name, tail)| name_matches(first, name) && segments(rest, tail)),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_patterns_match_names_folders_and_the_root() {
    assert!(matches("src/format/**", "src/format/expr.rs"));
    assert!(matches("src/format/**", "src/format/file/tests.rs"));
    assert!(!matches("src/format/**", "src/formats.rs"));
    assert!(matches("*.md", "README.md"));
    assert!(!matches("*.md", "docs/guide.md"));
    assert!(matches("docs/**", "docs/generated/manual.md"));
    assert!(matches("src/main.rs", "src/main.rs"));
    assert!(matches(".github/**", ".github/workflows/ci.yml"));
    assert!(matches("src/*/mod.rs", "src/config/mod.rs"));
    assert!(!matches("src/*/mod.rs", "src/config/toml/mod.rs"));
  }
}
