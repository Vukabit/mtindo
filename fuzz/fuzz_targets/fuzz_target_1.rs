#![no_main]

use libfuzzer_sys::fuzz_target;
use mtindo::{LayoutConfig, extract_blocks, reorganize_file};

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = extract_blocks(s);

        let config = LayoutConfig::default();
        let _ = reorganize_file(s.to_string(), &config, false);
    }
});
