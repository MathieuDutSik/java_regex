#![no_main]
//! Replacement-string fuzzer.
//!
//! Java's `Matcher.appendReplacement` has a finicky replacement-string mini-syntax
//! ($1, ${name}, \$, \\, etc.). This target exercises it heavily.
//!
//! Run with:
//!   cargo +nightly fuzz run fuzz_replace

use libfuzzer_sys::fuzz_target;
use arbitrary::{Arbitrary, Unstructured};
use java_regex::Regex;
use java_regex::gen::{RegexNode, FlagSet, render};

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    ast: RegexNode,
    flags: FlagSet,
    input: String,
    replacement: String,
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let Ok(fi) = FuzzInput::arbitrary(&mut u) else { return; };

    let pat = render(&fi.ast);
    let Ok(re) = Regex::with_flags(&pat, &fi.flags.to_flags_str()) else { return; };

    let _ = re.replace_all(&fi.input, &fi.replacement);
    let _ = re.replace_first(&fi.input, &fi.replacement);
});
