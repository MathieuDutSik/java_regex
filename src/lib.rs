
#[derive(Debug, Clone, PartialEq)]
pub enum Flag {
    Multiline,
    DotAll,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Regex {
    pattern: String,
    flags: Vec<Flag>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchResult {
    pub matched: bool,
    pub matches: Vec<MatchInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchInfo {
    pub matched_text: String,
    pub groups: Vec<String>,
}

impl Regex {
    pub fn new(pattern: &str) -> Self {
        Self {
            pattern: pattern.to_string(),
            flags: Vec::new(),
        }
    }

    pub fn with_flags(pattern: &str, flags: &[char]) -> Self {
        let mut regex = Self::new(pattern);
        for flag in flags {
            match flag {
                'm' => regex.flags.push(Flag::Multiline),
                's' => regex.flags.push(Flag::DotAll),
                _ => {}
            }
        }
        regex
    }

    pub fn matches(&self, input: &str) -> bool {
        let full_match = self.find_all(input);
        if full_match.matches.is_empty() {
            return false;
        }
        // For matches(), the entire input must be consumed
        full_match.matches.iter().all(|m| {
            m.matched_text.len() == input.len() ||
            (self.flags.contains(&Flag::Multiline) &&
             self.pattern.contains('^') &&
             self.pattern.contains('$'))
        })
    }

    pub fn find(&self, input: &str) -> Vec<MatchInfo> {
        self.find_all(input).matches
    }

    pub fn find_all(&self, input: &str) -> MatchResult {
        let matches = self.parse_and_match(input);
        MatchResult { matched: !matches.is_empty(), matches }
    }

    fn parse_and_match(&self, input: &str) -> Vec<MatchInfo> {
        let mut matches = Vec::new();

        // Handle special anchors
        let start_anchor = self.pattern.starts_with("\\A");
        let _end_anchor = self.pattern.ends_with("\\z");
        let _end_anchor_z = self.pattern.ends_with("\\Z");

        let start_pos = if start_anchor { 0 } else { 0 };

        for pos in start_pos..=input.len() {
            if pos >= input.len() && !self.pattern.is_empty() {
                break;
            }

            let remaining = &input[pos..];
            if let Some(result) = self.try_match_at(remaining, pos) {
                matches.push(result);
                // For non-overlapping matches, skip past this match
                if !matches.last().unwrap().matched_text.is_empty() {
                    let next_pos = pos + matches.last().unwrap().matched_text.len();
                    if next_pos > pos {
                        // Continue from next position
                        continue;
                    }
                }
            }
        }

        matches
    }

    fn try_match_at(&self, input: &str, _start_pos: usize) -> Option<MatchInfo> {
        let (matched, groups) = self.match_pattern(&self.pattern, input)?;

        if !matched.is_empty() {
            Some(MatchInfo {
                matched_text: matched,
                groups,
            })
        } else {
            None
        }
    }

    fn match_pattern(&self, pattern: &str, input: &str) -> Option<(String, Vec<String>)> {
        let mut pos = 0;
        let mut groups: Vec<String> = Vec::new();
        let mut result = String::new();

        let chars: Vec<char> = input.chars().collect();
        let pat_chars: Vec<char> = pattern.chars().collect();
        let mut i = 0;

        while i < pat_chars.len() {
            let c = pat_chars[i];

            match c {
                '^' => {
                    if pos > 0 {
                        return None;
                    }
                    i += 1;
                }
                '$' => {
                    if pos < chars.len() {
                        return None;
                    }
                    i += 1;
                }
                '\\' => {
                    if i + 1 >= pat_chars.len() {
                        return None;
                    }
                    let next = pat_chars[i + 1];
                    match next {
                        'A' => {
                            if pos > 0 {
                                return None;
                            }
                            i += 2;
                        }
                        'z' => {
                            if pos < chars.len() {
                                return None;
                            }
                            i += 2;
                        }
                        'Z' => {
                            if pos < chars.len() && (pos + 1 < chars.len() || chars[pos] != '\n') {
                                return None;
                            }
                            i += 2;
                        }
                        'b' => {
                            // Word boundary
                            let before_is_word = pos > 0 && is_word_char(chars[pos - 1]);
                            let after_is_word = pos < chars.len() && is_word_char(chars[pos]);
                            if before_is_word == after_is_word {
                                return None;
                            }
                            i += 2;
                        }
                        'B' => {
                            // Not word boundary
                            let before_is_word = pos > 0 && is_word_char(chars[pos - 1]);
                            let after_is_word = pos < chars.len() && is_word_char(chars[pos]);
                            if before_is_word != after_is_word {
                                return None;
                            }
                            i += 2;
                        }
                        _ => {
                            // Escaped character
                            let matched_char = next;
                            if pos < chars.len() && chars[pos] == matched_char {
                                result.push(matched_char);
                                pos += 1;
                            } else {
                                return None;
                            }
                            i += 2;
                        }
                    }
                }
                '.' => {
                    if pos < chars.len() {
                        if self.flags.contains(&Flag::DotAll) || chars[pos] != '\n' {
                            result.push(chars[pos]);
                            pos += 1;
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                    i += 1;
                }
                '[' => {
                    // Character class
                    let (class_end, class_pattern) = self.extract_char_class(&pat_chars, i)?;
                    let class_start = i;
                    i = class_end;

                    if pos < chars.len() && self.match_char_class(chars[pos], &class_pattern, class_start) {
                        result.push(chars[pos]);
                        pos += 1;
                    } else {
                        return None;
                    }
                }
                '(' => {
                    // Group
                    let (group_end, group_content) = self.extract_group(&pat_chars, i)?;
                    i = group_end;

                    let (group_matched, group_groups) = self.match_pattern(&group_content, &input[pos..])?;
                    let consumed = group_matched.len();

                    if !group_matched.is_empty() {
                        result.push_str(&group_matched);
                        pos += consumed;

                        // Check if it's a capturing group (not (?:...))
                        if !group_content.starts_with("?:") {
                            groups.push(group_matched);
                            groups.extend(group_groups);
                        }
                    } else {
                        return None;
                    }
                }
                '*' => {
                    if result.is_empty() {
                        return None;
                    }
                    // Greedy star
                    let mut end_pos = pos;
                    while end_pos < chars.len() {
                        end_pos += 1;
                    }

                    // Try to match as much as possible, then backtrack
                    let mut best_match = None;
                    for try_pos in (pos..=end_pos).rev() {
                        let test_input = &input[try_pos..];
                        if let Some(rest) = self.match_pattern(&pattern[i+1..], test_input) {
                            let full_match = &input[pos..try_pos + rest.0.len()];
                            best_match = Some((full_match.to_string(), rest.1));
                            break;
                        }
                    }

                    if let Some((matched, rest_groups)) = best_match {
                        let matched_len = matched.len();
                        result = matched.clone();
                        pos += matched_len;
                        groups.extend(rest_groups);
                        i += 1;
                    } else {
                        return None;
                    }
                }
                '+' => {
                    if result.is_empty() {
                        return None;
                    }
                    // Greedy plus
                    let mut end_pos = pos;
                    while end_pos < chars.len() {
                        end_pos += 1;
                    }

                    let mut best_match = None;
                    for try_pos in (pos..=end_pos).rev() {
                        let test_input = &input[try_pos..];
                        if let Some(rest) = self.match_pattern(&pattern[i+1..], test_input) {
                            let full_match = &input[pos..try_pos + rest.0.len()];
                            best_match = Some((full_match.to_string(), rest.1));
                            break;
                        }
                    }

                    if let Some((matched, rest_groups)) = best_match {
                        let matched_len = matched.len();
                        result = matched.clone();
                        pos += matched_len;
                        groups.extend(rest_groups);
                        i += 1;
                    } else {
                        return None;
                    }
                }
                '?' => {
                    // Check if it's a quantifier or non-capturing group
                    if i + 1 < pat_chars.len() && pat_chars[i + 1] == '?' {
                        // Reluctant quantifier
                        i += 2;
                        // For now, treat as greedy
                        if pos < chars.len() {
                            result.push(chars[pos]);
                            pos += 1;
                        } else {
                            return None;
                        }
                    } else if i + 1 < pat_chars.len() && pat_chars[i + 1] == '+' {
                        // Possessive quantifier
                        i += 2;
                        if pos < chars.len() {
                            result.push(chars[pos]);
                            pos += 1;
                        } else {
                            return None;
                        }
                    } else {
                        // Optional
                        i += 1;
                        if pos < chars.len() {
                            result.push(chars[pos]);
                            pos += 1;
                        }
                    }
                }
                '{' => {
                    // Quantifier
                    let (quant_end, quant_str) = self.extract_quantifier(&pat_chars, i)?;
                    i = quant_end;

                    let (min, max) = parse_quantifier(&quant_str)?;

                    // Count consecutive matches
                    let mut count = 0;
                    let mut end_pos = pos;
                    while end_pos < chars.len() {
                        if self.match_single_char(&pat_chars[i - quant_str.len() - 1..i], chars[end_pos]) {
                            count += 1;
                            end_pos += 1;
                        } else {
                            break;
                        }
                    }

                    // Backtrack if needed
                    let mut best_match = None;
                    for try_count in (min..=count.min(max)).rev() {
                        let test_pos = pos + try_count;
                        if let Some(rest) = self.match_pattern(&pattern[i..], &input[test_pos..]) {
                            let full_match = &input[pos..test_pos + rest.0.len()];
                            best_match = Some((full_match.to_string(), rest.1));
                            break;
                        }
                    }

                    if let Some((matched, rest_groups)) = best_match {
                        let matched_len = matched.len();
                        result = matched.clone();
                        pos += matched_len;
                        groups.extend(rest_groups);
                    } else {
                        return None;
                    }
                }
                '|' => {
                    // Alternative
                    let (alt_end, alt_pattern) = self.extract_alternative(&pat_chars, i)?;
                    i = alt_end;

                    // Try first alternative
                    if let Some(result) = self.match_pattern(&alt_pattern, input) {
                        return Some(result);
                    }

                    // Try second alternative
                    let second_alt: String = pat_chars[i + 1..].iter().collect();
                    if let Some(result) = self.match_pattern(&second_alt, input) {
                        return Some(result);
                    }

                    return None;
                }
                _ => {
                    // Literal character
                    if pos < chars.len() && chars[pos] == c {
                        result.push(c);
                        pos += 1;
                    } else {
                        return None;
                    }
                    i += 1;
                }
            }
        }

        Some((result, groups))
    }

    fn match_single_char(&self, pattern: &[char], c: char) -> bool {
        if pattern.is_empty() {
            return false;
        }

        let p = pattern[0];
        match p {
            '\\' => {
                if pattern.len() < 2 {
                    return false;
                }
                c == pattern[1]
            }
            '.' => true,
            _ => p == c,
        }
    }

    fn extract_char_class(&self, pat_chars: &[char], start: usize) -> Option<(usize, String)> {
        let mut i = start + 1;
        let mut class = String::new();

        while i < pat_chars.len() && pat_chars[i] != ']' {
            class.push(pat_chars[i]);
            i += 1;
        }

        if i >= pat_chars.len() {
            return None;
        }

        Some((i + 1, class))
    }

    fn match_char_class(&self, c: char, class_pattern: &str, _start: usize) -> bool {
        let chars: Vec<char> = class_pattern.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            let p = chars[i];

            if p == '^' && i == 0 {
                // Negated class
                i += 1;
                let negated = true;
                if self.match_char_class_inner(c, &chars[i..], _start) == negated {
                    return true;
                }
                return false;
            }

            if p == '[' && i + 1 < chars.len() && chars[i + 1] == '^' {
                // Intersection - skip this for now as it requires nested extraction
                i += 2;
                continue;
            }

            if p == '-' && i > 0 && i + 1 < chars.len() {
                // Range
                let prev = chars[i - 1];
                let next = chars[i + 1];
                if c as u32 >= prev as u32 && c as u32 <= next as u32 {
                    return true;
                }
                i += 2;
                continue;
            }

            if p == c {
                return true;
            }

            i += 1;
        }

        false
    }

    fn match_char_class_inner(&self, c: char, chars: &[char], _start: usize) -> bool {
        let mut i = 0;
        while i < chars.len() {
            let p = chars[i];

            if p == ']' {
                return false;
            }

            if p == '-' && i > 0 && i + 1 < chars.len() {
                let prev = chars[i - 1];
                let next = chars[i + 1];
                if c as u32 >= prev as u32 && c as u32 <= next as u32 {
                    return true;
                }
                i += 2;
                continue;
            }

            if p == c {
                return true;
            }

            i += 1;
        }

        false
    }

    fn extract_group(&self, pat_chars: &[char], start: usize) -> Option<(usize, String)> {
        let mut i = start + 1;
        let mut depth = 1;
        let mut group = String::new();

        while i < pat_chars.len() && depth > 0 {
            match pat_chars[i] {
                '(' => {
                    depth += 1;
                    group.push(pat_chars[i]);
                }
                ')' => {
                    depth -= 1;
                    if depth > 0 {
                        group.push(pat_chars[i]);
                    }
                }
                '\\' => {
                    if i + 1 < pat_chars.len() {
                        group.push(pat_chars[i]);
                        group.push(pat_chars[i + 1]);
                        i += 1;
                    }
                }
                _ => {
                    group.push(pat_chars[i]);
                }
            }
            i += 1;
        }

        if depth != 0 {
            return None;
        }

        Some((i, group))
    }

    fn extract_quantifier(&self, pat_chars: &[char], start: usize) -> Option<(usize, String)> {
        let mut i = start + 1;
        let mut quant = String::new();

        while i < pat_chars.len() && pat_chars[i] != '}' {
            quant.push(pat_chars[i]);
            i += 1;
        }

        if i >= pat_chars.len() {
            return None;
        }

        Some((i + 1, quant))
    }

    fn extract_alternative(&self, pat_chars: &[char], start: usize) -> Option<(usize, String)> {
        let mut i = start + 1;
        let mut depth = 0;
        let mut alt = String::new();

        while i < pat_chars.len() {
            match pat_chars[i] {
                '(' => depth += 1,
                ')' => depth -= 1,
                '|' if depth == 0 => break,
                '\\' => {
                    if i + 1 < pat_chars.len() {
                        alt.push(pat_chars[i]);
                        alt.push(pat_chars[i + 1]);
                        i += 1;
                    }
                }
                _ => {}
            }
            alt.push(pat_chars[i]);
            i += 1;
        }

        Some((i, alt))
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn parse_quantifier(quant: &str) -> Option<(usize, usize)> {
    let parts: Vec<&str> = quant.split(',').collect();

    let min: usize = parts[0].parse().ok()?;
    let max: usize = if parts.len() > 1 && !parts[1].is_empty() {
        parts[1].parse().ok()?
    } else {
        usize::MAX
    };

    Some((min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_match() {
        let regex = Regex::new("abc");
        assert!(regex.matches("abc"));
        assert!(!regex.matches("zabc"));
    }

    #[test]
    fn test_find() {
        let regex = Regex::new("abc");
        let matches = regex.find("zabc");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_text, "abc");
    }

    #[test]
    fn test_escaped_dot() {
        let regex = Regex::new("\\.");
        assert!(regex.matches("."));
    }

    #[test]
    fn test_escaped_backslash() {
        let regex = Regex::new("\\\\");
        assert!(regex.matches("\\"));
    }

    #[test]
    fn test_quantifiers() {
        let regex = Regex::new("a{3}");
        let matches = regex.find("aaab");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_text, "aaa");
    }

    #[test]
    fn test_groups() {
        let regex = Regex::new("(a)(b)(c)");
        let matches = regex.find("abc");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].groups, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    }

    #[test]
    fn test_alternative() {
        let regex = Regex::new("cat|dog");
        assert!(regex.matches("dog"));
    }

    #[test]
    fn test_word_boundary() {
        let regex = Regex::new("\\bcat\\b");
        let matches = regex.find("a cat! bobcat cat_");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_text, "cat");
    }

    #[test]
    fn test_multiline() {
        let regex = Regex::with_flags("^abc", &['m']);
        let matches = regex.find("abc\nabc");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_dotall() {
        let regex = Regex::with_flags("a.*b", &['s']);
        assert!(regex.matches("a\nb"));
    }
}
