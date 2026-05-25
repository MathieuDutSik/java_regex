//! Differential fuzzer: Rust java_regex vs. live OpenJDK `java.util.regex`.
//!
//! Spawns `DiffOracle.java` once as a long-lived JVM subprocess, then in a tight
//! loop generates random `RegexNode` ASTs + flags + inputs, runs them through
//! both engines, and reports mismatches.
//!
//! ## Building & running
//!
//! ```sh
//! # 1) Compile the oracle (one-time)
//! javac DiffOracle.java
//!
//! # 2) Run the fuzzer (requires the fuzz-gen feature so the gen AST is Arbitrary)
//! cargo run --release --example diff_fuzz --features fuzz-gen -- 20000
//!
//! # Customizations via env vars:
//! #   DIFF_SEED=42                 deterministic run
//! #   DIFF_JAVA=/path/to/bin/java  pick a specific JVM
//! #   DIFF_LOG=mismatches.jsonl    append every mismatch to this file
//! ```

use arbitrary::{Arbitrary, Unstructured};
use java_regex::gen::{render, FlagSet, RegexNode};
use java_regex::Regex;

use std::env;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;

// --- Tiny deterministic PRNG (SplitMix64) ----------------------------------
// Avoids pulling in `rand` as a dev-dep. Good enough for fuzz-input generation.

struct SplitMix64(u64);
impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let r = self.next().to_le_bytes();
            for (i, b) in chunk.iter_mut().enumerate() { *b = r[i]; }
        }
    }
}

// --- JVM oracle wrapper -----------------------------------------------------

struct Oracle {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Oracle {
    fn spawn(java: &str, classpath: &str) -> std::io::Result<Self> {
        let mut child = Command::new(java)
            .arg("-cp").arg(classpath)
            .arg("DiffOracle")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take().expect("no stdin");
        let stdout = BufReader::new(child.stdout.take().expect("no stdout"));
        Ok(Oracle { _child: child, stdin, stdout })
    }

    fn request(&mut self, req_json: &str) -> std::io::Result<serde_json::Value> {
        self.stdin.write_all(req_json.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof, "oracle closed stdout"));
        }
        serde_json::from_str(line.trim_end())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData,
                format!("bad oracle response: {e}: {line}")))
    }

    fn quit(&mut self) {
        let _ = self.stdin.write_all(b"{\"op\":\"quit\"}\n");
        let _ = self.stdin.flush();
    }
}

// --- Replacement-string generator -------------------------------------------
//
// Java's `Matcher.appendReplacement` DSL: `$1`, `${name}`, `\$`, `\\`, plus
// literal chars. We generate a small mix to exercise the DSL parser.

fn gen_replacement(rng: &mut SplitMix64) -> String {
    let mut s = String::new();
    let len = (rng.next() % 6) as usize;  // 0..5 segments
    for _ in 0..len {
        let kind = rng.next() % 8;
        match kind {
            0 => s.push('a'),
            1 => s.push('-'),
            2 => s.push_str("$1"),
            3 => s.push_str("$0"),
            4 => s.push_str("${foo}"),
            5 => s.push_str("\\$"),
            6 => s.push_str("\\\\"),
            _ => s.push('x'),
        }
    }
    s
}

// --- Mismatch reporting -----------------------------------------------------

#[derive(Default)]
struct Stats {
    iters: u64,
    rust_compile_err: u64,
    java_compile_err: u64,
    both_compile_err: u64,
    skipped_unsupported: u64,
    ok: u64,
    mismatches: u64,
}

fn log_mismatch(log_path: &Option<PathBuf>, kind: &str, detail: &serde_json::Value) {
    eprintln!("\n=== MISMATCH ({kind}) ===");
    eprintln!("{}", serde_json::to_string_pretty(detail).unwrap());
    if let Some(p) = log_path {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{{\"kind\":\"{kind}\",\"case\":{detail}}}");
        }
    }
}

// --- Comparison helpers -----------------------------------------------------
//
// Java's Matcher returns UTF-16 code-unit offsets; the Rust impl uses char
// offsets. We compare on matched text (always identical when both engines
// agree semantically) and skip strict position comparison if the input contains
// any supplementary code points.

fn input_is_bmp(s: &str) -> bool {
    s.chars().all(|c| (c as u32) <= 0xFFFF)
}

fn do_matches(or: &mut Oracle, re: &Regex, pat: &str, input: &str, flags: &str)
    -> Result<bool, String>
{
    let req = serde_json::json!({
        "op": "matches", "pattern": pat, "input": input, "flags": flags
    }).to_string();
    let resp = or.request(&req).map_err(|e| e.to_string())?;
    if resp["ok"].as_bool() != Some(true) { return Ok(true); }
    let java = resp["result"].as_bool().ok_or("bad matches result")?;
    let rust = re.matches(input);
    Ok(java == rust)
}

fn do_find(or: &mut Oracle, re: &Regex, pat: &str, input: &str, flags: &str)
    -> Result<bool, String>
{
    let req = serde_json::json!({
        "op": "find", "pattern": pat, "input": input, "flags": flags
    }).to_string();
    let resp = or.request(&req).map_err(|e| e.to_string())?;
    if resp["ok"].as_bool() != Some(true) { return Ok(true); }
    let java_arr = resp["result"].as_array().ok_or("bad find result")?;

    let rust_matches = re.find(input);
    if rust_matches.len() != java_arr.len() { return Ok(false); }

    let bmp = input_is_bmp(input);
    for (i, jv) in java_arr.iter().enumerate() {
        let jm = jv["m"].as_str().ok_or("bad m")?;
        let rm = &rust_matches[i].matched_text;
        if jm != rm { return Ok(false); }
        if bmp {
            let js = jv["s"].as_u64().ok_or("bad s")? as usize;
            let je = jv["e"].as_u64().ok_or("bad e")? as usize;
            if js != rust_matches[i].start || je != rust_matches[i].end {
                return Ok(false);
            }
        }
        // Compare per-group captures. Java emits `g`: array of strings or null.
        // We compare values (string content) — group positions would also need
        // BMP conversion, so for now matched text suffices.
        if let Some(java_groups) = jv["g"].as_array() {
            let rust_groups = &rust_matches[i].groups;
            if java_groups.len() != rust_groups.len() { return Ok(false); }
            for (k, jg) in java_groups.iter().enumerate() {
                let java_val = jg.as_str();
                let rust_val = rust_groups[k].as_deref();
                if java_val != rust_val { return Ok(false); }
            }
        }
    }
    Ok(true)
}

fn do_split(or: &mut Oracle, re: &Regex, pat: &str, input: &str, flags: &str)
    -> Result<bool, String>
{
    let req = serde_json::json!({
        "op": "split", "pattern": pat, "input": input, "flags": flags
    }).to_string();
    let resp = or.request(&req).map_err(|e| e.to_string())?;
    if resp["ok"].as_bool() != Some(true) { return Ok(true); }
    let java_arr: Vec<String> = resp["result"].as_array().ok_or("bad split result")?
        .iter().map(|v| v.as_str().unwrap_or("").to_string()).collect();

    let rust = re.split(input);
    Ok(rust == java_arr)
}

fn do_looking_at(or: &mut Oracle, re: &Regex, pat: &str, input: &str, flags: &str)
    -> Result<bool, String>
{
    let req = serde_json::json!({
        "op": "lookingAt", "pattern": pat, "input": input, "flags": flags
    }).to_string();
    let resp = or.request(&req).map_err(|e| e.to_string())?;
    if resp["ok"].as_bool() != Some(true) { return Ok(true); }
    let java = resp["result"].as_bool().ok_or("bad lookingAt result")?;
    let rust = re.looking_at(input).is_some();
    Ok(java == rust)
}

fn do_find_at(or: &mut Oracle, re: &Regex, pat: &str, input: &str, flags: &str,
              start: usize) -> Result<bool, String>
{
    let req = serde_json::json!({
        "op": "findAt", "pattern": pat, "input": input, "flags": flags,
        "start": start.to_string(),
    }).to_string();
    let resp = or.request(&req).map_err(|e| e.to_string())?;
    if resp["ok"].as_bool() != Some(true) { return Ok(true); }
    let rust = re.find_at(input, start);
    match (resp["result"].as_object(), rust) {
        (None, None) => Ok(true),
        (Some(jo), Some(rm)) => {
            let jm = jo.get("m").and_then(|v| v.as_str()).ok_or("bad m")?;
            Ok(jm == rm.matched_text)
        }
        _ => Ok(false),
    }
}

fn do_find_in_region(or: &mut Oracle, re: &Regex, pat: &str, input: &str, flags: &str,
                     start: usize, end: usize) -> Result<bool, String>
{
    let req = serde_json::json!({
        "op": "findInRegion", "pattern": pat, "input": input, "flags": flags,
        "regionStart": start.to_string(), "regionEnd": end.to_string(),
    }).to_string();
    let resp = or.request(&req).map_err(|e| e.to_string())?;
    if resp["ok"].as_bool() != Some(true) { return Ok(true); }
    let java_arr = resp["result"].as_array().ok_or("bad findInRegion result")?;
    let rust = re.find_in_region(input, start, Some(end));
    if rust.len() != java_arr.len() { return Ok(false); }
    for (i, jv) in java_arr.iter().enumerate() {
        let jm = jv["m"].as_str().ok_or("bad m")?;
        if jm != rust[i].matched_text { return Ok(false); }
    }
    Ok(true)
}

fn do_replace_all(or: &mut Oracle, re: &Regex, pat: &str, input: &str, flags: &str,
                  replacement: &str) -> Result<bool, String>
{
    let req = serde_json::json!({
        "op": "replaceAll", "pattern": pat, "input": input, "flags": flags,
        "replacement": replacement,
    }).to_string();
    let resp = or.request(&req).map_err(|e| e.to_string())?;
    if resp["ok"].as_bool() != Some(true) {
        // Java rejected the replacement string (invalid $N, etc.). We accept.
        return Ok(true);
    }
    let java = resp["result"].as_str().ok_or("bad replaceAll result")?;
    let rust = re.replace_all(input, replacement);
    Ok(java == rust)
}

// --- Main loop --------------------------------------------------------------

fn main() {
    let args: Vec<String> = env::args().collect();
    let total: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10_000);

    let java = env::var("DIFF_JAVA").unwrap_or_else(|_| "java".to_string());
    let cp = env::var("DIFF_CLASSPATH").unwrap_or_else(|_| ".".to_string());
    let log_path = env::var("DIFF_LOG").ok().map(PathBuf::from);
    let seed = env::var("DIFF_SEED").ok().and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0xdeadbeef)
        });
    eprintln!("diff_fuzz: seed={seed} total={total} java={java}");

    let mut oracle = Oracle::spawn(&java, &cp).expect("failed to spawn DiffOracle");
    let pong = oracle.request("{\"op\":\"ping\"}").expect("ping");
    assert_eq!(pong["result"].as_str(), Some("pong"), "oracle didn't pong");

    let mut rng = SplitMix64(seed);
    let mut stats = Stats::default();
    let start = Instant::now();
    let mut last_report = Instant::now();
    let mut buf = vec![0u8; 384];

    while stats.iters < total {
        stats.iters += 1;

        rng.fill(&mut buf);
        let mut u = Unstructured::new(&buf);
        let Ok(ast) = RegexNode::arbitrary(&mut u) else { continue; };
        let Ok(flags) = FlagSet::arbitrary(&mut u) else { continue; };
        let Ok(input_chars): Result<Vec<java_regex::gen::LitChar>, _>
            = Vec::arbitrary(&mut u) else { continue; };
        let input: String = input_chars.iter().take(16).map(|c| c.to_char()).collect();

        let flags_str = flags.to_flags_str();
        let pattern = render(&ast);

        let rust_re = Regex::with_flags(&pattern, &flags_str);

        // Ask Java to compile (via a cheap matches probe). If both fail to
        // compile, skip; if exactly one fails, classify but don't count as a
        // mismatch (compile-error semantics differ by design).
        let probe = serde_json::json!({
            "op": "matches", "pattern": pattern, "input": "",
            "flags": flags_str
        }).to_string();
        let probe_resp = match oracle.request(&probe) {
            Ok(v) => v,
            Err(e) => { eprintln!("oracle request failed: {e}"); break; }
        };
        let java_ok = probe_resp["ok"].as_bool() == Some(true);
        let rust_ok = rust_re.is_ok();

        match (rust_ok, java_ok) {
            (false, false) => { stats.both_compile_err += 1; continue; }
            (false, true)  => { stats.rust_compile_err += 1; continue; }
            (true, false)  => { stats.java_compile_err += 1; continue; }
            (true, true)   => {}
        }
        let rust_re = rust_re.unwrap();

        // Generate a small random replacement string mixing literals and the
        // Java replacement DSL ($N, ${name}, \$, \\). Stays under 16 chars.
        let replacement = gen_replacement(&mut rng);
        // Random region bounds for find_at / find_in_region. Keep within input length.
        let input_len = input.chars().count();
        let region_start = if input_len == 0 { 0 } else { (rng.next() as usize) % (input_len + 1) };
        let region_end   = if input_len == 0 { 0 } else { region_start + (rng.next() as usize) % (input_len + 1 - region_start) };

        let mut all_ok = true;
        for (kind, result) in [
            ("matches",       do_matches(&mut oracle, &rust_re, &pattern, &input, &flags_str)),
            ("find",          do_find(&mut oracle, &rust_re, &pattern, &input, &flags_str)),
            ("split",         do_split(&mut oracle, &rust_re, &pattern, &input, &flags_str)),
            ("replaceAll",    do_replace_all(&mut oracle, &rust_re, &pattern, &input, &flags_str, &replacement)),
            ("lookingAt",     do_looking_at(&mut oracle, &rust_re, &pattern, &input, &flags_str)),
            ("findAt",        do_find_at(&mut oracle, &rust_re, &pattern, &input, &flags_str, region_start)),
            ("findInRegion",  do_find_in_region(&mut oracle, &rust_re, &pattern, &input, &flags_str, region_start, region_end)),
        ] {
            match result {
                Ok(true)  => {}
                Ok(false) => {
                    all_ok = false;
                    let detail = serde_json::json!({
                        "pattern": pattern,
                        "flags": flags_str,
                        "input": input,
                        "op": kind,
                    });
                    log_mismatch(&log_path, kind, &detail);
                    stats.mismatches += 1;
                }
                Err(e) => {
                    eprintln!("oracle error on {kind}: {e}");
                    stats.skipped_unsupported += 1;
                    all_ok = false;
                    break;
                }
            }
        }
        if all_ok { stats.ok += 1; }

        if last_report.elapsed().as_secs() >= 5 {
            let elapsed = start.elapsed().as_secs_f64();
            let rate = stats.iters as f64 / elapsed;
            eprintln!("[{:>6}s] iters={} ok={} mismatches={} rust_only_err={} java_only_err={} both_err={} ({:.0}/s)",
                elapsed as u64, stats.iters, stats.ok, stats.mismatches,
                stats.rust_compile_err, stats.java_compile_err, stats.both_compile_err, rate);
            last_report = Instant::now();
        }
    }

    oracle.quit();
    let elapsed = start.elapsed().as_secs_f64();
    eprintln!("\n=== summary (seed={seed}) ===");
    eprintln!("  total iters         {}", stats.iters);
    eprintln!("  ok                  {}", stats.ok);
    eprintln!("  mismatches          {}", stats.mismatches);
    eprintln!("  rust-only compile errs  {}", stats.rust_compile_err);
    eprintln!("  java-only compile errs  {}", stats.java_compile_err);
    eprintln!("  both compile errs   {}", stats.both_compile_err);
    eprintln!("  skipped (oracle err) {}", stats.skipped_unsupported);
    eprintln!("  elapsed             {:.1}s", elapsed);
}
