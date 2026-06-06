use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::types::*;
use crate::unicode::*;

/// Whether a `Pattern` is "deterministic" in OpenJDK's sense — i.e., would be
/// compiled with `GroupCurly` (atomic) rather than `Loop` (with backtracking)
/// when used as a quantifier atom. Mirrors `TreeInfo.deterministic` propagation
/// in `java.util.regex.Pattern`.
///
/// The practical effect: when a single-branch deterministic body is quantified
/// like `(?:\R){2}` or `(?i:\R){2}`, the engine must not backtrack into the
/// atom's internal choices (e.g., `\R`'s `\r\n` vs single-char). Multi-branch
/// bodies (alternation) keep backtracking, matching OpenJDK's `Loop`.
fn is_deterministic_body(p: &Pattern) -> bool {
    p.branches.len() == 1 && p.branches[0].iter().all(node_is_deterministic)
}

/// Maximum match length of a Pattern, or None if the engine cannot prove a
/// finite upper bound. Same logic as `parser::pattern_max_length` — duplicated
/// here so the engine can detect "zero-width body" groups at match time.
/// Minimum match length of a Pattern. Walks the AST recursively; conservative
/// (returns 0 for things it can't statically size, like backrefs).
fn pattern_min_length(p: &Pattern) -> usize {
    p.branches.iter().map(|b| b.iter().map(node_min_length).sum::<usize>()).min().unwrap_or(0)
}

fn node_min_length(n: &Node) -> usize {
    match n {
        Node::Literal(_) | Node::Dot | Node::CharClass(_) => 1,
        Node::LinebreakMatcher => 1,  // \R minimum is 1 (single char)
        Node::Anchor(_) | Node::SetFlags(_) | Node::RestoreFlags(_)
        | Node::Lookahead { .. } | Node::Lookbehind { .. } => 0,
        Node::Group { inner, .. }
        | Node::FlagGroup { inner, .. }
        | Node::AtomicGroup { inner } => pattern_min_length(inner),
        Node::Quantified { inner, min, .. } => node_min_length(inner) * (*min as usize),
        _ => 0,
    }
}

fn pattern_max_length(p: &Pattern) -> Option<usize> {
    let mut max = 0;
    for branch in &p.branches {
        let mut total: usize = 0;
        for node in branch {
            total = total.checked_add(node_max_length(node)?)?;
        }
        if total > max { max = total; }
    }
    Some(max)
}

/// Computes the body's `maxLength` the same way Java's `Pattern.java::TreeInfo`
/// does: with `i32` wrapping arithmetic and the `MAX_REPS` constant. Used by
/// `check_lookbehind` to mirror Java's overflow-induced iteration skip.
///
/// Java compiles `(?:X)?`/`(?:X)??` (and `X?` when X is a group) into
/// `Branch[head, null]`/`Branch[null, head]`; Branch.study takes signed
/// `Math.max` across atoms (null contributes 0). For `(?:X){0,N}` or
/// `(?:X){0,N}?` (non-Ques) it uses `Curly` whose study multiplies inner_max
/// by cmax with i32 wrapping — overflows are NOT clamped. Concatenated
/// nodes' contributions accumulate via i32 wrapping addition.
const JAVA_MAX_REPS: i32 = 0x7FFFFFFF;

fn pattern_java_max(p: &Pattern) -> i32 {
    let mut max: Option<i32> = None;
    for branch in &p.branches {
        let mut total: i32 = 0;
        for node in branch {
            total = total.wrapping_add(node_java_max(node));
        }
        max = Some(max.map_or(total, |m| m.max(total)));
    }
    max.unwrap_or(0)
}

fn node_java_max(n: &Node) -> i32 {
    match n {
        Node::Literal(_) | Node::Dot | Node::CharClass(_) => 1,
        Node::LinebreakMatcher => 2,
        Node::Anchor(_) | Node::SetFlags(_) | Node::RestoreFlags(_)
        | Node::Lookahead { .. } | Node::Lookbehind { .. } => 0,
        Node::Group { inner, .. }
        | Node::FlagGroup { inner, .. }
        | Node::AtomicGroup { inner } => pattern_java_max(inner),
        Node::Quantified { inner, max, .. } => {
            if *max == 0 { return 0; }
            let inner_max = node_java_max(inner);
            if *max == 1 {
                // Java's Ques greedy/lazy is wrapped in `Branch[head, null]`
                // (or `Branch[null, head]`); Branch.study takes signed
                // Math.max across atoms, with null contributing 0. So an
                // overflowed (negative) inner_max gets clamped to 0 by the
                // sibling null-atom. Mirror that here.
                inner_max.max(0)
            } else if *max == u32::MAX {
                inner_max.wrapping_mul(JAVA_MAX_REPS)
            } else {
                inner_max.wrapping_mul(*max as i32)
            }
        }
        _ => 0,
    }
}


fn node_max_length(n: &Node) -> Option<usize> {
    match n {
        Node::Literal(_) | Node::Dot | Node::CharClass(_) => Some(1),
        Node::LinebreakMatcher => Some(2),
        Node::Anchor(_) | Node::SetFlags(_) | Node::RestoreFlags(_)
        | Node::Lookahead { .. } | Node::Lookbehind { .. } => Some(0),
        Node::Group { inner, .. }
        | Node::FlagGroup { inner, .. }
        | Node::AtomicGroup { inner } => pattern_max_length(inner),
        Node::Quantified { inner, max, .. } => {
            if *max == 0 {
                // `X{0}` matches zero times regardless of `X` — even if X's
                // own max length is unbounded (e.g. `\3{0}` where `\3` is a
                // backref to a non-existent group).
                return Some(0);
            }
            let inner_max = node_max_length(inner)?;
            if inner_max == 0 {
                Some(0)
            } else if *max == u32::MAX {
                None
            } else {
                inner_max.checked_mul(*max as usize)
            }
        }
        Node::Backreference(_) | Node::NamedBackreference(_) => None,
        Node::GraphemeCluster => None,
        _ => None,
    }
}

fn node_is_deterministic(n: &Node) -> bool {
    match n {
        // Alternation (multi-branch Pattern) is the canonical non-deterministic
        // construct — Group/FlagGroup with a multi-branch body falls through to
        // is_deterministic_body returning false.
        Node::Group { inner, .. }
        | Node::FlagGroup { inner, .. }
        | Node::AtomicGroup { inner } => is_deterministic_body(inner),
        // Java's `study()` marks Lookbehind/Lookahead as deterministic
        // regardless of internal alternation — lookarounds don't consume, so
        // their internal structure doesn't affect the outer quantifier's
        // backtracking model. Mirror that: a lookaround atom is deterministic
        // from the perspective of any enclosing quantifier.
        Node::Lookahead { .. }
        | Node::Lookbehind { .. } => true,
        // Java's `Curly.study` keeps `deterministic` only when min == max AND
        // the atom is deterministic.
        Node::Quantified { inner, min, max, .. } =>
            min == max && node_is_deterministic(inner),
        _ => true,
    }
}

fn single_char_lowercase(c: char) -> Option<char> {
    let mut iter = c.to_lowercase();
    let first = iter.next()?;
    if iter.next().is_some() { None } else { Some(first) }
}

fn single_char_uppercase(c: char) -> Option<char> {
    let mut iter = c.to_uppercase();
    let first = iter.next()?;
    if iter.next().is_some() { None } else { Some(first) }
}

pub struct Engine<'a> {
    pub input: &'a [char],
    pub flags: Flags,
    pub group_count: usize,
    pub named_groups: &'a BTreeMap<String, usize>,
    steps: u64,
    max_steps: u64,
    depth: u32,
    max_depth: u32,
    pub search_start: usize,
    /// Inclusive lower bound for matching positions (default 0).
    /// Mirrors OpenJDK `Matcher.from`.
    pub text_start: usize,
    /// Exclusive upper bound for matching positions (default `input.len()`).
    /// Mirrors OpenJDK `Matcher.to`. Anchors and bounds checks use this
    /// instead of `input.len()` so the full input is available for
    /// context-dependent lookups (e.g. `\Z`'s "previous char is \r" check
    /// needs to see chars OUTSIDE the region — same as OpenJDK's Dollar).
    pub text_end: usize,
}

#[derive(Clone, Debug)]
pub struct State {
    pub captures: Vec<Option<(usize, usize)>>,
    pub match_end: usize,
}

impl State {
    pub fn new(group_count: usize) -> Self {
        State {
            captures: vec![None; group_count + 1],
            match_end: 0,
        }
    }
}

impl<'a> Engine<'a> {
    pub fn new(input: &'a [char], flags: Flags, group_count: usize, named_groups: &'a BTreeMap<String, usize>) -> Self {
        Engine {
            input,
            flags,
            group_count,
            named_groups,
            steps: 0,
            max_steps: 5_000_000,
            depth: 0,
            max_depth: 500,
            search_start: 0,
            text_start: 0,
            text_end: input.len(),
        }
    }

    /// Effective end-of-text for matching: `text_end`, mirroring `Matcher.to`.
    #[inline]
    pub fn text_len(&self) -> usize { self.text_end }

    fn step(&mut self) -> bool {
        self.steps += 1;
        self.steps < self.max_steps
    }

    #[allow(clippy::type_complexity)]
    pub fn try_match_at(&mut self, pattern: &Pattern, pos: usize) -> Option<(usize, Vec<Option<(usize, usize)>>)> {
        let mut state = State::new(self.group_count);
        if self.match_pattern(pattern, &[], pos, &mut state) {
            Some((state.match_end, state.captures))
        } else {
            None
        }
    }

    /// Try to match the pattern at `pos`, using the caller-provided State so
    /// captures can persist across iterations. This mirrors OpenJDK's
    /// `Start.match` loop: between successive position attempts, `groups[]` is
    /// never cleared, so a failed attempt's captures (from internal paths that
    /// succeeded before the overall match failed — e.g. negative lookarounds
    /// whose inner matched) leak into subsequent attempts.
    ///
    /// Returns `Some(end_pos)` on success and `None` on failure. State is
    /// mutated either way.
    pub fn try_match_at_persistent(&mut self, pattern: &Pattern, pos: usize, state: &mut State) -> Option<usize> {
        for branch in &pattern.branches {
            let combined = branch.clone();
            if self.match_nodes(&combined, pos, state) {
                return Some(state.match_end);
            }
            // Intentionally NO save/restore: failed branch's mutations persist,
            // matching OpenJDK's `Branch.match` which similarly never resets
            // `matcher.groups[]` between alternatives.
        }
        None
    }

    pub fn match_pattern(&mut self, pattern: &Pattern, rest: &[Node], pos: usize, state: &mut State) -> bool {
        for branch in &pattern.branches {
            let mut combined = branch.clone();
            combined.extend_from_slice(rest);
            if self.match_nodes(&combined, pos, state) {
                return true;
            }
            // No save/restore between branches — captures from failed branches
            // leak into subsequent branches, matching OpenJDK's `Branch.match`
            // which doesn't reset `matcher.groups[]` between alternatives. This
            // is what makes capture leaks visible across nested lookarounds:
            // `(?<!(?!(?<bar>.)))` on "\t\n" leaks `bar=\t` from the inner
            // capture that the inversion-then-inversion buried.
        }
        false
    }

    fn match_nodes(&mut self, nodes: &[Node], pos: usize, state: &mut State) -> bool {
        if !self.step() { return false; }
        self.depth += 1;
        if self.depth > self.max_depth {
            self.depth -= 1;
            return false;
        }
        let result = self.match_nodes_inner(nodes, pos, state);
        self.depth -= 1;
        result
    }

    fn match_nodes_inner(&mut self, nodes: &[Node], pos: usize, state: &mut State) -> bool {

        if nodes.is_empty() {
            state.match_end = pos;
            return true;
        }

        match &nodes[0] {
            Node::Literal(ch) => {
                if pos < self.text_len() {
                    let matched = if self.flags.case_insensitive {
                        chars_eq_ci(self.input[pos], *ch, self.flags.unicode_case)
                    } else {
                        self.input[pos] == *ch
                    };
                    if matched {
                        return self.match_nodes(&nodes[1..], pos + 1, state);
                    }
                }
                false
            }

            Node::Dot => {
                if pos < self.text_len() && (self.flags.dotall || !self.is_lt(self.input[pos])) {
                    self.match_nodes(&nodes[1..], pos + 1, state)
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
                if pos < self.text_len() && self.match_char_class(cc, self.input[pos]) {
                    self.match_nodes(&nodes[1..], pos + 1, state)
                } else {
                    false
                }
            }

            Node::Group { index, inner, .. } => {
                // No per-branch save/restore on captures — mirrors OpenJDK's
                // Branch.match which doesn't reset groups[] between alternatives.
                // Per-group save/restore happens inside GroupEnd (the GroupTail
                // analog) for ONLY the slot owned by this Group.
                //
                // Flag scoping: append RestoreFlags(saved) at the end of each
                // branch so inline `(?mu)` setters inside don't leak past the
                // group. This mirrors OpenJDK's compile-time flag scoping where
                // `(?:(?mu))` doesn't propagate `m` to nodes after the group.
                let start = pos;
                let saved_flags = self.flags;
                for branch in &inner.branches {
                    let mut combined = branch.clone();
                    if let Some(idx) = index {
                        combined.push(Node::GroupEnd { index: *idx, start });
                    }
                    combined.push(Node::RestoreFlags(saved_flags));
                    combined.extend_from_slice(&nodes[1..]);
                    if self.match_nodes(&combined, pos, state) {
                        return true;
                    }
                }
                false
            }

            Node::GroupEnd { index, start } => {
                // Per-group save/restore — mirrors OpenJDK GroupTail.match:
                // save the slot, set it, call next, restore only on failure.
                // Captures of OTHER groups set during the body's matching are
                // left intact (so they can leak through).
                let saved = state.captures[*index];
                state.captures[*index] = Some((*start, pos));
                if self.match_nodes(&nodes[1..], pos, state) {
                    return true;
                }
                state.captures[*index] = saved;
                false
            }

            Node::Quantified { inner, min, max, kind } => {
                let rest = &nodes[1..];
                match kind {
                    QuantKind::Greedy => self.match_greedy(inner, *min, *max, 0, rest, pos, pos, state),
                    QuantKind::Reluctant => self.match_reluctant(inner, *min, *max, 0, rest, pos, pos, state),
                    QuantKind::Possessive => self.match_possessive(inner, *min, *max, rest, pos, state),
                }
            }

            Node::Lookahead { positive, inner } => {
                // Shares State directly (no temp_state clone) so that captures
                // set by the inner pattern persist even when the surrounding
                // negative assertion inverts the result to failure. This is
                // what OpenJDK does: lookarounds don't save/restore groups[]
                // around the inner match, so captures from `(?=(\w))` leak
                // even when a later `\s` causes the overall attempt to fail.
                // Flag scoping: save/restore self.flags around the inner so
                // inline `(?ims)` setters inside don't leak past the lookaround.
                let saved_flags = self.flags;
                let matched = self.match_pattern(inner, &[], pos, state);
                self.flags = saved_flags;
                if matched == *positive {
                    self.match_nodes(&nodes[1..], pos, state)
                } else {
                    false
                }
            }

            Node::Lookbehind { positive, inner } => {
                let saved_flags = self.flags;
                let found = self.check_lookbehind(inner, pos, state);
                self.flags = saved_flags;
                if found == *positive {
                    self.match_nodes(&nodes[1..], pos, state)
                } else {
                    false
                }
            }

            Node::AtomicGroup { inner } => {
                // Shares State directly — captures from the inner's attempts
                // (including failed paths) persist into the caller. Mirrors
                // OpenJDK `Ques(X, INDEPENDENT)` which just does
                // `atom.match && next.match` without any save/restore.
                // Flag scoping: inline `(?ims)` inside doesn't leak past the
                // atomic group.
                let saved_flags = self.flags;
                let ok = self.match_pattern(inner, &[], pos, state);
                self.flags = saved_flags;
                if ok {
                    let end = state.match_end;
                    self.match_nodes(&nodes[1..], end, state)
                } else {
                    false
                }
            }

            Node::Backreference(idx) => {
                self.match_backref_by_index(*idx, &nodes[1..], pos, state)
            }

            Node::NamedBackreference(name) => {
                if let Some(&idx) = self.named_groups.get(name) {
                    self.match_backref_by_index(idx, &nodes[1..], pos, state)
                } else {
                    false
                }
            }

            Node::LinebreakMatcher => {
                // \R is atomic: (?>\r\n|[\n\v\f\r\x85  ]) — if \r\n
                // matches, normally commit to it. Inside a lookbehind, though,
                // the engine must enumerate every possible match length so the
                // outer PositionCheck constraint can be satisfied. There, fall
                // back to the single-char alternative when the \r\n branch
                // fails downstream.
                if pos < self.text_len() {
                    if self.input[pos] == '\r' && pos + 1 < self.text_len() && self.input[pos + 1] == '\n' {
                        if self.match_nodes(&nodes[1..], pos + 2, state) {
                            return true;
                        }
                        // Backtrack to single-char branch; \R is not atomic.
                        return self.match_nodes(&nodes[1..], pos + 1, state);
                    }
                    if is_linebreak(self.input[pos]) {
                        return self.match_nodes(&nodes[1..], pos + 1, state);
                    }
                }
                false
            }

            Node::SetFlags(new_flags) => {
                // Java treats inline `(?s)` (and friends) as a compile-time
                // directive: the flag takes effect for the rest of the pattern,
                // permanently. In particular, when alternation backtracks past
                // a `(?s)` in one branch, the flag remains set for subsequent
                // branches. So we don't roll back on failure here. (Engine state
                // is reset per find() attempt, so this doesn't leak across
                // distinct search positions.) Scoped `(?s:…)` groups use the
                // separate FlagGroup/RestoreFlags pair and DO scope properly.
                self.flags = *new_flags;
                self.match_nodes(&nodes[1..], pos, state)
            }

            Node::FlagGroup { flags, inner } => {
                // No per-branch save/restore on captures — matches OpenJDK's
                // Branch.match (FlagGroup is just a non-capturing group with
                // flag context; the branches don't reset groups[]).
                let old_flags = self.flags;
                self.flags = *flags;
                for branch in &inner.branches {
                    let mut combined = branch.clone();
                    combined.push(Node::RestoreFlags(old_flags));
                    combined.extend_from_slice(&nodes[1..]);
                    if self.match_nodes(&combined, pos, state) {
                        return true;
                    }
                }
                self.flags = old_flags;
                false
            }

            Node::RestoreFlags(flags) => {
                // Save/restore around the recursive call: the flag change is
                // scoped to the remaining nodes (the part of the pattern after
                // the FlagGroup). When the recursive call returns we're back
                // inside the FlagGroup's body — possibly mid-quantifier-loop
                // — and the inner flags must remain in effect. Without
                // restoring, a failed `rest` attempt inside a quantifier
                // would leak the outer flags into subsequent atom matches.
                let saved = self.flags;
                self.flags = *flags;
                let ok = self.match_nodes(&nodes[1..], pos, state);
                self.flags = saved;
                ok
            }

            Node::GraphemeCluster => {
                // \X matches a Unicode extended grapheme cluster.
                // This is a simplified implementation covering the most common cases:
                // - \r\n as a single cluster
                // - base char + combining marks (Mn, Mc, Me)
                // - regional indicator pairs (flag emoji)
                // - emoji ZWJ sequences (emoji + ZWJ + emoji + ...)
                // - emoji with variation selectors and skin tone modifiers
                if pos >= self.text_len() { return false; }
                let mut p = pos;
                if self.input[p] == '\r' && p + 1 < self.text_len() && self.input[p + 1] == '\n' {
                    p += 2;
                    return self.match_nodes(&nodes[1..], p, state);
                }
                // Regional indicator sequence: pairs of RI chars form flag emoji
                if is_regional_indicator(self.input[p]) {
                    p += 1;
                    if p < self.text_len() && is_regional_indicator(self.input[p]) {
                        p += 1;
                    }
                    return self.match_nodes(&nodes[1..], p, state);
                }
                // Consume base character
                p += 1;
                // Extend: combining marks, ZWJ sequences, variation selectors, modifiers
                while p < self.text_len() {
                    let ch = self.input[p];
                    if is_combining_mark(ch) || is_variation_selector(ch) || is_emoji_modifier(ch) {
                        p += 1;
                    } else if ch == '\u{200D}' && p + 1 < self.text_len() {
                        // ZWJ: consume ZWJ + next char
                        p += 1;
                        p += 1;
                    } else {
                        break;
                    }
                }
                self.match_nodes(&nodes[1..], p, state)
            }

            Node::PositionCheck(target) => {
                if pos == *target {
                    self.match_nodes(&nodes[1..], pos, state)
                } else {
                    false
                }
            }

            Node::GreedyCont { atom, min, max, count, rest, prev_pos } => {
                if pos == *prev_pos {
                    // No progress made — atom matched zero-width. Since it can
                    // match empty forever, treat as having reached min, try rest.
                    self.match_nodes(rest, pos, state)
                } else {
                    self.match_greedy(atom, *min, *max, *count, rest, pos, *prev_pos, state)
                }
            }

            Node::ReluctantCont { atom, min, max, count, rest, prev_pos } => {
                if pos == *prev_pos {
                    // No progress made — atom matched zero-width. Since it can
                    // match empty forever, treat as having reached min, try rest.
                    self.match_nodes(rest, pos, state)
                } else {
                    self.match_reluctant(atom, *min, *max, *count, rest, pos, *prev_pos, state)
                }
            }
        }
    }

    /// Shared backreference matching for both numbered and named backrefs.
    fn match_backref_by_index(&mut self, idx: usize, rest: &[Node], pos: usize, state: &mut State) -> bool {
        if let Some(Some((start, end))) = state.captures.get(idx) {
            let captured: Vec<char> = self.input[*start..*end].to_vec();
            let mut p = pos;
            for &ch in &captured {
                if p >= self.text_len() { return false; }
                if self.flags.case_insensitive {
                    if !chars_eq_ci(self.input[p], ch, self.flags.unicode_case) { return false; }
                } else if self.input[p] != ch {
                    return false;
                }
                p += 1;
            }
            self.match_nodes(rest, p, state)
        } else {
            false
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn match_greedy(
        &mut self, atom: &Node, min: u32, max: u32, count: u32,
        rest: &[Node], pos: usize, iter_start: usize, state: &mut State,
    ) -> bool {
        if !self.step() { return false; }

        if count < max {
            // No save/restore — captures from failed quantifier-atom attempts
            // leak (matching OpenJDK Curly/Loop, which never reset matcher.groups[]).
            if self.try_match_atom_greedy(atom, min, max, count, rest, pos, state) {
                return true;
            }
        }

        if count >= min {
            if self.match_nodes(rest, pos, state) {
                // Mirror Java's GroupCurly.match0 backoff: after `next.match`
                // succeeds, the OUTER GroupCurly explicitly sets
                //   groups[groupIndex]   = i - k
                //   groups[groupIndex+1] = i
                // overriding any captures set deeper in the chain. Equivalent
                // here: when this match_greedy frame's rest succeeds (= Java's
                // outer GroupCurly continuation), and our atom is a capturing
                // Group, override captures[idx] to (iter_start, pos) = the
                // last-consumed iter's slice.
                //
                // GUARD: only fire when the atom would use Path 1 (= Java's
                // GroupCurly conversion). For Path 2 (Ques `Branch[head, null]`
                // or non-det `Prolog(Loop)`), Java has no such override.
                if count > 0 {
                    if let Node::Group { index: Some(idx), inner, .. } = atom {
                        let is_ques = min == 0 && max == 1;
                        let chain_based = is_ques || !is_deterministic_body(inner);
                        if !chain_based {
                            state.captures[*idx] = Some((iter_start, pos));
                        }
                    }
                }
                return true;
            }
        }

        false
    }

    #[allow(clippy::too_many_arguments)]
    fn try_match_atom_greedy(
        &mut self, atom: &Node, min: u32, max: u32, count: u32,
        rest: &[Node], pos: usize, state: &mut State,
    ) -> bool {
        match atom {
            Node::Literal(ch) => {
                if pos < self.text_len() {
                    let matched = if self.flags.case_insensitive {
                        chars_eq_ci(self.input[pos], *ch, self.flags.unicode_case)
                    } else {
                        self.input[pos] == *ch
                    };
                    if matched {
                        return self.match_greedy(atom, min, max, count + 1, rest, pos + 1, pos, state);
                    }
                }
                false
            }
            Node::Dot => {
                if pos < self.text_len() && (self.flags.dotall || !self.is_lt(self.input[pos])) {
                    self.match_greedy(atom, min, max, count + 1, rest, pos + 1, pos, state)
                } else {
                    false
                }
            }
            Node::CharClass(cc) => {
                if pos < self.text_len() && self.match_char_class(cc, self.input[pos]) {
                    self.match_greedy(atom, min, max, count + 1, rest, pos + 1, pos, state)
                } else {
                    false
                }
            }
            Node::Group { index, inner, .. } => {
                let start = pos;
                // Mirror Java's compile-time mode selection:
                //   * Ques (`{0,1}`)  → `Branch[head, null] → BranchConn → next`
                //     (chain-threaded — inner GroupTails see outer continuation
                //     failure and restore).
                //   * Non-det `{n,m}` → `Prolog(Loop|LazyLoop)` (also chain-
                //     threaded via the body's loop-back).
                //   * Det `{n,m}`     → `GroupCurly` (atomic — atom.tail.next
                //     is a sentinel returning true, so inner GroupTails do NOT
                //     restore; only the outer group's own slot is managed).
                let deterministic = is_deterministic_body(inner);
                let is_ques = min == 0 && max == 1;
                let chain_based = is_ques || !deterministic;
                let saved = state.captures.clone();
                let saved_flags = self.flags;
                for branch in &inner.branches {
                    if chain_based {
                        // Path 2: chain-threaded via GreedyCont. Mirrors Java's
                        // `Branch[head, null] → BranchConn → next` (for Ques)
                        // or `Loop` body loop-back (for non-det Curly). Inner
                        // GroupEnd in `branch` sees its full continuation and
                        // properly restores its slot on downstream failure.
                        let mut combined = branch.clone();
                        if let Some(idx) = index {
                            combined.push(Node::GroupEnd { index: *idx, start });
                        }
                        combined.push(Node::RestoreFlags(saved_flags));
                        combined.push(Node::GreedyCont {
                            atom: Box::new(atom.clone()),
                            min, max,
                            count: count + 1,
                            rest: rest.to_vec(),
                            prev_pos: pos,
                        });
                        if self.match_nodes(&combined, pos, state) {
                            return true;
                        }
                    } else {
                        // Path 1: atomic body match. Mirrors Java's `GroupCurly`
                        // — atom is matched in isolation; inner captures inside
                        // atom leak (Java's GroupTail.next within GroupCurly's
                        // atom is a sentinel always returning true).
                        let mut combined = branch.clone();
                        if let Some(idx) = index {
                            combined.push(Node::GroupEnd { index: *idx, start });
                        }
                        combined.push(Node::RestoreFlags(saved_flags));
                        let mut branch_state = state.clone();
                        let branch_ok = self.match_nodes(&combined, pos, &mut branch_state);
                        state.captures = branch_state.captures.clone();
                        if branch_ok {
                            let new_pos = branch_state.match_end;
                            if new_pos > pos {
                                if self.match_greedy(atom, min, max, count + 1, rest, new_pos, pos, state) {
                                    return true;
                                }
                                // GroupCurly restores its OWN slot on overall
                                // failure (when capturing). Inner captures
                                // (set by inner Groups inside the body) leak.
                                if let Some(idx) = index {
                                    state.captures[*idx] = saved[*idx];
                                }
                            } else {
                                // Zero-width body. GroupCurly.match0 restores
                                // its own slot and calls next.match. We only
                                // restore when body is provably zero-width-only
                                // (pattern_max_length == Some(0)) and `max > 1`
                                // (excludes Ques which is now chain-based).
                                if count == 0 && min == 0 && max > 1
                                    && pattern_max_length(inner) == Some(0)
                                {
                                    if let Some(idx) = index {
                                        state.captures[*idx] = saved[*idx];
                                    }
                                }
                                if (count + 1) < min {
                                    if self.match_greedy(atom, min, max, count + 1, rest, pos, pos, state) {
                                        return true;
                                    }
                                    if let Some(idx) = index {
                                        state.captures[*idx] = saved[*idx];
                                    }
                                } else if self.match_nodes(rest, pos, state) {
                                    return true;
                                } else if let Some(idx) = index {
                                    state.captures[*idx] = saved[*idx];
                                }
                            }
                        }
                    }
                }
                // Group failed. Do NOT restore captures — OpenJDK's `Branch.match`
                // has no save/restore between alternatives, so leaks from failed
                // branches persist in caller state.
                let _ = saved;
                false
            }
            Node::FlagGroup { flags, inner } => {
                // Java has no runtime FlagGroup — flags are scoped at parse
                // time. We mirror the same Ques/Curly/GroupCurly split as the
                // Group arm above:
                //   * `(?...:X)?`  → chain-threaded.
                //   * `(?...:X){n,m}` with non-det X → chain-threaded.
                //   * `(?...:X){n,m}` with det X     → atomic.
                let deterministic = is_deterministic_body(inner);
                let is_ques = min == 0 && max == 1;
                let chain_based = is_ques || !deterministic;
                let old_flags = self.flags;
                self.flags = *flags;
                for branch in &inner.branches {
                    if chain_based {
                        let mut combined = branch.clone();
                        combined.push(Node::RestoreFlags(old_flags));
                        combined.push(Node::GreedyCont {
                            atom: Box::new(atom.clone()),
                            min, max,
                            count: count + 1,
                            rest: rest.to_vec(),
                            prev_pos: pos,
                        });
                        if self.match_nodes(&combined, pos, state) {
                            self.flags = old_flags;
                            return true;
                        }
                        self.flags = *flags;
                    } else {
                        // Path 1: atomic body match, mirroring GroupCurly.
                        let mut branch_state = state.clone();
                        let branch_ok = self.match_nodes(branch, pos, &mut branch_state);
                        state.captures = branch_state.captures.clone();
                        if branch_ok {
                            let new_pos = branch_state.match_end;
                            self.flags = old_flags;
                            if new_pos > pos {
                                if self.match_greedy(atom, min, max, count + 1, rest, new_pos, pos, state) {
                                    return true;
                                }
                            } else if (count + 1) < min {
                                if self.match_greedy(atom, min, max, count + 1, rest, pos, pos, state) {
                                    return true;
                                }
                            } else if self.match_nodes(rest, pos, state) {
                                return true;
                            }
                            self.flags = *flags;
                        }
                    }
                }
                self.flags = old_flags;
                false
            }
            Node::LinebreakMatcher => {
                // Inside a quantifier (`\R{n}`, `\R+`, `\R*`), OpenJDK's Curly
                // does NOT backtrack into the atom — each iteration takes the
                // longest \R available, and the engine doesn't retry a shorter
                // \R if a later iteration fails. So we match atomically here.
                // (Sequential `\R\R` still backtracks via the match_nodes arm.)
                if pos < self.text_len() {
                    if self.input[pos] == '\r' && pos + 1 < self.text_len() && self.input[pos + 1] == '\n' {
                        return self.match_greedy(atom, min, max, count + 1, rest, pos + 2, pos, state);
                    }
                    if is_linebreak(self.input[pos]) {
                        return self.match_greedy(atom, min, max, count + 1, rest, pos + 1, pos, state);
                    }
                }
                false
            }
            Node::Backreference(idx) => {
                if let Some(Some((start, end))) = state.captures.get(*idx).cloned() {
                    let cap_len = end - start;
                    let captured: Vec<char> = self.input[start..end].to_vec();
                    let mut p = pos;
                    for &ch in &captured {
                        if p >= self.text_len() { return false; }
                        if self.flags.case_insensitive {
                            if !chars_eq_ci(self.input[p], ch, self.flags.unicode_case) { return false; }
                        } else if self.input[p] != ch {
                            return false;
                        }
                        p += 1;
                    }
                    if cap_len == 0 {
                        // Empty backref — zero-width iter. Java's Curly greedy
                        // arm detects `i == matcher.last` and proceeds to next
                        // without further iteration. Mirror that: if past min,
                        // go to rest; else iterate just enough to satisfy min.
                        if (count + 1) < min {
                            self.match_greedy(atom, min, max, count + 1, rest, p, pos, state)
                        } else {
                            self.match_nodes(rest, p, state)
                        }
                    } else {
                        self.match_greedy(atom, min, max, count + 1, rest, p, pos, state)
                    }
                } else {
                    false
                }
            }
            _ => {
                // No temp_state clone — let captures from the atom's internal
                // attempts leak into `state`, matching OpenJDK. This is what
                // exposes capture leaks from negative lookarounds whose inner
                // matched (e.g. `(?<!(a|bb))c?` on "ac" leaks group 1 = "a").
                if self.match_nodes(core::slice::from_ref(atom), pos, state) {
                    let new_pos = state.match_end;
                    if new_pos > pos {
                        self.match_greedy(atom, min, max, count + 1, rest, new_pos, pos, state)
                    } else if (count + 1) < min {
                        // Zero-width but min not yet reached — iterate.
                        self.match_greedy(atom, min, max, count + 1, rest, pos, pos, state)
                    } else {
                        // Zero-width, min satisfied. Try rest.
                        self.match_nodes(rest, pos, state)
                    }
                } else {
                    false
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn match_reluctant(
        &mut self, atom: &Node, min: u32, max: u32, count: u32,
        rest: &[Node], pos: usize, iter_start: usize, state: &mut State,
    ) -> bool {
        if !self.step() { return false; }

        if count >= min {
            // No save/restore — see comment on match_greedy.
            if self.match_nodes(rest, pos, state) {
                // Mirror Java's GroupCurly.match1: after `next.match` succeeds
                // at iteration N, the OUTER GroupCurly's prior iter set
                //   groups[groupIndex]   = i (= position at iter N entry)
                //   groups[groupIndex+1] = matcher.last (= position after consuming atom)
                // — i.e. the LAST consumed atom's slice. Equivalent here:
                // override captures[idx] = (iter_start, pos).
                //
                // GUARD: only fire when atom would use Path 1 (Java's
                // GroupCurly). For Path 2 (Ques or non-det Loop) Java has no
                // such override.
                if count > 0 {
                    if let Node::Group { index: Some(idx), inner, .. } = atom {
                        let is_ques = min == 0 && max == 1;
                        let chain_based = is_ques || !is_deterministic_body(inner);
                        if !chain_based {
                            state.captures[*idx] = Some((iter_start, pos));
                        }
                    }
                }
                return true;
            }
        }

        if count < max {
            self.try_match_atom_reluctant(atom, min, max, count, rest, pos, state)
        } else {
            false
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn try_match_atom_reluctant(
        &mut self, atom: &Node, min: u32, max: u32, count: u32,
        rest: &[Node], pos: usize, state: &mut State,
    ) -> bool {
        match atom {
            Node::Literal(ch) => {
                if pos < self.text_len() {
                    let matched = if self.flags.case_insensitive {
                        chars_eq_ci(self.input[pos], *ch, self.flags.unicode_case)
                    } else {
                        self.input[pos] == *ch
                    };
                    if matched {
                        return self.match_reluctant(atom, min, max, count + 1, rest, pos + 1, pos, state);
                    }
                }
                false
            }
            Node::Dot => {
                if pos < self.text_len() && (self.flags.dotall || !self.is_lt(self.input[pos])) {
                    self.match_reluctant(atom, min, max, count + 1, rest, pos + 1, pos, state)
                } else {
                    false
                }
            }
            Node::CharClass(cc) => {
                if pos < self.text_len() && self.match_char_class(cc, self.input[pos]) {
                    self.match_reluctant(atom, min, max, count + 1, rest, pos + 1, pos, state)
                } else {
                    false
                }
            }
            Node::Group { index, inner, .. } => {
                let start = pos;
                // Same compile-time mode selection as the greedy arm above —
                // see that arm's comment for the rationale.
                let deterministic = is_deterministic_body(inner);
                let is_ques = min == 0 && max == 1;
                let chain_based = is_ques || !deterministic;
                let saved = state.captures.clone();
                let saved_flags = self.flags;
                for branch in inner.branches.iter() {
                    if chain_based {
                        let mut combined = branch.clone();
                        if let Some(idx) = index {
                            combined.push(Node::GroupEnd { index: *idx, start });
                        }
                        combined.push(Node::RestoreFlags(saved_flags));
                        combined.push(Node::ReluctantCont {
                            atom: Box::new(atom.clone()),
                            min, max,
                            count: count + 1,
                            rest: rest.to_vec(),
                            prev_pos: pos,
                        });
                        if self.match_nodes(&combined, pos, state) {
                            return true;
                        }
                    } else {
                        // Path 1: atomic, mirrors GroupCurly.
                        let mut combined = branch.clone();
                        if let Some(idx) = index {
                            combined.push(Node::GroupEnd { index: *idx, start });
                        }
                        combined.push(Node::RestoreFlags(saved_flags));
                        let mut branch_state = state.clone();
                        let branch_ok = self.match_nodes(&combined, pos, &mut branch_state);
                        state.captures = branch_state.captures.clone();
                        if branch_ok {
                            let new_pos = branch_state.match_end;
                            if new_pos > pos {
                                if self.match_reluctant(atom, min, max, count + 1, rest, new_pos, pos, state) {
                                    return true;
                                }
                                if let Some(idx) = index {
                                    state.captures[*idx] = saved[*idx];
                                }
                            } else {
                                // Reluctant body matched zero-width.
                                //
                                // Java's Curly LAZY:
                                //   * cmin loop: iterate `min` times
                                //     unconditionally (no zero-width abort).
                                //   * Then `match1`: try `next.match` once at
                                //     the top; on failure, expand body — but
                                //     ZERO-WIDTH ABORTS without retrying next.
                                //
                                // So we must try `rest` exactly ONCE per
                                // entry-to-`min`: when `count + 1 == min`
                                // (just reaching min after the cmin-equivalent
                                // body match). When `count >= min` already,
                                // `match_reluctant` tried rest at the top and
                                // we must NOT retry (else captures set by the
                                // body's atomic match could satisfy a `\1` in
                                // the continuation that the first attempt had
                                // legitimately failed).
                                if count == 0 && min == 0 && max > 1
                                    && pattern_max_length(inner) == Some(0)
                                {
                                    if let Some(idx) = index {
                                        state.captures[*idx] = saved[*idx];
                                    }
                                }
                                if (count + 1) < min {
                                    if self.match_reluctant(atom, min, max, count + 1, rest, pos, pos, state) {
                                        return true;
                                    }
                                    if let Some(idx) = index {
                                        state.captures[*idx] = saved[*idx];
                                    }
                                } else if count + 1 == min && self.match_nodes(rest, pos, state) {
                                    return true;
                                } else if let Some(idx) = index {
                                    state.captures[*idx] = saved[*idx];
                                }
                            }
                        }
                    }
                }
                let _ = saved;
                false
            }
            Node::LinebreakMatcher => {
                // Atomic match inside a reluctant quantifier — see the
                // matching arm in `try_match_atom_greedy`. OpenJDK's Curly
                // doesn't backtrack into a quantified `\R`.
                if pos < self.text_len() {
                    if self.input[pos] == '\r' && pos + 1 < self.text_len() && self.input[pos + 1] == '\n' {
                        return self.match_reluctant(atom, min, max, count + 1, rest, pos + 2, pos, state);
                    }
                    if is_linebreak(self.input[pos]) {
                        return self.match_reluctant(atom, min, max, count + 1, rest, pos + 1, pos, state);
                    }
                }
                false
            }
            Node::FlagGroup { flags, inner } => {
                // Mirror of the Group reluctant arm above.
                let deterministic = is_deterministic_body(inner);
                let old_flags = self.flags;
                self.flags = *flags;
                let is_ques = min == 0 && max == 1;
                let chain_based = is_ques || !deterministic;
                for branch in &inner.branches {
                    if chain_based {
                        let mut combined = branch.clone();
                        combined.push(Node::RestoreFlags(old_flags));
                        combined.push(Node::ReluctantCont {
                            atom: Box::new(atom.clone()),
                            min, max,
                            count: count + 1,
                            rest: rest.to_vec(),
                            prev_pos: pos,
                        });
                        if self.match_nodes(&combined, pos, state) {
                            self.flags = old_flags;
                            return true;
                        }
                        self.flags = *flags;
                    } else {
                        // Path 1: atomic, mirrors GroupCurly.
                        let mut branch_state = state.clone();
                        let branch_ok = self.match_nodes(branch, pos, &mut branch_state);
                        state.captures = branch_state.captures.clone();
                        if branch_ok {
                            let new_pos = branch_state.match_end;
                            self.flags = old_flags;
                            if new_pos > pos {
                                if self.match_reluctant(atom, min, max, count + 1, rest, new_pos, pos, state) {
                                    return true;
                                }
                            } else if (count + 1) < min {
                                if self.match_reluctant(atom, min, max, count + 1, rest, pos, pos, state) {
                                    return true;
                                }
                            } else if count + 1 == min && self.match_nodes(rest, pos, state) {
                                // Mirror Java's Curly LAZY: same as the generic
                                // arm — only try rest once when count just
                                // reached min via the cmin-equivalent body
                                // match.
                                return true;
                            }
                            self.flags = *flags;
                        }
                    }
                }
                self.flags = old_flags;
                false
            }
            _ => {
                // Same no-temp_state strategy as the matching arm in
                // try_match_atom_greedy — let inner captures leak.
                if self.match_nodes(core::slice::from_ref(atom), pos, state) {
                    let new_pos = state.match_end;
                    if new_pos > pos {
                        self.match_reluctant(atom, min, max, count + 1, rest, new_pos, pos, state)
                    } else if (count + 1) < min {
                        // Zero-width but min not yet reached — iterate.
                        self.match_reluctant(atom, min, max, count + 1, rest, pos, pos, state)
                    } else if count + 1 == min {
                        // Mirror Java's Curly LAZY: cmin loop iterates `min`
                        // times unconditionally. After cmin, `match1` runs and
                        // tries `next.match` at the top. We mirror that single
                        // attempt when count+1 just reaches min. If count >=
                        // min already, `match_reluctant` tried rest at the top
                        // — do NOT retry here (Java's match1 zero-width abort
                        // `if (i == matcher.last) return false;` applies).
                        self.match_nodes(rest, pos, state)
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
        &mut self, atom: &Node, min: u32, max: u32,
        rest: &[Node], pos: usize, state: &mut State,
    ) -> bool {
        // Mirrors OpenJDK Curly POSSESSIVE: a cmin loop that iterates `min`
        // times unconditionally (without zero-width check), followed by
        // `match2` that iterates while atom advances. Captures leak — atom.match
        // shares state.
        let mut current_pos = pos;
        let mut count = 0u32;

        // cmin loop: must succeed `min` times, no zero-width abort.
        while count < min {
            if !self.match_nodes(core::slice::from_ref(atom), current_pos, state) {
                return false;
            }
            current_pos = state.match_end;
            count += 1;
        }

        // Post-cmin loop: keep iterating while atom matches AND advances.
        // Zero-width or fail breaks out (no possessive backtracking).
        while count < max {
            if !self.match_nodes(core::slice::from_ref(atom), current_pos, state) {
                break;
            }
            let new_pos = state.match_end;
            if new_pos == current_pos {
                break;
            }
            current_pos = new_pos;
            count += 1;
        }

        self.match_nodes(rest, current_pos, state)
    }

    fn is_lt(&self, c: char) -> bool {
        if self.flags.unix_lines { c == '\n' } else { is_line_terminator(c) }
    }

    fn is_after_line_terminator(&self, pos: usize) -> bool {
        // With anchoring bounds (default), positions at/before text_start
        // have no "preceding char" for line-terminator purposes.
        if pos <= self.text_start { return false; }
        let prev = self.input[pos - 1];
        if self.flags.unix_lines { return prev == '\n'; }
        if prev == '\n' {
            true
        } else if prev == '\r' {
            pos >= self.text_len() || self.input[pos] != '\n'
        } else {
            is_line_terminator(prev)
        }
    }

    fn check_anchor(&self, kind: AnchorKind, pos: usize) -> bool {
        match kind {
            AnchorKind::StartOfLine => {
                if self.flags.multiline {
                    // Java/Perl quirk: in multiline mode, ^ never matches at end of input
                    // (even after a trailing line terminator). OpenJDK's Caret has an
                    // explicit `if (i == endIndex) return false;` for the same reason.
                    if pos == self.text_len() { return false; }
                    pos == self.text_start || self.is_after_line_terminator(pos)
                } else {
                    pos == self.text_start
                }
            }
            AnchorKind::EndOfLine => {
                if self.flags.multiline {
                    if pos == self.text_len() { return true; }
                    if pos < self.text_len() && self.is_lt(self.input[pos]) {
                        if !self.flags.unix_lines && self.input[pos] == '\n' && pos > 0 && self.input[pos - 1] == '\r' {
                            return false;
                        }
                        return true;
                    }
                    false
                } else {
                    self.check_end_of_line(pos)
                }
            }
            AnchorKind::StartOfInput => pos == self.text_start,
            AnchorKind::EndOfInput => pos == self.text_len(),
            AnchorKind::EndOfInputBeforeFinalNewline => {
                if pos == self.text_len() { return true; }
                self.check_before_final_newline(pos)
            }
            AnchorKind::WordBoundary => {
                let before = self.word_char_before(pos);
                let after = self.word_char_after(pos);
                before != after
            }
            AnchorKind::NonWordBoundary => {
                let before = self.word_char_before(pos);
                let after = self.word_char_after(pos);
                before == after
            }
            AnchorKind::PreviousMatchEnd => pos == self.search_start,
        }
    }

    /// Check non-multiline $ and \Z: before final newline or at end.
    fn check_before_final_newline(&self, pos: usize) -> bool {
        let len = self.text_len();
        if len == 0 { return false; }
        if self.flags.unix_lines {
            return pos == len - 1 && self.input[pos] == '\n';
        }
        // \r\n at end: match before the \r (pos == len-2)
        if pos + 2 == len && self.input[pos] == '\r' && self.input[pos + 1] == '\n' {
            return true;
        }
        // Single line terminator at end (pos == len-1), but not the \n of a \r\n pair
        pos + 1 == len && matches!(self.input[pos], '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
            && !(self.input[pos] == '\n' && pos > 0 && self.input[pos - 1] == '\r')
    }

    /// Check if the character before pos is a word character,
    /// treating combining marks as inheriting word-status from preceding char.
    fn word_char_before(&self, pos: usize) -> bool {
        // With anchoring bounds (default), positions at/before text_start
        // have no preceding word char.
        if pos <= self.text_start { return false; }
        let mut i = pos - 1;
        // Skip back over combining marks to find the base character
        while i > 0 && is_combining_mark(self.input[i]) {
            i -= 1;
        }
        is_word_char(self.input[i], self.flags.unicode_class)
    }

    /// Check if the character at pos is a word character,
    /// treating combining marks as inheriting word-status from preceding char.
    fn word_char_after(&self, pos: usize) -> bool {
        if pos >= self.text_len() { return false; }
        if is_combining_mark(self.input[pos]) {
            // Combining mark inherits from preceding char
            return self.word_char_before(pos);
        }
        is_word_char(self.input[pos], self.flags.unicode_class)
    }

    fn check_end_of_line(&self, pos: usize) -> bool {
        if pos == self.text_len() { return true; }
        self.check_before_final_newline(pos)
    }

    fn check_lookbehind(&mut self, inner: &Pattern, pos: usize, state: &mut State) -> bool {
        // Shares the caller's State directly instead of using a temp clone.
        // This faithfully replicates OpenJDK: when the inner pattern matches
        // (setting captures), those captures persist even if the surrounding
        // assertion is negative — `(?<!(a|bb))c` on "ac" leaves group 1 = "a"
        // from the inner success that flipped the outer to fail.
        let rest = [Node::PositionCheck(pos)];
        let body_min = pattern_min_length(inner);
        if body_min > pos { return false; }
        let start_high = pos - body_min;
        // Compute `rmax` using Java's i32 wrapping arithmetic so we replicate
        // `Behind.match`/`NotBehind.match`'s iteration range exactly:
        //   from = max(i - rmax, startIndex)
        //   j ∈ [from, i - rmin]
        // For overflowed (negative) `rmax`, `i - rmax` (signed) can be large-
        // positive for small `i` — `j >= from` is FALSE and body iteration is
        // skipped (which makes negative lookbehind succeed and positive fail).
        // For `i` past the threshold, the subtraction itself overflows back
        // and `from` clamps to startIndex.
        let rmax = pattern_java_max(inner);
        let pos_i32 = pos as i32;
        let from_i32 = pos_i32.wrapping_sub(rmax);
        let start_low = if from_i32 < 0 {
            self.text_start
        } else if from_i32 as usize > start_high {
            return false; // Java's loop body never runs
        } else {
            (from_i32 as usize).max(self.text_start)
        };
        if start_low > start_high { return false; }
        for start in (start_low..=start_high).rev() {
            if self.match_pattern(inner, &rest, start, state) {
                return true;
            }
        }
        false
    }

    pub fn match_char_class(&self, cc: &CharClass, ch: char) -> bool {
        let matched = self.match_char_class_items(&cc.items, ch);
        if cc.negated { !matched } else { matched }
    }

    fn match_char_class_items(&self, items: &[CharClassItem], ch: char) -> bool {
        for item in items {
            match item {
                CharClassItem::Single(c) => {
                    if self.flags.case_insensitive {
                        if chars_eq_ci(ch, *c, self.flags.unicode_case) { return true; }
                    } else if ch == *c {
                        return true;
                    }
                }
                CharClassItem::Range(start, end) => {
                    if self.match_char_range(ch, *start, *end) { return true; }
                }
                CharClassItem::Predefined(pc) => {
                    if match_predefined_class(*pc, ch, self.flags.unicode_class) { return true; }
                }
                CharClassItem::UnicodeProperty { name, negated } => {
                    let uc = self.flags.unicode_class;
                    let mut matched = match_unicode_property_ext(name, ch, uc);
                    if !matched && self.flags.case_insensitive && !is_posix_class(name) {
                        // For Lu/Ll/Lt, case-insensitive matching treats them as LC (cased letter)
                        let name_lower = name.to_lowercase();
                        if matches!(name_lower.as_str(), "lu" | "uppercase_letter" | "ll" | "lowercase_letter" | "lt" | "titlecase_letter") {
                            matched = match_unicode_property_ext("lc", ch, uc);
                        } else if self.flags.unicode_case || name.starts_with("java") {
                            // Unicode case folding for unicode_case mode and java* properties
                            let upper = ch.to_uppercase().next().unwrap_or(ch);
                            let lower = ch.to_lowercase().next().unwrap_or(ch);
                            if upper != ch { matched = match_unicode_property_ext(name, upper, uc); }
                            if !matched && lower != ch { matched = match_unicode_property_ext(name, lower, uc); }
                        } else {
                            // ASCII case folding
                            let upper = ch.to_ascii_uppercase();
                            let lower = ch.to_ascii_lowercase();
                            if upper != ch { matched = match_unicode_property_ext(name, upper, uc); }
                            if !matched && lower != ch { matched = match_unicode_property_ext(name, lower, uc); }
                        }
                    }
                    if *negated { if !matched { return true; } }
                    else if matched { return true; }
                }
                CharClassItem::Nested(nested) => {
                    if self.match_char_class(nested, ch) { return true; }
                }
                CharClassItem::Intersection(left, right) => {
                    if self.match_char_class_items(left, ch) && self.match_char_class_items(right, ch) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn match_char_range(&self, ch: char, start: char, end: char) -> bool {
        if ch >= start && ch <= end { return true; }
        if self.flags.case_insensitive {
            // Java treats `[start-end]` under CASE_INSENSITIVE as the set of input
            // characters whose own case variant lands inside the (unmodified) range.
            // The previous implementation folded the range endpoints as well, which
            // shrinks ASCII letter ranges (e.g. `[1-c]` becomes `[1-C]` after
            // uppercasing) and wrongly excludes characters like 'g' whose uppercase
            // 'G' (0x47) is actually inside the original range 0x31..0x63.
            if self.flags.unicode_case {
                if let Some(u) = single_char_uppercase(ch) {
                    if u >= start && u <= end { return true; }
                }
                if let Some(l) = single_char_lowercase(ch) {
                    if l >= start && l <= end { return true; }
                }
            } else {
                let u = ch.to_ascii_uppercase();
                let l = ch.to_ascii_lowercase();
                if u >= start && u <= end { return true; }
                if l >= start && l <= end { return true; }
            }
            false
        } else {
            ch >= start && ch <= end
        }
    }
}
