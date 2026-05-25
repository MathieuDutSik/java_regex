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
            let inner_max = node_max_length(inner)?;
            if *max == u32::MAX {
                if inner_max <= 1 { Some(inner_max) } else { None }
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
        Node::Lookahead { inner, .. }
        | Node::Lookbehind { inner, .. } => is_deterministic_body(inner),
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
                // Shares State directly (no temp_state clone) so that captures
                // set by the inner pattern persist even when the surrounding
                // negative assertion inverts the result to failure. This is
                // what OpenJDK does: lookarounds don't save/restore groups[]
                // around the inner match, so captures from `(?=(\w))` leak
                // even when a later `\s` causes the overall attempt to fail.
                let matched = self.match_pattern(inner, &[], pos, state);
                if matched == *positive {
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
                let old_flags = self.flags;
                self.flags = *flags;
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
                    self.match_greedy(atom, *min, *max, *count, rest, pos, state)
                }
            }

            Node::ReluctantCont { atom, min, max, count, rest, prev_pos } => {
                if pos == *prev_pos {
                    // No progress made — atom matched zero-width. Since it can
                    // match empty forever, treat as having reached min, try rest.
                    self.match_nodes(rest, pos, state)
                } else {
                    self.match_reluctant(atom, *min, *max, *count, rest, pos, state)
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
        rest: &[Node], pos: usize, state: &mut State,
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
            return self.match_nodes(rest, pos, state);
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
                        return self.match_greedy(atom, min, max, count + 1, rest, pos + 1, state);
                    }
                }
                false
            }
            Node::Dot => {
                if pos < self.text_len() && (self.flags.dotall || !self.is_lt(self.input[pos])) {
                    self.match_greedy(atom, min, max, count + 1, rest, pos + 1, state)
                } else {
                    false
                }
            }
            Node::CharClass(cc) => {
                if pos < self.text_len() && self.match_char_class(cc, self.input[pos]) {
                    self.match_greedy(atom, min, max, count + 1, rest, pos + 1, state)
                } else {
                    false
                }
            }
            Node::Group { index, inner, .. } => {
                let start = pos;
                // OpenJDK quirk: when a Group with cmin=0 has a zero-max-length
                // body (truly empty `()`, or only zero-width content like
                // `\Q\E`, anchors, lookarounds), the group is treated as not
                // having executed — `g_i` stays unset (null). Mirrors
                // GroupCurly's `locals[localIndex] = -1` trick: GroupTail
                // sees the sentinel and skips the capture write.
                if count == 0 && min == 0 && pattern_max_length(inner) == Some(0) {
                    return self.match_nodes(rest, pos, state);
                }
                // OpenJDK uses `GroupCurly` (atomic) for deterministic bodies
                // and `Loop` (with continuation backtracking) for non-deterministic
                // ones. We model that split here.
                let deterministic = is_deterministic_body(inner);
                for branch in &inner.branches {
                    let saved = state.captures.clone();
                    let mut combined = branch.clone();
                    if let Some(idx) = index {
                        combined.push(Node::GroupEnd { index: *idx, start });
                    }
                    // Path 1: atomic — match the branch in isolation, then
                    // advance the quantifier with the resulting end position.
                    // For deterministic bodies, this is the ONLY path (matches
                    // OpenJDK's GroupCurly).
                    let mut branch_state = state.clone();
                    if self.match_nodes(&combined, pos, &mut branch_state) {
                        let new_pos = branch_state.match_end;
                        if new_pos > pos {
                            state.captures = branch_state.captures.clone();
                            if self.match_greedy(atom, min, max, count + 1, rest, new_pos, state) {
                                return true;
                            }
                            state.captures = saved.clone();
                        } else {
                            // Zero-width match — mirror OpenJDK's Curly which
                            // breaks out of the greedy loop on k == 0 and tries
                            // `next`. Treat the iteration as satisfying min so
                            // we don't loop forever on a zero-width atom.
                            state.captures = branch_state.captures.clone();
                            let effective = (count + 1).max(min);
                            if effective >= min && self.match_nodes(rest, pos, state) {
                                return true;
                            }
                            state.captures = saved.clone();
                        }
                    }
                    // Path 2: with continuation — Loop-style backtracking
                    // through the branch body. Only used for non-deterministic
                    // bodies, where OpenJDK uses `Loop` instead of `GroupCurly`.
                    if !deterministic {
                        let mut combined2 = branch.clone();
                        if let Some(idx) = index {
                            combined2.push(Node::GroupEnd { index: *idx, start });
                        }
                        combined2.push(Node::GreedyCont {
                            atom: Box::new(atom.clone()),
                            min, max,
                            count: count + 1,
                            rest: rest.to_vec(),
                            prev_pos: pos,
                        });
                        if self.match_nodes(&combined2, pos, state) {
                            return true;
                        }
                    }
                    state.captures = saved;
                }
                false
            }
            Node::FlagGroup { flags, inner } => {
                // Same atomic-vs-backtracking split as Group above. For
                // deterministic single-branch bodies (e.g. `(?i:\R){n}`),
                // matching the branch in isolation makes the inner atoms
                // atomic — which mirrors Java's behavior because Java has no
                // runtime FlagGroup node, only parse-time flag scoping.
                let deterministic = is_deterministic_body(inner);
                let old_flags = self.flags;
                self.flags = *flags;
                for branch in &inner.branches {
                    let saved = state.captures.clone();
                    // Path 1: atomic.
                    let mut branch_state = state.clone();
                    if self.match_nodes(branch, pos, &mut branch_state) {
                        let new_pos = branch_state.match_end;
                        self.flags = old_flags;
                        if new_pos > pos {
                            state.captures = branch_state.captures.clone();
                            if self.match_greedy(atom, min, max, count + 1, rest, new_pos, state) {
                                return true;
                            }
                            state.captures = saved.clone();
                        } else {
                            state.captures = branch_state.captures.clone();
                            let effective = (count + 1).max(min);
                            if effective >= min && self.match_nodes(rest, pos, state) {
                                return true;
                            }
                            state.captures = saved.clone();
                        }
                        self.flags = *flags;
                    }
                    // Path 2: with continuation — only for non-deterministic bodies.
                    if !deterministic {
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
                            return true;
                        }
                        state.captures = saved;
                        self.flags = *flags;
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
                    if cap_len == 0 { return false; }
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
                    self.match_greedy(atom, min, max, count + 1, rest, p, state)
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
                        self.match_greedy(atom, min, max, count + 1, rest, new_pos, state)
                    } else {
                        // Zero-width match — count as matched up to max, then try rest
                        let effective = (count + 1).max(min);
                        if effective >= min {
                            self.match_nodes(rest, pos, state)
                        } else {
                            false
                        }
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
        rest: &[Node], pos: usize, state: &mut State,
    ) -> bool {
        if !self.step() { return false; }

        if count >= min {
            // No save/restore — see comment on match_greedy.
            if self.match_nodes(rest, pos, state) { return true; }
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
                        return self.match_reluctant(atom, min, max, count + 1, rest, pos + 1, state);
                    }
                }
                false
            }
            Node::Dot => {
                if pos < self.text_len() && (self.flags.dotall || !self.is_lt(self.input[pos])) {
                    self.match_reluctant(atom, min, max, count + 1, rest, pos + 1, state)
                } else {
                    false
                }
            }
            Node::CharClass(cc) => {
                if pos < self.text_len() && self.match_char_class(cc, self.input[pos]) {
                    self.match_reluctant(atom, min, max, count + 1, rest, pos + 1, state)
                } else {
                    false
                }
            }
            Node::Group { index, inner, .. } => {
                let start = pos;
                // Same zero-max-body short-circuit as in try_match_atom_greedy
                // — see the comment there. Mirrors OpenJDK GroupCurly.
                if count == 0 && min == 0 && pattern_max_length(inner) == Some(0) {
                    return self.match_nodes(rest, pos, state);
                }
                let deterministic = is_deterministic_body(inner);
                for branch in &inner.branches {
                    let saved = state.captures.clone();
                    let mut combined = branch.clone();
                    if let Some(idx) = index {
                        combined.push(Node::GroupEnd { index: *idx, start });
                    }
                    // Path 1: atomic.
                    let mut branch_state = state.clone();
                    if self.match_nodes(&combined, pos, &mut branch_state) {
                        let new_pos = branch_state.match_end;
                        if new_pos > pos {
                            state.captures = branch_state.captures.clone();
                            if self.match_reluctant(atom, min, max, count + 1, rest, new_pos, state) {
                                return true;
                            }
                            state.captures = saved.clone();
                        } else {
                            state.captures = branch_state.captures.clone();
                            let effective = (count + 1).max(min);
                            if effective >= min && self.match_nodes(rest, pos, state) {
                                return true;
                            }
                            state.captures = saved.clone();
                        }
                    }
                    // Path 2: with continuation. Only for non-deterministic bodies.
                    if !deterministic {
                        let mut combined2 = branch.clone();
                        if let Some(idx) = index {
                            combined2.push(Node::GroupEnd { index: *idx, start });
                        }
                        combined2.push(Node::ReluctantCont {
                            atom: Box::new(atom.clone()),
                            min, max,
                            count: count + 1,
                            rest: rest.to_vec(),
                            prev_pos: pos,
                        });
                        if self.match_nodes(&combined2, pos, state) {
                            return true;
                        }
                    }
                    state.captures = saved;
                }
                false
            }
            Node::LinebreakMatcher => {
                // Atomic match inside a reluctant quantifier — see the
                // matching arm in `try_match_atom_greedy`. OpenJDK's Curly
                // doesn't backtrack into a quantified `\R`.
                if pos < self.text_len() {
                    if self.input[pos] == '\r' && pos + 1 < self.text_len() && self.input[pos + 1] == '\n' {
                        return self.match_reluctant(atom, min, max, count + 1, rest, pos + 2, state);
                    }
                    if is_linebreak(self.input[pos]) {
                        return self.match_reluctant(atom, min, max, count + 1, rest, pos + 1, state);
                    }
                }
                false
            }
            Node::FlagGroup { flags, inner } => {
                // Mirror of the Group arm above: atomic path always, plus a
                // continuation-backtracking path only for non-deterministic
                // (multi-branch / variable-length-quantified) bodies.
                let deterministic = is_deterministic_body(inner);
                let old_flags = self.flags;
                self.flags = *flags;
                for branch in &inner.branches {
                    let saved = state.captures.clone();
                    // Path 1: atomic.
                    let mut branch_state = state.clone();
                    if self.match_nodes(branch, pos, &mut branch_state) {
                        let new_pos = branch_state.match_end;
                        self.flags = old_flags;
                        if new_pos > pos {
                            state.captures = branch_state.captures.clone();
                            if self.match_reluctant(atom, min, max, count + 1, rest, new_pos, state) {
                                return true;
                            }
                            state.captures = saved.clone();
                        } else {
                            state.captures = branch_state.captures.clone();
                            let effective = (count + 1).max(min);
                            if effective >= min && self.match_nodes(rest, pos, state) {
                                return true;
                            }
                            state.captures = saved.clone();
                        }
                        self.flags = *flags;
                    }
                    // Path 2: with continuation — only non-deterministic.
                    if !deterministic {
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
                            return true;
                        }
                        state.captures = saved;
                        self.flags = *flags;
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
                        self.match_reluctant(atom, min, max, count + 1, rest, new_pos, state)
                    } else {
                        // Zero-width match — treat as satisfied, try rest
                        let effective = (count + 1).max(min);
                        if effective >= min {
                            self.match_nodes(rest, pos, state)
                        } else {
                            false
                        }
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
        let mut current_pos = pos;
        let mut count = 0u32;

        while count < max {
            let mut temp_state = state.clone();
            if self.match_nodes(core::slice::from_ref(atom), current_pos, &mut temp_state) {
                let new_pos = temp_state.match_end;
                state.captures = temp_state.captures;
                count += 1;
                // Zero-width match: stop to avoid infinite loop
                if new_pos == current_pos && count >= min { break; }
                current_pos = new_pos;
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
        // from the inner success that flipped the outer to fail. The inner
        // pattern's own alternation save/restore still cleans up captures
        // from genuinely-failed branches, so we only leak captures from
        // attempts that succeeded internally.
        let rest = [Node::PositionCheck(pos)];
        // With anchoring bounds (default), lookbehind doesn't look BEFORE
        // text_start. Mirrors OpenJDK's `from = max(i - rmax, matcher.from)`.
        for start in (self.text_start..=pos).rev() {
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
