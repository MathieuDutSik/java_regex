//! Rust implementation of Java's `java.util.regex.Pattern` API (Java 8+, targeting Java 13+ semantics).
//!
//! Supports the full Java regex syntax including Unicode properties, lookahead/lookbehind,
//! atomic groups, possessive quantifiers, backreferences, and all standard flags.
//! The `CANON_EQ` (canonical equivalence) flag is not supported.
//!
//! # Examples
//!
//! ```
//! use java_regex::Regex;
//!
//! let re = Regex::new(r"\d+").unwrap();
//! assert!(re.matches("123"));
//! assert_eq!(re.find("a1b22c").len(), 2);
//! assert_eq!(re.replace_all("a1b2", "#"), "a#b#");
//! ```

mod types;
mod unicode;
mod parser;
mod engine;

#[doc(hidden)]
pub mod gen;

use std::collections::HashMap;
use std::fmt;

pub use types::{PatternSyntaxError, MatchInfo};
use types::*;
use engine::{Engine, State};
use parser::Parser;

/// A compiled Java-compatible regular expression.
///
/// Create with [`Regex::new`] or [`Regex::with_flags`], then use
/// [`matches`](Regex::matches), [`find`](Regex::find), [`replace_all`](Regex::replace_all),
/// or [`split`](Regex::split) to apply it.
#[derive(Debug, Clone)]
pub struct Regex {
    source: String,
    pattern: Pattern,
    flags: Flags,
    group_count: usize,
    named_groups: HashMap<String, usize>,
}

impl fmt::Display for Regex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl Regex {
    /// Compile a regex pattern with default flags.
    pub fn new(pattern: &str) -> Result<Self, PatternSyntaxError> {
        Self::with_flags(pattern, "")
    }

    /// Compile a regex pattern with the given flags string.
    ///
    /// Supported flag characters:
    /// - `i` — case-insensitive matching
    /// - `m` — multiline mode (`^` and `$` match line boundaries)
    /// - `s` — dotall mode (`.` matches line terminators)
    /// - `x` — comments mode (whitespace and `#` comments ignored)
    /// - `u` — Unicode-aware case folding
    /// - `U` — Unicode character classes for POSIX properties
    /// - `d` — Unix lines mode (only `\n` is a line terminator)
    /// - `l` — literal mode (pattern is treated as a literal string)
    pub fn with_flags(pattern: &str, flags_str: &str) -> Result<Self, PatternSyntaxError> {
        let mut flags = Flags::default();
        for c in flags_str.chars() {
            match c {
                'i' => flags.case_insensitive = true,
                'm' => flags.multiline = true,
                's' => flags.dotall = true,
                'x' => flags.comments = true,
                'U' => flags.unicode_class = true,
                'd' => flags.unix_lines = true,
                'u' => flags.unicode_case = true,
                'l' => flags.literal = true,
                _ => {}
            }
        }

        let parsed_pattern;
        let group_count;
        let named_groups;

        if flags.literal {
            // LITERAL mode: treat the entire pattern as literal text
            let nodes: Vec<Node> = pattern.chars().map(Node::Literal).collect();
            parsed_pattern = Pattern { branches: vec![nodes] };
            group_count = 0;
            named_groups = HashMap::new();
        } else {
            let parser = Parser::new(pattern, flags);
            let result = parser.parse()?;
            parsed_pattern = result.0;
            group_count = result.1;
            named_groups = result.2;
        }

        Ok(Regex {
            source: pattern.to_string(),
            pattern: parsed_pattern,
            flags,
            group_count,
            named_groups,
        })
    }

    /// Returns the source pattern string.
    pub fn pattern(&self) -> &str {
        &self.source
    }

    /// Returns a literal pattern string that would match the given string.
    ///
    /// Equivalent to Java's `Pattern.quote()`. Wraps the string in `\Q...\E`.
    pub fn quote(s: &str) -> String {
        format!("\\Q{}\\E", s.replace("\\E", "\\E\\\\E\\Q"))
    }

    /// Returns true if the pattern matches the entire input.
    pub fn matches(&self, input: &str) -> bool {
        let input_chars: Vec<char> = input.chars().collect();
        let mut engine = Engine::new(&input_chars, self.flags, self.group_count, &self.named_groups);
        let mut state = State::new(self.group_count);
        let end_anchor = vec![Node::Anchor(AnchorKind::EndOfInput)];
        engine.match_pattern(&self.pattern, &end_anchor, 0, &mut state)
    }

    /// Returns true if the pattern matches the beginning of the input.
    /// Unlike `matches()`, the pattern does not need to match the entire input.
    pub fn looking_at(&self, input: &str) -> Option<MatchInfo> {
        let input_chars: Vec<char> = input.chars().collect();
        let mut engine = Engine::new(&input_chars, self.flags, self.group_count, &self.named_groups);
        if let Some((end_pos, captures)) = engine.try_match_at(&self.pattern, 0) {
            Some(self.build_match_info(&input_chars, 0, end_pos, &captures))
        } else {
            None
        }
    }

    /// Find all non-overlapping matches in the input.
    pub fn find(&self, input: &str) -> Vec<MatchInfo> {
        self.find_in_region(input, 0, None)
    }

    /// Find all non-overlapping matches within a region of the input.
    ///
    /// `start` and `end` are char indices. Only text in `[start, end)` is considered.
    /// Anchors like `^` and `$` respect the region boundaries.
    pub fn find_in_region(&self, input: &str, start: usize, end: Option<usize>) -> Vec<MatchInfo> {
        let all_chars: Vec<char> = input.chars().collect();
        let end = end.unwrap_or(all_chars.len());
        let region_chars: Vec<char> = all_chars[start..end].to_vec();
        let results = self.find_iter_impl(&region_chars, 0);
        // Adjust positions by start offset
        if start == 0 {
            results
        } else {
            results.into_iter().map(|mut m: MatchInfo| {
                m.start += start;
                m.end += start;
                m.group_positions = m.group_positions.into_iter().map(|gp: Option<(usize, usize)>| {
                    gp.map(|(s, e)| (s + start, e + start))
                }).collect();
                m
            }).collect()
        }
    }

    /// Find the first match starting at or after `start` (char index).
    ///
    /// Returns `None` if no match is found from that position onward.
    pub fn find_at(&self, input: &str, start: usize) -> Option<MatchInfo> {
        let input_chars: Vec<char> = input.chars().collect();
        let input_len = input_chars.len();
        let mut search_pos = start;
        while search_pos <= input_len {
            let mut engine = Engine::new(&input_chars, self.flags, self.group_count, &self.named_groups);
            engine.search_start = start;
            if let Some((end_pos, captures)) = engine.try_match_at(&self.pattern, search_pos) {
                return Some(self.build_match_info(&input_chars, search_pos, end_pos, &captures));
            }
            search_pos += 1;
        }
        None
    }

    fn find_iter_impl(&self, input_chars: &[char], start: usize) -> Vec<MatchInfo> {
        let mut results = Vec::new();
        let input_len = input_chars.len();
        let mut search_pos = start;
        let mut prev_match_end = start;

        while search_pos <= input_len {
            let mut engine = Engine::new(input_chars, self.flags, self.group_count, &self.named_groups);
            engine.search_start = prev_match_end;

            if let Some((end_pos, captures)) = engine.try_match_at(&self.pattern, search_pos) {
                results.push(self.build_match_info(input_chars, search_pos, end_pos, &captures));

                prev_match_end = end_pos;
                if end_pos == search_pos {
                    search_pos += 1;
                } else {
                    search_pos = end_pos;
                }
            } else {
                search_pos += 1;
            }
        }

        results
    }

    fn build_match_info(&self, input_chars: &[char], start: usize, end: usize,
                        captures: &[Option<(usize, usize)>]) -> MatchInfo {
        let matched_text: String = input_chars[start..end].iter().collect();

        let mut groups = Vec::new();
        let mut group_positions = Vec::new();
        for i in 1..=self.group_count {
            if let Some(Some((s, e))) = captures.get(i) {
                groups.push(Some(input_chars[*s..*e].iter().collect::<String>()));
                group_positions.push(Some((*s, *e)));
            } else {
                groups.push(None);
                group_positions.push(None);
            }
        }

        let mut named = HashMap::new();
        for (name, &idx) in &self.named_groups {
            if let Some(Some((s, e))) = captures.get(idx) {
                named.insert(name.clone(), input_chars[*s..*e].iter().collect::<String>());
            }
        }

        MatchInfo {
            matched_text,
            start,
            end,
            groups,
            group_positions,
            named_groups: named,
        }
    }

    /// Replace all matches with the replacement string.
    pub fn replace_all(&self, input: &str, replacement: &str) -> String {
        let input_chars: Vec<char> = input.chars().collect();
        self.replace_internal(&input_chars, replacement, false)
    }

    /// Replace the first match with the replacement string.
    pub fn replace_first(&self, input: &str, replacement: &str) -> String {
        let input_chars: Vec<char> = input.chars().collect();
        self.replace_internal(&input_chars, replacement, true)
    }

    /// Replace all matches using a callback function.
    ///
    /// The callback receives a [`MatchInfo`] for each match and returns the replacement string.
    pub fn replace_all_with<F>(&self, input: &str, f: F) -> String
    where F: Fn(&MatchInfo) -> String {
        let input_chars: Vec<char> = input.chars().collect();
        self.replace_with_internal(&input_chars, &f, false)
    }

    /// Replace the first match using a callback function.
    ///
    /// The callback receives a [`MatchInfo`] and returns the replacement string.
    pub fn replace_first_with<F>(&self, input: &str, f: F) -> String
    where F: Fn(&MatchInfo) -> String {
        let input_chars: Vec<char> = input.chars().collect();
        self.replace_with_internal(&input_chars, &f, true)
    }

    fn replace_with_internal<F>(&self, input_chars: &[char], f: &F, first_only: bool) -> String
    where F: Fn(&MatchInfo) -> String {
        let input_len = input_chars.len();
        let mut result = String::new();
        let mut last_end = 0;
        let mut search_pos = 0;
        let mut prev_match_end = 0;

        while search_pos <= input_len {
            let mut engine = Engine::new(input_chars, self.flags, self.group_count, &self.named_groups);
            engine.search_start = prev_match_end;

            if let Some((end_pos, captures)) = engine.try_match_at(&self.pattern, search_pos) {
                result.extend(&input_chars[last_end..search_pos]);
                let info = self.build_match_info(input_chars, search_pos, end_pos, &captures);
                result.push_str(&f(&info));

                last_end = end_pos;
                prev_match_end = end_pos;
                if end_pos == search_pos {
                    if search_pos < input_len {
                        result.push(input_chars[search_pos]);
                        last_end = search_pos + 1;
                    }
                    search_pos += 1;
                } else {
                    search_pos = end_pos;
                }

                if first_only { break; }
            } else {
                search_pos += 1;
            }
        }

        result.extend(&input_chars[last_end..]);
        result
    }

    fn replace_internal(&self, input_chars: &[char], replacement: &str, first_only: bool) -> String {
        let input_len = input_chars.len();
        let mut result = String::new();
        let mut last_end = 0;
        let mut search_pos = 0;

        let mut prev_match_end = 0;
        while search_pos <= input_len {
            let mut engine = Engine::new(input_chars, self.flags, self.group_count, &self.named_groups);
            engine.search_start = prev_match_end;

            if let Some((end_pos, mut captures)) = engine.try_match_at(&self.pattern, search_pos) {
                result.extend(&input_chars[last_end..search_pos]);

                if !captures.is_empty() {
                    captures[0] = Some((search_pos, end_pos));
                }

                let replaced = self.build_replacement(replacement, &captures, input_chars);
                result.push_str(&replaced);

                last_end = end_pos;
                prev_match_end = end_pos;
                if end_pos == search_pos {
                    if search_pos < input_len {
                        result.push(input_chars[search_pos]);
                        last_end = search_pos + 1;
                    }
                    search_pos += 1;
                } else {
                    search_pos = end_pos;
                }

                if first_only {
                    break;
                }
            } else {
                search_pos += 1;
            }
        }

        result.extend(&input_chars[last_end..]);
        result
    }

    fn build_replacement(&self, replacement: &str, captures: &[Option<(usize, usize)>], input_chars: &[char]) -> String {
        let mut result = String::new();
        let rep_chars: Vec<char> = replacement.chars().collect();
        let mut i = 0;

        while i < rep_chars.len() {
            if rep_chars[i] == '\\' && i + 1 < rep_chars.len() {
                result.push(rep_chars[i + 1]);
                i += 2;
            } else if rep_chars[i] == '$' {
                i += 1;
                if i < rep_chars.len() && rep_chars[i] == '{' {
                    i += 1;
                    let mut name = String::new();
                    while i < rep_chars.len() && rep_chars[i] != '}' {
                        name.push(rep_chars[i]);
                        i += 1;
                    }
                    if i < rep_chars.len() { i += 1; }
                    if let Some(&idx) = self.named_groups.get(&name) {
                        if let Some(Some((s, e))) = captures.get(idx) {
                            result.extend(&input_chars[*s..*e]);
                        }
                    }
                } else if i < rep_chars.len() && rep_chars[i].is_ascii_digit() {
                    let mut num = (rep_chars[i] as u32 - '0' as u32) as usize;
                    i += 1;
                    while i < rep_chars.len() && rep_chars[i].is_ascii_digit() {
                        let new_num = num * 10 + (rep_chars[i] as u32 - '0' as u32) as usize;
                        if new_num <= self.group_count {
                            num = new_num;
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    if let Some(Some((s, e))) = captures.get(num) {
                        result.extend(&input_chars[*s..*e]);
                    }
                } else {
                    result.push('$');
                }
            } else {
                result.push(rep_chars[i]);
                i += 1;
            }
        }

        result
    }

    /// Split the input by pattern matches (Java String.split semantics, limit=0).
    pub fn split(&self, input: &str) -> Vec<String> {
        self.split_with_limit(input, 0)
    }

    /// Split the input by pattern matches with a limit parameter.
    /// - limit > 0: at most `limit` parts, last part contains the remainder
    /// - limit == 0: no limit, trailing empty strings removed (Java default)
    /// - limit < 0: no limit, trailing empty strings preserved
    pub fn split_with_limit(&self, input: &str, limit: i32) -> Vec<String> {
        let input_chars: Vec<char> = input.chars().collect();
        let input_len = input_chars.len();
        let mut parts = Vec::new();
        let mut index = 0;
        let mut search_pos = 0;

        let mut prev_match_end = 0;
        while search_pos <= input_len {
            if limit > 0 && parts.len() as i32 >= limit - 1 {
                break;
            }

            let mut engine = Engine::new(&input_chars, self.flags, self.group_count, &self.named_groups);
            engine.search_start = prev_match_end;

            if let Some((end_pos, _captures)) = engine.try_match_at(&self.pattern, search_pos) {
                // Java quirk: a zero-width match at position 0 produces NO leading
                // empty substring. OpenJDK's Pattern.split has the explicit check:
                //     if (index == 0 && index == m.start() && m.start() == m.end()) continue;
                if index == 0 && search_pos == 0 && end_pos == 0 {
                    prev_match_end = end_pos;
                    search_pos = 1;
                    continue;
                }
                parts.push(input_chars[index..search_pos].iter().collect());
                index = end_pos;
                prev_match_end = end_pos;
                if end_pos == search_pos {
                    search_pos += 1;
                } else {
                    search_pos = end_pos;
                }
            } else {
                search_pos += 1;
            }
        }

        parts.push(input_chars[index..].iter().collect());

        // Java limit=0: remove trailing empty strings
        if limit == 0 {
            while parts.last().is_some_and(|s: &String| s.is_empty()) {
                parts.pop();
            }
        }

        if parts.is_empty() && index == 0 {
            parts.push(input.to_string());
        }

        parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_match() {
        let regex = Regex::new("abc").unwrap();
        assert!(regex.matches("abc"));
        assert!(!regex.matches("zabc"));
    }

    #[test]
    fn test_find() {
        let regex = Regex::new("abc").unwrap();
        let matches = regex.find("zabc");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_text, "abc");
    }

    #[test]
    fn test_escaped_dot() {
        let regex = Regex::new("\\.").unwrap();
        assert!(regex.matches("."));
        assert!(!regex.matches("a"));
    }

    #[test]
    fn test_escaped_backslash() {
        let regex = Regex::new("\\\\").unwrap();
        assert!(regex.matches("\\"));
    }

    #[test]
    fn test_quantifiers() {
        let regex = Regex::new("a{3}").unwrap();
        let matches = regex.find("aaab");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_text, "aaa");
    }

    #[test]
    fn test_groups() {
        let regex = Regex::new("(a)(b)(c)").unwrap();
        let matches = regex.find("abc");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].groups, vec![Some("a".to_string()), Some("b".to_string()), Some("c".to_string())]);
    }

    #[test]
    fn test_alternative() {
        let regex = Regex::new("cat|dog").unwrap();
        assert!(regex.matches("dog"));
        assert!(regex.matches("cat"));
        assert!(!regex.matches("bird"));
    }

    #[test]
    fn test_word_boundary() {
        let regex = Regex::new("\\bcat\\b").unwrap();
        let matches = regex.find("a cat! bobcat cat_");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_text, "cat");
    }

    #[test]
    fn test_multiline() {
        let regex = Regex::with_flags("^abc", "m").unwrap();
        let matches = regex.find("abc\nabc");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_dotall() {
        let regex = Regex::with_flags("a.*b", "s").unwrap();
        assert!(regex.matches("a\nb"));
    }

    #[test]
    fn test_quoted() {
        let regex = Regex::new("\\Q.*+?\\E").unwrap();
        assert!(regex.matches(".*+?"));
        assert!(!regex.matches("abc"));
    }

    #[test]
    fn test_char_class() {
        let regex = Regex::new("[abc]+").unwrap();
        assert!(regex.matches("abcba"));
        assert!(!regex.matches("def"));
    }

    #[test]
    fn test_negated_char_class() {
        let regex = Regex::new("[^abc]+").unwrap();
        assert!(regex.matches("def"));
        assert!(!regex.matches("abc"));
    }

    #[test]
    fn test_char_class_range() {
        let regex = Regex::new("[a-z]+").unwrap();
        assert!(regex.matches("hello"));
        assert!(!regex.matches("HELLO"));
    }

    #[test]
    fn test_char_class_intersection() {
        let regex = Regex::new("[a-z&&[^aeiou]]+").unwrap();
        assert!(regex.matches("bcdfg"));
        assert!(!regex.matches("aei"));
    }

    #[test]
    fn test_predefined_classes() {
        assert!(Regex::new("\\d+").unwrap().matches("123"));
        assert!(Regex::new("\\w+").unwrap().matches("hello_123"));
        assert!(Regex::new("\\s+").unwrap().matches("  \t"));
    }

    #[test]
    fn test_backreference() {
        let regex = Regex::new("(\\w+)\\s+\\1").unwrap();
        assert!(regex.matches("hello hello"));
        assert!(!regex.matches("hello world"));
    }

    #[test]
    fn test_lookahead() {
        let regex = Regex::new("\\w+(?=:)").unwrap();
        let matches = regex.find("key:value");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_text, "key");
    }

    #[test]
    fn test_lookbehind() {
        let regex = Regex::new("(?<=:)\\w+").unwrap();
        let matches = regex.find("key:value");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_text, "value");
    }

    #[test]
    fn test_atomic_group() {
        let regex = Regex::new("(?>a|ab)c").unwrap();
        assert!(!regex.matches("abc"));
    }

    #[test]
    fn test_case_insensitive() {
        let regex = Regex::with_flags("abc", "i").unwrap();
        assert!(regex.matches("ABC"));
        assert!(regex.matches("aBc"));
    }

    #[test]
    fn test_possessive() {
        let regex = Regex::new("a.*+b").unwrap();
        assert!(!regex.matches("a123b"));
    }

    #[test]
    fn test_reluctant() {
        let regex = Regex::new("<.*?>").unwrap();
        let matches = regex.find("<b>bold</b>");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].matched_text, "<b>");
        assert_eq!(matches[1].matched_text, "</b>");
    }

    #[test]
    fn test_replace_all() {
        let regex = Regex::new("\\d+").unwrap();
        assert_eq!(regex.replace_all("a1b22c333", "#"), "a#b#c#");
    }

    #[test]
    fn test_replace_with_groups() {
        let regex = Regex::new("(\\w+),(\\w+)").unwrap();
        assert_eq!(regex.replace_all("Doe,John", "$2 $1"), "John Doe");
    }

    #[test]
    fn test_split() {
        let regex = Regex::new("\\s*,\\s*").unwrap();
        assert_eq!(regex.split("a, b,  c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_compile_error_duplicate_name() {
        assert!(Regex::new("(?<x>a)(?<x>b)").is_err());
    }

    #[test]
    fn test_compile_error_bad_range() {
        assert!(Regex::new("[z-a]").is_err());
    }

    #[test]
    fn test_hex_escape() {
        let regex = Regex::new("\\x41+").unwrap();
        assert!(regex.matches("AAA"));
    }

    #[test]
    fn test_unicode_escape() {
        let regex = Regex::new("\\u0041+").unwrap();
        assert!(regex.matches("AAA"));
    }

    #[test]
    fn test_empty_match() {
        let regex = Regex::new(".*").unwrap();
        assert!(regex.matches(""));
        assert!(regex.matches("anything"));
    }

    #[test]
    fn test_group_quantifier() {
        let regex = Regex::new("(ab){2,3}").unwrap();
        assert!(regex.matches("abab"));
        assert!(regex.matches("ababab"));
        assert!(!regex.matches("ab"));
    }

    #[test]
    fn test_comments_mode() {
        let regex = Regex::new("(?x)a \\s+ b # comment").unwrap();
        assert!(regex.matches("a   b"));
    }

    #[test]
    fn test_linebreak() {
        let regex = Regex::new("\\R").unwrap();
        let matches = regex.find("a\r\nb\nc\rd");
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].matched_text, "\r\n");
        assert_eq!(matches[1].matched_text, "\n");
        assert_eq!(matches[2].matched_text, "\r");
    }

    #[test]
    fn test_named_groups() {
        let regex = Regex::new("(?<last>\\w+),(?<first>\\w+)").unwrap();
        let result = regex.replace_all("Doe,John", "${first} ${last}");
        assert_eq!(result, "John Doe");
    }

    #[test]
    fn test_octal() {
        let regex = Regex::new("\\012").unwrap();
        assert!(regex.matches("\n"));
    }

    #[test]
    fn test_control_escape() {
        let regex = Regex::new("\\cJ").unwrap();
        assert!(regex.matches("\n"));
    }

    #[test]
    fn test_nested_char_class_with_colon() {
        let regex = Regex::new("[[:digit:]]+").unwrap();
        assert!(regex.matches("dig:t"));
        assert!(!regex.matches("023"));
    }

    #[test]
    fn test_anchors() {
        let regex = Regex::new("\\Aabc").unwrap();
        let matches = regex.find("abc\nxabc");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_text, "abc");
    }

    #[test]
    fn test_inline_flags() {
        let regex = Regex::new("(?i)abc").unwrap();
        assert!(regex.matches("ABC"));
    }

    #[test]
    fn test_non_capturing_group() {
        let regex = Regex::new("(?:ab)+c").unwrap();
        assert!(regex.matches("ababc"));
        let matches = regex.find("ababc");
        assert_eq!(matches[0].groups.len(), 0);
    }

    #[test]
    fn test_star_empty() {
        let regex = Regex::new("a*").unwrap();
        assert!(regex.matches(""));
        assert!(regex.matches("aaa"));
    }

    #[test]
    fn test_negative_lookahead() {
        let regex = Regex::new("foo(?!bar)").unwrap();
        let matches = regex.find("foobaz foobar");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_text, "foo");
    }

    #[test]
    fn test_backtracking_alternation() {
        let regex = Regex::new("(a|ab)c").unwrap();
        assert!(regex.matches("abc"));
    }

    #[test]
    fn test_unicode_property() {
        let regex = Regex::new("\\p{L}+").unwrap();
        let matches = regex.find("Grüße");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_text, "Grüße");
    }

    #[test]
    fn test_variable_lookbehind() {
        let regex = Regex::new("(?<=\\w+)\\d+").unwrap();
        let matches = regex.find("a1");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_text, "1");
    }

    #[test]
    fn test_looking_at() {
        let regex = Regex::new("\\d+").unwrap();
        let m = regex.looking_at("123abc");
        assert!(m.is_some());
        assert_eq!(m.unwrap().matched_text, "123");

        assert!(regex.looking_at("abc123").is_none());
    }

    #[test]
    fn test_replace_first() {
        let regex = Regex::new("\\d+").unwrap();
        assert_eq!(regex.replace_first("a1b2c3", "#"), "a#b2c3");
    }

    #[test]
    fn test_replace_first_with_groups() {
        let regex = Regex::new("(\\w+),(\\w+)").unwrap();
        assert_eq!(regex.replace_first("a,b and c,d", "$2,$1"), "b,a and c,d");
    }

    #[test]
    fn test_split_with_limit() {
        let regex = Regex::new(",").unwrap();
        assert_eq!(regex.split_with_limit("a,b,c,d", 2), vec!["a", "b,c,d"]);
        assert_eq!(regex.split_with_limit("a,b,c,d", 3), vec!["a", "b", "c,d"]);
        assert_eq!(regex.split_with_limit("a,b,c,d", -1), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn test_split_with_limit_trailing_empty() {
        let regex = Regex::new(",").unwrap();
        // limit=0 (default): trailing empties removed
        assert_eq!(regex.split("a,,b,"), vec!["a", "", "b"]);
        // limit=-1: trailing empties preserved
        assert_eq!(regex.split_with_limit("a,,b,", -1), vec!["a", "", "b", ""]);
    }

    #[test]
    fn test_group_positions() {
        let regex = Regex::new("(\\w+)@(\\w+)").unwrap();
        let matches = regex.find("user@host");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].group_positions, vec![Some((0, 4)), Some((5, 9))]);
    }

    #[test]
    fn test_backtracking_limit() {
        // Catastrophic backtracking pattern — should not hang
        let regex = Regex::new("(a+)+b").unwrap();
        // This would take exponential time without step limits
        assert!(!regex.matches("aaaaaaaaaaaaaaaaaa"));
    }

    #[test]
    fn test_literal_flag() {
        let regex = Regex::with_flags("a.b+c", "l").unwrap();
        assert!(regex.matches("a.b+c"));
        assert!(!regex.matches("axbc"));
    }

    #[test]
    fn test_quote() {
        let quoted = Regex::quote("hello.world*");
        let regex = Regex::new(&quoted).unwrap();
        assert!(regex.matches("hello.world*"));
        assert!(!regex.matches("helloXworld"));
    }

    #[test]
    fn test_find_at() {
        let regex = Regex::new("\\d+").unwrap();
        let m = regex.find_at("a1b22c333", 3);
        assert!(m.is_some());
        assert_eq!(m.unwrap().matched_text, "22");
    }

    #[test]
    fn test_find_in_region() {
        let regex = Regex::new("\\d+").unwrap();
        let matches = regex.find_in_region("a1b22c333d", 2, Some(9));
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].matched_text, "22");
        assert_eq!(matches[0].start, 3);
        assert_eq!(matches[1].matched_text, "333");
        assert_eq!(matches[1].start, 6);
    }

    #[test]
    fn test_replace_all_with() {
        let regex = Regex::new("\\d+").unwrap();
        let result = regex.replace_all_with("a1b22c333", |m| {
            format!("[{}]", m.matched_text.len())
        });
        assert_eq!(result, "a[1]b[2]c[3]");
    }

    #[test]
    fn test_replace_first_with() {
        let regex = Regex::new("\\w+").unwrap();
        let result = regex.replace_first_with("hello world", |m| {
            m.matched_text.to_uppercase()
        });
        assert_eq!(result, "HELLO world");
    }

    #[test]
    fn test_display() {
        let regex = Regex::new("\\d+").unwrap();
        assert_eq!(format!("{}", regex), "\\d+");
    }

    #[test]
    fn test_pattern() {
        let regex = Regex::new("(?i)abc").unwrap();
        assert_eq!(regex.pattern(), "(?i)abc");
    }

    // -- regressions found by examples/diff_fuzz.rs against OpenJDK 25 -------

    #[test]
    fn test_split_zero_width_at_start_no_leading_empty() {
        // OpenJDK Pattern.split skips a zero-width match at position 0.
        let r = Regex::with_flags(r"\Q\E", "i").unwrap();
        assert_eq!(r.split("\t"), vec!["\t"]);
        let r = Regex::new("(?:)").unwrap();
        assert_eq!(r.split("\r"), vec!["\r"]);
    }

    #[test]
    fn test_multiline_caret_not_at_end_of_input() {
        // Java's multiline ^ never matches at end of input (Perl/Java quirk).
        let r = Regex::with_flags("^", "m").unwrap();
        assert!(!r.matches(""));
        let r = Regex::with_flags("^", "m").unwrap();
        let ms = r.find("\r\ré\n");
        assert_eq!(ms.len(), 3, "expected matches at positions 0, 1, 2 only");
        assert_eq!(ms.iter().map(|m| m.start).collect::<Vec<_>>(), vec![0, 1, 2]);
    }

    #[test]
    fn test_quantified_deterministic_atom_is_atomic() {
        // OpenJDK's Curly/GroupCurly does not backtrack into a quantified atom
        // when the atom's body is "deterministic" (no top-level alternation,
        // no variable-count nested quantifier). So `\R{2}` on "\r\n" does NOT
        // match — iter 1 takes the longer \r\n branch, iter 2 has nothing left,
        // and the engine doesn't retry iter 1 with the shorter \r-only branch.
        // The same applies when \R is wrapped in any kind of non-alternation
        // group, including non-capturing, capturing, atomic, and flag groups.
        for pat in &[
            r"\R{2}",
            r"(?:\R){2}",
            r"(\R){2}",
            r"(?>\R){2}",
            r"(?i:\R){2}",
            r"(?im:\R){2}",
        ] {
            assert!(!Regex::new(pat).unwrap().matches("\r\n"),
                "expected no match for {pat:?}");
        }
    }

    #[test]
    fn test_quantified_non_deterministic_atom_backtracks() {
        // Multi-branch (alternation) bodies stay non-atomic — Loop in OpenJDK,
        // continuation-based backtracking here.
        assert!(Regex::new(r"(?:a|aa){2}").unwrap().matches("aaa"));
        assert!(Regex::new(r"(a|aa){2}").unwrap().matches("aaaa"));
        assert!(Regex::new(r"(?:ab|a){2}").unwrap().matches("aab"));
        // Mix: alternation between deterministic atoms (one of which is \R).
        assert!(Regex::new(r"(?:a|\R){2}").unwrap().matches("a\r\n"));
    }

    #[test]
    fn test_linebreak_is_not_atomic() {
        // \R is documented as `\r\n|[line-break-chars]` (a regular alternation,
        // not an atomic group). Java accepts `\R\n` matching "\r\n" because the
        // first \R backtracks from `\r\n` to just `\r`, letting the trailing
        // `\n` succeed. The previous implementation was atomic and rejected.
        let r = Regex::new(r"\R\n").unwrap();
        assert!(r.matches("\r\n"));
        // (?<!\R) at position 1 of "\r\n" should fail: \r alone is a \R.
        let r = Regex::new(r"(?<!\R)").unwrap();
        let ms = r.find("\r\n");
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].start, 0);
    }

    #[test]
    fn test_inline_flags_propagate_across_alternation_branches() {
        // Java treats inline `(?s)` as a compile-time directive: any branch
        // parsed after it sees the flag, regardless of whether the branch
        // containing it actually matches at run time. So `(?s)|.` matches "\n"
        // (branch 1 sets dotall, branch 2's `.` then matches `\n`).
        let r = Regex::new(r"(?s)|.").unwrap();
        assert!(r.matches("\n"));
        let r = Regex::new(r"(?s)xx|.").unwrap();
        assert!(r.matches("\n"));
        // Inside a scoped FlagGroup, the propagation still applies within scope:
        let r = Regex::new(r"(?iu:(?s)|(?:.).\R)").unwrap();
        assert!(r.matches("\nΑ\r"));
    }

    #[test]
    fn test_chained_intersection_mirrors_openjdk_quirk() {
        // In `[A && [P]x && C]` where the middle operand contains a nested
        // class followed by a literal, OpenJDK does NOT chain the trailing
        // `&& C` at the outer level — the literal triggers a recursive
        // sub-class scope that absorbs `x && C`, leaving the outer intersection
        // as `A && ([P] ∪ (x && C))`. For `[abc && [\w]a && z]` this means
        // the result is `{a,b,c}` (not empty, as a straightforward 3-way
        // intersection would give). Mirroring OpenJDK is intentional.
        let r = Regex::new(r"[abc&&[\w]a&&z]").unwrap();
        let texts: Vec<_> = r.find("abcxz").into_iter().map(|m| m.matched_text).collect();
        assert_eq!(texts, vec!["a", "b", "c"]);
        // Sanity: when no literal sits between the nested class and the next &&,
        // the chain IS preserved (proper 3-way intersection, result is empty).
        let r = Regex::new(r"[abc&&[\w]&&z]").unwrap();
        assert!(r.find("abcz").is_empty());
    }

    #[test]
    fn test_case_insensitive_range_outside_ascii_bounds() {
        // `[1-c]` with /i should match 'g' because uppercase 'G' (0x47)
        // is inside the range 0x31..0x63.
        let r = Regex::with_flags("[1-c]", "i").unwrap();
        assert!(r.matches("g"));
        assert!(r.matches("G"));
        let r = Regex::with_flags("[=-e]", "i").unwrap();
        assert!(r.matches("f"));
        // Sanity: a range that doesn't span any uppercase ASCII should still reject.
        let r = Regex::with_flags("[d-e]", "i").unwrap();
        assert!(!r.matches("g"));
    }
}
