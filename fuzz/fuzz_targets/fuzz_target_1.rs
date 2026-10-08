#![no_main]
use libfuzzer_sys::fuzz_target;
use mtindo::{extract_blocks, reflow_delimited_blocks, reorganize_file, wrap_line, LayoutConfig};

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = extract_blocks(s);
        let _ = wrap_line(s, 80, 4);
        let _ = reflow_delimited_blocks(s, 80, 4);
        
        let config = LayoutConfig::default();
        let _ = reorganize_file(s.to_string(), &config, false);
    }
});
