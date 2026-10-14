#![no_main]

//! The TOML parser never panics, and every error points inside the document.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| mtindo::fuzzing::toml::check(data));
