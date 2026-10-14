#![no_main]

//! Source built from the bytes, which always parses, through layout, formatting and the two
//! together: each keeps its meaning and settles in one run. The first three bytes pick the
//! formatting settings; the rest choose the source.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| mtindo::fuzzing::generated::check(data));
