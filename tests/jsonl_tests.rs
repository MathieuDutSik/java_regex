use java_regex::Regex;
use serde::Deserialize;
use std::fs;

#[derive(Deserialize, Debug)]
struct TestCase {
    id: String,
    pattern: String,
    input: Option<String>,
    op: String,
    expect: serde_json::Value,
    flags: Option<String>,
    replacement: Option<String>,
    #[allow(dead_code)]
    note: Option<String>,
}

#[derive(Deserialize, Debug)]
struct FindMatch {
    m: String,
    #[serde(default)]
    #[allow(dead_code)]
    g: Option<Vec<serde_json::Value>>,
}

fn run_test(test: &TestCase) -> Result<bool, String> {
    let flags_str = test.flags.as_deref().unwrap_or("");

    match test.op.as_str() {
        "compile_error" => {
            match Regex::with_flags(&test.pattern, flags_str) {
                Err(_) => Ok(true),
                Ok(_) => Err(format!("{}: Expected compile error but pattern compiled successfully", test.id)),
            }
        }
        "matches" => {
            let regex = match Regex::with_flags(&test.pattern, flags_str) {
                Ok(r) => r,
                Err(e) => return Err(format!("{}: Compile error: {}", test.id, e)),
            };
            let input = test.input.as_deref().unwrap_or("");
            let expected = test.expect.as_bool().unwrap_or(false);
            let result = regex.matches(input);
            if result == expected {
                Ok(true)
            } else {
                Err(format!("{}: matches({:?}) = {}, expected {}", test.id, input, result, expected))
            }
        }
        "find" => {
            let regex = match Regex::with_flags(&test.pattern, flags_str) {
                Ok(r) => r,
                Err(e) => return Err(format!("{}: Compile error: {}", test.id, e)),
            };
            let input = test.input.as_deref().unwrap_or("");
            let matches = regex.find(input);
            let expected: Vec<FindMatch> = serde_json::from_value(test.expect.clone())
                .map_err(|e| format!("{}: Bad expect format: {}", test.id, e))?;

            if matches.len() != expected.len() {
                return Err(format!(
                    "{}: find({:?}) got {} matches, expected {} (got: {:?}, expected: {:?})",
                    test.id, input, matches.len(), expected.len(),
                    matches.iter().map(|m| &m.matched_text).collect::<Vec<_>>(),
                    expected.iter().map(|m| &m.m).collect::<Vec<_>>(),
                ));
            }

            for (i, (got, exp)) in matches.iter().zip(expected.iter()).enumerate() {
                if got.matched_text != exp.m {
                    return Err(format!(
                        "{}: find match[{}] = {:?}, expected {:?}",
                        test.id, i, got.matched_text, exp.m,
                    ));
                }
            }

            Ok(true)
        }
        "replaceAll" => {
            let regex = match Regex::with_flags(&test.pattern, flags_str) {
                Ok(r) => r,
                Err(e) => return Err(format!("{}: Compile error: {}", test.id, e)),
            };
            let input = test.input.as_deref().unwrap_or("");
            let replacement = test.replacement.as_deref().unwrap_or("");
            let expected = test.expect.as_str().unwrap_or("");
            let result = regex.replace_all(input, replacement);
            if result == expected {
                Ok(true)
            } else {
                Err(format!("{}: replaceAll = {:?}, expected {:?}", test.id, result, expected))
            }
        }
        "split" => {
            let regex = match Regex::with_flags(&test.pattern, flags_str) {
                Ok(r) => r,
                Err(e) => return Err(format!("{}: Compile error: {}", test.id, e)),
            };
            let input = test.input.as_deref().unwrap_or("");
            let expected: Vec<String> = serde_json::from_value(test.expect.clone())
                .map_err(|e| format!("{}: Bad expect format: {}", test.id, e))?;
            let result = regex.split(input);
            if result == expected {
                Ok(true)
            } else {
                Err(format!("{}: split = {:?}, expected {:?}", test.id, result, expected))
            }
        }
        _ => Err(format!("{}: Unknown op: {}", test.id, test.op)),
    }
}

fn run_test_file(path: &str) -> (usize, usize, Vec<String>) {
    let content = fs::read_to_string(path).unwrap_or_else(|_| panic!("Failed to read {}", path));
    let mut passed = 0;
    let mut total = 0;
    let mut failures = Vec::new();

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let test: TestCase = match serde_json::from_str(line) {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!("Parse error: {} on line: {}", e, &line[..line.len().min(100)]));
                total += 1;
                continue;
            }
        };
        total += 1;
        match run_test(&test) {
            Ok(true) => passed += 1,
            Ok(false) => failures.push(format!("{}: returned false", test.id)),
            Err(e) => failures.push(e),
        }
    }

    (passed, total, failures)
}

#[test]
fn test_jsonl_105() {
    let (passed, total, failures) = run_test_file("java_regex_tests.jsonl");

    println!("\n=== java_regex_tests.jsonl ===");
    println!("Passed: {}/{}", passed, total);
    if !failures.is_empty() {
        println!("\nFailures ({}):", failures.len());
        for f in &failures {
            println!("  {}", f);
        }
    }

    // Require at least 80% pass rate
    let pass_rate = passed as f64 / total as f64;
    assert!(pass_rate >= 0.80, "Pass rate {:.1}% < 80%", pass_rate * 100.0);
}

#[test]
fn test_jsonl_5000() {
    let (passed, total, failures) = run_test_file("java_regex_tests_5000.jsonl");

    println!("\n=== java_regex_tests_5000.jsonl ===");
    println!("Passed: {}/{}", passed, total);
    if !failures.is_empty() {
        println!("\nFirst 50 failures:");
        for f in failures.iter().take(50) {
            println!("  {}", f);
        }
    }

    let pass_rate = passed as f64 / total as f64;
    assert!(pass_rate >= 0.80, "Pass rate {:.1}% < 80%", pass_rate * 100.0);
}
