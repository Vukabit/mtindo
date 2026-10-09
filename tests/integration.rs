use mtindo::{ItemKind, extract_blocks};

#[test]
fn test_extract_blocks_handles_raw_strings() {
    let input = "fn get_raw() {\n    let s = r#\" { struct Fake; } \"#;\n}\nstruct Real { }";
    let blocks = extract_blocks(input);
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].kind, ItemKind::FnDecl);
    assert_eq!(blocks[1].kind, ItemKind::StructDecl);
}
