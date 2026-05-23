#![no_main]
//! Engine robustness fuzzer.
//!
//! Uses the structured `gen::RegexNode` generator (via `arbitrary`) to produce
//! patterns that are syntactically plausible, then runs every Regex operation
//! on a generated input. Panics, infinite loops, and OOM are bugs.
//!
//! Run with:
//!   cargo +nightly fuzz run fuzz_engine

use libfuzzer_sys::fuzz_target;
use arbitrary::{Arbitrary, Unstructured};
use java_regex::Regex;
use java_regex::gen::{RegexNode, FlagSet, render};

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    ast: RegexNode,
    flags: FlagSet,
    input: String,
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let Ok(fi) = FuzzInput::arbitrary(&mut u) else { return; };

    let pat = render(&fi.ast);
    let Ok(re) = Regex::with_flags(&pat, &fi.flags.to_flags_str()) else { return; };

    let _ = re.matches(&fi.input);
    let _ = re.find(&fi.input);
    let _ = re.looking_at(&fi.input);
    let _ = re.split(&fi.input);
});
