#![no_main]

//! Comments everywhere: layout must keep every comment, and never write what the verifier
//! would refuse.
//!
//! The first bytes pick the settings and where comments go; the rest is a source file. Each
//! gap between tokens that already holds whitespace may get a numbered comment, line or
//! block, so no token is split or joined. When the result parses and layout rearranges it:
//! - the verifier finds nothing wrong (a finding would stop a real write: a layout bug);
//! - every inserted comment is still there, as often as before;
//! - a second layout changes nothing.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| mtindo::fuzzing::comments::check(data));
