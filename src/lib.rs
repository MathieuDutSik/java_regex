use std::collections::HashMap;
use std::fmt;

// ==================== Error Types ====================

#[derive(Debug, Clone)]
pub struct PatternSyntaxError {
    pub message: String,
}

impl fmt::Display for PatternSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PatternSyntaxException: {}", self.message)
    }
}

// ==================== Flags ====================

#[derive(Clone, Copy, Debug, Default)]
struct Flags {
    case_insensitive: bool, // i / CASE_INSENSITIVE
    multiline: bool,        // m / MULTILINE
    dotall: bool,           // s / DOTALL
    comments: bool,         // x / COMMENTS
    unicode_class: bool,    // U / UNICODE_CHARACTER_CLASS
}

// ==================== AST Types ====================

/// A pattern is an alternation of branches.
#[derive(Clone, Debug)]
struct Pattern {
    branches: Vec<Vec<Node>>,
}

#[derive(Clone, Debug)]
enum Node {
    Literal(char),
    Dot,
    Anchor(AnchorKind),
    CharClass(CharClass),
    Quantified {
        inner: Box<Node>,
        min: u32,
        max: u32,
        kind: QuantKind,
    },
    Group {
        index: Option<usize>,
        #[allow(dead_code)]
        name: Option<String>,
        inner: Pattern,
    },
    Lookahead {
        positive: bool,
        inner: Pattern,
    },
    Lookbehind {
        positive: bool,
        inner: Pattern,
    },
    AtomicGroup {
        inner: Pattern,
    },
    Backreference(usize),
    NamedBackreference(String),
    LinebreakMatcher, // \R
    GraphemeCluster,  // \X
    SetFlags(Flags),  // inline flag change (?i) etc.
    FlagGroup {        // (?i:...) scoped flag group
        flags: Flags,
        inner: Pattern,
    },

    // Engine-internal nodes (not produced by parser)
    GroupEnd {
        index: usize,
        start: usize,
    },
    RestoreFlags(Flags), // engine-internal: restore flags after FlagGroup
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum AnchorKind {
    StartOfLine,                    // ^
    EndOfLine,                      // $
    StartOfInput,                   // \A
    EndOfInput,                     // \z
    EndOfInputBeforeFinalNewline,   // \Z
    WordBoundary,                   // \b
    NonWordBoundary,                // \B
    PreviousMatchEnd,               // \G
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum QuantKind {
    Greedy,
    Reluctant,
    Possessive,
}

#[derive(Clone, Debug)]
struct CharClass {
    negated: bool,
    items: Vec<CharClassItem>,
}

#[derive(Clone, Debug)]
enum CharClassItem {
    Single(char),
    Range(char, char),
    Predefined(PredefinedClass),
    UnicodeProperty {
        name: String,
        negated: bool,
    },
    Nested(CharClass),
    Intersection(Vec<CharClassItem>, Vec<CharClassItem>),
    PosixClass {
        name: String,
        negated: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PredefinedClass {
    Digit,        // \d
    NonDigit,     // \D
    Word,         // \w
    NonWord,      // \W
    Whitespace,   // \s
    NonWhitespace,// \S
    HorizWhitespace,  // \h
    NonHorizWhitespace,// \H
    VertWhitespace,    // \v
    NonVertWhitespace, // \V
}

// ==================== Parser ====================

struct Parser {
    chars: Vec<char>,
    pos: usize,
    flags: Flags,
    group_count: usize,
    named_groups: HashMap<String, usize>,
    all_named_backrefs: Vec<String>,
}

impl Parser {
    fn new(pattern: &str, flags: Flags) -> Self {
        Parser {
            chars: pattern.chars().collect(),
            pos: 0,
            flags,
            group_count: 0,
            named_groups: HashMap::new(),
            all_named_backrefs: Vec::new(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
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

    fn parse(mut self) -> Result<(Pattern, usize, HashMap<String, usize>), PatternSyntaxError> {
        let pattern = self.parse_pattern()?;
        if self.pos < self.chars.len() {
            return Err(PatternSyntaxError {
                message: format!("Unexpected character '{}' at position {}", self.chars[self.pos], self.pos),
            });
        }
        // Validate named backreferences
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
            // Skip whitespace and comments in comments mode
            if self.flags.comments {
                self.skip_comments_whitespace();
            }
            match self.peek() {
                None => break,
                Some('|') | Some(')') => break,
                _ => {}
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
                        if ch == '\n' {
                            self.advance();
                            break;
                        }
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
            '.' => {
                self.advance();
                Ok(Node::Dot)
            }
            '^' => {
                self.advance();
                Ok(Node::Anchor(AnchorKind::StartOfLine))
            }
            '$' => {
                self.advance();
                Ok(Node::Anchor(AnchorKind::EndOfLine))
            }
            '[' => self.parse_char_class_node(),
            '(' => self.parse_group(),
            '*' | '+' | '?' => {
                Err(PatternSyntaxError {
                    message: format!("Dangling meta character '{}'", c),
                })
            }
            _ => {
                self.advance();
                Ok(Node::Literal(c))
            }
        }
    }

    fn parse_escape(&mut self) -> Result<Node, PatternSyntaxError> {
        self.advance(); // consume '\'
        let c = self.advance().ok_or_else(|| PatternSyntaxError {
            message: "Unexpected end of pattern after \\".to_string(),
        })?;

        match c {
            // Predefined character classes
            'd' => Ok(Node::CharClass(CharClass {
                negated: false,
                items: vec![CharClassItem::Predefined(PredefinedClass::Digit)],
            })),
            'D' => Ok(Node::CharClass(CharClass {
                negated: false,
                items: vec![CharClassItem::Predefined(PredefinedClass::NonDigit)],
            })),
            'w' => Ok(Node::CharClass(CharClass {
                negated: false,
                items: vec![CharClassItem::Predefined(PredefinedClass::Word)],
            })),
            'W' => Ok(Node::CharClass(CharClass {
                negated: false,
                items: vec![CharClassItem::Predefined(PredefinedClass::NonWord)],
            })),
            's' => Ok(Node::CharClass(CharClass {
                negated: false,
                items: vec![CharClassItem::Predefined(PredefinedClass::Whitespace)],
            })),
            'S' => Ok(Node::CharClass(CharClass {
                negated: false,
                items: vec![CharClassItem::Predefined(PredefinedClass::NonWhitespace)],
            })),
            'h' => Ok(Node::CharClass(CharClass {
                negated: false,
                items: vec![CharClassItem::Predefined(PredefinedClass::HorizWhitespace)],
            })),
            'H' => Ok(Node::CharClass(CharClass {
                negated: false,
                items: vec![CharClassItem::Predefined(PredefinedClass::NonHorizWhitespace)],
            })),
            'v' => Ok(Node::CharClass(CharClass {
                negated: false,
                items: vec![CharClassItem::Predefined(PredefinedClass::VertWhitespace)],
            })),
            'V' => Ok(Node::CharClass(CharClass {
                negated: false,
                items: vec![CharClassItem::Predefined(PredefinedClass::NonVertWhitespace)],
            })),

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

            // \R linebreak matcher
            'R' => Ok(Node::LinebreakMatcher),

            // \X grapheme cluster
            'X' => Ok(Node::GraphemeCluster),

            // Quoting
            'Q' => self.parse_quoted(),

            // Unicode properties
            'p' => self.parse_unicode_property(false),
            'P' => self.parse_unicode_property(true),

            // Hex escape
            'x' => self.parse_hex_escape(),

            // Unicode escape
            'u' => self.parse_unicode_escape(),

            // Octal escape
            '0' => self.parse_octal_escape(),

            // Control character
            'c' => {
                let ctrl = self.advance().ok_or_else(|| PatternSyntaxError {
                    message: "Expected control character after \\c".to_string(),
                })?;
                let code = (ctrl as u32) ^ 0x40;
                Ok(Node::Literal(char::from_u32(code).unwrap_or('\0')))
            }

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

            // Octal (1-3 digits starting with 0-3 for 3-digit, or 0-7 for 2-digit)
            ch if ('1'..='3').contains(&ch) && self.remaining() >= 2
                && self.chars.get(self.pos).is_some_and(|c| ('0'..='7').contains(c))
                && self.chars.get(self.pos + 1).is_some_and(|c| ('0'..='7').contains(c))
                && (ch as u32 - '0' as u32) * 64
                    + (self.chars[self.pos] as u32 - '0' as u32) * 8
                    + (self.chars[self.pos + 1] as u32 - '0' as u32) <= 0o377 => {
                // Only match as octal if not a valid backreference
                // Actually, \1-\9 are always backrefs, handled above
                // This shouldn't be reached for 1-9
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

            _ => {
                // Unknown escape - treat as literal
                Ok(Node::Literal(c))
            }
        }
    }

    fn parse_quoted(&mut self) -> Result<Node, PatternSyntaxError> {
        // \Q...\E - everything between is literal
        let mut chars = Vec::new();
        loop {
            if self.pos >= self.chars.len() {
                // \Q without \E - rest is literal
                break;
            }
            if self.pos + 1 < self.chars.len() && self.chars[self.pos] == '\\' && self.chars[self.pos + 1] == 'E' {
                self.pos += 2;
                break;
            }
            chars.push(self.chars[self.pos]);
            self.pos += 1;
        }
        // Convert to a sequence by returning first char and putting rest back
        // Actually, return a group containing literals
        if chars.is_empty() {
            // Empty \Q\E - match empty string
            return Ok(Node::Group {
                index: None,
                name: None,
                inner: Pattern { branches: vec![vec![]] },
            });
        }
        if chars.len() == 1 {
            return Ok(Node::Literal(chars[0]));
        }
        // Return as non-capturing group of literals
        let nodes: Vec<Node> = chars.into_iter().map(Node::Literal).collect();
        Ok(Node::Group {
            index: None,
            name: None,
            inner: Pattern { branches: vec![nodes] },
        })
    }

    fn parse_unicode_property(&mut self, negated: bool) -> Result<Node, PatternSyntaxError> {
        if self.peek() == Some('{') {
            self.advance();
            let mut name = String::new();
            while let Some(c) = self.peek() {
                if c == '}' {
                    self.advance();
                    break;
                }
                name.push(c);
                self.advance();
            }
            if !is_valid_unicode_property(&name) {
                return Err(PatternSyntaxError {
                    message: format!("Unknown Unicode property: {}", name),
                });
            }
            Ok(Node::CharClass(CharClass {
                negated: false,
                items: vec![CharClassItem::UnicodeProperty { name, negated }],
            }))
        } else {
            // Single character property name like \pL
            let c = self.advance().ok_or_else(|| PatternSyntaxError {
                message: "Expected property name after \\p".to_string(),
            })?;
            Ok(Node::CharClass(CharClass {
                negated: false,
                items: vec![CharClassItem::UnicodeProperty {
                    name: c.to_string(),
                    negated,
                }],
            }))
        }
    }

    fn parse_hex_escape(&mut self) -> Result<Node, PatternSyntaxError> {
        if self.peek() == Some('{') {
            self.advance();
            let mut hex = String::new();
            while let Some(c) = self.peek() {
                if c == '}' {
                    self.advance();
                    break;
                }
                hex.push(c);
                self.advance();
            }
            let code = u32::from_str_radix(&hex, 16).map_err(|_| PatternSyntaxError {
                message: format!("Invalid hex escape: {}", hex),
            })?;
            Ok(Node::Literal(char::from_u32(code).unwrap_or('\0')))
        } else {
            // \xHH
            let mut hex = String::new();
            for _ in 0..2 {
                if let Some(c) = self.peek() {
                    if c.is_ascii_hexdigit() {
                        hex.push(c);
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
            if hex.is_empty() {
                return Err(PatternSyntaxError {
                    message: "Invalid hex escape".to_string(),
                });
            }
            let code = u32::from_str_radix(&hex, 16).map_err(|_| PatternSyntaxError {
                message: format!("Invalid hex escape: {}", hex),
            })?;
            Ok(Node::Literal(char::from_u32(code).unwrap_or('\0')))
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<Node, PatternSyntaxError> {
        // \uHHHH
        let mut hex = String::new();
        for _ in 0..4 {
            if let Some(c) = self.peek() {
                if c.is_ascii_hexdigit() {
                    hex.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
        }
        let code = u32::from_str_radix(&hex, 16).map_err(|_| PatternSyntaxError {
            message: format!("Invalid unicode escape: {}", hex),
        })?;
        Ok(Node::Literal(char::from_u32(code).unwrap_or('\0')))
    }

    fn parse_octal_escape(&mut self) -> Result<Node, PatternSyntaxError> {
        // \0OOO - up to 3 octal digits after the leading 0
        // The leading 0 has already been consumed by parse_escape
        // After \0, we can have up to 3 more octal digits (any 0-7)
        let mut oct = String::new();
        for _ in 0..3 {
            if let Some(c) = self.peek() {
                if ('0'..='7').contains(&c) {
                    oct.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
        }
        if oct.is_empty() {
            return Ok(Node::Literal('\0'));
        }
        let code = u32::from_str_radix(&oct, 8).unwrap_or(0);
        // Clamp to 0xFF (Java octal escapes are \0 to \0377)
        if code > 0o377 {
            // Too large, back off last digit
            // Actually just use modular — or more correctly, reparse
            return Ok(Node::Literal(char::from_u32(code).unwrap_or('\0')));
        }
        Ok(Node::Literal(char::from_u32(code).unwrap_or('\0')))
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
            return Err(PatternSyntaxError {
                message: "Empty group name".to_string(),
            });
        }
        Ok(name)
    }

    fn parse_group(&mut self) -> Result<Node, PatternSyntaxError> {
        self.advance(); // consume '('

        if self.peek() == Some('?') {
            self.advance(); // consume '?'
            match self.peek() {
                Some(':') => {
                    self.advance();
                    let inner = self.parse_pattern()?;
                    self.expect(')')?;
                    Ok(Node::Group {
                        index: None,
                        name: None,
                        inner,
                    })
                }
                Some('<') => {
                    self.advance();
                    match self.peek() {
                        Some('=') => {
                            // Positive lookbehind
                            self.advance();
                            let inner = self.parse_pattern()?;
                            self.expect(')')?;
                            // Check for variable-length lookbehind
                            self.check_lookbehind_length(&inner)?;
                            Ok(Node::Lookbehind {
                                positive: true,
                                inner,
                            })
                        }
                        Some('!') => {
                            // Negative lookbehind
                            self.advance();
                            let inner = self.parse_pattern()?;
                            self.expect(')')?;
                            self.check_lookbehind_length(&inner)?;
                            Ok(Node::Lookbehind {
                                positive: false,
                                inner,
                            })
                        }
                        _ => {
                            // Named group (?<name>...)
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
                            Ok(Node::Group {
                                index: Some(index),
                                name: Some(name),
                                inner,
                            })
                        }
                    }
                }
                Some('=') => {
                    // Positive lookahead
                    self.advance();
                    let inner = self.parse_pattern()?;
                    self.expect(')')?;
                    Ok(Node::Lookahead {
                        positive: true,
                        inner,
                    })
                }
                Some('!') => {
                    // Negative lookahead
                    self.advance();
                    let inner = self.parse_pattern()?;
                    self.expect(')')?;
                    Ok(Node::Lookahead {
                        positive: false,
                        inner,
                    })
                }
                Some('>') => {
                    // Atomic group
                    self.advance();
                    let inner = self.parse_pattern()?;
                    self.expect(')')?;
                    Ok(Node::AtomicGroup { inner })
                }
                _ => {
                    // Inline flags: (?imsx), (?imsx:...), (?-imsx), etc.
                    self.parse_inline_flags()
                }
            }
        } else {
            // Capturing group
            self.group_count += 1;
            let index = self.group_count;
            let inner = self.parse_pattern()?;
            self.expect(')')?;
            Ok(Node::Group {
                index: Some(index),
                name: None,
                inner,
            })
        }
    }

    fn parse_inline_flags(&mut self) -> Result<Node, PatternSyntaxError> {
        let mut set_flags = Flags::default();
        let mut clear_flags = Flags::default();
        let mut clearing = false;

        loop {
            match self.peek() {
                Some('i') => {
                    self.advance();
                    if clearing { clear_flags.case_insensitive = true; }
                    else { set_flags.case_insensitive = true; }
                }
                Some('m') => {
                    self.advance();
                    if clearing { clear_flags.multiline = true; }
                    else { set_flags.multiline = true; }
                }
                Some('s') => {
                    self.advance();
                    if clearing { clear_flags.dotall = true; }
                    else { set_flags.dotall = true; }
                }
                Some('x') => {
                    self.advance();
                    if clearing { clear_flags.comments = true; }
                    else { set_flags.comments = true; }
                }
                Some('U') => {
                    self.advance();
                    if clearing { clear_flags.unicode_class = true; }
                    else { set_flags.unicode_class = true; }
                }
                Some('-') => {
                    self.advance();
                    clearing = true;
                }
                Some(':') => {
                    // (?flags:...)
                    self.advance();
                    let saved = self.flags;
                    self.apply_flags(set_flags, clear_flags);
                    let active_flags = self.flags;
                    let inner = self.parse_pattern()?;
                    self.flags = saved;
                    self.expect(')')?;
                    return Ok(Node::FlagGroup {
                        flags: active_flags,
                        inner,
                    });
                }
                Some(')') => {
                    // (?flags) - apply to rest of enclosing group
                    self.advance();
                    self.apply_flags(set_flags, clear_flags);
                    // Return a SetFlags node so the engine knows about the flag change
                    return Ok(Node::SetFlags(self.flags));
                }
                _ => {
                    return Err(PatternSyntaxError {
                        message: "Invalid inline flag".to_string(),
                    });
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
        if clear.case_insensitive { self.flags.case_insensitive = false; }
        if clear.multiline { self.flags.multiline = false; }
        if clear.dotall { self.flags.dotall = false; }
        if clear.comments { self.flags.comments = false; }
        if clear.unicode_class { self.flags.unicode_class = false; }
    }

    fn check_lookbehind_length(&self, pattern: &Pattern) -> Result<(), PatternSyntaxError> {
        for branch in &pattern.branches {
            for node in branch {
                if self.has_variable_length(node) {
                    return Err(PatternSyntaxError {
                        message: "Look-behind group does not have an obvious maximum length".to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    fn has_variable_length(&self, node: &Node) -> bool {
        match node {
            Node::Quantified { min, max, .. } => *min != *max,
            Node::Group { inner, .. } => {
                // Check if all branches have same length
                let lengths: Vec<Option<u32>> = inner.branches.iter()
                    .map(|b| self.branch_fixed_length(b))
                    .collect();
                if lengths.iter().any(|l| l.is_none()) {
                    return true;
                }
                let first = lengths[0];
                !lengths.iter().all(|l| *l == first)
            }
            _ => false,
        }
    }

    fn branch_fixed_length(&self, branch: &[Node]) -> Option<u32> {
        let mut len = 0u32;
        for node in branch {
            match node {
                Node::Literal(_) | Node::Dot | Node::CharClass(_) => len += 1,
                Node::Quantified { min, max, inner, .. } => {
                    if min != max { return None; }
                    let inner_len = self.node_fixed_length(inner)?;
                    len += min * inner_len;
                }
                Node::Group { inner, .. } => {
                    let branch_lens: Vec<Option<u32>> = inner.branches.iter()
                        .map(|b| self.branch_fixed_length(b))
                        .collect();
                    if branch_lens.iter().any(|l| l.is_none()) { return None; }
                    let first = branch_lens[0]?;
                    if !branch_lens.iter().all(|l| *l == Some(first)) { return None; }
                    len += first;
                }
                Node::Anchor(_) | Node::Lookahead { .. } | Node::Lookbehind { .. } => {}
                Node::LinebreakMatcher => return None, // variable length (\r\n vs \n)
                Node::GraphemeCluster => return None,
                _ => return None,
            }
        }
        Some(len)
    }

    fn node_fixed_length(&self, node: &Node) -> Option<u32> {
        match node {
            Node::Literal(_) | Node::Dot | Node::CharClass(_) => Some(1),
            Node::Group { inner, .. } => {
                let first = self.branch_fixed_length(&inner.branches[0])?;
                if inner.branches.iter().all(|b| self.branch_fixed_length(b) == Some(first)) {
                    Some(first)
                } else {
                    None
                }
            }
            Node::Anchor(_) => Some(0),
            _ => None,
        }
    }

    fn maybe_parse_quantifier(&mut self, node: Node) -> Result<Node, PatternSyntaxError> {
        // Don't quantify anchors or groups that are just flag setters
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

        Ok(Node::Quantified {
            inner: Box::new(node),
            min,
            max,
            kind,
        })
    }

    fn parse_quantifier_braces(&mut self) -> Result<(u32, u32), PatternSyntaxError> {
        let mut min_str = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                min_str.push(c);
                self.advance();
            } else {
                break;
            }
        }

        if min_str.is_empty() {
            return Err(PatternSyntaxError {
                message: "Invalid quantifier".to_string(),
            });
        }

        let min: u32 = min_str.parse().map_err(|_| PatternSyntaxError {
            message: "Invalid quantifier number".to_string(),
        })?;

        match self.peek() {
            Some('}') => {
                self.advance();
                Ok((min, min)) // {n} exact
            }
            Some(',') => {
                self.advance();
                if self.peek() == Some('}') {
                    self.advance();
                    Ok((min, u32::MAX)) // {n,}
                } else {
                    let mut max_str = String::new();
                    while let Some(c) = self.peek() {
                        if c.is_ascii_digit() {
                            max_str.push(c);
                            self.advance();
                        } else {
                            break;
                        }
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
            _ => Err(PatternSyntaxError {
                message: "Invalid quantifier".to_string(),
            }),
        }
    }

    // ==================== Character Class Parsing ====================

    fn parse_char_class_node(&mut self) -> Result<Node, PatternSyntaxError> {
        let cc = self.parse_char_class()?;
        Ok(Node::CharClass(cc))
    }

    fn parse_char_class(&mut self) -> Result<CharClass, PatternSyntaxError> {
        self.expect('[')?;

        let negated = if self.peek() == Some('^') {
            self.advance();
            true
        } else {
            false
        };

        let items = self.parse_char_class_items()?;
        self.expect(']')?;

        Ok(CharClass { negated, items })
    }

    fn parse_char_class_items(&mut self) -> Result<Vec<CharClassItem>, PatternSyntaxError> {
        let mut left_items = Vec::new();

        // First character can be ] or - literally
        if self.peek() == Some(']') && left_items.is_empty() {
            // Empty class not allowed; this ] closes it
            return Ok(left_items);
        }

        loop {
            // Skip whitespace and comments in COMMENTS mode inside char classes
            if self.flags.comments {
                self.skip_comments_whitespace();
            }
            match self.peek() {
                None => return Err(PatternSyntaxError {
                    message: "Unclosed character class".to_string(),
                }),
                Some(']') => break,
                Some('&') if self.pos + 1 < self.chars.len() && self.chars[self.pos + 1] == '&' => {
                    // Intersection — may be chained: [a-z&&[a-m]&&[g-z]]
                    self.advance(); // first &
                    self.advance(); // second &
                    let right_items = if self.peek() == Some('[') {
                        let nested = self.parse_char_class()?;
                        vec![CharClassItem::Nested(nested)]
                    } else {
                        self.parse_char_class_items()?
                    };
                    let mut result = vec![CharClassItem::Intersection(left_items, right_items)];
                    // Handle chained &&
                    while self.pos + 1 < self.chars.len()
                        && self.peek() == Some('&')
                        && self.chars[self.pos + 1] == '&'
                    {
                        self.advance(); // first &
                        self.advance(); // second &
                        let next_items = if self.peek() == Some('[') {
                            let nested = self.parse_char_class()?;
                            vec![CharClassItem::Nested(nested)]
                        } else {
                            self.parse_char_class_items()?
                        };
                        result = vec![CharClassItem::Intersection(result, next_items)];
                    }
                    return Ok(result);
                }
                _ => {
                    let item = self.parse_char_class_item()?;
                    // Check for range
                    if self.peek() == Some('-') && self.pos + 1 < self.chars.len() && self.chars[self.pos + 1] != ']' {
                        if let CharClassItem::Single(start) = item {
                            self.advance(); // consume '-'
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
                                // Not a simple range, push start and - as literals
                                left_items.push(CharClassItem::Single(start));
                                left_items.push(CharClassItem::Single('-'));
                                left_items.push(end_item);
                                continue;
                            }
                        }
                    }
                    left_items.push(item);
                }
            }
        }

        Ok(left_items)
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
                    'p' => self.parse_char_class_unicode_property(false),
                    'P' => self.parse_char_class_unicode_property(true),
                    't' => Ok(CharClassItem::Single('\t')),
                    'n' => Ok(CharClassItem::Single('\n')),
                    'r' => Ok(CharClassItem::Single('\r')),
                    'f' => Ok(CharClassItem::Single('\x0C')),
                    'a' => Ok(CharClassItem::Single('\x07')),
                    'e' => Ok(CharClassItem::Single('\x1B')),
                    'x' => {
                        if self.peek() == Some('{') {
                            self.advance();
                            let mut hex = String::new();
                            while let Some(c) = self.peek() {
                                if c == '}' { self.advance(); break; }
                                hex.push(c);
                                self.advance();
                            }
                            let code = u32::from_str_radix(&hex, 16).unwrap_or(0);
                            Ok(CharClassItem::Single(char::from_u32(code).unwrap_or('\0')))
                        } else {
                            let mut hex = String::new();
                            for _ in 0..2 {
                                if let Some(c) = self.peek() {
                                    if c.is_ascii_hexdigit() { hex.push(c); self.advance(); }
                                    else { break; }
                                }
                            }
                            let code = u32::from_str_radix(&hex, 16).unwrap_or(0);
                            Ok(CharClassItem::Single(char::from_u32(code).unwrap_or('\0')))
                        }
                    }
                    'u' => {
                        let mut hex = String::new();
                        for _ in 0..4 {
                            if let Some(c) = self.peek() {
                                if c.is_ascii_hexdigit() { hex.push(c); self.advance(); }
                                else { break; }
                            }
                        }
                        let code = u32::from_str_radix(&hex, 16).unwrap_or(0);
                        Ok(CharClassItem::Single(char::from_u32(code).unwrap_or('\0')))
                    }
                    '0' => {
                        let mut oct = String::new();
                        for _ in 0..3 {
                            if let Some(c) = self.peek() {
                                if ('0'..='7').contains(&c) { oct.push(c); self.advance(); }
                                else { break; }
                            }
                        }
                        let code = if oct.is_empty() { 0 } else { u32::from_str_radix(&oct, 8).unwrap_or(0) };
                        Ok(CharClassItem::Single(char::from_u32(code).unwrap_or('\0')))
                    }
                    'Q' => {
                        // \Q...\E inside character class - treat as literals
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
                        // Can't return multiple items, wrap in nested
                        Ok(CharClassItem::Nested(CharClass { negated: false, items }))
                    }
                    _ => Ok(CharClassItem::Single(c)),
                }
            }
            Some('[') => {
                // Check for POSIX class [[:alpha:]]
                if self.pos + 2 < self.chars.len() && self.chars[self.pos + 1] == ':' {
                    let saved = self.pos;
                    self.advance(); // [
                    self.advance(); // :
                    let negated = if self.peek() == Some('^') {
                        self.advance();
                        true
                    } else {
                        false
                    };
                    let mut name = String::new();
                    while let Some(c) = self.peek() {
                        if c == ':' { break; }
                        name.push(c);
                        self.advance();
                    }
                    if self.peek() == Some(':') && self.pos + 1 < self.chars.len() && self.chars[self.pos + 1] == ']' {
                        self.advance(); // :
                        self.advance(); // ]
                        return Ok(CharClassItem::PosixClass { name, negated });
                    }
                    // Not a POSIX class, revert
                    self.pos = saved;
                }
                // Nested character class
                let nested = self.parse_char_class()?;
                Ok(CharClassItem::Nested(nested))
            }
            Some(c) => {
                self.advance();
                Ok(CharClassItem::Single(c))
            }
            None => Err(PatternSyntaxError {
                message: "Unexpected end in character class".to_string(),
            }),
        }
    }

    fn parse_char_class_unicode_property(&mut self, negated: bool) -> Result<CharClassItem, PatternSyntaxError> {
        if self.peek() == Some('{') {
            self.advance();
            let mut name = String::new();
            while let Some(c) = self.peek() {
                if c == '}' { self.advance(); break; }
                name.push(c);
                self.advance();
            }
            Ok(CharClassItem::UnicodeProperty { name, negated })
        } else {
            let c = self.advance().ok_or_else(|| PatternSyntaxError {
                message: "Expected property name".to_string(),
            })?;
            Ok(CharClassItem::UnicodeProperty { name: c.to_string(), negated })
        }
    }
}

// ==================== Match Engine ====================

struct Engine {
    input: Vec<char>,
    flags: Flags,
    group_count: usize,
    named_groups: HashMap<String, usize>,
    steps: u64,
    max_steps: u64,
    search_start: usize, // for \G anchor
}

#[derive(Clone, Debug)]
struct State {
    captures: Vec<Option<(usize, usize)>>,
    match_end: usize,
}

impl State {
    fn new(group_count: usize) -> Self {
        State {
            captures: vec![None; group_count + 1],
            match_end: 0,
        }
    }
}

impl Engine {
    fn new(input: &str, flags: Flags, group_count: usize, named_groups: HashMap<String, usize>) -> Self {
        Engine {
            input: input.chars().collect(),
            flags,
            group_count,
            named_groups,
            steps: 0,
            max_steps: 5_000_000,
            search_start: 0,
        }
    }

    fn step(&mut self) -> bool {
        self.steps += 1;
        self.steps < self.max_steps
    }

    /// Try to match pattern at position pos.
    /// Returns (end_pos, captures) if successful.
    #[allow(clippy::type_complexity)]
    fn try_match_at(&mut self, pattern: &Pattern, pos: usize) -> Option<(usize, Vec<Option<(usize, usize)>>)> {
        let mut state = State::new(self.group_count);
        if self.match_pattern(pattern, &[], pos, &mut state) {
            Some((state.match_end, state.captures))
        } else {
            None
        }
    }

    /// Match a pattern (alternation of branches) at pos, then continue with rest.
    fn match_pattern(&mut self, pattern: &Pattern, rest: &[Node], pos: usize, state: &mut State) -> bool {
        for branch in &pattern.branches {
            let saved = state.captures.clone();
            let mut combined = branch.clone();
            combined.extend_from_slice(rest);
            if self.match_nodes(&combined, pos, state) {
                return true;
            }
            state.captures = saved;
        }
        false
    }

    /// Match a sequence of nodes starting at pos.
    fn match_nodes(&mut self, nodes: &[Node], pos: usize, state: &mut State) -> bool {
        if !self.step() {
            return false;
        }

        if nodes.is_empty() {
            state.match_end = pos;
            return true;
        }

        match &nodes[0] {
            Node::Literal(ch) => {
                if self.flags.case_insensitive {
                    if pos < self.input.len() && chars_eq_ci(self.input[pos], *ch) {
                        self.match_nodes(&nodes[1..], pos + 1, state)
                    } else {
                        false
                    }
                } else if pos < self.input.len() && self.input[pos] == *ch {
                    self.match_nodes(&nodes[1..], pos + 1, state)
                } else {
                    false
                }
            }

            Node::Dot => {
                if pos < self.input.len() {
                    if self.flags.dotall || !is_line_terminator(self.input[pos]) {
                        self.match_nodes(&nodes[1..], pos + 1, state)
                    } else {
                        false
                    }
                } else {
                    false
                }
            }

            Node::Anchor(kind) => {
                if self.check_anchor(*kind, pos) {
                    self.match_nodes(&nodes[1..], pos, state)
                } else {
                    false
                }
            }

            Node::CharClass(cc) => {
                if pos < self.input.len() && self.match_char_class(cc, self.input[pos]) {
                    self.match_nodes(&nodes[1..], pos + 1, state)
                } else {
                    false
                }
            }

            Node::Group { index, inner, .. } => {
                let start = pos;
                for branch in &inner.branches {
                    let saved = state.captures.clone();
                    let mut combined = branch.clone();
                    if let Some(idx) = index {
                        combined.push(Node::GroupEnd { index: *idx, start });
                    }
                    combined.extend_from_slice(&nodes[1..]);
                    if self.match_nodes(&combined, pos, state) {
                        return true;
                    }
                    state.captures = saved;
                }
                false
            }

            Node::GroupEnd { index, start } => {
                state.captures[*index] = Some((*start, pos));
                self.match_nodes(&nodes[1..], pos, state)
            }

            Node::Quantified { inner, min, max, kind } => {
                let rest = &nodes[1..];
                match kind {
                    QuantKind::Greedy => self.match_greedy(inner, *min, *max, 0, rest, pos, state),
                    QuantKind::Reluctant => self.match_reluctant(inner, *min, *max, 0, rest, pos, state),
                    QuantKind::Possessive => self.match_possessive(inner, *min, *max, rest, pos, state),
                }
            }

            Node::Lookahead { positive, inner } => {
                let mut temp_state = State::new(self.group_count);
                temp_state.captures = state.captures.clone();
                let matched = self.match_pattern(inner, &[], pos, &mut temp_state);
                if matched == *positive {
                    // Copy captures from lookahead (Java does this for positive lookahead)
                    if *positive {
                        state.captures = temp_state.captures;
                    }
                    self.match_nodes(&nodes[1..], pos, state)
                } else {
                    false
                }
            }

            Node::Lookbehind { positive, inner } => {
                let found = self.check_lookbehind(inner, pos, state);
                if found == *positive {
                    self.match_nodes(&nodes[1..], pos, state)
                } else {
                    false
                }
            }

            Node::AtomicGroup { inner } => {
                // Match inner, but don't allow backtracking into it
                let mut temp_state = State::new(self.group_count);
                temp_state.captures = state.captures.clone();
                if self.match_pattern(inner, &[], pos, &mut temp_state) {
                    state.captures = temp_state.captures;
                    self.match_nodes(&nodes[1..], temp_state.match_end, state)
                } else {
                    false
                }
            }

            Node::Backreference(idx) => {
                if let Some(Some((start, end))) = state.captures.get(*idx) {
                    let captured: Vec<char> = self.input[*start..*end].to_vec();
                    let mut p = pos;
                    for &ch in &captured {
                        if p >= self.input.len() { return false; }
                        if self.flags.case_insensitive {
                            if !chars_eq_ci(self.input[p], ch) { return false; }
                        } else if self.input[p] != ch {
                            return false;
                        }
                        p += 1;
                    }
                    self.match_nodes(&nodes[1..], p, state)
                } else {
                    false
                }
            }

            Node::NamedBackreference(name) => {
                if let Some(&idx) = self.named_groups.get(name) {
                    if let Some(Some((start, end))) = state.captures.get(idx) {
                        let captured: Vec<char> = self.input[*start..*end].to_vec();
                        let mut p = pos;
                        for &ch in &captured {
                            if p >= self.input.len() { return false; }
                            if self.flags.case_insensitive {
                                if !chars_eq_ci(self.input[p], ch) { return false; }
                            } else if self.input[p] != ch {
                                return false;
                            }
                            p += 1;
                        }
                        self.match_nodes(&nodes[1..], p, state)
                    } else {
                        false
                    }
                } else {
                    false
                }
            }

            Node::LinebreakMatcher => {
                // \R matches \r\n, \n, \r, \u000B, \u000C, \u0085, \u2028, \u2029
                if pos < self.input.len() {
                    if self.input[pos] == '\r' && pos + 1 < self.input.len() && self.input[pos + 1] == '\n' {
                        return self.match_nodes(&nodes[1..], pos + 2, state);
                    }
                    if is_linebreak(self.input[pos]) {
                        return self.match_nodes(&nodes[1..], pos + 1, state);
                    }
                }
                false
            }

            Node::SetFlags(new_flags) => {
                let old_flags = self.flags;
                self.flags = *new_flags;
                let result = self.match_nodes(&nodes[1..], pos, state);
                if !result {
                    self.flags = old_flags;
                }
                result
            }

            Node::FlagGroup { flags, inner } => {
                let old_flags = self.flags;
                self.flags = *flags;
                // Match inner pattern, then RestoreFlags, then rest
                for branch in &inner.branches {
                    let saved = state.captures.clone();
                    let mut combined = branch.clone();
                    combined.push(Node::RestoreFlags(old_flags));
                    combined.extend_from_slice(&nodes[1..]);
                    if self.match_nodes(&combined, pos, state) {
                        return true;
                    }
                    state.captures = saved;
                }
                self.flags = old_flags;
                false
            }

            Node::RestoreFlags(flags) => {
                self.flags = *flags;
                self.match_nodes(&nodes[1..], pos, state)
            }

            Node::GraphemeCluster => {
                // \X - simplified: match one or more chars that form a grapheme cluster
                // For simplicity: match one base char + any following combining marks
                if pos >= self.input.len() {
                    return false;
                }
                let mut p = pos + 1;
                while p < self.input.len() && is_combining_mark(self.input[p]) {
                    p += 1;
                }
                // Handle regional indicator sequences (flag emoji: pairs of regional indicators)
                if is_regional_indicator(self.input[pos]) {
                    while p < self.input.len() && is_regional_indicator(self.input[p]) {
                        p += 1;
                    }
                }
                self.match_nodes(&nodes[1..], p, state)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn match_greedy(
        &mut self,
        atom: &Node,
        min: u32,
        max: u32,
        count: u32,
        rest: &[Node],
        pos: usize,
        state: &mut State,
    ) -> bool {
        if !self.step() {
            return false;
        }

        // Try to match one more instance of atom
        if count < max {
            let saved = state.captures.clone();
            // Build: [atom_expansion, GreedyCont_equivalent]
            // For efficiency, directly try to match atom and recurse
            if self.try_match_atom_greedy(atom, min, max, count, rest, pos, state) {
                return true;
            }
            state.captures = saved;
        }

        // Try rest with current count
        if count >= min {
            return self.match_nodes(rest, pos, state);
        }

        false
    }

    #[allow(clippy::too_many_arguments)]
    fn try_match_atom_greedy(
        &mut self,
        atom: &Node,
        min: u32,
        max: u32,
        count: u32,
        rest: &[Node],
        pos: usize,
        state: &mut State,
    ) -> bool {
        // Match one instance of atom, then try more greedy iterations
        match atom {
            Node::Literal(ch) => {
                if self.flags.case_insensitive {
                    if pos < self.input.len() && chars_eq_ci(self.input[pos], *ch) {
                        self.match_greedy(atom, min, max, count + 1, rest, pos + 1, state)
                    } else {
                        false
                    }
                } else if pos < self.input.len() && self.input[pos] == *ch {
                    self.match_greedy(atom, min, max, count + 1, rest, pos + 1, state)
                } else {
                    false
                }
            }
            Node::Dot => {
                if pos < self.input.len() && (self.flags.dotall || self.input[pos] != '\n') {
                    self.match_greedy(atom, min, max, count + 1, rest, pos + 1, state)
                } else {
                    false
                }
            }
            Node::CharClass(cc) => {
                if pos < self.input.len() && self.match_char_class(cc, self.input[pos]) {
                    self.match_greedy(atom, min, max, count + 1, rest, pos + 1, state)
                } else {
                    false
                }
            }
            Node::Group { index, inner, .. } => {
                let start = pos;
                for branch in &inner.branches {
                    let saved = state.captures.clone();
                    // Match branch, then GroupEnd, then GreedyCont
                    let mut combined = branch.clone();
                    if let Some(idx) = index {
                        combined.push(Node::GroupEnd { index: *idx, start });
                    }
                    // After branch + GroupEnd, we need to continue with more greedy iterations
                    // We'll use a helper: if branch matches, we get new_pos, then recurse
                    let mut branch_state = state.clone();
                    if self.match_nodes_to_end(&combined, pos, &mut branch_state) {
                        let new_pos = branch_state.match_end;
                        if new_pos > pos {
                            state.captures = branch_state.captures.clone();
                            if self.match_greedy(atom, min, max, count + 1, rest, new_pos, state) {
                                return true;
                            }
                        } else if new_pos == pos && count >= min {
                            // Zero-width match - stop looping, try rest
                            state.captures = branch_state.captures.clone();
                            if self.match_nodes(rest, pos, state) {
                                return true;
                            }
                        }
                    }
                    state.captures = saved;
                }
                false
            }
            Node::LinebreakMatcher => {
                if pos < self.input.len() {
                    if self.input[pos] == '\r' && pos + 1 < self.input.len() && self.input[pos + 1] == '\n' {
                        return self.match_greedy(atom, min, max, count + 1, rest, pos + 2, state);
                    }
                    if is_linebreak(self.input[pos]) {
                        return self.match_greedy(atom, min, max, count + 1, rest, pos + 1, state);
                    }
                }
                false
            }
            Node::Backreference(idx) => {
                if let Some(Some((start, end))) = state.captures.get(*idx).cloned() {
                    let cap_len = end - start;
                    if cap_len == 0 {
                        return false; // avoid infinite loop on empty backreference
                    }
                    let captured: Vec<char> = self.input[start..end].to_vec();
                    let mut p = pos;
                    for &ch in &captured {
                        if p >= self.input.len() { return false; }
                        if self.flags.case_insensitive {
                            if !chars_eq_ci(self.input[p], ch) { return false; }
                        } else if self.input[p] != ch {
                            return false;
                        }
                        p += 1;
                    }
                    self.match_greedy(atom, min, max, count + 1, rest, p, state)
                } else {
                    false
                }
            }
            _ => {
                // For other node types, use the generic approach
                let mut temp_state = state.clone();
                if self.match_nodes(std::slice::from_ref(atom), pos, &mut temp_state) {
                    let new_pos = temp_state.match_end;
                    if new_pos > pos {
                        state.captures = temp_state.captures;
                        self.match_greedy(atom, min, max, count + 1, rest, new_pos, state)
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn match_reluctant(
        &mut self,
        atom: &Node,
        min: u32,
        max: u32,
        count: u32,
        rest: &[Node],
        pos: usize,
        state: &mut State,
    ) -> bool {
        if !self.step() {
            return false;
        }

        // Reluctant: try rest first (prefer fewer matches)
        if count >= min {
            let saved = state.captures.clone();
            if self.match_nodes(rest, pos, state) {
                return true;
            }
            state.captures = saved;
        }

        // Try one more
        if count < max {
            self.try_match_atom_reluctant(atom, min, max, count, rest, pos, state)
        } else {
            false
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn try_match_atom_reluctant(
        &mut self,
        atom: &Node,
        min: u32,
        max: u32,
        count: u32,
        rest: &[Node],
        pos: usize,
        state: &mut State,
    ) -> bool {
        match atom {
            Node::Literal(ch) => {
                if self.flags.case_insensitive {
                    if pos < self.input.len() && chars_eq_ci(self.input[pos], *ch) {
                        self.match_reluctant(atom, min, max, count + 1, rest, pos + 1, state)
                    } else {
                        false
                    }
                } else if pos < self.input.len() && self.input[pos] == *ch {
                    self.match_reluctant(atom, min, max, count + 1, rest, pos + 1, state)
                } else {
                    false
                }
            }
            Node::Dot => {
                if pos < self.input.len() && (self.flags.dotall || self.input[pos] != '\n') {
                    self.match_reluctant(atom, min, max, count + 1, rest, pos + 1, state)
                } else {
                    false
                }
            }
            Node::CharClass(cc) => {
                if pos < self.input.len() && self.match_char_class(cc, self.input[pos]) {
                    self.match_reluctant(atom, min, max, count + 1, rest, pos + 1, state)
                } else {
                    false
                }
            }
            Node::Group { index, inner, .. } => {
                let start = pos;
                for branch in &inner.branches {
                    let saved = state.captures.clone();
                    let mut combined = branch.clone();
                    if let Some(idx) = index {
                        combined.push(Node::GroupEnd { index: *idx, start });
                    }
                    let mut branch_state = state.clone();
                    if self.match_nodes_to_end(&combined, pos, &mut branch_state) {
                        let new_pos = branch_state.match_end;
                        if new_pos > pos {
                            state.captures = branch_state.captures.clone();
                            if self.match_reluctant(atom, min, max, count + 1, rest, new_pos, state) {
                                return true;
                            }
                        }
                    }
                    state.captures = saved;
                }
                false
            }
            _ => {
                let mut temp_state = state.clone();
                if self.match_nodes(std::slice::from_ref(atom), pos, &mut temp_state) {
                    let new_pos = temp_state.match_end;
                    if new_pos > pos {
                        state.captures = temp_state.captures;
                        self.match_reluctant(atom, min, max, count + 1, rest, new_pos, state)
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
        }
    }

    fn match_possessive(
        &mut self,
        atom: &Node,
        min: u32,
        max: u32,
        rest: &[Node],
        pos: usize,
        state: &mut State,
    ) -> bool {
        // Match as many as possible, no backtracking
        let mut current_pos = pos;
        let mut count = 0u32;

        while count < max {
            let mut temp_state = state.clone();
            if self.match_nodes(std::slice::from_ref(atom), current_pos, &mut temp_state) {
                let new_pos = temp_state.match_end;
                if new_pos <= current_pos {
                    break; // no progress
                }
                state.captures = temp_state.captures;
                current_pos = new_pos;
                count += 1;
            } else {
                break;
            }
        }

        if count >= min {
            self.match_nodes(rest, current_pos, state)
        } else {
            false
        }
    }

    /// Match nodes to completion (no rest), returning match_end in state.
    fn match_nodes_to_end(&mut self, nodes: &[Node], pos: usize, state: &mut State) -> bool {
        self.match_nodes(nodes, pos, state)
    }

    fn check_anchor(&self, kind: AnchorKind, pos: usize) -> bool {
        match kind {
            AnchorKind::StartOfLine => {
                if self.flags.multiline {
                    pos == 0 || (pos > 0 && self.input[pos - 1] == '\n')
                } else {
                    pos == 0
                }
            }
            AnchorKind::EndOfLine => {
                if self.flags.multiline {
                    if pos < self.input.len() {
                        is_line_terminator(self.input[pos])
                    } else {
                        // At end of input: match $ only if input doesn't end with a line terminator
                        self.input.is_empty() || !is_line_terminator(*self.input.last().unwrap())
                    }
                } else {
                    pos == self.input.len()
                        || (pos == self.input.len() - 1 && self.input[pos] == '\n')
                }
            }
            AnchorKind::StartOfInput => pos == 0,
            AnchorKind::EndOfInput => pos == self.input.len(),
            AnchorKind::EndOfInputBeforeFinalNewline => {
                pos == self.input.len()
                    || (pos == self.input.len() - 1 && self.input[pos] == '\n')
            }
            AnchorKind::WordBoundary => {
                let before = if pos > 0 { is_word_char(self.input[pos - 1], self.flags.unicode_class) } else { false };
                let after = if pos < self.input.len() { is_word_char(self.input[pos], self.flags.unicode_class) } else { false };
                before != after
            }
            AnchorKind::NonWordBoundary => {
                let before = if pos > 0 { is_word_char(self.input[pos - 1], self.flags.unicode_class) } else { false };
                let after = if pos < self.input.len() { is_word_char(self.input[pos], self.flags.unicode_class) } else { false };
                before == after
            }
            AnchorKind::PreviousMatchEnd => {
                pos == self.search_start
            }
        }
    }

    fn check_lookbehind(&mut self, inner: &Pattern, pos: usize, state: &mut State) -> bool {
        // Try all possible start positions
        for start in (0..=pos).rev() {
            let mut temp_state = State::new(self.group_count);
            temp_state.captures = state.captures.clone();
            if self.match_pattern(inner, &[], start, &mut temp_state)
                && temp_state.match_end == pos {
                // Copy captures from lookbehind
                state.captures = temp_state.captures;
                return true;
            }
        }
        false
    }

    fn match_char_class(&self, cc: &CharClass, ch: char) -> bool {
        let matched = self.match_char_class_items(&cc.items, ch);
        if cc.negated { !matched } else { matched }
    }

    fn match_char_class_items(&self, items: &[CharClassItem], ch: char) -> bool {
        for item in items {
            match item {
                CharClassItem::Single(c) => {
                    if self.flags.case_insensitive {
                        if chars_eq_ci(ch, *c) { return true; }
                    } else if ch == *c {
                        return true;
                    }
                }
                CharClassItem::Range(start, end) => {
                    if self.flags.case_insensitive {
                        let ch_lower = ch.to_lowercase().next().unwrap_or(ch);
                        let ch_upper = ch.to_uppercase().next().unwrap_or(ch);
                        let s_lower = start.to_lowercase().next().unwrap_or(*start);
                        let e_lower = end.to_lowercase().next().unwrap_or(*end);
                        let s_upper = start.to_uppercase().next().unwrap_or(*start);
                        let e_upper = end.to_uppercase().next().unwrap_or(*end);
                        if (ch_lower >= s_lower && ch_lower <= e_lower) ||
                           (ch_upper >= s_upper && ch_upper <= e_upper) ||
                           (ch >= *start && ch <= *end) {
                            return true;
                        }
                    } else if ch >= *start && ch <= *end {
                        return true;
                    }
                }
                CharClassItem::Predefined(pc) => {
                    if match_predefined_class(*pc, ch, self.flags.unicode_class) {
                        return true;
                    }
                }
                CharClassItem::UnicodeProperty { name, negated } => {
                    let matched = match_unicode_property(name, ch);
                    if *negated { if !matched { return true; } }
                    else if matched { return true; }
                }
                CharClassItem::Nested(nested) => {
                    if self.match_char_class(nested, ch) {
                        return true;
                    }
                }
                CharClassItem::Intersection(left, right) => {
                    let left_match = self.match_char_class_items(left, ch);
                    let right_match = self.match_char_class_items(right, ch);
                    if left_match && right_match {
                        return true;
                    }
                }
                CharClassItem::PosixClass { name, negated } => {
                    let matched = match_posix_class(name, ch);
                    if *negated { if !matched { return true; } }
                    else if matched { return true; }
                }
            }
        }
        false
    }
}

// ==================== Character Matching Helpers ====================

fn chars_eq_ci(a: char, b: char) -> bool {
    if a == b { return true; }
    let a_lower = a.to_lowercase().next().unwrap_or(a);
    let b_lower = b.to_lowercase().next().unwrap_or(b);
    if a_lower == b_lower { return true; }
    let a_upper = a.to_uppercase().next().unwrap_or(a);
    let b_upper = b.to_uppercase().next().unwrap_or(b);
    a_upper == b_upper
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

fn is_word_char(c: char, unicode: bool) -> bool {
    if unicode {
        c.is_alphanumeric() || c == '_'
    } else {
        c.is_ascii_alphanumeric() || c == '_'
    }
}

fn is_linebreak(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\x0B' | '\x0C' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

fn is_combining_mark(c: char) -> bool {
    let cat = unicode_general_category(c);
    matches!(cat, UnicodeCategory::Mn | UnicodeCategory::Mc | UnicodeCategory::Me)
}

fn is_regional_indicator(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

fn match_predefined_class(pc: PredefinedClass, ch: char, unicode: bool) -> bool {
    match pc {
        PredefinedClass::Digit => {
            if unicode { ch.is_numeric() } else { ch.is_ascii_digit() }
        }
        PredefinedClass::NonDigit => {
            if unicode { !ch.is_numeric() } else { !ch.is_ascii_digit() }
        }
        PredefinedClass::Word => is_word_char(ch, unicode),
        PredefinedClass::NonWord => !is_word_char(ch, unicode),
        PredefinedClass::Whitespace => {
            if unicode {
                ch.is_whitespace()
            } else {
                matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0C' | '\x0B')
            }
        }
        PredefinedClass::NonWhitespace => {
            if unicode {
                !ch.is_whitespace()
            } else {
                !matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0C' | '\x0B')
            }
        }
        PredefinedClass::HorizWhitespace => {
            matches!(ch, '\t' | ' ' | '\u{00A0}' | '\u{1680}' | '\u{180E}' |
                '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}')
        }
        PredefinedClass::NonHorizWhitespace => {
            !matches!(ch, '\t' | ' ' | '\u{00A0}' | '\u{1680}' | '\u{180E}' |
                '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}')
        }
        PredefinedClass::VertWhitespace => {
            matches!(ch, '\n' | '\x0B' | '\x0C' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
        }
        PredefinedClass::NonVertWhitespace => {
            !matches!(ch, '\n' | '\x0B' | '\x0C' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
        }
    }
}

fn is_valid_unicode_property(name: &str) -> bool {
    let name = name.strip_prefix("Is").unwrap_or(name);
    let name_lower = name.to_lowercase();
    matches!(name_lower.as_str(),
        "l" | "letter" | "lu" | "uppercase_letter" | "upper" | "ll" | "lowercase_letter" | "lower" |
        "lt" | "titlecase_letter" | "lm" | "modifier_letter" | "lo" | "other_letter" |
        "m" | "mark" | "mn" | "nonspacing_mark" | "mc" | "spacing_mark" |
        "n" | "number" | "nd" | "decimal_digit_number" | "digit" | "nl" | "letter_number" | "no" | "other_number" |
        "p" | "punctuation" | "punct" |
        "pc" | "connector_punctuation" | "pd" | "dash_punctuation" |
        "ps" | "open_punctuation" | "pe" | "close_punctuation" |
        "pi" | "initial_punctuation" | "pf" | "final_punctuation" | "po" | "other_punctuation" |
        "s" | "symbol" | "sm" | "math_symbol" | "sc" | "currency_symbol" | "sk" | "modifier_symbol" | "so" | "other_symbol" |
        "z" | "separator" | "zs" | "space_separator" | "zl" | "line_separator" | "zp" | "paragraph_separator" |
        "c" | "control" | "other" | "cc" | "cntrl" | "cf" | "format" | "co" | "private_use" | "cn" | "unassigned" |
        "alpha" | "alnum" | "ascii" | "blank" | "graph" | "print" | "space" | "white_space" | "xdigit" |
        "greek" | "isgreek" | "latin" | "islatin" | "cyrillic" | "iscyrillic" |
        "han" | "ishan" | "arabic" | "isarabic" | "armenian" | "isarmenian" |
        "hebrew" | "ishebrew" | "thai" | "isthai" | "hiragana" | "ishiragana" |
        "katakana" | "iskatakana" | "devanagari" | "isdevanagari"
    )
}

fn match_unicode_property(name: &str, ch: char) -> bool {
    // Handle "Is" prefix for script names
    let name = name.strip_prefix("Is").unwrap_or(name);
    // Handle "In" prefix for block names
    let name_lower = name.to_lowercase();

    match name_lower.as_str() {
        // POSIX classes (ASCII-only) — must be checked before Unicode categories
        "upper" => ch.is_ascii_uppercase(),
        "lower" => ch.is_ascii_lowercase(),

        // Unicode General Categories
        "l" | "letter" => ch.is_alphabetic(),
        "lu" | "uppercase_letter" => ch.is_uppercase(),
        "ll" | "lowercase_letter" => ch.is_lowercase(),
        "lt" | "titlecase_letter" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Lt)
        }
        "lm" | "modifier_letter" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Lm)
        }
        "lo" | "other_letter" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Lo)
        }
        "m" | "mark" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Mn | UnicodeCategory::Mc | UnicodeCategory::Me)
        }
        "mn" | "nonspacing_mark" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Mn)
        }
        "mc" | "spacing_mark" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Mc)
        }
        "n" | "number" => ch.is_numeric(),
        "nd" | "decimal_digit_number" | "digit" => ch.is_ascii_digit() || {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Nd)
        },
        "nl" | "letter_number" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Nl)
        }
        "no" | "other_number" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::No)
        }
        "p" | "punctuation" | "punct" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Pc | UnicodeCategory::Pd | UnicodeCategory::Ps |
                UnicodeCategory::Pe | UnicodeCategory::Pi | UnicodeCategory::Pf | UnicodeCategory::Po)
        }
        "pc" | "connector_punctuation" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Pc)
        }
        "pd" | "dash_punctuation" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Pd)
        }
        "ps" | "open_punctuation" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Ps)
        }
        "pe" | "close_punctuation" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Pe)
        }
        "pi" | "initial_punctuation" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Pi)
        }
        "pf" | "final_punctuation" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Pf)
        }
        "po" | "other_punctuation" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Po)
        }
        "s" | "symbol" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Sm | UnicodeCategory::Sc | UnicodeCategory::Sk | UnicodeCategory::So)
        }
        "sm" | "math_symbol" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Sm)
        }
        "sc" | "currency_symbol" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Sc)
        }
        "sk" | "modifier_symbol" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Sk)
        }
        "so" | "other_symbol" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::So)
        }
        "z" | "separator" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Zs | UnicodeCategory::Zl | UnicodeCategory::Zp)
        }
        "c" | "control" | "other" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Cc | UnicodeCategory::Cf | UnicodeCategory::Co | UnicodeCategory::Cn)
        }
        "cc" | "cntrl" => {
            let cat = unicode_general_category(ch);
            matches!(cat, UnicodeCategory::Cc)
        }

        // POSIX-style
        "alpha" => ch.is_ascii_alphabetic(),
        "alnum" => ch.is_ascii_alphanumeric(),
        "ascii" => ch.is_ascii(),
        "blank" => ch == ' ' || ch == '\t',
        "graph" => ch.is_ascii_graphic(),
        "print" => ch.is_ascii_graphic() || ch == ' ',
        "space" | "white_space" => ch.is_ascii_whitespace(),
        "xdigit" => ch.is_ascii_hexdigit(),

        // Unicode scripts
        "greek" | "isgreek" => is_script_greek(ch),
        "latin" | "islatin" => is_script_latin(ch),
        "cyrillic" | "iscyrillic" => is_script_cyrillic(ch),
        "han" | "ishan" => is_script_han(ch),
        "arabic" | "isarabic" => is_script_arabic(ch),
        "armenian" | "isarmenian" => ('\u{0530}'..='\u{058F}').contains(&ch) || ('\u{FB00}'..='\u{FB17}').contains(&ch),
        "hebrew" | "ishebrew" => ('\u{0590}'..='\u{05FF}').contains(&ch) || ('\u{FB1D}'..='\u{FB4F}').contains(&ch),
        "thai" | "isthai" => ('\u{0E00}'..='\u{0E7F}').contains(&ch),
        "hiragana" | "ishiragana" => ('\u{3040}'..='\u{309F}').contains(&ch),
        "katakana" | "iskatakana" => ('\u{30A0}'..='\u{30FF}').contains(&ch),
        "devanagari" | "isdevanagari" => ('\u{0900}'..='\u{097F}').contains(&ch),

        _ => false,
    }
}

fn match_posix_class(name: &str, ch: char) -> bool {
    match name {
        "digit" => ch.is_ascii_digit(),
        "alpha" => ch.is_ascii_alphabetic(),
        "alnum" => ch.is_ascii_alphanumeric(),
        "upper" => ch.is_ascii_uppercase(),
        "lower" => ch.is_ascii_lowercase(),
        "space" => ch.is_ascii_whitespace(),
        "blank" => ch == ' ' || ch == '\t',
        "punct" => ch.is_ascii_punctuation(),
        "graph" => ch.is_ascii_graphic(),
        "print" => ch.is_ascii_graphic() || ch == ' ',
        "cntrl" => ch.is_ascii_control(),
        "xdigit" => ch.is_ascii_hexdigit(),
        "ascii" => ch.is_ascii(),
        _ => false,
    }
}

// Unicode script detection helpers
fn is_script_greek(ch: char) -> bool {
    ('\u{0370}'..='\u{03FF}').contains(&ch) ||
    ('\u{1F00}'..='\u{1FFF}').contains(&ch)
}

fn is_script_latin(ch: char) -> bool {
    ch.is_ascii_uppercase() ||
    ch.is_ascii_lowercase() ||
    ('\u{00C0}'..='\u{00FF}').contains(&ch) ||
    ('\u{0100}'..='\u{024F}').contains(&ch) ||
    ('\u{1E00}'..='\u{1EFF}').contains(&ch)
}

fn is_script_cyrillic(ch: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&ch) ||
    ('\u{0500}'..='\u{052F}').contains(&ch)
}

fn is_script_han(ch: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&ch) ||
    ('\u{3400}'..='\u{4DBF}').contains(&ch)
}

fn is_script_arabic(ch: char) -> bool {
    ('\u{0600}'..='\u{06FF}').contains(&ch) ||
    ('\u{0750}'..='\u{077F}').contains(&ch)
}

// Simplified Unicode General Category detection
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
enum UnicodeCategory {
    Lu, Ll, Lt, Lm, Lo,  // Letter
    Mn, Mc, Me,           // Mark
    Nd, Nl, No,           // Number
    Pc, Pd, Ps, Pe, Pi, Pf, Po, // Punctuation
    Sm, Sc, Sk, So,       // Symbol
    Zs, Zl, Zp,           // Separator
    Cc, Cf, Co, Cn,       // Other
}

fn unicode_general_category(ch: char) -> UnicodeCategory {
    if ch.is_ascii_uppercase() || (ch.is_uppercase() && !ch.is_ascii()) {
        UnicodeCategory::Lu
    } else if ch.is_ascii_lowercase() || (ch.is_lowercase() && !ch.is_ascii()) {
        UnicodeCategory::Ll
    } else if ch.is_ascii_digit() || ch.is_numeric() {
        UnicodeCategory::Nd
    } else if ch.is_alphabetic() && !ch.is_uppercase() && !ch.is_lowercase() {
        UnicodeCategory::Lo
    } else if ch.is_ascii_control() || ch.is_control() {
        if ch.is_ascii_control() { UnicodeCategory::Cc } else { UnicodeCategory::Cf }
    } else if ch.is_whitespace() {
        if ch == '\n' || ch == '\r' || ch == '\t' || ch == '\x0B' || ch == '\x0C' {
            UnicodeCategory::Cc
        } else {
            UnicodeCategory::Zs
        }
    } else {
        // Try to categorize punctuation and symbols
        let cp = ch as u32;
        if matches!(ch, '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>') {
            if matches!(ch, '(' | '[' | '{' | '<') { UnicodeCategory::Ps } else { UnicodeCategory::Pe }
        } else if matches!(ch, '!' | '"' | '#' | '%' | '&' | '\'' | '*' | ',' | '.' | '/' | ':' | ';' | '?' | '@' | '\\' | '_') {
            UnicodeCategory::Po
        } else if matches!(ch, '+' | '=' | '|' | '~' | '^') {
            UnicodeCategory::Sm
        } else if matches!(ch, '$' | '\u{00A2}'..='\u{00A5}') {
            UnicodeCategory::Sc
        } else if ch == '-' {
            UnicodeCategory::Pd
        } else if ch == '`' {
            UnicodeCategory::Sk
        } else if (0xE000..=0xF8FF).contains(&cp) {
            UnicodeCategory::Co
        } else if (0x0300..=0x036F).contains(&cp) || (0x0483..=0x0489).contains(&cp) ||
                  (0x0591..=0x05BD).contains(&cp) || (0x0610..=0x061A).contains(&cp) ||
                  (0x064B..=0x065F).contains(&cp) || (0x0670..=0x0670).contains(&cp) ||
                  (0x06D6..=0x06DC).contains(&cp) || (0x06DF..=0x06E4).contains(&cp) ||
                  (0x0900..=0x0903).contains(&cp) || (0x093A..=0x094F).contains(&cp) ||
                  (0x0951..=0x0957).contains(&cp) || (0x0962..=0x0963).contains(&cp) ||
                  (0x0981..=0x0983).contains(&cp) || (0x09BC..=0x09CD).contains(&cp) ||
                  (0x0A01..=0x0A03).contains(&cp) || (0x0A3C..=0x0A4D).contains(&cp) ||
                  (0xFE20..=0xFE2F).contains(&cp) || (0x20D0..=0x20FF).contains(&cp) {
            UnicodeCategory::Mn
        } else {
            UnicodeCategory::Cn
        }
    }
}

// ==================== Public API ====================

#[derive(Debug, Clone)]
pub struct Regex {
    pattern: Pattern,
    flags: Flags,
    group_count: usize,
    named_groups: HashMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchResult {
    pub matched: bool,
    pub matches: Vec<MatchInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchInfo {
    pub matched_text: String,
    pub start: usize,
    pub end: usize,
    pub groups: Vec<Option<String>>,
    pub named_groups: HashMap<String, String>,
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

        // Use EndOfInput anchor as continuation to ensure full match
        let end_anchor = vec![Node::Anchor(AnchorKind::EndOfInput)];
        engine.match_pattern(&self.pattern, &end_anchor, 0, &mut state)
    }

    /// Find all non-overlapping matches.
    pub fn find(&self, input: &str) -> Vec<MatchInfo> {
        let mut results = Vec::new();
        let input_chars: Vec<char> = input.chars().collect();
        let input_len = input_chars.len();
        let mut search_pos = 0;
        let mut prev_match_end = 0usize; // for \G anchor

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
                    matched_text: matched_text.clone(),
                    start: search_pos,
                    end: end_pos,
                    groups,
                    named_groups: named,
                });

                prev_match_end = end_pos;
                if end_pos == search_pos {
                    search_pos += 1; // Avoid infinite loop on zero-width match
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

            if let Some((end_pos, captures)) = engine.try_match_at(&self.pattern, search_pos) {
                // Append text before match
                result.extend(&input_chars[last_end..search_pos]);

                // Build replacement
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

        // Append remaining text
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
                    // Named group ${name}
                    i += 1;
                    let mut name = String::new();
                    while i < rep_chars.len() && rep_chars[i] != '}' {
                        name.push(rep_chars[i]);
                        i += 1;
                    }
                    if i < rep_chars.len() { i += 1; } // skip }
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
                // Skip zero-width matches at the same position (avoid infinite loop)
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

        // Add remaining
        parts.push(input_chars[last_end..].iter().collect());

        // Java: remove trailing empty strings
        while parts.last().is_some_and(|s: &String| s.is_empty()) {
            parts.pop();
        }

        // Java: if no match was found at all, return the original input
        if parts.is_empty() && last_end == 0 {
            parts.push(input.to_string());
        }

        parts
    }
}

// ==================== Tests ====================

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
        assert!(!regex.matches("abc")); // atomic group prevents backtracking
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
        assert!(!regex.matches("a123b")); // possessive prevents backtracking
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
    fn test_posix_class() {
        let regex = Regex::new("[[:digit:]]+").unwrap();
        assert!(regex.matches("123"));
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
}

