use mtindo::{
    apply_config_value, classify_import, parse_block_metadata, ItemKind, LayoutConfig, Visibility,
};

#[test]
fn test_config_booleans() {
    let mut config = LayoutConfig::default();

    apply_config_value(&mut config, "sort_alphabetically", "false");
    assert_eq!(config.sort_alphabetically, false);
    apply_config_value(&mut config, "group_impls", "\"false\"");
    assert_eq!(config.group_impls, false);
    apply_config_value(&mut config, "sort_imports", "'false'");
    assert_eq!(config.sort_imports, false);
    apply_config_value(&mut config, "import_grouping", " true ");
    assert_eq!(config.import_grouping, true);
}

#[test]
fn test_config_integers() {
    let mut config = LayoutConfig::default();

    apply_config_value(&mut config, "max_width", "120");
    assert_eq!(config.max_width, 120);
    apply_config_value(&mut config, "tab_width", "8");
    assert_eq!(config.tab_width, 8);
    apply_config_value(&mut config, "blank_lines_between_blocks", "\"2\"");
    assert_eq!(config.blank_lines_between_blocks, 2);
    apply_config_value(&mut config, "max_width", "invalid_number");
    assert_eq!(config.max_width, 120);
}

#[test]
fn test_config_type_order_arrays() {
    let mut config = LayoutConfig::default();

    let raw_array = "[ \"StructDecl\", 'FnDecl' , ModDecl]";
    apply_config_value(&mut config, "type_order", raw_array);

    assert_eq!(config.type_order.len(), 3);
    assert_eq!(config.type_order[0], ItemKind::StructDecl);
    assert_eq!(config.type_order[1], ItemKind::FnDecl);
    assert_eq!(config.type_order[2], ItemKind::ModDecl);
}

#[test]
fn test_import_classification() {
    assert_eq!(classify_import("use std::collections::HashMap;"), mtindo::ImportGroup::Std);
    assert_eq!(classify_import("pub use core::fmt;"), mtindo::ImportGroup::Std);
    assert_eq!(classify_import("use alloc::vec::Vec;"), mtindo::ImportGroup::Std);

    assert_eq!(classify_import("use crate::internal::module;"), mtindo::ImportGroup::Local);
    assert_eq!(classify_import("pub(crate) use super::parent;"), mtindo::ImportGroup::Local);
    assert_eq!(classify_import("use self::local;"), mtindo::ImportGroup::Local);

    assert_eq!(classify_import("use serde::Deserialize;"), mtindo::ImportGroup::External);
    assert_eq!(classify_import("pub use tokio::sync::Mutex;"), mtindo::ImportGroup::External);
}

#[test]
fn test_item_kind_from_str() {
    assert_eq!(ItemKind::from_str("ModDecl"), ItemKind::ModDecl);
    assert_eq!(ItemKind::from_str("\"UseDecl\""), ItemKind::UseDecl);
    assert_eq!(ItemKind::from_str("'MacroDecl'"), ItemKind::MacroDecl);
    assert_eq!(ItemKind::from_str("  ConstDecl  "), ItemKind::ConstDecl);
    assert_eq!(ItemKind::from_str("InvalidGarbage"), ItemKind::Unknown);
}

#[test]
fn test_metadata_impl_targeting() {
    let standard_impl = parse_block_metadata("impl MyStruct {", false, 0);
    assert_eq!(standard_impl.kind, ItemKind::ImplBlock);
    assert_eq!(standard_impl.parent_target, "MyStruct");
    assert_eq!(standard_impl.identifier, "MyStruct");
    let trait_impl = parse_block_metadata("impl Default for MyStruct {", false, 0);
    assert_eq!(trait_impl.kind, ItemKind::ImplBlock);
    assert_eq!(trait_impl.parent_target, "MyStruct");
    assert_eq!(trait_impl.identifier, "Default for MyStruct");
    let generic_impl = parse_block_metadata("impl<'a> Iterator for MyStruct<'a> {", false, 0);
    assert_eq!(generic_impl.kind, ItemKind::ImplBlock);
    assert_eq!(generic_impl.parent_target, "MyStruct");
}

#[test]
fn test_metadata_macro_extraction() {
    let block = parse_block_metadata("macro_rules! my_macro {", false, 0);
    assert_eq!(block.kind, ItemKind::MacroDecl);
    assert_eq!(block.identifier, "my_macro");
}

#[test]
fn test_metadata_skip_logic() {
    let block = parse_block_metadata("struct Ignored {}", true, 5);
    assert!(block.skip);
    assert_eq!(block.kind, ItemKind::Unknown);
    assert_eq!(block.original_index, 5);
    assert_eq!(block.identifier, "SkippedBlock");
}

#[test]
fn test_metadata_variable_declarations() {
    let stat = parse_block_metadata("static GLOBAL: usize = 0;", false, 0);
    assert_eq!(stat.kind, ItemKind::StaticDecl);
    assert_eq!(stat.identifier, "GLOBAL");
    let mut_stat = parse_block_metadata("static mut UNSAFE_GLOBAL: usize = 0;", false, 0);
    assert_eq!(mut_stat.kind, ItemKind::StaticDecl);
    assert_eq!(mut_stat.identifier, "UNSAFE_GLOBAL");
    let con = parse_block_metadata("pub const MAX_SIZE: usize = 100;", false, 0);
    assert_eq!(con.kind, ItemKind::ConstDecl);
    assert_eq!(con.identifier, "MAX_SIZE");
}

#[test]
fn test_metadata_visibility_extraction() {
    let block1 = parse_block_metadata("pub struct A {}", false, 0);
    assert_eq!(block1.visibility, Visibility::Pub);
    let block2 = parse_block_metadata("pub(crate) fn b() {}", false, 0);
    assert_eq!(block2.visibility, Visibility::PubCrate);
    let block3 = parse_block_metadata("pub(super) const C: usize = 1;", false, 0);
    assert_eq!(block3.visibility, Visibility::PubSuper);
    let block4 = parse_block_metadata("struct D {}", false, 0);
    assert_eq!(block4.visibility, Visibility::Private);
}
