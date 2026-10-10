#![no_main]

//! The TOML parser never panics, and every error points inside the document.

use libfuzzer_sys::fuzz_target;
use mtindo::config::toml::parse;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Err(e) = parse(text) {
        let lines = text.split('\n').count();
        assert!(e.line >= 1 && e.line <= lines, "line {} of {}: {}", e.line, lines, e);
        let line_text = text.split('\n').nth(e.line - 1).unwrap_or("");
        assert!(
            e.column >= 1 && e.column <= line_text.chars().count() + 1,
            "column {} on {:?}: {}",
            e.column,
            line_text,
            e
        );
    }
});
