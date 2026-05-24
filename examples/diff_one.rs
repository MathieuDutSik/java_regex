//! One-shot diff probe — feeds a single (pattern, flags, input) case to both
//! engines via DiffOracle and prints both results. Used to iteratively shrink
//! a failing case during manual triage.
//!
//! Usage:
//!     cargo run --release --example diff_one -- '<op>' '<pattern>' '<flags>' '<input>'
//!
//! Example:
//!     cargo run --release --example diff_one -- find '\Q\E' '' $'\t'

use std::env;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: diff_one <op:matches|find|split|replaceAll> <pattern> <flags> <input> [replacement]");
        std::process::exit(2);
    }
    let op = &args[1];
    let pattern = &args[2];
    let flags = &args[3];
    let input = &args[4];
    let replacement = args.get(5).cloned().unwrap_or_default();

    let java = env::var("DIFF_JAVA").unwrap_or_else(|_| "java".to_string());
    let cp = env::var("DIFF_CLASSPATH").unwrap_or_else(|_| ".".to_string());

    let mut child = Command::new(&java)
        .arg("-cp").arg(&cp).arg("DiffOracle")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit())
        .spawn().expect("spawn java");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    let req = serde_json::json!({
        "op": op,
        "pattern": pattern,
        "input": input,
        "flags": flags,
        "replacement": replacement,
    }).to_string();
    writeln!(stdin, "{}", req).unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let java_resp: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
    writeln!(stdin, "{{\"op\":\"quit\"}}").unwrap();

    let rust_resp = match java_regex::Regex::with_flags(pattern, flags) {
        Err(e) => serde_json::json!({"ok": false, "error": "compile_error", "msg": e.to_string()}),
        Ok(re) => match op.as_str() {
            "matches" => serde_json::json!({"ok": true, "result": re.matches(input)}),
            "find" => {
                let arr: Vec<_> = re.find(input).into_iter().map(|m|
                    serde_json::json!({"m": m.matched_text, "s": m.start, "e": m.end})
                ).collect();
                serde_json::json!({"ok": true, "result": arr})
            }
            "split" => serde_json::json!({"ok": true, "result": re.split(input)}),
            "replaceAll" => serde_json::json!({"ok": true, "result": re.replace_all(input, &replacement)}),
            other => { eprintln!("unknown op: {other}"); std::process::exit(2); }
        }
    };

    let agree = match op.as_str() {
        "find" => {
            // Compare matched text only (Java has UTF-16 offsets).
            let j = java_resp.get("result").and_then(|v| v.as_array());
            let r = rust_resp.get("result").and_then(|v| v.as_array());
            match (j, r) {
                (Some(j), Some(r)) => j.len() == r.len()
                    && j.iter().zip(r).all(|(a, b)| a["m"] == b["m"]),
                _ => java_resp == rust_resp,
            }
        }
        _ => java_resp == rust_resp,
    };

    println!("op       = {op}");
    println!("pattern  = {pattern:?}");
    println!("flags    = {flags:?}");
    println!("input    = {input:?}");
    println!("rust     = {rust_resp}");
    println!("java     = {java_resp}");
    println!("agree    = {agree}");
    let _ = child.wait();
    if !agree { std::process::exit(1); }
}
