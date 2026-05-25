use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Error returned when a regex pattern fails to compile.
///
/// Formatted to match OpenJDK's `PatternSyntaxException`:
/// ```text
/// <message> near index <N>
/// <pattern>
/// <padding>^
/// ```
/// The trailing pattern + caret are omitted only when constructed without
/// context (e.g. by callers outside the parser).
#[derive(Debug, Clone)]
pub struct PatternSyntaxError {
    pub message: String,
    /// The full source pattern at the time of the error, or empty when none.
    pub pattern: String,
    /// Position in `pattern.chars()` where the error was detected.
    pub index: usize,
}

impl PatternSyntaxError {
    pub fn new(message: String) -> Self {
        Self { message, pattern: String::new(), index: 0 }
    }

    pub fn with_context(message: String, pattern: String, index: usize) -> Self {
        Self { message, pattern, index }
    }
}

impl fmt::Display for PatternSyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.pattern.is_empty() {
            write!(f, "{}", self.message)
        } else {
            // Width of the leading-padding caret depends on display width of
            // the chars before `index`. ASCII = 1 col each; non-ASCII chars
            // in source patterns are rare and treated as 1 col here (matches
            // OpenJDK, which also uses column count rather than visual width).
            write!(f, "{} near index {}\n{}\n", self.message, self.index, self.pattern)?;
            for _ in 0..self.index { f.write_str(" ")?; }
            f.write_str("^")
        }
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
    pub literal: bool,          // LITERAL (no inline flag)
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
        prev_pos: usize,
    },
    ReluctantCont {        // engine-internal: continue reluctant quantifier loop
        atom: Box<Node>,
        min: u32,
        max: u32,
        count: u32,
        rest: Vec<Node>,
        prev_pos: usize,
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

/// Information about a single regex match, including captured groups.
///
/// Positions (`start`, `end`, `group_positions`) are char indices, not byte indices.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchInfo {
    /// The full matched text.
    pub matched_text: String,
    /// Start position (char index) of the match.
    pub start: usize,
    /// End position (char index, exclusive) of the match.
    pub end: usize,
    /// Captured group texts, indexed from 0 (group 1 is `groups[0]`).
    pub groups: Vec<Option<String>>,
    /// Captured group positions as `(start, end)` char indices.
    pub group_positions: Vec<Option<(usize, usize)>>,
    /// Named group captures, keyed by group name.
    pub named_groups: BTreeMap<String, String>,
}
