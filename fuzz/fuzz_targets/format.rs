#![no_main]

//! Formatting must keep a program's meaning and settle in one run, whatever the input and
//! settings: the meaning check is clean on the output, and formatting the output again
//! changes nothing. The first bytes pick the settings; the rest is the file.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| mtindo::fuzzing::format::check(data));
