//! Compare java_regex's match throughput against three peer engines on a
//! standard mix of patterns. Reports per-engine timings + correctness (whether
//! the engine even accepts the pattern, and how many matches it finds).
//!
//! Honest about where we win (Java-exact semantics, lookarounds + backrefs
//! that `regex` won't compile) and where we lose (we're a pure backtracking
//! NFA; `regex` uses a DFA hybrid).
//!
//! Run with:
//!     cargo run --release --example bench_engines
//!
//! Output is plain text on stdout, one line per (pattern, engine) pair.

use std::time::Instant;

struct Case {
    label: &'static str,
    pattern: &'static str,
    input_factory: fn() -> String,
    /// How many times to repeat the search per engine. Per-case so the
    /// catastrophic-backtracking row stays under CI budget.
    iterations: u32,
}

fn input_short_ascii() -> String {
    "the quick brown fox jumps over the lazy dog 12345".repeat(20)
}

fn input_paragraph() -> String {
    let words = [
        "lorem", "ipsum", "dolor", "sit", "amet", "consectetur",
        "adipiscing", "elit", "sed", "do", "eiusmod", "tempor",
        "incididunt", "ut", "labore", "et", "dolore", "magna",
    ];
    let mut s = String::with_capacity(8_000);
    for i in 0..600 { s.push_str(words[i % words.len()]); s.push(' '); }
    s
}

fn input_with_urls() -> String {
    let urls = [
        "https://example.com/foo?bar=1",
        "http://en.wikipedia.org/wiki/Regex",
        "https://crates.io/crates/regex",
        "ftp://ftp.example.org/pub/release.tar.gz",
    ];
    let mut s = String::with_capacity(10_000);
    for i in 0..300 {
        s.push_str("see ");
        s.push_str(urls[i % urls.len()]);
        s.push_str(" for details, ");
    }
    s
}

fn input_with_emails() -> String {
    let emails = [
        "alice@example.com",
        "bob.smith+filter@dept.example.org",
        "carol-99@sub.example.co.uk",
        "noreply@example.com",
    ];
    let mut s = String::with_capacity(8_000);
    for i in 0..300 {
        s.push_str("contact ");
        s.push_str(emails[i % emails.len()]);
        s.push_str(" — ");
    }
    s
}

fn input_backtracking() -> String {
    // Classic catastrophic-backtracking input: many a's, then a non-b.
    let mut s = String::with_capacity(40);
    for _ in 0..18 { s.push('a'); }
    s.push('!');  // breaks the match — engines without optimisation flounder
    s
}

const CASES: &[Case] = &[
    Case { label: "literal-find",        pattern: r"fox",
           input_factory: input_short_ascii,  iterations: 100 },
    Case { label: "alternation-find",    pattern: r"\b(?:quick|lazy|over)\b",
           input_factory: input_short_ascii,  iterations: 100 },
    Case { label: "url",                 pattern: r"\b(?:https?|ftp)://[^\s,]+",
           input_factory: input_with_urls,    iterations: 30 },
    Case { label: "email",               pattern: r"\b[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}\b",
           input_factory: input_with_emails,  iterations: 30 },
    Case { label: "word-segment",        pattern: r"\b\w{5,}\b",
           input_factory: input_paragraph,    iterations: 50 },
    // Catastrophic case: 1 iteration is enough to make the asymmetry obvious
    // (java_regex / onig do exponential backtracking; regex / fancy-regex
    // bail fast). Keep at 1 so CI doesn't time out.
    Case { label: "catastrophic-(a+)+",  pattern: r"(a+)+b",
           input_factory: input_backtracking, iterations: 1 },
];

fn fmt_us(d: std::time::Duration) -> String {
    let micros = d.as_micros();
    if micros < 1_000 { format!("{}us", micros) }
    else if micros < 1_000_000 { format!("{:.1}ms", micros as f64 / 1_000.0) }
    else { format!("{:.2}s", d.as_secs_f64()) }
}

fn bench_java_regex(pattern: &str, input: &str, iters: u32) -> Option<(usize, std::time::Duration)> {
    let re = java_regex::Regex::new(pattern).ok()?;
    let start = Instant::now();
    let mut total = 0;
    for _ in 0..iters { total += re.find(input).len(); }
    Some((total / iters as usize, start.elapsed()))
}

fn bench_regex(pattern: &str, input: &str, iters: u32) -> Option<(usize, std::time::Duration)> {
    let re = regex::Regex::new(pattern).ok()?;
    let start = Instant::now();
    let mut total = 0;
    for _ in 0..iters { total += re.find_iter(input).count(); }
    Some((total / iters as usize, start.elapsed()))
}

fn bench_fancy_regex(pattern: &str, input: &str, iters: u32) -> Option<(usize, std::time::Duration)> {
    let re = fancy_regex::Regex::new(pattern).ok()?;
    let start = Instant::now();
    let mut total = 0;
    for _ in 0..iters {
        let mut count = 0;
        let mut iter = re.find_iter(input);
        while let Some(Ok(_)) = iter.next() { count += 1; }
        total += count;
    }
    Some((total / iters as usize, start.elapsed()))
}

fn bench_onig(pattern: &str, input: &str, iters: u32) -> Option<(usize, std::time::Duration)> {
    let re = onig::Regex::new(pattern).ok()?;
    let start = Instant::now();
    let mut total = 0;
    for _ in 0..iters { total += re.find_iter(input).count(); }
    Some((total / iters as usize, start.elapsed()))
}

type Runner = fn(&str, &str, u32) -> Option<(usize, std::time::Duration)>;

fn main() {
    println!("# bench_engines: iterations vary per case (see source)\n");
    println!("{:<24} {:<10} {:<24} {:>10} {:>8}",
        "pattern", "engine", "matches", "total", "per-iter");
    println!("{}", "-".repeat(82));

    for case in CASES {
        let input = (case.input_factory)();
        for (engine, runner) in [
            ("java_regex",   bench_java_regex   as Runner),
            ("regex",        bench_regex        as Runner),
            ("fancy-regex",  bench_fancy_regex  as Runner),
            ("onig",         bench_onig         as Runner),
        ] {
            match runner(case.pattern, &input, case.iterations) {
                Some((matches, total)) => {
                    let per = total / case.iterations;
                    println!("{:<24} {:<10} matches/iter={:<13} {:>10} {:>8}",
                        case.label, engine, matches, fmt_us(total), fmt_us(per));
                }
                None => {
                    println!("{:<24} {:<10} (pattern rejected by this engine)",
                        case.label, engine);
                }
            }
        }
        println!();
    }
}
