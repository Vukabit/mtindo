use lofmt::{
    apply_width_constraints, extract_blocks, reflow_delimited_blocks, reorganize_file, wrap_line,
    LayoutConfig,
};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn test_wrap_line_no_panic(ref s in any::<String>()) {
        let _ = wrap_line(s, 80, 4);
    }

    #[test]
    fn test_reflow_delimited_blocks_no_panic(ref s in any::<String>()) {
        let _ = reflow_delimited_blocks(s, 80, 4);
    }

    #[test]
    fn test_apply_width_constraints_no_panic(ref s in any::<String>()) {
        let _ = apply_width_constraints(s, 80, 4);
    }

    #[test]
    fn test_extract_blocks_no_panic(ref s in any::<String>()) {
        let _ = extract_blocks(s);
    }

    #[test]
    fn test_reorganize_file_no_panic(ref s in any::<String>()) {
        let config = LayoutConfig::default();
        let _ = reorganize_file(s.clone(), &config, false);
    }
}
