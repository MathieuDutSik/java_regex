use java_regex::Regex;
use serde::Deserialize;
use std::fs;

#[derive(Deserialize)]
struct TestCase {
    id: String,
    pattern: String,
    input: Option<String>,
    op: String,
    expect: serde_json::Value,
    flags: Option<String>,
    replacement: Option<String>,
}

#[derive(Deserialize)]
struct FindMatch {
    m: String,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = &args[1];
    let content = fs::read_to_string(path).expect("Failed to read file");
    let mut passed = 0;
    let mut total = 0;
    let mut failures = Vec::new();

    for line in content.lines() {
        if line.trim().is_empty() { continue; }
        let test: TestCase = match serde_json::from_str(line) {
            Ok(t) => t,
            Err(_) => { total += 1; continue; }
        };
        total += 1;
        let flags_str = test.flags.as_deref().unwrap_or("");
        match test.op.as_str() {
            "compile_error" => {
                match Regex::with_flags(&test.pattern, flags_str) {
                    Err(_) => passed += 1,
                    Ok(_) => failures.push(format!("{}: expected compile error", test.id)),
                }
            }
            "matches" => {
                let regex = match Regex::with_flags(&test.pattern, flags_str) {
                    Ok(r) => r,
                    Err(e) => { failures.push(format!("{}: compile: {}", test.id, e)); continue; }
                };
                let input = test.input.as_deref().unwrap_or("");
                let expected = test.expect.as_bool().unwrap_or(false);
                if regex.matches(input) == expected { passed += 1; }
                else { failures.push(format!("{}: matches={}, expected={}", test.id, !expected, expected)); }
            }
            "find" => {
                let regex = match Regex::with_flags(&test.pattern, flags_str) {
                    Ok(r) => r,
                    Err(e) => { failures.push(format!("{}: compile: {}", test.id, e)); continue; }
                };
                let input = test.input.as_deref().unwrap_or("");
                let matches = regex.find(input);
                let expected: Vec<FindMatch> = match serde_json::from_value(test.expect.clone()) {
                    Ok(v) => v,
                    Err(_) => { failures.push(format!("{}: bad expect", test.id)); continue; }
                };
                if matches.len() == expected.len() && matches.iter().zip(expected.iter()).all(|(g, e)| g.matched_text == e.m) {
                    passed += 1;
                } else {
                    failures.push(format!("{}: find got {:?}, expected {:?}",
                        test.id,
                        matches.iter().map(|m| &m.matched_text).collect::<Vec<_>>(),
                        expected.iter().map(|m| &m.m).collect::<Vec<_>>()));
                }
            }
            "replaceAll" => {
                let regex = match Regex::with_flags(&test.pattern, flags_str) {
                    Ok(r) => r,
                    Err(e) => { failures.push(format!("{}: compile: {}", test.id, e)); continue; }
                };
                let input = test.input.as_deref().unwrap_or("");
                let replacement = test.replacement.as_deref().unwrap_or("");
                let expected = test.expect.as_str().unwrap_or("");
                let result = regex.replace_all(input, replacement);
                if result == expected { passed += 1; }
                else { failures.push(format!("{}: replaceAll={:?}, expected={:?}", test.id, result, expected)); }
            }
            "split" => {
                let regex = match Regex::with_flags(&test.pattern, flags_str) {
                    Ok(r) => r,
                    Err(e) => { failures.push(format!("{}: compile: {}", test.id, e)); continue; }
                };
                let input = test.input.as_deref().unwrap_or("");
                let expected: Vec<String> = match serde_json::from_value(test.expect.clone()) {
                    Ok(v) => v,
                    Err(_) => { failures.push(format!("{}: bad expect", test.id)); continue; }
                };
                let result = regex.split(input);
                if result == expected { passed += 1; }
                else { failures.push(format!("{}: split={:?}, expected={:?}", test.id, result, expected)); }
            }
            _ => {}
        }
    }

    println!("{}: {}/{}", path, passed, total);
    for f in failures.iter().take(100) {
        println!("  {}", f);
    }
}
