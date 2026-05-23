#![no_main]
//! Parser robustness fuzzer.
//!
//! Feeds arbitrary UTF-8 byte sequences to `Regex::new` and a randomly chosen
//! flag string. Compile errors are fine; panics, infinite loops, and OOM are
//! the bugs we're hunting.
//!
//! Run with:
//!   cargo +nightly fuzz run fuzz_parser

use libfuzzer_sys::fuzz_target;
use arbitrary::{Arbitrary, Unstructured};

#[derive(Arbitrary, Debug)]
struct Input<'a> {
    flags: u8,
    pattern: &'a str,
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let Ok(input) = Input::arbitrary(&mut u) else { return; };
    let flags = flags_from_byte(input.flags);
    let _ = java_regex::Regex::with_flags(input.pattern, &flags);
});

fn flags_from_byte(b: u8) -> String {
    let mut s = String::new();
    if b & 0b0000_0001 != 0 { s.push('i'); }
    if b & 0b0000_0010 != 0 { s.push('m'); }
    if b & 0b0000_0100 != 0 { s.push('s'); }
    if b & 0b0000_1000 != 0 { s.push('x'); }
    if b & 0b0001_0000 != 0 { s.push('u'); }
    if b & 0b0010_0000 != 0 { s.push('U'); }
    if b & 0b0100_0000 != 0 { s.push('d'); }
    if b & 0b1000_0000 != 0 { s.push('l'); }
    s
}
