#![no_main]

//! Formatting must keep a program's meaning and settle in one run, whatever the input and
//! settings: the meaning check is clean on the output, and formatting the output again
//! changes nothing. The first bytes pick the settings; the rest is the file.

use libfuzzer_sys::fuzz_target;
use mtindo::format::file::format_file;
use mtindo::options::format::{FormatConfig, NewlineStyle};
use mtindo::syntax::Edition;
use mtindo::verify::format::verify_format;

fuzz_target!(|data: &[u8]| {
    let Some((&[width, tabs, style], rest)) = data.split_first_chunk::<3>() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    let config = FormatConfig {
        max_width: 20 + usize::from(width % 120),
        tab_spaces: usize::from(tabs % 9),
        hard_tabs: tabs & 0x80 != 0,
        newline_style: match style % 4 {
            0 => NewlineStyle::Auto,
            1 => NewlineStyle::Native,
            2 => NewlineStyle::Unix,
            _ => NewlineStyle::Windows,
        },
        blank_lines_upper_bound: usize::from(style >> 2 & 3),
        ..FormatConfig::default()
    };
    let edition = Edition::E2021;
    let once = format_file(text, edition, &config);
    let found = verify_format(text, &once, edition);
    assert!(found.is_empty(), "meaning changed:\n{}\n---\n{}\n{:?}", text, once, found);
    let twice = format_file(&once, edition, &config);
    assert_eq!(once, twice, "a second run changed it:\n{}", text);
});
