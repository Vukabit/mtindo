use crate::syntax::{Edition, parse};

/// Checks one input; panics on any violation.
pub fn check(data: &[u8]) {
  if let Ok(text) = std::str::from_utf8(data) {
    for edition in [
      Edition::E2015,
      Edition::E2018,
      Edition::E2021,
      Edition::E2024,
    ] {
      let tree = parse(text, edition).syntax_node();
      assert_eq!(tree.text(), text, "the tree does not reproduce the input");
    }
  }
}
