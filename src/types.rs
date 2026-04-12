use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone)]
pub struct PatternSyntaxError {
    pub message: String,
}

impl fmt::Display for PatternSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PatternSyntaxException: {}", self.message)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Flags {
    pub case_insensitive: bool, // i / CASE_INSENSITIVE
    pub multiline: bool,        // m / MULTILINE
    pub dotall: bool,           // s / DOTALL
    pub comments: bool,         // x / COMMENTS
    pub unicode_class: bool,    // U / UNICODE_CHARACTER_CLASS
    pub unix_lines: bool,       // d / UNIX_LINES
    pub unicode_case: bool,     // u / UNICODE_CASE
}

/// A pattern is an alternation of branches.
#[derive(Clone, Debug)]
pub struct Pattern {
    pub branches: Vec<Vec<Node>>,
}

#[derive(Clone, Debug)]
pub enum Node {
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
    RestoreFlags(Flags),  // engine-internal: restore flags after FlagGroup
    #[allow(dead_code)]
    PositionCheck(usize), // engine-internal: assert current pos == target
    GreedyCont {           // engine-internal: continue greedy quantifier loop
        atom: Box<Node>,
        min: u32,
        max: u32,
        count: u32,
        rest: Vec<Node>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnchorKind {
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
pub enum QuantKind {
    Greedy,
    Reluctant,
    Possessive,
}

#[derive(Clone, Debug)]
pub struct CharClass {
    pub negated: bool,
    pub items: Vec<CharClassItem>,
}

#[derive(Clone, Debug)]
pub enum CharClassItem {
    Single(char),
    Range(char, char),
    Predefined(PredefinedClass),
    UnicodeProperty {
        name: String,
        negated: bool,
    },
    Nested(CharClass),
    Intersection(Vec<CharClassItem>, Vec<CharClassItem>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PredefinedClass {
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

#[derive(Debug, Clone, PartialEq)]
pub struct MatchInfo {
    pub matched_text: String,
    pub start: usize,
    pub end: usize,
    pub groups: Vec<Option<String>>,
    pub named_groups: HashMap<String, String>,
}
