mod types;
mod unicode;
mod parser;
mod engine;

use std::collections::HashMap;

pub use types::{PatternSyntaxError, MatchInfo};
use types::*;
use engine::{Engine, State};
use parser::Parser;

#[derive(Debug, Clone, PartialEq)]
pub struct MatchResult {
    pub matched: bool,
    pub matches: Vec<MatchInfo>,
}

#[derive(Debug, Clone)]
pub struct Regex {
    pattern: Pattern,
    flags: Flags,
    group_count: usize,
    named_groups: HashMap<String, usize>,
}

impl Regex {
    pub fn new(pattern: &str) -> Result<Self, PatternSyntaxError> {
        Self::with_flags(pattern, "")
    }

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
                _ => {}
            }
        }

        let parser = Parser::new(pattern, flags);
        let (parsed_pattern, group_count, named_groups) = parser.parse()?;

        Ok(Regex {
            pattern: parsed_pattern,
            flags,
            group_count,
            named_groups,
        })
    }

    /// Returns true if the pattern matches the entire input.
    pub fn matches(&self, input: &str) -> bool {
        let mut engine = Engine::new(input, self.flags, self.group_count, self.named_groups.clone());
        let mut state = State::new(self.group_count);
        let end_anchor = vec![Node::Anchor(AnchorKind::EndOfInput)];
        engine.match_pattern(&self.pattern, &end_anchor, 0, &mut state)
    }

    /// Find all non-overlapping matches.
    pub fn find(&self, input: &str) -> Vec<MatchInfo> {
        let mut results = Vec::new();
        let input_chars: Vec<char> = input.chars().collect();
        let input_len = input_chars.len();
        let mut search_pos = 0;
        let mut prev_match_end = 0usize;

        while search_pos <= input_len {
            let mut engine = Engine::new(input, self.flags, self.group_count, self.named_groups.clone());
            engine.search_start = prev_match_end;

            if let Some((end_pos, captures)) = engine.try_match_at(&self.pattern, search_pos) {
                let matched_text: String = input_chars[search_pos..end_pos].iter().collect();

                let mut groups = Vec::new();
                for i in 1..=self.group_count {
                    if let Some(Some((s, e))) = captures.get(i) {
                        groups.push(Some(input_chars[*s..*e].iter().collect()));
                    } else {
                        groups.push(None);
                    }
                }

                let mut named = HashMap::new();
                for (name, &idx) in &self.named_groups {
                    if let Some(Some((s, e))) = captures.get(idx) {
                        named.insert(name.clone(), input_chars[*s..*e].iter().collect());
                    }
                }

                results.push(MatchInfo {
                    matched_text,
                    start: search_pos,
                    end: end_pos,
                    groups,
                    named_groups: named,
                });

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

    /// Replace all matches with the replacement string.
    pub fn replace_all(&self, input: &str, replacement: &str) -> String {
        let input_chars: Vec<char> = input.chars().collect();
        let input_len = input_chars.len();
        let mut result = String::new();
        let mut last_end = 0;
        let mut search_pos = 0;

        while search_pos <= input_len {
            let mut engine = Engine::new(input, self.flags, self.group_count, self.named_groups.clone());

            if let Some((end_pos, mut captures)) = engine.try_match_at(&self.pattern, search_pos) {
                result.extend(&input_chars[last_end..search_pos]);

                if !captures.is_empty() {
                    captures[0] = Some((search_pos, end_pos));
                }

                let replaced = self.build_replacement(replacement, &captures, &input_chars);
                result.push_str(&replaced);

                last_end = end_pos;
                if end_pos == search_pos {
                    if search_pos < input_len {
                        result.push(input_chars[search_pos]);
                        last_end = search_pos + 1;
                    }
                    search_pos += 1;
                } else {
                    search_pos = end_pos;
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

    /// Split the input by pattern matches (Java String.split semantics).
    pub fn split(&self, input: &str) -> Vec<String> {
        let input_chars: Vec<char> = input.chars().collect();
        let input_len = input_chars.len();
        let mut parts = Vec::new();
        let mut last_end = 0;
        let mut search_pos = 0;

        while search_pos <= input_len {
            let mut engine = Engine::new(input, self.flags, self.group_count, self.named_groups.clone());

            if let Some((end_pos, _captures)) = engine.try_match_at(&self.pattern, search_pos) {
                if end_pos == search_pos && end_pos == last_end {
                    search_pos += 1;
                    continue;
                }
                parts.push(input_chars[last_end..search_pos].iter().collect());
                last_end = end_pos;
                if end_pos == search_pos {
                    search_pos += 1;
                } else {
                    search_pos = end_pos;
                }
            } else {
                search_pos += 1;
            }
        }

        parts.push(input_chars[last_end..].iter().collect());

        while parts.last().is_some_and(|s: &String| s.is_empty()) {
            parts.pop();
        }

        if parts.is_empty() && last_end == 0 {
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
}
