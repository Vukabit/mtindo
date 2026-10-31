//! Layout must keep a program's meaning and settle in one run, whatever the input: when it
//! arranges a file, the layout meaning check is clean on the result (mto refuses to write a
//! file that fails it), and arranging the result again changes nothing. Inputs it declines
//! to arrange are fine; panicking on any input is not.

use crate::layout::{configured_edition, parse_any_edition};
use crate::verify::verify_layout;
use crate::{LayoutConfig, try_reorganize};

/// Checks one input; panics on any violation.
pub fn check(data: &[u8]) {
  super::on_stack(data, checked);
}

/// [`check`] on the caller's thread.
pub(super) fn checked(data: &[u8]) {
  let Ok(text) = std::str::from_utf8(data) else {
    return;
  };
  let config = LayoutConfig::default();
  let Ok(once) = try_reorganize(text, &config, false) else {
    return;
  };
  // Checked in the edition layout read it in, as mto checks it before writing.
  let (edition, _) = parse_any_edition(text, configured_edition(&config));
  let found = verify_layout(text, &once, edition);
  assert!(
    found.is_empty(),
    "meaning changed:\n{}\n---\n{}\n{:?}",
    text,
    once,
    found
  );
  let twice = try_reorganize(&once, &config, false);
  assert_eq!(
    twice.as_deref(),
    Ok(once.as_str()),
    "a second run changed it:\n{}",
    text
  );
}
