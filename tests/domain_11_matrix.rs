use mtindo::config::layout_from_settings;
use mtindo::{LayoutConfig, reorganize_file};

const CORPUS_ATTRIBUTES_AND_MACROS: &str = r#"
use std::collections::HashMap;
use std::sync::Arc;

#[macro_export]
macro_rules! emit_event {
    ($name:expr) => {
        println!("Event: {}", $name);
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SystemSignal {
    Startup,
    Shutdown { force: bool },
    Payload(Vec<u8>),
}

#[inline(always)]
pub unsafe fn raw_dispatch(ptr: *const u8, len: usize) -> bool {
    !ptr.is_null() && len > 0
}
"#;

// Use 5 hashes here so the inner r#" and r###" strings don't prematurely terminate the corpus payload
const CORPUS_DELIMITER_AND_RAW_STRINGS: &str = r#####"
pub static RAW_PATTERN: &str = r#"function\s+([a-zA-Z0-9_]+)\s*\("#;
pub static ESCAPED_LITERAL: &str = "line 1\nline 2\t\"quoted\"";

pub fn evaluate_regex_bounds() {
    let raw_query = r###"SELECT * FROM users WHERE meta LIKE '%"key": "val"%' "###;
    let character_constant = '\'';
    let newline_constant = '\n';
}

// mto: off
fn preserve_unorganized_b() {}
fn preserve_unorganized_a() {}
// mto: on

fn organized_tail() {}
"#####;

const CORPUS_MIXED_HIERARCHIES: &str = r#"
mod sub_module;
pub mod public_module;

use std::fmt;
use crate::engine::*;

pub trait Dispatcher {
    type Output;
    fn dispatch(&self) -> Self::Output;
}

pub(crate) struct InternalState {
    flag: bool,
}

pub(super) fn resolve_super() {}
pub fn resolve_public() {}
fn resolve_private() {}
"#;

const CORPUS_STICKY_IMPLS: &str = r#"
fn global_helper_b() {}
fn global_helper_a() {}

pub struct Alpha {
    pub value: u32,
}

impl Alpha {
    pub fn new() -> Self {
        Alpha { value: 0 }
    }
}

pub struct Beta<T> {
    pub inner: T,
}

impl<T: Clone> Clone for Beta<T> {
    fn clone(&self) -> Self {
        Beta { inner: self.inner.clone() }
    }
}

impl Alpha {
    pub fn reset(&mut self) {
        self.value = 0;
    }
}
"#;

fn generate_configuration_matrix() -> Vec<LayoutConfig> {
    let mut permutations = Vec::new();

    let sort_alpha_options = [false, true];
    let group_impls_options = [false, true];
    let sort_imports_options = [false, true];
    let import_grouping_options = [false, true];
    let blank_lines_options = [0, 1, 2];
    let max_width_options = [80, 120];
    let tab_width_options = [2, 4];

    for &sort_alpha in &sort_alpha_options {
        for &group_impls in &group_impls_options {
            for &sort_imports in &sort_imports_options {
                for &import_grouping in &import_grouping_options {
                    for &blank_lines in &blank_lines_options {
                        for &max_width in &max_width_options {
                            for &tab_width in &tab_width_options {
                                let config = layout_from_settings(&[
                                    ("sort_alphabetically", &sort_alpha.to_string()),
                                    ("group_impls", &group_impls.to_string()),
                                    ("sort_imports", &sort_imports.to_string()),
                                    ("import_grouping", &import_grouping.to_string()),
                                    ("blank_lines_between_blocks", &blank_lines.to_string()),
                                    ("max_width", &max_width.to_string()),
                                    ("tab_spaces", &tab_width.to_string()),
                                ])
                                .unwrap();

                                permutations.push(config);
                            }
                        }
                    }
                }
            }
        }
    }

    permutations
}

#[test]
fn test_corpus_matrix_permutations() {
    let corpus = [
        ("STICKY_IMPLS", CORPUS_STICKY_IMPLS),
        ("ATTRIBUTES_AND_MACROS", CORPUS_ATTRIBUTES_AND_MACROS),
        (
            "DELIMITER_AND_RAW_STRINGS",
            CORPUS_DELIMITER_AND_RAW_STRINGS,
        ),
        ("MIXED_HIERARCHIES", CORPUS_MIXED_HIERARCHIES),
    ];

    let configs = generate_configuration_matrix();

    for (name, snippet) in corpus {
        for (idx, config) in configs.iter().enumerate() {
            let first_pass = reorganize_file(snippet.to_string(), config, false);

            assert!(
                !first_pass.is_empty(),
                "Matrix failure: Empty output in corpus '{}' under permutation index {}",
                name,
                idx
            );

            let second_pass = reorganize_file(first_pass.clone(), config, false);

            assert_eq!(
                first_pass, second_pass,
                "Idempotency violation in corpus '{}' under permutation index {}",
                name, idx
            );
        }
    }
}
