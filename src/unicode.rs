use unicode_general_category::GeneralCategory as UGC;

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
        c.is_alphanumeric() || c == '_'
    } else {
        c.is_ascii_alphanumeric() || c == '_'
    }
}

pub fn is_linebreak(c: char) -> bool {
    matches!(c, '\n' | '\x0B' | '\x0C' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
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
    // Script names require "Is" prefix in Java
    let name_lower_full = name.to_lowercase();
    if is_script_name(&name_lower_full) {
        return true;
    }
    // Block names use "In" prefix
    if is_block_name(&name_lower_full) {
        return true;
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

pub fn match_unicode_property(name: &str, ch: char) -> bool {
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
        "javaMirrored" => return false, // simplified
        "javaIdentifierIgnorable" => return ch.is_control() && !ch.is_whitespace(),
        "javaUnicodeIdentifierStart" => return ch.is_alphabetic(),
        "javaUnicodeIdentifierPart" => return ch.is_alphanumeric() || ch == '_',
        _ => {}
    }

    // Script names require "Is" prefix in Java (e.g. \p{IsLatin} not \p{Latin})
    if let Some(script) = name.strip_prefix("Is").or_else(|| name.strip_prefix("is")) {
        let script_lower = script.to_lowercase();
        if let Some(result) = match_script(&script_lower, ch) {
            return result;
        }
    }

    // Block names use "In" prefix (e.g. \p{InBasicLatin})
    if let Some(block) = name.strip_prefix("In").or_else(|| name.strip_prefix("in")) {
        let block_norm = normalize_block_name(block);
        if let Some(result) = match_block(&block_norm, ch) {
            return result;
        }
    }

    let name = name.strip_prefix("Is").unwrap_or(name);
    let name_lower = name.to_lowercase();

    // POSIX classes (ASCII-only)
    match name_lower.as_str() {
        "upper" => return ch.is_ascii_uppercase(),
        "lower" => return ch.is_ascii_lowercase(),
        "alpha" => return ch.is_ascii_alphabetic(),
        "digit" => return ch.is_ascii_digit(),
        "alnum" => return ch.is_ascii_alphanumeric(),
        "ascii" => return ch.is_ascii(),
        "blank" => return ch == ' ' || ch == '\t',
        "punct" => return matches!(ch, '!'..='/' | ':'..='@' | '['..='`' | '{'..='~'),
        "graph" => return ch.is_ascii_graphic(),
        "print" => return ch.is_ascii_graphic() || ch == ' ',
        "cntrl" => return ch.is_ascii_control(),
        "space" | "white_space" => return ch.is_ascii_whitespace(),
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

/// Check if a lowercased name (after stripping "Is" prefix) is a known script.
fn is_script_name(name_lower_full: &str) -> bool {
    // Script names must start with "is" in Java
    if let Some(script) = name_lower_full.strip_prefix("is") {
        matches!(script,
            "greek" | "latin" | "cyrillic" | "han" | "arabic" |
            "armenian" | "hebrew" | "thai" | "hiragana" | "katakana" | "devanagari"
        )
    } else {
        false
    }
}

/// Match a script name (already stripped of "Is" prefix, lowercased).
/// Returns Some(bool) if it's a known script, None otherwise.
fn match_script(script_lower: &str, ch: char) -> Option<bool> {
    match script_lower {
        "greek" => Some(is_script_greek(ch)),
        "latin" => Some(is_script_latin(ch)),
        "cyrillic" => Some(is_script_cyrillic(ch)),
        "han" => Some(is_script_han(ch)),
        "arabic" => Some(is_script_arabic(ch)),
        "armenian" => Some(('\u{0530}'..='\u{058F}').contains(&ch) || ('\u{FB00}'..='\u{FB17}').contains(&ch)),
        "hebrew" => Some(('\u{0590}'..='\u{05FF}').contains(&ch) || ('\u{FB1D}'..='\u{FB4F}').contains(&ch)),
        "thai" => Some(('\u{0E00}'..='\u{0E7F}').contains(&ch)),
        "hiragana" => Some(('\u{3040}'..='\u{309F}').contains(&ch)),
        "katakana" => Some(('\u{30A0}'..='\u{30FF}').contains(&ch)),
        "devanagari" => Some(('\u{0900}'..='\u{097F}').contains(&ch)),
        _ => None,
    }
}

/// Normalize a Unicode block name: lowercase, remove spaces and underscores, keep hyphens.
fn normalize_block_name(name: &str) -> String {
    name.chars()
        .filter(|c| *c != ' ' && *c != '_')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Check if name starts with "in"/"In" and names a known Unicode block.
fn is_block_name(name: &str) -> bool {
    let norm = normalize_block_name(name);
    if let Some(block) = norm.strip_prefix("in") {
        match_block(block, 'A').is_some()
    } else {
        false
    }
}

/// Match a Unicode block name (already stripped of "In" prefix, normalized).
fn match_block(block_norm: &str, ch: char) -> Option<bool> {
    match block_norm {
        "basiclatin" => Some(('\u{0000}'..='\u{007F}').contains(&ch)),
        "latin-1supplement" => Some(('\u{0080}'..='\u{00FF}').contains(&ch)),
        "latinextended-a" => Some(('\u{0100}'..='\u{017F}').contains(&ch)),
        "latinextended-b" => Some(('\u{0180}'..='\u{024F}').contains(&ch)),
        "ipaextensions" => Some(('\u{0250}'..='\u{02AF}').contains(&ch)),
        "spacingmodifierletters" => Some(('\u{02B0}'..='\u{02FF}').contains(&ch)),
        "combiningdiacriticalmarks" => Some(('\u{0300}'..='\u{036F}').contains(&ch)),
        "greekandcoptic" | "greek" => Some(('\u{0370}'..='\u{03FF}').contains(&ch)),
        "cyrillic" => Some(('\u{0400}'..='\u{04FF}').contains(&ch)),
        "arabic" => Some(('\u{0600}'..='\u{06FF}').contains(&ch)),
        "devanagari" => Some(('\u{0900}'..='\u{097F}').contains(&ch)),
        "thai" => Some(('\u{0E00}'..='\u{0E7F}').contains(&ch)),
        "generalpunctuation" => Some(('\u{2000}'..='\u{206F}').contains(&ch)),
        "superscriptsandsubscripts" => Some(('\u{2070}'..='\u{209F}').contains(&ch)),
        "currencysymbols" => Some(('\u{20A0}'..='\u{20CF}').contains(&ch)),
        "letterlikesymbols" => Some(('\u{2100}'..='\u{214F}').contains(&ch)),
        "numberforms" => Some(('\u{2150}'..='\u{218F}').contains(&ch)),
        "arrows" => Some(('\u{2190}'..='\u{21FF}').contains(&ch)),
        "mathematicaloperators" => Some(('\u{2200}'..='\u{22FF}').contains(&ch)),
        "boxdrawing" => Some(('\u{2500}'..='\u{257F}').contains(&ch)),
        "geometricshapes" => Some(('\u{25A0}'..='\u{25FF}').contains(&ch)),
        "miscellaneoussymbols" => Some(('\u{2600}'..='\u{26FF}').contains(&ch)),
        "cjkunifiedideographs" => Some(('\u{4E00}'..='\u{9FFF}').contains(&ch)),
        "hiragana" => Some(('\u{3040}'..='\u{309F}').contains(&ch)),
        "katakana" => Some(('\u{30A0}'..='\u{30FF}').contains(&ch)),
        "hangulsyllables" => Some(('\u{AC00}'..='\u{D7AF}').contains(&ch)),
        "privateusearea" => Some(('\u{E000}'..='\u{F8FF}').contains(&ch)),
        "alphabeticpresentationforms" => Some(('\u{FB00}'..='\u{FB4F}').contains(&ch)),
        "arabicpresentationforms-a" => Some(('\u{FB50}'..='\u{FDFF}').contains(&ch)),
        "arabicpresentationforms-b" => Some(('\u{FE70}'..='\u{FEFF}').contains(&ch)),
        "latinextendedadditional" => Some(('\u{1E00}'..='\u{1EFF}').contains(&ch)),
        "armenian" => Some(('\u{0530}'..='\u{058F}').contains(&ch)),
        "hebrew" => Some(('\u{0590}'..='\u{05FF}').contains(&ch)),
        "bengali" => Some(('\u{0980}'..='\u{09FF}').contains(&ch)),
        "tamil" => Some(('\u{0B80}'..='\u{0BFF}').contains(&ch)),
        "telugu" => Some(('\u{0C00}'..='\u{0C7F}').contains(&ch)),
        "georgian" => Some(('\u{10A0}'..='\u{10FF}').contains(&ch)),
        "hanguljamoextended-a" => Some(('\u{A960}'..='\u{A97F}').contains(&ch)),
        "hanguljamoextended-b" => Some(('\u{D7B0}'..='\u{D7FF}').contains(&ch)),
        "highsurrogates" => { let c = ch as u32; Some((0xD800..=0xDB7F).contains(&c)) }
        "highprivateusesurrogates" => { let c = ch as u32; Some((0xDB80..=0xDBFF).contains(&c)) }
        "lowsurrogates" => { let c = ch as u32; Some((0xDC00..=0xDFFF).contains(&c)) }
        "halfwidthandfullwidthforms" => Some(('\u{FF00}'..='\u{FFEF}').contains(&ch)),
        "specials" => Some(('\u{FFF0}'..='\u{FFFF}').contains(&ch)),
        "deseret" => Some(('\u{10400}'..='\u{1044F}').contains(&ch)),
        "olditalic" => Some(('\u{10300}'..='\u{1032F}').contains(&ch)),
        "gothic" => Some(('\u{10330}'..='\u{1034F}').contains(&ch)),
        _ => None,
    }
}
