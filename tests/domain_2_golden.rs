use mtindo::{LayoutConfig, reorganize_file};

#[test]
fn test_golden_bodies_untouched() {
  let input = "
fn long_args(alpha: usize, beta: usize, gamma: usize, delta: usize, epsilon: usize, zeta: usize) {}
// This is a very long comment that stays exactly as written, however far past the width limit it runs.
fn a() {}
";

  let expected = "// This is a very long comment that stays exactly as written, however far past the width limit it runs.\nfn a() {}\n\nfn long_args(alpha: usize, beta: usize, gamma: usize, delta: usize, epsilon: usize, zeta: usize) {}\n";

  let config = LayoutConfig {
    max_width: 40,
    tab_width: 4,
    ..LayoutConfig::default()
  };

  let pass_1 = reorganize_file(input.to_string(), &config, false);
  assert_eq!(pass_1, expected);

  let pass_2 = reorganize_file(pass_1.clone(), &config, false);
  assert_eq!(pass_2, expected);
}

#[test]
fn test_golden_idempotency_and_visibility() {
  let input = "
fn b_private() {}
pub(crate) fn c_crate() {}
pub fn a_pub() {}
pub(super) fn d_super() {}
pub(in crate::x) fn e_in() {}
";

  // Widest first: pub, pub(crate), pub(in path), pub(super), private.
  let expected = "pub fn a_pub() {}\n\npub(crate) fn c_crate() {}\n\npub(in crate::x) fn e_in() {}\n\npub(super) fn d_super() {}\n\nfn b_private() {}\n";

  let config = LayoutConfig::default();

  let pass_1 = reorganize_file(input.to_string(), &config, false);
  assert_eq!(pass_1, expected);

  let pass_2 = reorganize_file(pass_1.clone(), &config, false);
  assert_eq!(pass_2, expected);
}

#[test]
fn test_golden_impl_separation_when_grouping_disabled() {
  let input = "
impl Alpha {}
struct Alpha {}
";

  let expected = "struct Alpha {}\n\nimpl Alpha {}\n";

  let config = LayoutConfig {
    group_impls: false,
    ..LayoutConfig::default()
  };

  let pass_1 = reorganize_file(input.to_string(), &config, false);
  assert_eq!(pass_1, expected);

  let pass_2 = reorganize_file(pass_1.clone(), &config, false);
  assert_eq!(pass_2, expected);
}

#[test]
fn test_golden_import_sorting_and_grouping() {
  let input = "
use crate::local_module;
use std::collections::HashMap;
use tokio::sync::Mutex;
use alloc::vec::Vec;
use super::parent_module;
use serde::Deserialize;
";

  let expected = "use alloc::vec::Vec;\nuse std::collections::HashMap;\n\nuse serde::Deserialize;\nuse tokio::sync::Mutex;\n\nuse super::parent_module;\nuse crate::local_module;\n";

  let config = LayoutConfig {
    sort_imports: true,
    import_grouping: true,
    ..LayoutConfig::default()
  };

  let pass_1 = reorganize_file(input.to_string(), &config, false);
  assert_eq!(pass_1, expected);

  let pass_2 = reorganize_file(pass_1.clone(), &config, false);
  assert_eq!(pass_2, expected);
}

#[test]
fn test_golden_mto_off_preservation() {
  let input = "
pub fn b() {}

// mto: off
pub fn a() {
    let x = 1;
}
// mto: on

pub fn c() {}
";

  let expected =
    "pub fn b() {}\n\n// mto: off\npub fn a() {\n    let x = 1;\n}\n// mto: on\n\npub fn c() {}\n";

  let config = LayoutConfig::default();

  let pass_1 = reorganize_file(input.to_string(), &config, false);
  assert_eq!(pass_1, expected);

  let pass_2 = reorganize_file(pass_1.clone(), &config, false);
  assert_eq!(pass_2, expected);
}

#[test]
fn test_golden_raw_strings_and_macros() {
  let input = "
macro_rules! my_macro {
    () => { let s = r#\" pub struct Fake {} \"#; }
}
struct Real {}
";

  let expected = "macro_rules! my_macro {\n    () => { let s = r#\" pub struct Fake {} \"#; }\n}\n\nstruct Real {}\n";

  let config = LayoutConfig::default();

  let pass_1 = reorganize_file(input.to_string(), &config, false);
  assert_eq!(pass_1, expected);

  let pass_2 = reorganize_file(pass_1.clone(), &config, false);
  assert_eq!(pass_2, expected);
}

#[test]
fn test_golden_sticky_impls() {
  let input = "
impl Trait for Zebra {}
struct Alpha {}
impl Alpha {}
struct Zebra {}
impl Default for Alpha {}
";

  let expected = "struct Alpha {}\n\nimpl Alpha {}\n\nimpl Default for Alpha {}\n\nstruct Zebra {}\n\nimpl Trait for Zebra {}\n";

  let config = LayoutConfig {
    group_impls: true,
    ..LayoutConfig::default()
  };

  let pass_1 = reorganize_file(input.to_string(), &config, false);
  assert_eq!(pass_1, expected);

  let pass_2 = reorganize_file(pass_1.clone(), &config, false);
  assert_eq!(pass_2, expected);
}
