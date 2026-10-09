#![no_main]

use libfuzzer_sys::fuzz_target;
use mtindo::syntax::Edition;
use mtindo::syntax::lexer::tokenize;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        for edition in [
            Edition::E2015,
            Edition::E2018,
            Edition::E2021,
            Edition::E2024,
        ] {
            let tokens = tokenize(text, edition);
            let mut offset = 0;
            for token in tokens.iter() {
                assert!(token.len > 0, "empty token");
                offset += token.len as usize;
                assert!(
                    text.is_char_boundary(offset),
                    "token ends inside a character"
                );
            }
            assert_eq!(offset, text.len(), "tokens do not cover the input");
        }
    }
});
