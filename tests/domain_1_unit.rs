use mtindo::config::layout_from_settings;
use mtindo::layout::{Unit, layout};
use mtindo::{ItemKind, LayoutConfig, Visibility, classify_import};

#[test]
fn test_config_booleans() {
    let config = layout_from_settings(&[
        ("sort_alphabetically", "false"),
        ("group_impls", "false"),
        ("sort_imports", "false"),
        ("import_grouping", "true"),
    ])
    .unwrap();
    assert!(!config.sort_alphabetically);
    assert!(!config.group_impls);
    assert!(!config.sort_imports);
    assert!(config.import_grouping);
    // A quoted boolean is a string, which a boolean setting refuses.
    let error = layout_from_settings(&[("group_impls", "\"false\"")]).unwrap_err();
    assert_eq!(
        error.message,
        "`group_impls` must be true or false, not a string"
    );
}

#[test]
fn test_config_integers() {
    let config = layout_from_settings(&[
        ("max_width", "120"),
        ("tab_spaces", "8"),
        ("blank_lines_between_blocks", "2"),
        // The standard formatter's upper bound is 1 unless raised; mto stays within it.
        ("blank_lines_upper_bound", "2"),
    ])
    .unwrap();
    assert_eq!(config.max_width, 120);
    assert_eq!(config.tab_width, 8);
    assert_eq!(config.blank_lines_between_blocks, 2);
    let error = layout_from_settings(&[("max_width", "invalid_number")]).unwrap_err();
    assert_eq!(
        error.message,
        "`max_width` must be a whole number, not a string"
    );
    let error = layout_from_settings(&[("tab_width", "8")]).unwrap_err();
    assert_eq!(
        error.message,
        "unknown key `tab_width` (renamed `tab_spaces`)"
    );
}

#[test]
fn test_config_type_order_arrays() {
    let config =
        layout_from_settings(&[("type_order", "[ \"StructDecl\", 'FnDecl' , \"moddecl\"]")])
            .unwrap();
    assert_eq!(
        config.type_order,
        vec![ItemKind::StructDecl, ItemKind::FnDecl, ItemKind::ModDecl]
    );
}

#[test]
fn test_files_with_syntax_errors_are_left_alone() {
    let reason = layout(
        "fn b() {}
fn a( {}
",
        &LayoutConfig::default(),
    )
    .err()
    .unwrap();
    assert!(reason.contains("syntax errors"), "{}", reason);
}

#[test]
fn test_import_classification() {
    assert_eq!(
        classify_import("use std::collections::HashMap;"),
        mtindo::ImportGroup::Std
    );
    assert_eq!(
        classify_import("pub use core::fmt;"),
        mtindo::ImportGroup::Std
    );
    assert_eq!(
        classify_import("use alloc::vec::Vec;"),
        mtindo::ImportGroup::Std
    );

    assert_eq!(
        classify_import("use crate::internal::module;"),
        mtindo::ImportGroup::Local
    );
    assert_eq!(
        classify_import("pub(crate) use super::parent;"),
        mtindo::ImportGroup::Local
    );
    assert_eq!(
        classify_import("use self::local;"),
        mtindo::ImportGroup::Local
    );

    assert_eq!(
        classify_import("use serde::Deserialize;"),
        mtindo::ImportGroup::External
    );
    assert_eq!(
        classify_import("pub use tokio::sync::Mutex;"),
        mtindo::ImportGroup::External
    );
}

#[test]
fn test_item_kind_from_name() {
    assert_eq!(ItemKind::from_name("ModDecl"), ItemKind::ModDecl);
    assert_eq!(ItemKind::from_name("\"UseDecl\""), ItemKind::UseDecl);
    assert_eq!(ItemKind::from_name("'MacroDecl'"), ItemKind::MacroDecl);
    assert_eq!(ItemKind::from_name("  ConstDecl  "), ItemKind::ConstDecl);
    assert_eq!(ItemKind::from_name("InvalidGarbage"), ItemKind::Unknown);
}

#[test]
fn test_metadata_impl_targeting() {
    let standard_impl = unit("impl MyStruct {}");
    assert_eq!(standard_impl.kind, ItemKind::ImplBlock);
    assert_eq!(standard_impl.parent_target, "MyStruct");
    assert_eq!(standard_impl.identifier, "MyStruct");
    let trait_impl = unit("impl Default for MyStruct {}");
    assert_eq!(trait_impl.kind, ItemKind::ImplBlock);
    assert_eq!(trait_impl.parent_target, "MyStruct");
    assert_eq!(trait_impl.identifier, "Default for MyStruct");
    let generic_impl = unit("impl<'a> Iterator for MyStruct<'a> {}");
    assert_eq!(generic_impl.kind, ItemKind::ImplBlock);
    assert_eq!(generic_impl.parent_target, "MyStruct");
    // The old text scanner read `T>` here; the parser finds the type.
    let bounded = unit("impl<T: ?Sized + Clone> Wrapper<T> {}");
    assert_eq!(bounded.parent_target, "Wrapper");
}

#[test]
fn test_metadata_macro_extraction() {
    let block = unit("macro_rules! my_macro { () => {} }");
    assert_eq!(block.kind, ItemKind::MacroDecl);
    assert_eq!(block.identifier, "my_macro");
}

#[test]
fn test_metadata_skip_logic() {
    let found = units(
        "fn a() {}

// mto: off
struct Ignored {}
// mto: on

fn b() {}
",
    );
    assert_eq!(found.len(), 3);
    let region = &found[1];
    assert!(region.pinned);
    assert_eq!(region.kind, ItemKind::Unknown);
    assert_eq!(region.index, 1);
    assert_eq!(region.identifier, "SkippedBlock");
    assert!(region.text.starts_with("// mto: off") && region.text.ends_with("// mto: on"));
}

#[test]
fn test_metadata_variable_declarations() {
    let stat = unit("static GLOBAL: usize = 0;");
    assert_eq!(stat.kind, ItemKind::StaticDecl);
    assert_eq!(stat.identifier, "GLOBAL");
    let mut_stat = unit("static mut UNSAFE_GLOBAL: usize = 0;");
    assert_eq!(mut_stat.kind, ItemKind::StaticDecl);
    assert_eq!(mut_stat.identifier, "UNSAFE_GLOBAL");
    let con = unit("pub const MAX_SIZE: usize = 100;");
    assert_eq!(con.kind, ItemKind::ConstDecl);
    assert_eq!(con.identifier, "MAX_SIZE");
}

#[test]
fn test_metadata_visibility_extraction() {
    assert_eq!(unit("pub struct A {}").visibility, Visibility::Pub);
    assert_eq!(
        unit("pub(crate) fn b() {}").visibility,
        Visibility::PubCrate
    );
    assert_eq!(
        unit("pub(super) const C: usize = 1;").visibility,
        Visibility::PubSuper
    );
    assert_eq!(unit("struct D {}").visibility, Visibility::Private);
    // A field's `pub(crate)` is not the struct's visibility (the old scanner misread it).
    assert_eq!(
        unit("pub struct Id(pub(crate) u64);").visibility,
        Visibility::Pub
    );
}

/// The one unit layout finds in `code`.
fn unit(code: &str) -> Unit {
    let mut found = units(code);
    assert_eq!(found.len(), 1, "{}", code);
    found.remove(0)
}

/// The units layout finds in `code`, in input order.
fn units(code: &str) -> Vec<Unit> {
    layout(code, &LayoutConfig::default()).unwrap().before
}
