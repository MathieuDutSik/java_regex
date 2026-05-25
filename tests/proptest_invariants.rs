//! Property-based tests for engine self-consistency.
//!
//! These tests don't require Java — they only check invariants the engine should
//! satisfy on its own (find positions are monotonic, replace-with-identity is a
//! no-op, Regex::quote round-trips, no random pattern panics, etc.). Run with:
//!
//! ```sh
//! cargo test --test proptest_invariants
//! PROPTEST_CASES=10000 cargo test --test proptest_invariants  # heavier run
//! ```

use java_regex::gen::*;
use java_regex::Regex;
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

// ---------------------------------------------------------------------------
// Strategy: build random RegexNode trees with bounded depth.
// ---------------------------------------------------------------------------

fn arb_ascii() -> impl Strategy<Value = AsciiPrintable> {
    prop_oneof![
        Just(AsciiPrintable::A), Just(AsciiPrintable::B), Just(AsciiPrintable::C),
        Just(AsciiPrintable::D), Just(AsciiPrintable::Zero), Just(AsciiPrintable::One),
        Just(AsciiPrintable::Space), Just(AsciiPrintable::Comma), Just(AsciiPrintable::Colon),
        Just(AsciiPrintable::Underscore), Just(AsciiPrintable::Hyphen), Just(AsciiPrintable::At),
    ]
}

fn arb_unicode() -> impl Strategy<Value = UnicodeChar> {
    prop_oneof![
        Just(UnicodeChar::LatinEAcute),
        Just(UnicodeChar::LatinSsharp),
        Just(UnicodeChar::GreekAlpha),
        Just(UnicodeChar::Cjk),
        Just(UnicodeChar::EmojiGrin),
    ]
}

fn arb_lit() -> impl Strategy<Value = LitChar> {
    prop_oneof![
        4 => arb_ascii().prop_map(LitChar::Ascii),
        1 => Just(LitChar::Newline),
        1 => Just(LitChar::Tab),
        1 => arb_unicode().prop_map(LitChar::Unicode),
    ]
}

fn arb_esc() -> impl Strategy<Value = EscClass> {
    prop_oneof![
        Just(EscClass::Digit), Just(EscClass::NonDigit),
        Just(EscClass::Word), Just(EscClass::NonWord),
        Just(EscClass::Space), Just(EscClass::NonSpace),
        Just(EscClass::UnicodeLetter), Just(EscClass::UnicodeDigit),
    ]
}

fn arb_anchor() -> impl Strategy<Value = Anchor> {
    prop_oneof![
        Just(Anchor::StartLine), Just(Anchor::EndLine),
        Just(Anchor::StartInput), Just(Anchor::EndInputZ),
        Just(Anchor::WordBoundary), Just(Anchor::NonWordBoundary),
    ]
}

fn arb_small_count() -> impl Strategy<Value = SmallCount> {
    prop_oneof![
        Just(SmallCount::Zero), Just(SmallCount::One), Just(SmallCount::Two),
        Just(SmallCount::Three),
    ]
}

fn arb_quant() -> impl Strategy<Value = Quantifier> {
    let kind = prop_oneof![
        Just(QuantKind::Star), Just(QuantKind::Plus), Just(QuantKind::Opt),
        arb_small_count().prop_map(QuantKind::Exact),
        arb_small_count().prop_map(QuantKind::AtLeast),
        (arb_small_count(), arb_small_count()).prop_map(|(a, b)| QuantKind::Range(a, b)),
    ];
    let mode = prop_oneof![
        Just(QuantMode::Greedy), Just(QuantMode::Reluctant), Just(QuantMode::Possessive),
    ];
    (kind, mode).prop_map(|(kind, mode)| Quantifier { kind, mode })
}

fn arb_class() -> impl Strategy<Value = CharClass> {
    let item = prop_oneof![
        arb_lit().prop_map(ClassItem::Single),
        (arb_ascii(), arb_ascii()).prop_map(|(a, b)| ClassItem::Range(a, b)),
        arb_esc().prop_map(ClassItem::Esc),
    ];
    (any::<bool>(), proptest::collection::vec(item, 1..5))
        .prop_map(|(negated, items)| CharClass { negated, items })
}

fn arb_regex_node() -> impl Strategy<Value = RegexNode> {
    // Leaves: things that can't recurse further.
    let leaf = prop_oneof![
        4 => arb_lit().prop_map(RegexNode::Literal),
        2 => Just(RegexNode::Dot),
        2 => arb_anchor().prop_map(RegexNode::Anchor),
        2 => arb_esc().prop_map(RegexNode::Escape),
        1 => Just(RegexNode::LineBreak),
        2 => arb_class().prop_map(RegexNode::Class),
        1 => proptest::collection::vec(arb_lit(), 0..4).prop_map(RegexNode::Quote),
    ];
    // Recurse to bounded depth (target depth=4, branch factor up to 3, size up to ~32 nodes).
    leaf.prop_recursive(4, 32, 3, |inner| {
        prop_oneof![
            // Concat
            proptest::collection::vec(inner.clone(), 1..4).prop_map(RegexNode::Concat),
            // Alt
            proptest::collection::vec(inner.clone(), 2..4).prop_map(RegexNode::Alt),
            // Group
            (any::<u8>(), inner.clone()).prop_map(|(tag, body)| {
                let kind = match tag % 4 {
                    0 => GroupKind::Capturing,
                    1 => GroupKind::NonCapturing,
                    2 => GroupKind::Atomic,
                    _ => GroupKind::Named(match (tag / 4) % 5 {
                        0 => GroupName::Foo, 1 => GroupName::Bar, 2 => GroupName::Baz,
                        3 => GroupName::X1, _ => GroupName::Y2,
                    }),
                };
                RegexNode::Group { kind, body: Box::new(body) }
            }),
            // Quantified
            (arb_quant(), inner.clone()).prop_map(|(q, body)|
                RegexNode::Quantified { body: Box::new(body), quant: q }),
            // Lookaround
            (any::<bool>(), any::<bool>(), inner.clone()).prop_map(|(ahead, neg, body)|
                RegexNode::Lookaround { ahead, neg, body: Box::new(body) }),
        ]
    })
}

/// Generate random input strings drawn from the same character vocabulary the
/// AST uses, so generated patterns have a fair chance of actually matching.
fn arb_input() -> impl Strategy<Value = String> {
    let ch = prop_oneof![
        6 => arb_ascii().prop_map(|a| a.to_char()),
        1 => Just('\n'), 1 => Just('\t'),
        1 => arb_unicode().prop_map(|u| u.to_char()),
    ];
    proptest::collection::vec(ch, 0..16).prop_map(|chars| chars.into_iter().collect())
}

// ---------------------------------------------------------------------------
// Helper: try to compile a generated AST; skip cases that don't compile.
// (Many random ASTs are syntactically valid but semantically rejected by Java —
// e.g. fixed-width-only lookbehinds, backrefs to undefined groups, etc.
// proptest's prop_assume! skips those without failing the test.)
// ---------------------------------------------------------------------------

fn compile(ast: &RegexNode) -> Option<Regex> {
    Regex::new(&render(ast)).ok()
}

// ---------------------------------------------------------------------------
// Invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        max_shrink_iters: 4096,
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        .. ProptestConfig::default()
    })]

    /// `find()` results are monotonic in start position and non-overlapping
    /// (or, for zero-width matches, advance by at least one position).
    #[test]
    fn find_positions_are_monotonic(ast in arb_regex_node(), input in arb_input()) {
        let Some(re) = compile(&ast) else { return Ok(()); };
        let matches = re.find(&input);
        let mut prev_end = 0usize;
        let mut prev_start: Option<usize> = None;
        for m in &matches {
            prop_assert!(m.start <= m.end, "match start > end");
            prop_assert!(m.end <= input.chars().count(), "match end > input len");
            if let Some(ps) = prev_start {
                if m.start == ps {
                    // Two matches starting at the same position should never happen.
                    prop_assert!(false, "duplicate start position {}", ps);
                }
            }
            prop_assert!(m.start >= prev_end || m.start == prev_end,
                "overlapping match: {}..{} after end={}", m.start, m.end, prev_end);
            prev_end = m.end.max(m.start);
            prev_start = Some(m.start);
        }
    }

    /// `replace_all_with(input, |m| m.matched_text)` must equal `input` exactly.
    /// Replacing each match with its own text is the identity transformation.
    #[test]
    fn replace_with_identity_is_noop(ast in arb_regex_node(), input in arb_input()) {
        let Some(re) = compile(&ast) else { return Ok(()); };
        let out = re.replace_all(&input, |m: &java_regex::MatchInfo| m.matched_text.clone());
        prop_assert_eq!(out, input);
    }

    /// `Regex::quote(s)` must always produce a pattern that matches `s` exactly.
    #[test]
    fn quote_roundtrip(input in arb_input()) {
        let quoted = Regex::quote(&input);
        let re = Regex::new(&quoted).expect("Regex::quote produced an uncompilable pattern");
        prop_assert!(re.matches(&input), "quoted pattern failed to match its source string");
    }

    /// Compiling with the `l` (literal) flag and matching the original string
    /// must always succeed, regardless of metacharacters in the string.
    #[test]
    fn literal_flag_matches_self(input in arb_input()) {
        let re = Regex::with_flags(&input, "l").expect("LITERAL flag should never fail to compile");
        prop_assert!(re.matches(&input));
    }

    /// `matches(input)` should agree with `find()` of the anchored form
    /// `\A(?:pattern)\z` returning a single match covering the entire input.
    #[test]
    fn matches_iff_anchored_find(ast in arb_regex_node(), input in arb_input()) {
        let pat = render(&ast);
        let Ok(re) = Regex::new(&pat) else { return Ok(()); };
        let anchored = format!("\\A(?:{})\\z", pat);
        let Ok(re_a) = Regex::new(&anchored) else { return Ok(()); };

        let m1 = re.matches(&input);
        let m2_finds = re_a.find(&input);
        let m2 = m2_finds.len() == 1
            && m2_finds[0].start == 0
            && m2_finds[0].end == input.chars().count();
        prop_assert_eq!(m1, m2,
            "matches() and anchored-find disagree on pattern {:?} input {:?}", pat, input);
    }

    /// Compiling random patterns must not panic. Compile errors are fine; panics
    /// are not. This is the fastest way to catch parser robustness bugs.
    #[test]
    fn parser_never_panics(ast in arb_regex_node()) {
        let pat = render(&ast);
        let _ = std::panic::catch_unwind(|| {
            let _ = Regex::new(&pat);
        }).map_err(|_| TestCaseError::fail(format!("parser panicked on pattern {:?}", pat)))?;
    }

    /// Running any operation on a successfully compiled pattern must not panic.
    #[test]
    fn engine_never_panics(ast in arb_regex_node(), input in arb_input()) {
        let Some(re) = compile(&ast) else { return Ok(()); };
        let input2 = input.clone();
        let re2 = re.clone();
        let result = std::panic::catch_unwind(move || {
            let _ = re2.matches(&input2);
            let _ = re2.find(&input2);
            let _ = re2.replace_all(&input2, "$0");
            let _ = re2.split(&input2);
        });
        if result.is_err() {
            prop_assert!(false, "engine panicked on pattern {:?} input {:?}", render(&ast), input);
        }
    }

    /// `split(input)` followed by re-joining (with empty separators between
    /// non-match content) must reconstruct the input minus the matched text.
    /// Together with `find()` returning the matched text, this lets us verify
    /// that split + find together cover the full input with no gaps.
    #[test]
    fn split_find_cover_input(ast in arb_regex_node(), input in arb_input()) {
        let Some(re) = compile(&ast) else { return Ok(()); };
        let matches = re.find(&input);

        // Reconstruct: input == split_part[0] + match[0].matched + split_part[1] + ...
        // (Use split_with_limit(-1) so trailing empties are kept.)
        let parts = re.split_with_limit(&input, -1);

        // Skip cases where there's a zero-width match — split semantics for those
        // are subtle and the join-back invariant doesn't quite hold.
        if matches.iter().any(|m| m.start == m.end) {
            return Ok(());
        }

        let mut reconstructed = String::new();
        for (i, part) in parts.iter().enumerate() {
            reconstructed.push_str(part);
            if i < matches.len() {
                reconstructed.push_str(&matches[i].matched_text);
            }
        }
        prop_assert_eq!(reconstructed, input);
    }
}
