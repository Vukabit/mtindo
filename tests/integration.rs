use mtindo::layout::layout;
use mtindo::{ItemKind, LayoutConfig};

/// The old scanner cut an item in two at a `}` inside generic arguments.
#[test]
fn test_braces_inside_generics_stay_in_one_item() {
    let input = "fn b() {}\n\nstruct A<const N: i32 = { 1 }>;\n\nfn a() {}\n";
    let arranged = layout(input, &LayoutConfig::default()).unwrap();
    assert_eq!(arranged.before.len(), 3);
    assert_eq!(
        arranged.text,
        "struct A<const N: i32 = { 1 }>;\n\nfn a() {}\n\nfn b() {}\n"
    );
}

#[test]
fn test_extract_blocks_handles_raw_strings() {
    let input = "fn get_raw() {\n    let s = r#\" { struct Fake; } \"#;\n}\nstruct Real { }";
    let units = layout(input, &LayoutConfig::default()).unwrap().before;
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].kind, ItemKind::FnDecl);
    assert_eq!(units[1].kind, ItemKind::StructDecl);
}
