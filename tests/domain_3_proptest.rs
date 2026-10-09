use mtindo::{LayoutConfig, extract_blocks, reorganize_file};
use proptest::prelude::*;

fn item_strategy() -> impl Strategy<Value = String> {
    let name = "[a-z]{1,6}";
    prop_oneof![
        name.prop_map(|n| format!("fn {}() {{}}", n)),
        name.prop_map(|n| format!("pub fn {}() {{\n    let s = \"{{ {} }}\";\n}}", n, n)),
        name.prop_map(|n| format!("struct {} {{}}", n)),
        name.prop_map(|n| format!("impl {} {{}}", n)),
        name.prop_map(|n| format!("use std::{};", n)),
        name.prop_map(|n| format!("const {}: u8 = 1;", n.to_uppercase())),
        name.prop_map(|n| format!("// note {}\nfn {}_c() {{}}", n, n)),
        name.prop_map(|n| format!("macro_rules! {} {{ () => {{}} }}", n)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]
    #[test]
    fn test_extract_blocks_no_panic(ref s in any::<String>()) {
        let _ = extract_blocks(s);
    }
    #[test]
    fn test_reorganize_file_no_panic(ref s in any::<String>()) {
        let config = LayoutConfig::default();
        let _ = reorganize_file(s.clone(), &config, false);
    }
    #[test]
    fn test_reorganize_keeps_every_line_and_is_idempotent(items in prop::collection::vec(item_strategy(), 1..12)) {
        let source = items.join("\n");
        let config = LayoutConfig::default();
        let first = reorganize_file(source.clone(), &config, false);
        let second = reorganize_file(first.clone(), &config, false);
        prop_assert_eq!(&first, &second);
        let mut before: Vec<&str> = source.lines().filter(|l| !l.trim().is_empty()).collect();
        let mut after: Vec<&str> = first.lines().filter(|l| !l.trim().is_empty()).collect();
        before.sort();
        after.sort();
        prop_assert_eq!(before, after);
    }
}
