use unicode_general_category::GeneralCategory as UGC;
use unicode_script::{Script, UnicodeScript};

use crate::types::PredefinedClass;

pub fn get_ugc(ch: char) -> UGC {
    unicode_general_category::get_general_category(ch)
}

pub fn chars_eq_ci(a: char, b: char, unicode_case: bool) -> bool {
    if a == b { return true; }
    if unicode_case {
        let a_lower = a.to_lowercase().next().unwrap_or(a);
        let b_lower = b.to_lowercase().next().unwrap_or(b);
        if a_lower == b_lower { return true; }
        let a_upper = a.to_uppercase().next().unwrap_or(a);
        let b_upper = b.to_uppercase().next().unwrap_or(b);
        a_upper == b_upper
    } else {
        a.to_ascii_lowercase() == b.to_ascii_lowercase()
            || a.to_ascii_uppercase() == b.to_ascii_uppercase()
    }
}

pub fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

pub fn is_word_char(c: char, unicode: bool) -> bool {
    if unicode {
        // Java's Unicode \w: [\p{Alpha}\p{gc=Mn}\p{gc=Me}\p{gc=Mc}\p{Digit}\p{gc=Pc}]
        c.is_alphanumeric() || c == '_' || is_combining_mark(c)
    } else {
        c.is_ascii_alphanumeric() || c == '_'
    }
}

pub fn is_linebreak(c: char) -> bool {
    matches!(c, '\n' | '\x0B' | '\x0C' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

/// Approximation of Unicode Bidi_Mirrored property for Java's Character.isMirrored().
/// All Ps (open) and Pe (close) punctuation are mirrored, plus Pi/Pf quotes,
/// and a subset of Sm (math) symbols that have mirrored counterparts.
fn is_bidi_mirrored(ch: char) -> bool {
    let cat = get_ugc(ch);
    matches!(cat, UGC::OpenPunctuation | UGC::ClosePunctuation |
        UGC::InitialPunctuation | UGC::FinalPunctuation)
        || (cat == UGC::MathSymbol && is_mirrored_math(ch))
        || matches!(ch, '<' | '>')
}

fn is_mirrored_math(ch: char) -> bool {
    // Math symbols with Bidi_Mirrored=Yes (the most common ones)
    matches!(ch,
        '\u{2208}'..='\u{220D}' | '\u{2215}' | '\u{221F}'..='\u{2222}' |
        '\u{2224}' | '\u{2226}' | '\u{222B}'..='\u{2233}' |
        '\u{2239}' | '\u{223B}'..='\u{224C}' | '\u{2252}'..='\u{2255}' |
        '\u{225F}'..='\u{2260}' | '\u{2261}'..='\u{2262}' |
        '\u{2264}'..='\u{226B}' | '\u{226E}'..='\u{228C}' |
        '\u{228F}'..='\u{2298}' | '\u{22A2}'..='\u{22B8}' |
        '\u{22BE}'..='\u{22BF}' | '\u{22C9}'..='\u{22CD}' |
        '\u{22D0}'..='\u{22D1}' | '\u{22D6}'..='\u{22ED}' |
        '\u{22F0}'..='\u{22FF}' | '\u{2308}'..='\u{230B}' |
        '\u{2320}'..='\u{2321}' | '\u{27C3}'..='\u{27C6}' |
        '\u{27D5}'..='\u{27D6}' | '\u{27DC}'..='\u{27DE}' |
        '\u{27E2}'..='\u{27E5}')
}

fn is_unicode_punct(ch: char) -> bool {
    matches!(get_ugc(ch),
        UGC::ConnectorPunctuation | UGC::DashPunctuation | UGC::OpenPunctuation |
        UGC::ClosePunctuation | UGC::InitialPunctuation | UGC::FinalPunctuation |
        UGC::OtherPunctuation)
}

fn is_unicode_graph(ch: char) -> bool {
    let cat = get_ugc(ch);
    !matches!(cat, UGC::SpaceSeparator | UGC::LineSeparator | UGC::ParagraphSeparator |
        UGC::Control | UGC::Surrogate | UGC::Unassigned)
        && !ch.is_whitespace()
}

fn is_unicode_print(ch: char) -> bool {
    let cat = get_ugc(ch);
    !matches!(cat, UGC::Control | UGC::Surrogate | UGC::Unassigned)
        || matches!(cat, UGC::SpaceSeparator | UGC::LineSeparator | UGC::ParagraphSeparator)
}

pub fn is_posix_class(name: &str) -> bool {
    matches!(name.to_lowercase().as_str(),
        "alpha" | "alnum" | "ascii" | "blank" | "cntrl" | "digit" |
        "graph" | "lower" | "print" | "punct" | "space" | "upper" |
        "xdigit" | "white_space")
}

pub fn is_combining_mark(c: char) -> bool {
    let cat = get_ugc(c);
    matches!(cat, UGC::NonspacingMark | UGC::SpacingMark | UGC::EnclosingMark)
}

pub fn is_regional_indicator(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

pub fn match_predefined_class(pc: PredefinedClass, ch: char, unicode: bool) -> bool {
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
            if unicode { ch.is_whitespace() }
            else { matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0C' | '\x0B') }
        }
        PredefinedClass::NonWhitespace => {
            if unicode { !ch.is_whitespace() }
            else { !matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0C' | '\x0B') }
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

pub fn is_valid_unicode_property(name: &str) -> bool {
    if name.starts_with("java") {
        return matches!(name,
            "javaLowerCase" | "javaUpperCase" | "javaTitleCase" |
            "javaDigit" | "javaLetter" | "javaLetterOrDigit" |
            "javaAlphabetic" | "javaWhitespace" | "javaSpaceChar" |
            "javaMirrored" | "javaDefined" | "javaIdentifierIgnorable" |
            "javaISOControl" | "javaUnicodeIdentifierStart" | "javaUnicodeIdentifierPart"
        );
    }
    // Script names require "Is" prefix in Java (e.g. \p{IsLatin})
    if let Some(script) = name.strip_prefix("Is").or_else(|| name.strip_prefix("is")) {
        if resolve_script_name(script).is_some() {
            return true;
        }
    }
    // Block names use "In" prefix (e.g. \p{InBasicLatin})
    if let Some(block) = name.strip_prefix("In").or_else(|| name.strip_prefix("in")) {
        if resolve_block_name(block).is_some() {
            return true;
        }
    }
    let name = name.strip_prefix("Is").unwrap_or(name);
    let name_lower = name.to_lowercase();
    matches!(name_lower.as_str(),
        "l" | "letter" | "lu" | "uppercase_letter" | "upper" | "ll" | "lowercase_letter" | "lower" |
        "lc" | "cased_letter" |
        "lt" | "titlecase_letter" | "lm" | "modifier_letter" | "lo" | "other_letter" |
        "m" | "mark" | "mn" | "nonspacing_mark" | "mc" | "spacing_mark" | "me" | "enclosing_mark" |
        "n" | "number" | "nd" | "decimal_digit_number" | "digit" | "nl" | "letter_number" | "no" | "other_number" |
        "p" | "punctuation" | "punct" |
        "pc" | "connector_punctuation" | "pd" | "dash_punctuation" |
        "ps" | "open_punctuation" | "pe" | "close_punctuation" |
        "pi" | "initial_punctuation" | "pf" | "final_punctuation" | "po" | "other_punctuation" |
        "s" | "symbol" | "sm" | "math_symbol" | "sc" | "currency_symbol" | "sk" | "modifier_symbol" | "so" | "other_symbol" |
        "z" | "separator" | "zs" | "space_separator" | "zl" | "line_separator" | "zp" | "paragraph_separator" |
        "c" | "control" | "other" | "cc" | "cntrl" | "cf" | "format" | "co" | "private_use" | "cn" | "unassigned" |
        "alpha" | "alnum" | "ascii" | "blank" | "graph" | "print" | "space" | "white_space" | "xdigit" |
        "l1" | "latin1"
    )
}

/// Convenience wrapper: matches without UNICODE_CHARACTER_CLASS flag.
#[allow(dead_code)]
pub fn match_unicode_property(name: &str, ch: char) -> bool {
    match_unicode_property_ext(name, ch, false)
}

/// Match a Unicode property, with `unicode_class` controlling whether
/// POSIX classes use ASCII (false) or Unicode (true) definitions.
pub fn match_unicode_property_ext(name: &str, ch: char, unicode_class: bool) -> bool {
    // Java-specific properties (exact case)
    match name {
        "javaLowerCase" => return ch.is_lowercase(),
        "javaUpperCase" => return ch.is_uppercase(),
        "javaTitleCase" => return matches!(get_ugc(ch), UGC::TitlecaseLetter),
        "javaDigit" => return ch.is_ascii_digit() || ch.is_numeric(),
        "javaLetter" => return ch.is_alphabetic(),
        "javaLetterOrDigit" => return ch.is_alphanumeric(),
        "javaAlphabetic" => return ch.is_alphabetic(),
        "javaWhitespace" => {
            // Java's Character.isWhitespace: Zs/Zl/Zp except non-breaking spaces, plus control whitespace
            return match ch {
                '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' |
                '\u{001C}' | '\u{001D}' | '\u{001E}' | '\u{001F}' => true,
                _ => {
                    let cat = get_ugc(ch);
                    (matches!(cat, UGC::SpaceSeparator | UGC::LineSeparator | UGC::ParagraphSeparator))
                        && ch != '\u{00A0}' && ch != '\u{2007}' && ch != '\u{202F}'
                }
            };
        }
        "javaSpaceChar" => return matches!(get_ugc(ch), UGC::SpaceSeparator | UGC::LineSeparator | UGC::ParagraphSeparator),
        "javaISOControl" => return ch.is_control(),
        "javaDefined" => return ch != '\u{FFFF}',
        "javaMirrored" => return is_bidi_mirrored(ch),
        "javaIdentifierIgnorable" => return ch.is_control() && !ch.is_whitespace(),
        "javaUnicodeIdentifierStart" => return ch.is_alphabetic(),
        "javaUnicodeIdentifierPart" => return ch.is_alphanumeric() || ch == '_',
        _ => {}
    }

    // Script names require "Is" prefix in Java (e.g. \p{IsLatin})
    if let Some(script) = name.strip_prefix("Is").or_else(|| name.strip_prefix("is")) {
        if let Some(expected) = resolve_script_name(script) {
            return ch.script() == expected;
        }
    }

    // Block names use "In" prefix (e.g. \p{InBasicLatin})
    if let Some(block) = name.strip_prefix("In").or_else(|| name.strip_prefix("in")) {
        if let Some(expected_block) = resolve_block_name(block) {
            return match unicode_blocks::find_unicode_block(ch) {
                Some(b) => b == expected_block,
                None => false,
            };
        }
    }

    let name = name.strip_prefix("Is").unwrap_or(name);
    let name_lower = name.to_lowercase();

    // POSIX classes — ASCII-only by default, Unicode when UNICODE_CHARACTER_CLASS flag is set.
    // The `u` (bool) parameter below is `unicode_class`.
    let u = unicode_class;
    match name_lower.as_str() {
        "upper" => return if u { ch.is_uppercase() } else { ch.is_ascii_uppercase() },
        "lower" => return if u { ch.is_lowercase() } else { ch.is_ascii_lowercase() },
        "alpha" => return if u { ch.is_alphabetic() } else { ch.is_ascii_alphabetic() },
        "digit" => return if u { ch.is_numeric() } else { ch.is_ascii_digit() },
        "alnum" => return if u { ch.is_alphanumeric() } else { ch.is_ascii_alphanumeric() },
        "ascii" => return ch.is_ascii(),
        "blank" => return if u { ch == '\t' || matches!(get_ugc(ch), UGC::SpaceSeparator) }
                          else { ch == ' ' || ch == '\t' },
        "punct" => return if u { is_unicode_punct(ch) }
                          else { matches!(ch, '!'..='/' | ':'..='@' | '['..='`' | '{'..='~') },
        "graph" => return if u { is_unicode_graph(ch) } else { ch.is_ascii_graphic() },
        "print" => return if u { is_unicode_print(ch) } else { ch.is_ascii_graphic() || ch == ' ' },
        "cntrl" => return if u { matches!(get_ugc(ch), UGC::Control) } else { ch.is_ascii_control() },
        "space" | "white_space" => return if u { ch.is_whitespace() } else { ch.is_ascii_whitespace() },
        "xdigit" => return ch.is_ascii_hexdigit(),
        "l1" | "latin1" => return (ch as u32) <= 0xFF,
        _ => {}
    }

    // Unicode General Categories — compute category once
    let cat = get_ugc(ch);
    match_ugc_category(&name_lower, cat)
}

fn match_ugc_category(name: &str, cat: UGC) -> bool {
    match name {
        "l" | "letter" => matches!(cat,
            UGC::UppercaseLetter | UGC::LowercaseLetter | UGC::TitlecaseLetter |
            UGC::ModifierLetter | UGC::OtherLetter),
        "lu" | "uppercase_letter" => matches!(cat, UGC::UppercaseLetter),
        "ll" | "lowercase_letter" => matches!(cat, UGC::LowercaseLetter),
        "lc" | "cased_letter" => matches!(cat, UGC::UppercaseLetter | UGC::LowercaseLetter | UGC::TitlecaseLetter),
        "lt" | "titlecase_letter" => matches!(cat, UGC::TitlecaseLetter),
        "lm" | "modifier_letter" => matches!(cat, UGC::ModifierLetter),
        "lo" | "other_letter" => matches!(cat, UGC::OtherLetter),
        "m" | "mark" => matches!(cat, UGC::NonspacingMark | UGC::SpacingMark | UGC::EnclosingMark),
        "mn" | "nonspacing_mark" => matches!(cat, UGC::NonspacingMark),
        "mc" | "spacing_mark" => matches!(cat, UGC::SpacingMark),
        "me" | "enclosing_mark" => matches!(cat, UGC::EnclosingMark),
        "n" | "number" => matches!(cat, UGC::DecimalNumber | UGC::LetterNumber | UGC::OtherNumber),
        "nd" | "decimal_digit_number" => matches!(cat, UGC::DecimalNumber),
        "nl" | "letter_number" => matches!(cat, UGC::LetterNumber),
        "no" | "other_number" => matches!(cat, UGC::OtherNumber),
        "p" | "punctuation" => matches!(cat,
            UGC::ConnectorPunctuation | UGC::DashPunctuation | UGC::OpenPunctuation |
            UGC::ClosePunctuation | UGC::InitialPunctuation | UGC::FinalPunctuation | UGC::OtherPunctuation),
        "pc" | "connector_punctuation" => matches!(cat, UGC::ConnectorPunctuation),
        "pd" | "dash_punctuation" => matches!(cat, UGC::DashPunctuation),
        "ps" | "open_punctuation" => matches!(cat, UGC::OpenPunctuation),
        "pe" | "close_punctuation" => matches!(cat, UGC::ClosePunctuation),
        "pi" | "initial_punctuation" => matches!(cat, UGC::InitialPunctuation),
        "pf" | "final_punctuation" => matches!(cat, UGC::FinalPunctuation),
        "po" | "other_punctuation" => matches!(cat, UGC::OtherPunctuation),
        "s" | "symbol" => matches!(cat, UGC::MathSymbol | UGC::CurrencySymbol | UGC::ModifierSymbol | UGC::OtherSymbol),
        "sm" | "math_symbol" => matches!(cat, UGC::MathSymbol),
        "sc" | "currency_symbol" => matches!(cat, UGC::CurrencySymbol),
        "sk" | "modifier_symbol" => matches!(cat, UGC::ModifierSymbol),
        "so" | "other_symbol" => matches!(cat, UGC::OtherSymbol),
        "z" | "separator" => matches!(cat, UGC::SpaceSeparator | UGC::LineSeparator | UGC::ParagraphSeparator),
        "zs" | "space_separator" => matches!(cat, UGC::SpaceSeparator),
        "zl" | "line_separator" => matches!(cat, UGC::LineSeparator),
        "zp" | "paragraph_separator" => matches!(cat, UGC::ParagraphSeparator),
        "c" | "control" | "other" => matches!(cat, UGC::Control | UGC::Format | UGC::PrivateUse | UGC::Unassigned),
        "cc" => matches!(cat, UGC::Control),
        "cf" | "format" => matches!(cat, UGC::Format),
        "co" | "private_use" => matches!(cat, UGC::PrivateUse),
        "cn" | "unassigned" => matches!(cat, UGC::Unassigned),
        _ => false,
    }
}

/// Resolve a script name (after stripping "Is" prefix) to a unicode_script::Script.
fn resolve_script_name(name: &str) -> Option<Script> {
    // Try full name first (e.g. "Latin", "Greek")
    if let Some(s) = Script::from_full_name(name) {
        return Some(s);
    }
    // Try short name (e.g. "Latn", "Grek")
    if let Some(s) = Script::from_short_name(name) {
        return Some(s);
    }
    // Try case-insensitive: capitalize first letter
    let mut capitalized = name.to_lowercase();
    if let Some(first) = capitalized.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    Script::from_full_name(&capitalized)
}

/// Resolve a block name (after stripping "In" prefix) to a UnicodeBlock.
/// Java block names have spaces/underscores removed and are case-insensitive,
/// e.g. "BasicLatin", "BASIC_LATIN", "basiclatin" all map to "Basic Latin".
fn resolve_block_name(name: &str) -> Option<unicode_blocks::UnicodeBlock> {
    let normalized = normalize_block_name_for_lookup(name);
    // Iterate all known blocks and compare normalized names
    // unicode_blocks doesn't provide a from_name lookup, so we check the char range
    // by finding a block whose normalized name matches
    BLOCK_LIST.iter().find(|(norm, _)| *norm == normalized).map(|(_, block)| *block)
}

/// Normalize block name for comparison: lowercase, remove spaces, underscores, hyphens.
fn normalize_block_name_for_lookup(name: &str) -> String {
    name.chars()
        .filter(|c| *c != ' ' && *c != '_' && *c != '-')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Mapping from normalized block names to UnicodeBlock constants.
/// Normalized = lowercase, no spaces/underscores/hyphens.
const BLOCK_LIST: &[(&str, unicode_blocks::UnicodeBlock)] = &[
    ("basiclatin", unicode_blocks::BASIC_LATIN),
    ("latin1supplement", unicode_blocks::LATIN_1_SUPPLEMENT),
    ("latinextendeda", unicode_blocks::LATIN_EXTENDED_A),
    ("latinextendedb", unicode_blocks::LATIN_EXTENDED_B),
    ("ipaextensions", unicode_blocks::IPA_EXTENSIONS),
    ("spacingmodifierletters", unicode_blocks::SPACING_MODIFIER_LETTERS),
    ("combiningdiacriticalmarks", unicode_blocks::COMBINING_DIACRITICAL_MARKS),
    ("greekandcoptic", unicode_blocks::GREEK_AND_COPTIC),
    ("greek", unicode_blocks::GREEK_AND_COPTIC),
    ("cyrillic", unicode_blocks::CYRILLIC),
    ("cyrillicsupplement", unicode_blocks::CYRILLIC_SUPPLEMENT),
    ("armenian", unicode_blocks::ARMENIAN),
    ("hebrew", unicode_blocks::HEBREW),
    ("arabic", unicode_blocks::ARABIC),
    ("syriac", unicode_blocks::SYRIAC),
    ("devanagari", unicode_blocks::DEVANAGARI),
    ("bengali", unicode_blocks::BENGALI),
    ("tamil", unicode_blocks::TAMIL),
    ("telugu", unicode_blocks::TELUGU),
    ("thai", unicode_blocks::THAI),
    ("georgian", unicode_blocks::GEORGIAN),
    ("hanguljamoextendeda", unicode_blocks::HANGUL_JAMO_EXTENDED_A),
    ("hanguljamoextendedb", unicode_blocks::HANGUL_JAMO_EXTENDED_B),
    ("generalpunctuation", unicode_blocks::GENERAL_PUNCTUATION),
    ("superscriptsandsubscripts", unicode_blocks::SUPERSCRIPTS_AND_SUBSCRIPTS),
    ("currencysymbols", unicode_blocks::CURRENCY_SYMBOLS),
    ("letterlikesymbols", unicode_blocks::LETTERLIKE_SYMBOLS),
    ("numberforms", unicode_blocks::NUMBER_FORMS),
    ("arrows", unicode_blocks::ARROWS),
    ("mathematicaloperators", unicode_blocks::MATHEMATICAL_OPERATORS),
    ("boxdrawing", unicode_blocks::BOX_DRAWING),
    ("geometricshapes", unicode_blocks::GEOMETRIC_SHAPES),
    ("miscellaneoussymbols", unicode_blocks::MISCELLANEOUS_SYMBOLS),
    ("cjkunifiedideographs", unicode_blocks::CJK_UNIFIED_IDEOGRAPHS),
    ("hiragana", unicode_blocks::HIRAGANA),
    ("katakana", unicode_blocks::KATAKANA),
    ("hangulsyllables", unicode_blocks::HANGUL_SYLLABLES),
    ("privateusearea", unicode_blocks::PRIVATE_USE_AREA),
    ("alphabeticpresentationforms", unicode_blocks::ALPHABETIC_PRESENTATION_FORMS),
    ("arabicpresentationformsa", unicode_blocks::ARABIC_PRESENTATION_FORMS_A),
    ("arabicpresentationformsb", unicode_blocks::ARABIC_PRESENTATION_FORMS_B),
    ("latinextendedadditional", unicode_blocks::LATIN_EXTENDED_ADDITIONAL),
    ("halfwidthandfullwidthforms", unicode_blocks::HALFWIDTH_AND_FULLWIDTH_FORMS),
    ("specials", unicode_blocks::SPECIALS),
    ("deseret", unicode_blocks::DESERET),
    ("olditalic", unicode_blocks::OLD_ITALIC),
    ("gothic", unicode_blocks::GOTHIC),
];
