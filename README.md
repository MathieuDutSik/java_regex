# java_regex

A Rust implementation of Java's `java.util.regex.Pattern` API, byte-for-byte
compatible with OpenJDK 25. `no_std` (only needs `alloc`), zero unsafe code,
fuzz-tested against a live JVM.

## Why this crate

Rust's de-facto regex library, [`regex`](https://crates.io/crates/regex), is
intentionally limited: no backreferences, no lookbehind, no possessive
quantifiers — features it deliberately excludes to guarantee linear-time
matching. That makes `regex` an excellent choice when those limits work for
you. When they don't, the alternatives are:

| Crate | Lookarounds | Backrefs | Java-compat | no_std |
|---|---|---|---|---|
| [`regex`](https://crates.io/crates/regex) | no | no | n/a | yes |
| [`fancy-regex`](https://crates.io/crates/fancy-regex) | yes | yes | partial | no |
| [`onig`](https://crates.io/crates/onig) | yes | yes | no | no (C dep) |
| **`java_regex` (this crate)** | **yes** | **yes** | **byte-for-byte vs OpenJDK 25** | **yes** |

This crate exists for one specific use case: **porting regular expressions
from Java**. If you have a `Pattern.compile(...)` somewhere in a JVM codebase
and you want it to behave identically in Rust — same matches, same captures,
same compile errors, same OpenJDK quirks — this is the crate.

## Quick start

```toml
[dependencies]
java_regex = "0.1"
```

```rust
use java_regex::{Regex, MatchInfo};

let re = Regex::new(r"(\w+)@(\w+\.[a-z]{2,})").unwrap();

// Find returns all non-overlapping matches.
let matches = re.find("alice@example.com, bob@example.org");
assert_eq!(matches.len(), 2);
assert_eq!(matches[0].groups[0].as_deref(), Some("alice"));
assert_eq!(matches[0].groups[1].as_deref(), Some("example.com"));

// Replace with Java's $N / ${name} DSL:
let re = Regex::new(r"(\w+),(\w+)").unwrap();
assert_eq!(re.replace_all("Doe,John", "$2 $1"), "John Doe");

// Or with a closure (any FnMut(&MatchInfo) -> String):
let re = Regex::new(r"\d+").unwrap();
assert_eq!(
    re.replace_all("a1b22c333", |m: &MatchInfo| format!("[{}]", m.matched_text.len())),
    "a[1]b[2]c[3]"
);

// Split — same semantics as Java String.split:
let re = Regex::new(r"\s*,\s*").unwrap();
assert_eq!(re.split("a, b,  c"), vec!["a", "b", "c"]);
```

The full API mirrors `java.util.regex.Matcher`: `matches`, `looking_at`,
`find`, `find_at`, `find_in_region`, `find_iter`, `replace_all`, `replace_first`,
`split`, `split_with_limit`, `quote`.

## Compatibility

This crate has been validated against OpenJDK 25 by a continuous differential
fuzzer that spawns a long-lived JVM and feeds both engines randomly-generated
patterns, inputs, and operations. At ~20 000 cases per second, the latest
700 000-case batch produced **zero** semantic divergences on BMP inputs.

The only documented difference is structural: `Matcher.start()` / `end()`
return UTF-16 code-unit offsets, while we return Unicode-codepoint offsets.
Matched *text* is always identical; only the integer indices differ when the
input contains supplementary characters. See [DIFFERENCES.md](DIFFERENCES.md).

The crate also faithfully reproduces eight well-known OpenJDK quirks
(multiline `^` end-of-input behavior, atomic quantified atoms, chained `&&`
parser asymmetry, capture leaks across `find()` positions, etc.). See
[QUIRKS.md](QUIRKS.md) for each one's pattern, behavior, and OpenJDK source
class.

For a full reference specification of what OpenJDK regex actually does — the
behavior the Javadoc underspecifies — see [SPEC.md](SPEC.md).

## Performance

We're a pure backtracking NFA — semantically faithful to Java, comparable to
`fancy-regex` for backref/lookaround patterns, slower than `regex` (which
uses DFAs and can't support backrefs/lookaround in the first place).

Indicative timings from `cargo run --release --example bench_engines`:

| Pattern | java_regex | regex | fancy-regex | onig |
|---|---:|---:|---:|---:|
| literal find | 35 µs | <1 µs | <1 µs | 2 µs |
| URL extraction | 2.1 ms | 25 µs | 264 µs | 75 µs |
| email regex | 768 µs | 483 µs | 273 µs | 104 µs |
| catastrophic `(a+)+` | 129 ms | 1 µs | 1 µs | 2.9 ms |

If you don't *need* Java semantics, use `regex`. If you do, the cost is real
but bounded.

## `no_std`

The library has no `std` dependency — only `alloc`. It builds cleanly for
bare-metal targets (verified on `thumbv7em-none-eabi` in CI). All allocations
go through Rust's global allocator; no filesystem, threads, or stdin/stdout.

## Status

Pre-1.0. The API is stable enough for use, but breaking changes are still on
the table if they meaningfully improve Java compat or ergonomics.

Issues and PRs welcome.

## Further reading

- [SPEC.md](SPEC.md) — Reference spec of OpenJDK regex behavior.
- [QUIRKS.md](QUIRKS.md) — The eight OpenJDK quirks this crate reproduces, with worked examples.
- [DIFFERENCES.md](DIFFERENCES.md) — The one intentional deviation (UTF-16 vs UTF-8 offsets).
- [FUZZING.md](FUZZING.md) — How to run the proptest, cargo-fuzz, differential, and benchmark suites.

## License

Dual MIT / Apache-2.0.
