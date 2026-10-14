#![no_main]

//! Layout must keep a program's meaning and settle in one run, whatever the input: when it
//! arranges a file, the layout meaning check is clean on the result (mto refuses to write a
//! file that fails it), and arranging the result again changes nothing. Inputs it declines
//! to arrange are fine; panicking on any input is not.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| mtindo::fuzzing::layout::check(data));
