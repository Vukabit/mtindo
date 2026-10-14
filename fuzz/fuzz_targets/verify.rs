#![no_main]

//! The verifier must never raise a false alarm or panic:
//! - a file checked against itself is clean;
//! - for a file that parses, moving its items around (header first, `macro_rules!` in their
//!   order, everything joined by blank lines in the file's own line endings) is clean;
//! - checking what layout writes never panics; its findings are not asserted on here.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| mtindo::fuzzing::verify::check(data));
