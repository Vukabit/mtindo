#![no_main]

//! See `mtindo::fuzzing::lexer`.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| mtindo::fuzzing::lexer::check(data));
