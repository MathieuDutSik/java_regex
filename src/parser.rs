use std::collections::HashMap;

use crate::types::*;
use crate::unicode::is_valid_unicode_property;

pub struct Parser {
    chars: Vec<char>,
    pos: usize,
    pub flags: Flags,
    pub group_count: usize,
    pub named_groups: HashMap<String, usize>,
    all_named_backrefs: Vec<String>,
}

impl Parser {
    pub fn new(pattern: &str, flags: Flags) -> Self {
        Parser {
            chars: pattern.chars().collect(),
            pos: 0,
            flags,
            group_count: 0,
            named_groups: HashMap::new(),
            all_named_backrefs: Vec::new(),
        }
    }

    pub fn parse(mut self) -> Result<(Pattern, usize, HashMap<String, usize>), PatternSyntaxError> {
        let pattern = self.parse_pattern()?;
        if self.pos < self.chars.len() {
            return Err(PatternSyntaxError {
                message: format!("Unexpected character '{}' at position {}", self.chars[self.pos], self.pos),
            });
        }
        for name in &self.all_named_backrefs {
            if !self.named_groups.contains_key(name) {
                return Err(PatternSyntaxError {
                    message: format!("Unknown named group: {}", name),
                });
            }
        }
        let gc = self.group_count;
        let ng = self.named_groups;
        Ok((pattern, gc, ng))
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        if c.is_some() { self.pos += 1; }
        c
    }

    fn expect(&mut self, expected: char) -> Result<(), PatternSyntaxError> {
        match self.advance() {
            Some(c) if c == expected => Ok(()),
            _ => Err(PatternSyntaxError {
                message: format!("Expected '{}'", expected),
            }),
        }
    }

    fn remaining(&self) -> usize {
        self.chars.len() - self.pos
    }

    fn parse_pattern(&mut self) -> Result<Pattern, PatternSyntaxError> {
        let mut branches = vec![self.parse_branch()?];
        while self.peek() == Some('|') {
            self.advance();
            branches.push(self.parse_branch()?);
        }
        Ok(Pattern { branches })
    }

    fn parse_branch(&mut self) -> Result<Vec<Node>, PatternSyntaxError> {
        let mut nodes = Vec::new();
        loop {
            if self.flags.comments {
                self.skip_comments_whitespace();
            }
            match self.peek() {
                None => break,
                Some('|') | Some(')') => break,
                _ => {}
            }
            // Handle \Q...\E specially: emit all-but-last as literals,
            // let only the last go through quantifier parsing
            if self.pos + 1 < self.chars.len() && self.chars[self.pos] == '\\' && self.chars[self.pos + 1] == 'Q' {
                self.pos += 2;
                let mut quoted_chars = Vec::new();
                loop {
                    if self.pos >= self.chars.len() { break; }
                    if self.pos + 1 < self.chars.len() && self.chars[self.pos] == '\\' && self.chars[self.pos + 1] == 'E' {
                        self.pos += 2;
                        break;
                    }
                    quoted_chars.push(self.chars[self.pos]);
                    self.pos += 1;
                }
                if quoted_chars.is_empty() {
                    continue;
                }
                for &ch in &quoted_chars[..quoted_chars.len() - 1] {
                    nodes.push(Node::Literal(ch));
                }
                let last = Node::Literal(*quoted_chars.last().unwrap());
                let node = self.maybe_parse_quantifier(last)?;
                nodes.push(node);
                continue;
            }
            let node = self.parse_atom()?;
            let node = self.maybe_parse_quantifier(node)?;
            nodes.push(node);
        }
        Ok(nodes)
    }

    fn skip_comments_whitespace(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c == ' ' || c == '\t' || c == '\n' || c == '\r' => {
                    self.advance();
                }
                Some('#') => {
                    self.advance();
                    while let Some(ch) = self.peek() {
                        if ch == '\n' { self.advance(); break; }
                        self.advance();
                    }
                }
                _ => break,
            }
        }
    }

    fn parse_atom(&mut self) -> Result<Node, PatternSyntaxError> {
        let c = self.peek().ok_or_else(|| PatternSyntaxError {
            message: "Unexpected end of pattern".to_string(),
        })?;

        match c {
            '\\' => self.parse_escape(),
            '.' => { self.advance(); Ok(Node::Dot) }
            '^' => { self.advance(); Ok(Node::Anchor(AnchorKind::StartOfLine)) }
            '$' => { self.advance(); Ok(Node::Anchor(AnchorKind::EndOfLine)) }
            '[' => self.parse_char_class_node(),
            '(' => self.parse_group(),
            '*' | '+' | '?' => {
                Err(PatternSyntaxError {
                    message: format!("Dangling meta character '{}'", c),
                })
            }
            '{' => {
                let saved = self.pos;
                self.advance();
                match self.parse_quantifier_braces() {
                    Ok((min, max)) => {
                        let kind = match self.peek() {
                            Some('?') => { self.advance(); QuantKind::Reluctant }
                            Some('+') => { self.advance(); QuantKind::Possessive }
                            _ => QuantKind::Greedy,
                        };
                        let empty = Node::Group {
                            index: None,
                            name: None,
                            inner: Pattern { branches: vec![vec![]] },
                        };
                        Ok(Node::Quantified { inner: Box::new(empty), min, max, kind })
                    }
                    Err(_) => {
                        self.pos = saved;
                        Err(PatternSyntaxError {
                            message: format!("Illegal repetition near index {}", self.pos),
                        })
                    }
                }
            }
            _ => { self.advance(); Ok(Node::Literal(c)) }
        }
    }

    fn parse_escape(&mut self) -> Result<Node, PatternSyntaxError> {
        self.advance(); // consume '\'
        let c = self.advance().ok_or_else(|| PatternSyntaxError {
            message: "Unexpected end of pattern after \\".to_string(),
        })?;

        match c {
            // Predefined character classes
            'd' => Ok(self.predefined_node(PredefinedClass::Digit)),
            'D' => Ok(self.predefined_node(PredefinedClass::NonDigit)),
            'w' => Ok(self.predefined_node(PredefinedClass::Word)),
            'W' => Ok(self.predefined_node(PredefinedClass::NonWord)),
            's' => Ok(self.predefined_node(PredefinedClass::Whitespace)),
            'S' => Ok(self.predefined_node(PredefinedClass::NonWhitespace)),
            'h' => Ok(self.predefined_node(PredefinedClass::HorizWhitespace)),
            'H' => Ok(self.predefined_node(PredefinedClass::NonHorizWhitespace)),
            'v' => Ok(self.predefined_node(PredefinedClass::VertWhitespace)),
            'V' => Ok(self.predefined_node(PredefinedClass::NonVertWhitespace)),

            // Anchors
            'A' => Ok(Node::Anchor(AnchorKind::StartOfInput)),
            'z' => Ok(Node::Anchor(AnchorKind::EndOfInput)),
            'Z' => Ok(Node::Anchor(AnchorKind::EndOfInputBeforeFinalNewline)),
            'b' => Ok(Node::Anchor(AnchorKind::WordBoundary)),
            'B' => Ok(Node::Anchor(AnchorKind::NonWordBoundary)),
            'G' => Ok(Node::Anchor(AnchorKind::PreviousMatchEnd)),

            // Special escape sequences
            't' => Ok(Node::Literal('\t')),
            'n' => Ok(Node::Literal('\n')),
            'r' => Ok(Node::Literal('\r')),
            'f' => Ok(Node::Literal('\x0C')),
            'a' => Ok(Node::Literal('\x07')),
            'e' => Ok(Node::Literal('\x1B')),

            'R' => Ok(Node::LinebreakMatcher),
            'X' => Ok(Node::GraphemeCluster),
            'Q' => self.parse_quoted(),
            'p' => self.parse_unicode_property_node(false),
            'P' => self.parse_unicode_property_node(true),
            'x' => self.parse_hex_char().map(Node::Literal),
            'u' => self.parse_unicode_char().map(Node::Literal),
            '0' => self.parse_octal_char().map(Node::Literal),
            'c' => self.parse_control_char().map(Node::Literal),

            // Backreference (numbered)
            '1'..='9' => {
                let mut num = (c as u32 - '0' as u32) as usize;
                while let Some(d) = self.peek() {
                    if d.is_ascii_digit() {
                        let new_num = num * 10 + (d as u32 - '0' as u32) as usize;
                        if new_num <= self.group_count {
                            num = new_num;
                            self.advance();
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                Ok(Node::Backreference(num))
            }

            // Named backreference
            'k' => {
                self.expect('<')?;
                let name = self.parse_group_name()?;
                self.expect('>')?;
                self.all_named_backrefs.push(name.clone());
                Ok(Node::NamedBackreference(name))
            }

            // Octal 3-digit (1-3 leading digit)
            ch if ('1'..='3').contains(&ch) && self.remaining() >= 2
                && self.chars.get(self.pos).is_some_and(|c| ('0'..='7').contains(c))
                && self.chars.get(self.pos + 1).is_some_and(|c| ('0'..='7').contains(c))
                && (ch as u32 - '0' as u32) * 64
                    + (self.chars[self.pos] as u32 - '0' as u32) * 8
                    + (self.chars[self.pos + 1] as u32 - '0' as u32) <= 0o377 => {
                let d1 = ch as u32 - '0' as u32;
                let d2 = self.advance().unwrap() as u32 - '0' as u32;
                let d3 = self.advance().unwrap() as u32 - '0' as u32;
                let code = d1 * 64 + d2 * 8 + d3;
                Ok(Node::Literal(char::from_u32(code).unwrap_or('\0')))
            }

            // Escaped metacharacters
            '\\' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$' | '-' | '!' | '=' | '<' | '>' | '/' | '#' | ' ' | '&' | '~' | '@' | '`' | '\'' | '"' | ',' | ';' | ':' => {
                Ok(Node::Literal(c))
            }

            'E' => {
                Err(PatternSyntaxError {
                    message: format!("Illegal/unsupported escape sequence near index {}", self.pos - 1),
                })
            }

            _ => Ok(Node::Literal(c)),
        }
    }

    // Shared escape-character parsers used by both parse_escape and parse_char_class_item

    fn parse_hex_char(&mut self) -> Result<char, PatternSyntaxError> {
        if self.peek() == Some('{') {
            self.advance();
            let mut hex = String::new();
            while let Some(c) = self.peek() {
                if c == '}' { self.advance(); break; }
                hex.push(c);
                self.advance();
            }
            let code = u32::from_str_radix(&hex, 16).map_err(|_| PatternSyntaxError {
                message: format!("Invalid hex escape: {}", hex),
            })?;
            Ok(char::from_u32(code).unwrap_or('\0'))
        } else {
            let mut hex = String::new();
            for _ in 0..2 {
                if let Some(c) = self.peek() {
                    if c.is_ascii_hexdigit() { hex.push(c); self.advance(); }
                    else { break; }
                }
            }
            if hex.len() != 2 {
                return Err(PatternSyntaxError { message: "Invalid hex escape".to_string() });
            }
            let code = u32::from_str_radix(&hex, 16).map_err(|_| PatternSyntaxError {
                message: format!("Invalid hex escape: {}", hex),
            })?;
            Ok(char::from_u32(code).unwrap_or('\0'))
        }
    }

    fn parse_unicode_char(&mut self) -> Result<char, PatternSyntaxError> {
        let mut hex = String::new();
        for _ in 0..4 {
            if let Some(c) = self.peek() {
                if c.is_ascii_hexdigit() { hex.push(c); self.advance(); }
                else { break; }
            }
        }
        let code = u32::from_str_radix(&hex, 16).map_err(|_| PatternSyntaxError {
            message: format!("Invalid unicode escape: {}", hex),
        })?;
        Ok(char::from_u32(code).unwrap_or('\0'))
    }

    fn parse_octal_char(&mut self) -> Result<char, PatternSyntaxError> {
        // Java's octal: \0 already consumed, read up to 3 more digits.
        // If first digit is 0-3, read up to 2 more (total value ≤ 0377).
        // If first digit is 4-7, read only 1 more (total value ≤ 077).
        let first = match self.peek() {
            Some(c) if ('0'..='7').contains(&c) => { self.advance(); c }
            _ => {
                return Err(PatternSyntaxError {
                    message: format!("Illegal octal escape sequence near index {}", self.pos),
                });
            }
        };
        let max_more = if ('0'..='3').contains(&first) { 2 } else { 1 };
        let mut oct = String::new();
        oct.push(first);
        for _ in 0..max_more {
            if let Some(c) = self.peek() {
                if ('0'..='7').contains(&c) { oct.push(c); self.advance(); }
                else { break; }
            }
        }
        let code = u32::from_str_radix(&oct, 8).unwrap_or(0);
        Ok(char::from_u32(code).unwrap_or('\0'))
    }

    fn parse_control_char(&mut self) -> Result<char, PatternSyntaxError> {
        let ctrl = self.advance().ok_or_else(|| PatternSyntaxError {
            message: "Expected control character after \\c".to_string(),
        })?;
        let code = (ctrl as u32) ^ 0x40;
        Ok(char::from_u32(code).unwrap_or('\0'))
    }

    fn predefined_node(&self, pc: PredefinedClass) -> Node {
        Node::CharClass(CharClass {
            negated: false,
            items: vec![CharClassItem::Predefined(pc)],
        })
    }

    fn parse_quoted(&mut self) -> Result<Node, PatternSyntaxError> {
        let mut chars = Vec::new();
        loop {
            if self.pos >= self.chars.len() { break; }
            if self.pos + 1 < self.chars.len() && self.chars[self.pos] == '\\' && self.chars[self.pos + 1] == 'E' {
                self.pos += 2;
                break;
            }
            chars.push(self.chars[self.pos]);
            self.pos += 1;
        }
        if chars.is_empty() {
            return Ok(Node::Group {
                index: None,
                name: None,
                inner: Pattern { branches: vec![vec![]] },
            });
        }
        if chars.len() == 1 {
            return Ok(Node::Literal(chars[0]));
        }
        let nodes: Vec<Node> = chars.into_iter().map(Node::Literal).collect();
        Ok(Node::Group {
            index: None,
            name: None,
            inner: Pattern { branches: vec![nodes] },
        })
    }

    fn parse_unicode_property_node(&mut self, negated: bool) -> Result<Node, PatternSyntaxError> {
        let (name, neg) = self.parse_property_name(negated)?;
        Ok(Node::CharClass(CharClass {
            negated: false,
            items: vec![CharClassItem::UnicodeProperty { name, negated: neg }],
        }))
    }

    /// Parse a unicode property name (shared between node and char class contexts).
    /// Returns (name, negated).
    fn parse_property_name(&mut self, negated: bool) -> Result<(String, bool), PatternSyntaxError> {
        if self.peek() == Some('{') {
            self.advance();
            let mut name = String::new();
            while let Some(c) = self.peek() {
                if c == '}' { self.advance(); break; }
                name.push(c);
                self.advance();
            }
            if !is_valid_unicode_property(&name) {
                return Err(PatternSyntaxError {
                    message: format!("Unknown Unicode property: {}", name),
                });
            }
            Ok((name, negated))
        } else {
            let c = self.advance().ok_or_else(|| PatternSyntaxError {
                message: "Expected property name after \\p".to_string(),
            })?;
            Ok((c.to_string(), negated))
        }
    }

    fn parse_group_name(&mut self) -> Result<String, PatternSyntaxError> {
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                name.push(c);
                self.advance();
            } else {
                break;
            }
        }
        if name.is_empty() {
            return Err(PatternSyntaxError { message: "Empty group name".to_string() });
        }
        if name.starts_with(|c: char| c.is_ascii_digit()) {
            return Err(PatternSyntaxError {
                message: format!("Group name must start with a letter, not '{}'", name.chars().next().unwrap()),
            });
        }
        Ok(name)
    }

    fn parse_group(&mut self) -> Result<Node, PatternSyntaxError> {
        self.advance(); // consume '('

        if self.peek() == Some('?') {
            self.advance();
            match self.peek() {
                Some(':') => {
                    self.advance();
                    let inner = self.parse_pattern()?;
                    self.expect(')')?;
                    Ok(Node::Group { index: None, name: None, inner })
                }
                Some('<') => {
                    self.advance();
                    match self.peek() {
                        Some('=') => {
                            self.advance();
                            let inner = self.parse_pattern()?;
                            self.expect(')')?;
                            Ok(Node::Lookbehind { positive: true, inner })
                        }
                        Some('!') => {
                            self.advance();
                            let inner = self.parse_pattern()?;
                            self.expect(')')?;
                            Ok(Node::Lookbehind { positive: false, inner })
                        }
                        _ => {
                            let name = self.parse_group_name()?;
                            self.expect('>')?;
                            if self.named_groups.contains_key(&name) {
                                return Err(PatternSyntaxError {
                                    message: format!("Duplicate group name: {}", name),
                                });
                            }
                            self.group_count += 1;
                            let index = self.group_count;
                            self.named_groups.insert(name.clone(), index);
                            let inner = self.parse_pattern()?;
                            self.expect(')')?;
                            Ok(Node::Group { index: Some(index), name: Some(name), inner })
                        }
                    }
                }
                Some('=') => {
                    self.advance();
                    let inner = self.parse_pattern()?;
                    self.expect(')')?;
                    Ok(Node::Lookahead { positive: true, inner })
                }
                Some('!') => {
                    self.advance();
                    let inner = self.parse_pattern()?;
                    self.expect(')')?;
                    Ok(Node::Lookahead { positive: false, inner })
                }
                Some('>') => {
                    self.advance();
                    let inner = self.parse_pattern()?;
                    self.expect(')')?;
                    Ok(Node::AtomicGroup { inner })
                }
                _ => self.parse_inline_flags(),
            }
        } else {
            self.group_count += 1;
            let index = self.group_count;
            let inner = self.parse_pattern()?;
            self.expect(')')?;
            Ok(Node::Group { index: Some(index), name: None, inner })
        }
    }

    fn parse_inline_flags(&mut self) -> Result<Node, PatternSyntaxError> {
        let mut set_flags = Flags::default();
        let mut clear_flags = Flags::default();
        let mut clearing = false;

        loop {
            match self.peek() {
                Some(ch @ ('i' | 'm' | 's' | 'x' | 'U' | 'd' | 'u')) => {
                    self.advance();
                    let target = if clearing { &mut clear_flags } else { &mut set_flags };
                    match ch {
                        'i' => target.case_insensitive = true,
                        'm' => target.multiline = true,
                        's' => target.dotall = true,
                        'x' => target.comments = true,
                        'U' => target.unicode_class = true,
                        'd' => target.unix_lines = true,
                        'u' => target.unicode_case = true,
                        _ => unreachable!(),
                    }
                }
                Some('-') => { self.advance(); clearing = true; }
                Some(':') => {
                    self.advance();
                    let saved = self.flags;
                    self.apply_flags(set_flags, clear_flags);
                    let active_flags = self.flags;
                    let inner = self.parse_pattern()?;
                    self.flags = saved;
                    self.expect(')')?;
                    return Ok(Node::FlagGroup { flags: active_flags, inner });
                }
                Some(')') => {
                    self.advance();
                    self.apply_flags(set_flags, clear_flags);
                    return Ok(Node::SetFlags(self.flags));
                }
                _ => {
                    return Err(PatternSyntaxError { message: "Invalid inline flag".to_string() });
                }
            }
        }
    }

    fn apply_flags(&mut self, set: Flags, clear: Flags) {
        if set.case_insensitive { self.flags.case_insensitive = true; }
        if set.multiline { self.flags.multiline = true; }
        if set.dotall { self.flags.dotall = true; }
        if set.comments { self.flags.comments = true; }
        if set.unicode_class { self.flags.unicode_class = true; }
        if set.unix_lines { self.flags.unix_lines = true; }
        if set.unicode_case { self.flags.unicode_case = true; }
        if clear.case_insensitive { self.flags.case_insensitive = false; }
        if clear.multiline { self.flags.multiline = false; }
        if clear.dotall { self.flags.dotall = false; }
        if clear.comments { self.flags.comments = false; }
        if clear.unicode_class { self.flags.unicode_class = false; }
        if clear.unix_lines { self.flags.unix_lines = false; }
        if clear.unicode_case { self.flags.unicode_case = false; }
    }

    fn maybe_parse_quantifier(&mut self, node: Node) -> Result<Node, PatternSyntaxError> {
        if self.flags.comments {
            self.skip_comments_whitespace();
        }
        let (min, max) = match self.peek() {
            Some('*') => { self.advance(); (0, u32::MAX) }
            Some('+') => { self.advance(); (1, u32::MAX) }
            Some('?') => { self.advance(); (0, 1) }
            Some('{') => {
                self.advance();
                match self.parse_quantifier_braces() {
                    Ok((min, max)) => (min, max),
                    Err(_) => {
                        return Err(PatternSyntaxError {
                            message: format!("Illegal repetition near index {}", self.pos),
                        });
                    }
                }
            }
            _ => return Ok(node),
        };

        let kind = match self.peek() {
            Some('?') => { self.advance(); QuantKind::Reluctant }
            Some('+') => { self.advance(); QuantKind::Possessive }
            _ => QuantKind::Greedy,
        };

        Ok(Node::Quantified { inner: Box::new(node), min, max, kind })
    }

    fn parse_quantifier_braces(&mut self) -> Result<(u32, u32), PatternSyntaxError> {
        let mut min_str = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() { min_str.push(c); self.advance(); }
            else { break; }
        }
        if min_str.is_empty() {
            return Err(PatternSyntaxError { message: "Invalid quantifier".to_string() });
        }
        let min: u32 = min_str.parse().map_err(|_| PatternSyntaxError {
            message: "Invalid quantifier number".to_string(),
        })?;

        match self.peek() {
            Some('}') => { self.advance(); Ok((min, min)) }
            Some(',') => {
                self.advance();
                if self.peek() == Some('}') {
                    self.advance();
                    Ok((min, u32::MAX))
                } else {
                    let mut max_str = String::new();
                    while let Some(c) = self.peek() {
                        if c.is_ascii_digit() { max_str.push(c); self.advance(); }
                        else { break; }
                    }
                    self.expect('}')?;
                    let max: u32 = max_str.parse().map_err(|_| PatternSyntaxError {
                        message: "Invalid quantifier number".to_string(),
                    })?;
                    if min > max {
                        return Err(PatternSyntaxError {
                            message: format!("Illegal repetition range near index {}", self.pos),
                        });
                    }
                    Ok((min, max))
                }
            }
            _ => Err(PatternSyntaxError { message: "Invalid quantifier".to_string() }),
        }
    }

    // ==================== Character Class Parsing ====================

    fn parse_char_class_node(&mut self) -> Result<Node, PatternSyntaxError> {
        let cc = self.parse_char_class()?;
        Ok(Node::CharClass(cc))
    }

    fn parse_char_class(&mut self) -> Result<CharClass, PatternSyntaxError> {
        self.expect('[')?;
        let negated = if self.peek() == Some('^') { self.advance(); true } else { false };
        let items = self.parse_char_class_items()?;
        self.expect(']')?;
        Ok(CharClass { negated, items })
    }

    fn parse_char_class_items(&mut self) -> Result<Vec<CharClassItem>, PatternSyntaxError> {
        let mut left_items = Vec::new();
        let mut at_start = true;

        loop {
            if self.flags.comments {
                self.skip_comments_whitespace();
            }
            match self.peek() {
                None => return Err(PatternSyntaxError { message: "Unclosed character class".to_string() }),
                Some(']') if !at_start => break,
                Some(']') if at_start => {
                    self.advance();
                    left_items.push(CharClassItem::Single(']'));
                    at_start = false;
                    continue;
                }
                Some('[') => {
                    let nested = self.parse_char_class()?;
                    left_items.push(CharClassItem::Nested(nested));
                    at_start = false;
                    continue;
                }
                Some('&') if self.pos + 1 < self.chars.len() && self.chars[self.pos + 1] == '&' => {
                    self.advance();
                    self.advance();
                    let right_items = self.parse_intersection_rhs()?;
                    let mut result = vec![CharClassItem::Intersection(left_items, right_items)];
                    while self.pos + 1 < self.chars.len()
                        && self.peek() == Some('&')
                        && self.chars[self.pos + 1] == '&'
                    {
                        self.advance();
                        self.advance();
                        let next_items = self.parse_intersection_rhs()?;
                        result = vec![CharClassItem::Intersection(result, next_items)];
                    }
                    return Ok(result);
                }
                _ => {
                    at_start = false;
                    let item = self.parse_char_class_item()?;
                    if self.peek() == Some('-') && self.pos + 1 < self.chars.len() && self.chars[self.pos + 1] != ']' {
                        if let CharClassItem::Single(start) = item {
                            self.advance();
                            let end_item = self.parse_char_class_item()?;
                            if let CharClassItem::Single(end) = end_item {
                                if start > end {
                                    return Err(PatternSyntaxError {
                                        message: format!("Invalid range: {}-{}", start, end),
                                    });
                                }
                                left_items.push(CharClassItem::Range(start, end));
                                continue;
                            } else {
                                return Err(PatternSyntaxError {
                                    message: "Illegal character range".to_string(),
                                });
                            }
                        }
                    }
                    left_items.push(item);
                }
            }
        }

        Ok(left_items)
    }

    /// Parse the right-hand side of &&. This collects nested [...] groups and bare
    /// items until the next && or enclosing ].
    fn parse_intersection_rhs(&mut self) -> Result<Vec<CharClassItem>, PatternSyntaxError> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None => return Err(PatternSyntaxError { message: "Unclosed character class".to_string() }),
                Some(']') => break,
                Some('&') if self.pos + 1 < self.chars.len() && self.chars[self.pos + 1] == '&' => break,
                Some('[') => {
                    let nested = self.parse_char_class()?;
                    items.push(CharClassItem::Nested(nested));
                }
                _ => {
                    let item = self.parse_char_class_item()?;
                    if self.peek() == Some('-') && self.pos + 1 < self.chars.len()
                        && self.chars[self.pos + 1] != ']'
                        && self.chars[self.pos + 1] != '['
                    {
                        if let CharClassItem::Single(start) = item {
                            self.advance();
                            let end_item = self.parse_char_class_item()?;
                            if let CharClassItem::Single(end) = end_item {
                                items.push(CharClassItem::Range(start, end));
                                continue;
                            }
                        }
                    }
                    items.push(item);
                }
            }
        }
        if items.is_empty() {
            return Err(PatternSyntaxError { message: "Empty intersection operand".to_string() });
        }
        Ok(items)
    }

    fn parse_char_class_item(&mut self) -> Result<CharClassItem, PatternSyntaxError> {
        match self.peek() {
            Some('\\') => {
                self.advance();
                let c = self.advance().ok_or_else(|| PatternSyntaxError {
                    message: "Unexpected end in character class".to_string(),
                })?;
                match c {
                    'd' => Ok(CharClassItem::Predefined(PredefinedClass::Digit)),
                    'D' => Ok(CharClassItem::Predefined(PredefinedClass::NonDigit)),
                    'w' => Ok(CharClassItem::Predefined(PredefinedClass::Word)),
                    'W' => Ok(CharClassItem::Predefined(PredefinedClass::NonWord)),
                    's' => Ok(CharClassItem::Predefined(PredefinedClass::Whitespace)),
                    'S' => Ok(CharClassItem::Predefined(PredefinedClass::NonWhitespace)),
                    'h' => Ok(CharClassItem::Predefined(PredefinedClass::HorizWhitespace)),
                    'H' => Ok(CharClassItem::Predefined(PredefinedClass::NonHorizWhitespace)),
                    'v' => Ok(CharClassItem::Predefined(PredefinedClass::VertWhitespace)),
                    'V' => Ok(CharClassItem::Predefined(PredefinedClass::NonVertWhitespace)),
                    'p' => {
                        let (name, negated) = self.parse_property_name(false)?;
                        Ok(CharClassItem::UnicodeProperty { name, negated })
                    }
                    'P' => {
                        let (name, negated) = self.parse_property_name(true)?;
                        Ok(CharClassItem::UnicodeProperty { name, negated })
                    }
                    't' => Ok(CharClassItem::Single('\t')),
                    'n' => Ok(CharClassItem::Single('\n')),
                    'r' => Ok(CharClassItem::Single('\r')),
                    'f' => Ok(CharClassItem::Single('\x0C')),
                    'a' => Ok(CharClassItem::Single('\x07')),
                    'e' => Ok(CharClassItem::Single('\x1B')),
                    'x' => self.parse_hex_char().map(CharClassItem::Single),
                    'u' => self.parse_unicode_char().map(CharClassItem::Single),
                    '0' => self.parse_octal_char().map(CharClassItem::Single),
                    'c' => self.parse_control_char().map(CharClassItem::Single),
                    'Q' => {
                        let mut items = Vec::new();
                        loop {
                            if self.pos >= self.chars.len() { break; }
                            if self.pos + 1 < self.chars.len() && self.chars[self.pos] == '\\' && self.chars[self.pos + 1] == 'E' {
                                self.pos += 2;
                                break;
                            }
                            items.push(CharClassItem::Single(self.chars[self.pos]));
                            self.pos += 1;
                        }
                        if items.len() == 1 { return Ok(items.into_iter().next().unwrap()); }
                        Ok(CharClassItem::Nested(CharClass { negated: false, items }))
                    }
                    '1'..='9' => Err(PatternSyntaxError {
                        message: format!("Illegal backreference in character class near index {}", self.pos - 1),
                    }),
                    _ => Ok(CharClassItem::Single(c)),
                }
            }
            Some('[') => {
                let nested = self.parse_char_class()?;
                Ok(CharClassItem::Nested(nested))
            }
            Some(c) => { self.advance(); Ok(CharClassItem::Single(c)) }
            None => Err(PatternSyntaxError { message: "Unexpected end in character class".to_string() }),
        }
    }
}
