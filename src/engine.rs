use std::collections::HashMap;

use crate::types::*;
use crate::unicode::*;

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

pub struct Engine {
    pub input: Vec<char>,
    pub flags: Flags,
    pub group_count: usize,
    pub named_groups: HashMap<String, usize>,
    steps: u64,
    max_steps: u64,
    depth: u32,
    max_depth: u32,
    pub search_start: usize,
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

impl Engine {
    pub fn new(input: &str, flags: Flags, group_count: usize, named_groups: HashMap<String, usize>) -> Self {
        Engine {
            input: input.chars().collect(),
            flags,
            group_count,
            named_groups,
            steps: 0,
            max_steps: 5_000_000,
            depth: 0,
            max_depth: 500,
            search_start: 0,
        }
    }

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
                if pos < self.input.len() {
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
                if pos < self.input.len() && (self.flags.dotall || !self.is_lt(self.input[pos])) {
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
                    if *positive { state.captures = temp_state.captures; }
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
                if pos < self.input.len() {
                    // Try \r\n first, then fall back to just \r
                    if self.input[pos] == '\r' && pos + 1 < self.input.len() && self.input[pos + 1] == '\n' {
                        if self.match_nodes(&nodes[1..], pos + 2, state) {
                            return true;
                        }
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
                if !result { self.flags = old_flags; }
                result
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
                if pos >= self.input.len() { return false; }
                let mut p = pos;
                if self.input[p] == '\r' && p + 1 < self.input.len() && self.input[p + 1] == '\n' {
                    p += 2;
                    return self.match_nodes(&nodes[1..], p, state);
                }
                p += 1;
                while p < self.input.len() && is_combining_mark(self.input[p]) {
                    p += 1;
                }
                if is_regional_indicator(self.input[pos]) {
                    while p < self.input.len() && is_regional_indicator(self.input[p]) {
                        p += 1;
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
                if p >= self.input.len() { return false; }
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
            let saved = state.captures.clone();
            if self.try_match_atom_greedy(atom, min, max, count, rest, pos, state) {
                return true;
            }
            state.captures = saved;
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
                if pos < self.input.len() {
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
                if pos < self.input.len() && (self.flags.dotall || !self.is_lt(self.input[pos])) {
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
                    let mut combined = branch.clone();
                    if let Some(idx) = index {
                        combined.push(Node::GroupEnd { index: *idx, start });
                    }
                    let mut branch_state = state.clone();
                    if self.match_nodes_to_end(&combined, pos, &mut branch_state) {
                        let new_pos = branch_state.match_end;
                        if new_pos > pos {
                            state.captures = branch_state.captures.clone();
                            if self.match_greedy(atom, min, max, count + 1, rest, new_pos, state) {
                                return true;
                            }
                            state.captures = saved.clone();
                        } else if new_pos == pos && count + 1 >= min {
                            state.captures = branch_state.captures.clone();
                            if self.match_nodes(rest, pos, state) {
                                return true;
                            }
                            state.captures = saved.clone();
                        }
                    }
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
                    if cap_len == 0 { return false; }
                    let captured: Vec<char> = self.input[start..end].to_vec();
                    let mut p = pos;
                    for &ch in &captured {
                        if p >= self.input.len() { return false; }
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
                let mut temp_state = state.clone();
                if self.match_nodes(std::slice::from_ref(atom), pos, &mut temp_state) {
                    let new_pos = temp_state.match_end;
                    if new_pos > pos {
                        state.captures = temp_state.captures;
                        self.match_greedy(atom, min, max, count + 1, rest, new_pos, state)
                    } else {
                        // Zero-width match — count as matched up to max, then try rest
                        state.captures = temp_state.captures;
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
            let saved = state.captures.clone();
            if self.match_nodes(rest, pos, state) { return true; }
            state.captures = saved;
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
                if pos < self.input.len() {
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
                if pos < self.input.len() && (self.flags.dotall || !self.is_lt(self.input[pos])) {
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

                    // Use ReluctantCont to enable backtracking within the group
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
                        // Zero-width match — treat as satisfied, try rest
                        state.captures = temp_state.captures;
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
            if self.match_nodes(std::slice::from_ref(atom), current_pos, &mut temp_state) {
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

    fn match_nodes_to_end(&mut self, nodes: &[Node], pos: usize, state: &mut State) -> bool {
        self.match_nodes(nodes, pos, state)
    }

    fn is_lt(&self, c: char) -> bool {
        if self.flags.unix_lines { c == '\n' } else { is_line_terminator(c) }
    }

    fn is_after_line_terminator(&self, pos: usize) -> bool {
        if pos == 0 { return false; }
        let prev = self.input[pos - 1];
        if self.flags.unix_lines { return prev == '\n'; }
        if prev == '\n' {
            true
        } else if prev == '\r' {
            pos >= self.input.len() || self.input[pos] != '\n'
        } else {
            is_line_terminator(prev)
        }
    }

    fn check_anchor(&self, kind: AnchorKind, pos: usize) -> bool {
        match kind {
            AnchorKind::StartOfLine => {
                if self.flags.multiline {
                    if pos == 0 { !self.input.is_empty() }
                    else { pos < self.input.len() && self.is_after_line_terminator(pos) }
                } else {
                    pos == 0
                }
            }
            AnchorKind::EndOfLine => {
                if self.flags.multiline {
                    if pos == self.input.len() { return true; }
                    if pos < self.input.len() && self.is_lt(self.input[pos]) {
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
            AnchorKind::StartOfInput => pos == 0,
            AnchorKind::EndOfInput => pos == self.input.len(),
            AnchorKind::EndOfInputBeforeFinalNewline => {
                if pos == self.input.len() { return true; }
                self.check_before_final_newline(pos)
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
            AnchorKind::PreviousMatchEnd => pos == self.search_start,
        }
    }

    /// Check non-multiline $ and \Z: before final newline or at end.
    fn check_before_final_newline(&self, pos: usize) -> bool {
        let len = self.input.len();
        if self.flags.unix_lines {
            len >= 1 && pos == len - 1 && self.input[pos] == '\n'
        } else {
            // \r\n at end: match before the \r
            if len >= 2 && pos == len - 2 && self.input[pos] == '\r' && self.input[pos + 1] == '\n' {
                return true;
            }
            // Single \n at end — but NOT if preceded by \r (that's part of \r\n, handled above)
            if len >= 1 && pos == len - 1 && self.input[pos] == '\n' {
                return pos == 0 || self.input[pos - 1] != '\r';
            }
            // Single \r at end
            len >= 1 && pos == len - 1 && self.input[pos] == '\r'
        }
    }

    fn check_end_of_line(&self, pos: usize) -> bool {
        if pos == self.input.len() { return true; }
        self.check_before_final_newline(pos)
    }

    fn check_lookbehind(&mut self, inner: &Pattern, pos: usize, state: &mut State) -> bool {
        let rest = [Node::PositionCheck(pos)];
        for start in (0..=pos).rev() {
            let mut temp_state = State::new(self.group_count);
            temp_state.captures = state.captures.clone();
            if self.match_pattern(inner, &rest, start, &mut temp_state) {
                state.captures = temp_state.captures;
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
                    let mut matched = match_unicode_property(name, ch);
                    if !matched && self.flags.case_insensitive {
                        // For Lu/Ll/Lt, case-insensitive matching treats them as LC (cased letter)
                        let name_lower = name.to_lowercase();
                        if matches!(name_lower.as_str(), "lu" | "uppercase_letter" | "ll" | "lowercase_letter" | "lt" | "titlecase_letter") {
                            matched = match_unicode_property("lc", ch);
                        } else if self.flags.unicode_case || name.starts_with("java") {
                            // Unicode case folding for unicode_case mode and java* properties
                            let upper = ch.to_uppercase().next().unwrap_or(ch);
                            let lower = ch.to_lowercase().next().unwrap_or(ch);
                            if upper != ch { matched = match_unicode_property(name, upper); }
                            if !matched && lower != ch { matched = match_unicode_property(name, lower); }
                        } else {
                            // ASCII case folding
                            let upper = ch.to_ascii_uppercase();
                            let lower = ch.to_ascii_lowercase();
                            if upper != ch { matched = match_unicode_property(name, upper); }
                            if !matched && lower != ch { matched = match_unicode_property(name, lower); }
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
        if self.flags.case_insensitive {
            if self.flags.unicode_case {
                if ch >= start && ch <= end { return true; }
                let ch_lower = single_char_lowercase(ch);
                let s_lower = single_char_lowercase(start);
                let e_lower = single_char_lowercase(end);
                if let (Some(cl), Some(sl), Some(el)) = (ch_lower, s_lower, e_lower) {
                    if cl >= sl && cl <= el { return true; }
                }
                let ch_upper = single_char_uppercase(ch);
                let s_upper = single_char_uppercase(start);
                let e_upper = single_char_uppercase(end);
                if let (Some(cu), Some(su), Some(eu)) = (ch_upper, s_upper, e_upper) {
                    if cu >= su && cu <= eu { return true; }
                }
                false
            } else {
                let ch_lower = ch.to_ascii_lowercase();
                let ch_upper = ch.to_ascii_uppercase();
                let s_lower = start.to_ascii_lowercase();
                let e_lower = end.to_ascii_lowercase();
                let s_upper = start.to_ascii_uppercase();
                let e_upper = end.to_ascii_uppercase();
                (ch_lower >= s_lower && ch_lower <= e_lower) ||
                (ch_upper >= s_upper && ch_upper <= e_upper) ||
                (ch >= start && ch <= end)
            }
        } else {
            ch >= start && ch <= end
        }
    }
}
