use lofmt::{reorganize_file, LayoutConfig};

#[test]
fn test_golden_idempotency_and_visibility() {
    let input = "
fn b_private() {}
pub(crate) fn c_crate() {}
pub fn a_pub() {}
pub(super) fn d_super() {}
";

    let expected = "pub fn a_pub() {}\n\npub(super) fn d_super() {}\n\npub(crate) fn c_crate() {}\n\nfn b_private() {}\n";

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

    let mut config = LayoutConfig::default();
    config.group_impls = true;

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

    let mut config = LayoutConfig::default();
    config.group_impls = false;

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
fn test_golden_import_sorting_and_grouping() {
    let input = "
use crate::local_module;
use std::collections::HashMap;
use tokio::sync::Mutex;
use alloc::vec::Vec;
use super::parent_module;
use serde::Deserialize;
";

    let expected = "use alloc::vec::Vec;\nuse std::collections::HashMap;\n\nuse serde::Deserialize;\nuse tokio::sync::Mutex;\n\nuse crate::local_module;\nuse super::parent_module;\n";

    let mut config = LayoutConfig::default();
    config.sort_imports = true;
    config.import_grouping = true;

    let pass_1 = reorganize_file(input.to_string(), &config, false);
    assert_eq!(pass_1, expected);
    
    let pass_2 = reorganize_file(pass_1.clone(), &config, false);
    assert_eq!(pass_2, expected);
}

#[test]
fn test_golden_lofmt_off_preservation() {
    let input = "
pub fn b() {}

// lofmt: off
pub fn a() {
    let x = 1;
}
// lofmt: on

pub fn c() {}
";

    let expected = "pub fn b() {}\n\n// lofmt: off\npub fn a() {\n    let x = 1;\n}\n// lofmt: on\n\npub fn c() {}\n";

    let config = LayoutConfig::default();

    let pass_1 = reorganize_file(input.to_string(), &config, false);
    assert_eq!(pass_1, expected);
    
    let pass_2 = reorganize_file(pass_1.clone(), &config, false);
    assert_eq!(pass_2, expected);
}

#[test]
fn test_golden_max_width_and_prose_wrapping() {
    let input = "
fn long_args(a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) {}
// This is a very long comment that must be wrapped correctly by the engine without breaking syntax.
let x = 1;
";

    let expected = "fn long_args(\n    a: usize,\n    b: usize,\n    c: usize,\n    d: usize,\n    e: usize,\n    f: usize,\n) {}\n\n// This is a very long comment that must\n// be wrapped correctly by the engine\n// without breaking syntax.\nlet x = 1;\n";

    let mut config = LayoutConfig::default();
    config.max_width = 40;
    config.tab_width = 4;

    let pass_1 = reorganize_file(input.to_string(), &config, false);
    assert_eq!(pass_1, expected);
    
    let pass_2 = reorganize_file(pass_1.clone(), &config, false);
    assert_eq!(pass_2, expected);
}
