#![no_main]

//! See `mtindo::fuzzing::parser`.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| mtindo::fuzzing::parser::check(data));
