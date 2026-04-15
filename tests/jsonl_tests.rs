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
                let truncated: String = line.chars().take(100).collect();
                failures.push(format!("Parse error: {} on line: {}", e, truncated));
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

fn run_and_assert(path: &str) {
    let (passed, total, failures) = run_test_file(path);

    println!("\n=== {} ===", path);
    println!("Passed: {}/{}", passed, total);
    if !failures.is_empty() {
        println!("\nFailures ({}):", failures.len());
        for f in failures.iter().take(200) {
            println!("  {}", f);
        }
    }

    let pass_rate = passed as f64 / total as f64;
    assert!(pass_rate >= 0.80, "{}: pass rate {:.1}% < 80%", path, pass_rate * 100.0);
}

#[test]
fn test_jsonl_105() { run_and_assert("tests/java_regex_tests.jsonl"); }

#[test]
fn test_jsonl_5000() { run_and_assert("tests/java_regex_tests_5000.jsonl"); }

#[test]
fn test_jsonl_new() { run_and_assert("tests/java_regex_tests_new.jsonl"); }

#[test]
fn test_jsonl_gen5() { run_and_assert("tests/java_regex_tests_gen5.jsonl"); }

#[test]
fn test_jsonl_gen6() { run_and_assert("tests/java_regex_tests_gen6.jsonl"); }

#[test]
fn test_jsonl_gen7() { run_and_assert("tests/java_regex_tests_gen7.jsonl"); }

#[test]
fn test_jsonl_gen8() { run_and_assert("tests/java_regex_tests_gen8.jsonl"); }

#[test]
fn test_jsonl_gen9() { run_and_assert("tests/java_regex_tests_gen9.jsonl"); }

#[test]
fn test_jsonl_gen10() { run_and_assert("tests/java_regex_tests_gen10.jsonl"); }

#[test]
fn test_jsonl_gen11() { run_and_assert("tests/java_regex_tests_gen11.jsonl"); }

#[test]
fn test_jsonl_gen12() { run_and_assert("tests/java_regex_tests_gen12.jsonl"); }

#[test]
fn test_jsonl_gen13() { run_and_assert("tests/java_regex_tests_gen13.jsonl"); }

#[test]
fn test_jsonl_gen14() { run_and_assert("tests/java_regex_tests_gen14.jsonl"); }

#[test]
fn test_jsonl_gen15() { run_and_assert("tests/java_regex_tests_gen15.jsonl"); }

#[test]
fn test_jsonl_gen16() { run_and_assert("tests/java_regex_tests_gen16.jsonl"); }

#[test]
fn test_jsonl_gen17() { run_and_assert("tests/java_regex_tests_gen17.jsonl"); }

#[test]
fn test_jsonl_gen18() { run_and_assert("tests/java_regex_tests_gen18.jsonl"); }

#[test]
fn test_jsonl_gen19() { run_and_assert("tests/java_regex_tests_gen19.jsonl"); }

#[test]
fn test_jsonl_gen20() { run_and_assert("tests/java_regex_tests_gen20.jsonl"); }

#[test]
fn test_openjdk() { run_and_assert("tests/java_regex_tests_openjdk.jsonl"); }

#[test]
fn test_diff() { run_and_assert("tests/java_regex_tests_diff.jsonl"); }

#[test]
fn test_diff2() { run_and_assert("tests/java_regex_tests_diff2.jsonl"); }
