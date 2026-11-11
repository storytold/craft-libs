//! Word lookup: stem and flag-set probes, affix rule application, compounds and case variants.

use super::*;

/// Does a compound rule's token list match a split of the word into two or more parts? `part(a,
/// b)` is the flags of the stem between char cuts `a` and `b` (`None`: not a stem). States are
/// (cut, token, parts so far capped at 2), each visited once.
fn compound_matches<'a>(rule: &[(Option<u16>, Repeat)], part: &mut impl FnMut(usize, usize) -> Option<&'a [u16]>, n: usize) -> bool {
    let t = rule.len();
    let idx = |cut: usize, tok: usize, k: usize| (cut * (t + 1) + tok) * 3 + k;
    let mut seen = vec![false; n * (t + 1) * 3];
    let mut stack = vec![(0usize, 0usize, 0usize)];
    while let Some((cut, tok, k)) = stack.pop() {
        match seen.get_mut(idx(cut, tok, k)) {
            Some(s) if !*s => *s = true,
            _ => continue,
        }
        if tok == t {
            if cut + 1 == n && k >= 2 {
                return true;
            }
            continue;
        }
        let Some(&(flag, repeat)) = rule.get(tok) else { continue };
        if repeat != Repeat::Once {
            stack.push((cut, tok + 1, k));
        }
        for next in cut + 1..n {
            if let Some(flag) = flag
                && part(cut, next).is_some_and(|set| has(set, flag))
            {
                let k = (k + 1).min(2);
                stack.push((next, if repeat == Repeat::Any { tok } else { tok + 1 }, k));
            }
        }
    }
    false
}

impl AffixDict {
    /// What lookups exclude: forbidden words, and NOSUGGEST and SUBSTANDARD stems too for
    /// suggestions.
    pub(super) fn excluded(&self, suggestible: bool) -> Excluded {
        let e = Excluded::default().and(self.forbidden);
        if !suggestible {
            return e;
        }
        let mut e = e.and(self.no_suggest).and(self.substandard);
        e.2 = true;
        // `MAXCPDSUGS 0`: compounds are right but never suggested.
        if self.cpd.as_ref().is_some_and(|c| c.no_suggestions) { e.without_compounds() } else { e }
    }

    /// Flag set of a stem, if `w` is one.
    pub(super) fn stem_flags(&self, w: &str) -> Option<&[u16]> {
        self.stem_flags_except(w, self.excluded(false))
    }

    /// Flag set of a stem, if `w` is one and carries none of the `excluded` flags. A stem
    /// carrying FORBIDDENWORD (listed, not a word) or ONLYINCOMPOUND (a word only inside a
    /// compound, [`Self::compound`]) is no hit, and (this is the only stem lookup outside
    /// compounds) no affixed form is built on it either.
    fn stem_flags_except(&self, w: &str, excluded: Excluded) -> Option<&[u16]> {
        self.stem_set(w).filter(|set| !excluded.hits(set) && !self.only_in_compound.is_some_and(|f| has(set, f)))
    }

    /// Does one homonym of the stem `w` (excluded ones aside) have flags meeting `pred`?
    fn homonym_meets(&self, w: &str, excluded: Excluded, pred: impl Fn(&[u16]) -> bool) -> bool {
        let Some((at, set)) = self.stem_lookup(w) else { return false };
        let usable = |set: &[u16]| !excluded.hits(set) && !self.only_in_compound.is_some_and(|f| has(set, f));
        if !usable(set) {
            return false;
        }
        match self.homonyms.get(&at) {
            Some(sets) => sets.iter().map(|&h| self.set(h)).any(|h| usable(h) && pred(h)),
            None => pred(set),
        }
    }

    /// Is `w` listed with `FORBIDDENWORD`? Then it is no word, however else it could be built
    /// (affixes, compounds), as in Hunspell.
    pub(super) fn is_forbidden(&self, w: &str) -> bool {
        self.forbidden.is_some_and(|f| self.stem_set(w).is_some_and(|set| has(set, f)))
    }

    /// Flag set of the stem `w`, whatever flags it carries.
    pub(super) fn stem_set(&self, w: &str) -> Option<&[u16]> {
        self.stem_lookup(w).map(|(_, set)| set)
    }

    /// Index and flag set of the stem `w`, whatever flags it carries.
    pub(super) fn stem_lookup(&self, w: &str) -> Option<(u32, &[u16])> {
        if self.table.is_empty() {
            return None;
        }
        let mask = self.table.len() - 1;
        let mut slot = hash_str(w) as usize & mask;
        for _ in 0..self.table.len() {
            let t = *self.table.get(slot)?;
            if t == EMPTY {
                return None;
            }
            if self.word(t as usize) == Some(w) {
                return Some((t, self.set(*self.flags.get(t as usize)?)));
            }
            slot = (slot + 1) & mask;
        }
        None
    }

    /// Do the continuation classes of `r` hold `flag`?
    pub(super) fn cont_holds(&self, r: &Rule, flag: Option<u16>) -> bool {
        flag.is_some_and(|f| has(self.set(r.cont), f))
    }

    /// Can a derived form carry `flag` (does some rule's continuation classes hold it)?
    fn is_cont_flag(&self, flag: u16) -> bool {
        self.cont_flag.get(usize::from(flag)).copied().unwrap_or(false)
    }

    /// Does `w` carry any flag of `flags` — as a stem, or as the form of a suffix rule whose
    /// continuation classes include one (chained suffixation, depth-bounded)?
    ///
    /// The rules of a group that could lead to `w` are merged into one set of their flags (sorted)
    /// and followed once, not once per rule: the answer is the same, but thousands of rules of one
    /// flag cost one walk, not one each (#3613).
    ///
    /// With `pfx` set (a prefix was taken off: the flags of the prefixes that could be it), every
    /// rule on the way must allow cross products and one prefix's flag must come from the stem or
    /// a continuation class on the way (`meg` + `épít` + `és` + `e`).
    fn form_has_flag(&self, w: &str, flags: &[u16], depth: u8, pfx: Option<&FlagProbe>, excluded: Excluded, steps: &Steps) -> bool {
        let fits = |f: &[u16]| steps.scan(f.len()) && intersects(f, flags) && pfx.is_none_or(|p| p.meets(f));
        if if pfx.is_some() { self.homonym_meets(w, excluded, fits) } else { self.stem_flags_except(w, excluded).is_some_and(fits) } {
            return true;
        }
        // Without continuation classes no rule leads from a form to another.
        if depth == 0 || !self.has_cont {
            return false;
        }
        let mut root = String::with_capacity(w.len() + 8);
        let mut inner: Vec<u16> = Vec::new();
        // Rules whose continuation classes give the prefix its flag: under them it is met.
        let mut met: Vec<u16> = Vec::new();
        // Does a continuation set hold one of `flags`? Answered once per distinct set (rules
        // share interned sets) and, for many flags, through a bitmap: a lookup costs at most the
        // total size of the distinct sets it meets, not rules × set size.
        if !steps.scan(flags.len()) {
            return false;
        }
        let probe = FlagProbe::new(flags);
        let mut seen: FxMap<u32, bool> = FxMap::default();
        let cuts = std::iter::once(w.len()).chain(w.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0 || self.full_strip));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (w.get(..b), w.get(b..)) else { continue };
            // Each split is a lookup of its appended text.
            if !steps.scan(add.len()) {
                return false;
            }
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups.iter().filter(|g| !g.cont_rules.is_empty()) {
                if !steps.spend(g.cont_rules.len() + GROUP_STEPS) {
                    return false;
                }
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                inner.clear();
                met.clear();
                for r in g.cont_rules.iter().filter_map(|&i| self.suffixes.get(i as usize)) {
                    let cont = self.set(r.cont);
                    let meets = *seen.entry(r.cont).or_insert_with(|| steps.scan(cont.len()) && probe.meets(cont));
                    if meets
                        && (pfx.is_none() || r.cross)
                        && !self.cont_holds(r, self.circumfix)
                        && !self.cont_holds(r, self.only_in_compound)
                        && steps.cond(r)
                        && r.cond_matches(&root, true)
                    {
                        if pfx.is_some_and(|p| steps.scan(cont.len()) && p.meets(cont)) { met.push(r.flag) } else { inner.push(r.flag) }
                    }
                }
                for (list, p) in [(&mut inner, pfx), (&mut met, None)] {
                    list.sort_unstable();
                    list.dedup();
                    if !list.is_empty() && self.form_has_flag(&root, list, depth - 1, p, excluded, steps) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Is `w` exactly (case and all) a stem or an affixed form of one? `ICONV` and `IGNORE`
    /// apply first.
    pub fn contains(&self, w: &str) -> bool {
        self.contains_except(&self.normalize(w), self.excluded(false))
    }

    /// Exactly spelled dictionary words eligible for suggestions (NOSUGGEST and SUBSTANDARD
    /// stems excluded).
    pub fn contains_suggestible(&self, w: &str) -> bool {
        self.contains_except(&self.normalize(w), self.excluded(true))
    }

    /// [`AffixDict::contains_suggestible`], without flag-based compounds when `compounds` is
    /// false (suggestion candidates past the compound check budget).
    pub(crate) fn contains_suggestible_with(&self, w: &str, compounds: bool) -> bool {
        let e = self.excluded(true);
        self.contains_except(&self.normalize(w), if compounds { e } else { e.without_compounds() })
    }

    /// Suggestions for `word`, best first, at most `max`, from the dictionary's own tables (as
    /// for a Language pack).
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        crate::affix_suggest::suggest_in(self, word, max, &crate::affix_suggest::Tuning::DEFAULT)
    }

    /// Is `w`, known to be no suggestible simple word or `COMPOUNDRULE` compound, a flag-based
    /// compound? The search spends its steps from `budget` (at most
    /// [`compound::MAX_SUGGEST_WORK`] for this word); the word isn't looked up again.
    pub(crate) fn flag_compound_spending_suggestible(&self, w: &str, budget: &mut usize, pieces: &mut compound::Pieces) -> bool {
        let w = self.normalize(w);
        let e = self.excluded(true);
        let allowed = (*budget).min(compound::MAX_SUGGEST_WORK);
        let mut left = allowed;
        let found = self.flag_compound_spending(&w, e, matches!(casing(&w), Casing::Initial | Casing::Upper), &mut left, Some(pieces));
        *budget = budget.saturating_sub(allowed - left);
        found
    }

    /// Whether the dictionary compounds by flags (suggestion candidates then cost more).
    pub(crate) fn flag_compounds(&self) -> bool {
        self.cpd.is_some()
    }

    /// `w` after the `ICONV` conversions (at each position the longest match wins) and without
    /// the `IGNORE` characters. Input longer than [`MAX_INPUT_BYTES`] is no word and comes back
    /// unchanged, so the length check after this rejects it without converting it.
    pub(super) fn normalize<'a>(&self, w: &'a str) -> std::borrow::Cow<'a, str> {
        if w.len() > MAX_INPUT_BYTES {
            return std::borrow::Cow::Borrowed(w);
        }
        self.iconv.apply(w, &self.ignore)
    }

    /// A suggestion as it is shown: `w` after the `OCONV` conversions (the French dictionary
    /// writes `’` for the `'` its stems use).
    pub fn output<'a>(&self, w: &'a str) -> std::borrow::Cow<'a, str> {
        self.oconv.apply(w, &[])
    }

    /// [`AffixDict::contains`] on normalized input, where stems carrying an `excluded` flag
    /// don't count (forbidden words; case variants of `KEEPCASE` stems).
    fn contains_except(&self, w: &str, excluded: Excluded) -> bool {
        self.contains_in(w, excluded, matches!(casing(w), Casing::Initial | Casing::Upper))
    }

    /// [`AffixDict::contains_except`], where `capitalized` says whether the word as written
    /// starts with a capital (`w` may be its lower-case form): a compound whose last part has
    /// `FORCEUCASE` is right only then.
    fn contains_in(&self, w: &str, excluded: Excluded, capitalized: bool) -> bool {
        self.simple_word(w, excluded) || self.compound(w, excluded) || self.flag_compound(w, excluded, capitalized)
    }

    /// Is `w` a stem or an affixed form of one (no compound)? Affixes that are only valid inside
    /// a compound (`ONLYINCOMPOUND`) don't count.
    pub(super) fn simple_word(&self, w: &str, excluded: Excluded) -> bool {
        self.simple_word_with(w, excluded, true)
    }

    /// [`AffixDict::simple_word`], with prefixes only when `prefixes` is true.
    fn simple_word_with(&self, w: &str, excluded: Excluded, prefixes: bool) -> bool {
        self.simple_word_budgeted(w, excluded, prefixes).unwrap_or(false)
    }

    /// [`AffixDict::simple_word_with`], or `None` when its steps ran out before an answer.
    pub(super) fn simple_word_budgeted(&self, w: &str, excluded: Excluded, prefixes: bool) -> Option<bool> {
        if w.is_empty() || w.len() > MAX_WORD_BYTES || self.is_forbidden(w) {
            return Some(false);
        }
        // The steps taken for this word, all ways together: past MAX_SIMPLE_WORK it is
        // taken for no word (hostile rules can't make one word cost seconds). Inside a
        // suggestion call the word gets at most MAX_SUGGEST_SIMPLE_WORK of the call's budget.
        let shared = SUGGEST_STEPS.with(std::cell::Cell::get);
        let cap = shared.map_or(MAX_SIMPLE_WORK, |left| left.min(MAX_SUGGEST_SIMPLE_WORK));
        let steps = Steps(std::cell::Cell::new(cap));
        let found = self.simple_word_in(w, excluded, prefixes, &steps);
        if let Some(left) = shared {
            let spent = cap.saturating_sub(steps.0.get()).max(1);
            SUGGEST_STEPS.with(|s| s.set(Some(left.saturating_sub(spent))));
        }
        (found || steps.0.get() > 0).then_some(found)
    }

    /// [`AffixDict::simple_word_with`] in the steps `steps` allows.
    fn simple_word_in(&self, w: &str, excluded: Excluded, prefixes: bool, steps: &Steps) -> bool {
        // A stem that needs an affix is no word on its own.
        if self.stem_flags_except(w, excluded.and(self.need_affix)).is_some() || self.suffixed(w, None, false, false, excluded, steps) {
            return true;
        }
        if !prefixes {
            return false;
        }
        // The prefix rules whose appended text starts `w`, found by that text (one hash lookup per
        // length) rather than by trying every rule.
        let ends = std::iter::once(0).chain(w.char_indices().map(|(i, c)| i + c.len_utf8()));
        let rules = ends
            .take(self.max_pfx_chars + 1)
            .filter_map(|b| self.pfx_index.get(w.get(..b)?))
            .flatten()
            .filter_map(|&i| self.prefixes.get(i as usize));
        // What each rule leaves: (root, circumfix, needs another affix, cross, flag). The rules
        // that leave the same root alike are answered together, in one walk each.
        let mut taken: Vec<(String, bool, bool, bool, u16)> = Vec::new();
        for r in rules {
            if !steps.spend(1 + r.cond.len()) {
                return false;
            }
            let Some(rest) = w.strip_prefix(&*r.add) else { continue };
            if (rest.is_empty() && !self.full_strip) || self.cont_holds(r, self.only_in_compound) {
                continue;
            }
            let root = format!("{}{rest}", r.strip);
            if r.cond_matches(&root, false) {
                // A circumfix prefix works only with its suffix, and one that needs another
                // affix only with a suffix too.
                taken.push((root, self.cont_holds(r, self.circumfix), self.cont_holds(r, self.need_affix), r.cross, r.flag));
            }
        }
        taken.sort_unstable();
        let mut plain: Vec<u16> = Vec::new();
        let mut crossed: Vec<u16> = Vec::new();
        for group in taken.chunk_by(|a, b| (&a.0, a.1, a.2) == (&b.0, b.1, b.2)) {
            if !steps.spend(group.len() + GROUP_STEPS) {
                return false;
            }
            let Some((root, circ, needs, ..)) = group.first() else { continue };
            plain.clear();
            crossed.clear();
            for &(.., cross, flag) in group {
                if !circ && !needs {
                    plain.push(flag);
                }
                if cross {
                    crossed.push(flag);
                }
            }
            plain.sort_unstable();
            plain.dedup();
            crossed.sort_unstable();
            crossed.dedup();
            if !plain.is_empty() {
                if self.stem_flags_except(root, excluded).is_some_and(|f| steps.scan(f.len()) && intersects(f, &plain)) {
                    return true;
                }
                plain.retain(|&f| self.is_cont_flag(f));
                if !plain.is_empty() && self.form_has_flag(root, &plain, INNER_CHAIN, None, excluded, steps) {
                    return true;
                }
            }
            if !crossed.is_empty() && self.suffixed(root, Some(&FlagProbe::new(&crossed)), *circ, *needs, excluded, steps) {
                return true;
            }
        }
        false
    }

    /// Is `w` a compound of two or more stems (no affixes) whose flags spell out one of the
    /// `COMPOUNDRULE` patterns, each part at least `COMPOUNDMIN` characters? `21st` is `2/m` +
    /// `1st/p` under `n*mp`. A dynamic programme over (byte offset, token, parts so far), so a
    /// hostile pattern costs at most a few thousand lookups. A forbidden word is never rebuilt
    /// from its parts.
    fn compound(&self, w: &str, excluded: Excluded) -> bool {
        if self.compound_rules.is_empty()
            || w.len() > MAX_COMPOUND_BYTES
            || !w.chars().next().is_some_and(|c| self.compound_firsts.contains(&c))
            || self.is_forbidden(w)
        {
            return false;
        }
        // FORBIDDENWORD and NEEDAFFIX stems are no compound part (Hunspell rejects a compound
        // with a NEEDAFFIX part), checked here so the `excluded` slots can't run out.
        let bare = |set: &[u16]| [self.forbidden, self.need_affix].into_iter().flatten().all(|f| !has(set, f));
        // Char boundaries of `w`, `0` and `w.len()` included.
        let cuts: Vec<usize> = w.char_indices().map(|(i, _)| i).chain(std::iter::once(w.len())).collect();
        let n = cuts.len();
        // Flag set of each part `w[cuts[a]..cuts[b]]`, by (a, b), looked up when a rule first
        // reaches cut `a` (`None`: not looked up yet; `Some(None)`: not a stem). Only the parts
        // after a cut a rule reaches are looked up, so a word with no first part costs `n`.
        let mut parts: Vec<Option<Option<&[u16]>>> = vec![None; n * n];
        let min = self.compound_min;
        let mut part = |a: usize, b: usize| -> Option<&[u16]> {
            if b < a.saturating_add(min) {
                return None;
            }
            let slot = parts.get_mut(a * n + b)?;
            *slot.get_or_insert_with(|| {
                let (Some(&x), Some(&y)) = (cuts.get(a), cuts.get(b)) else { return None };
                w.get(x..y)
                    .and_then(|p| self.stem_lookup(p))
                    .map(|(stem, set)| self.rule_sets.get(&stem).map_or(set, |&s| self.set(s)))
                    .filter(|set| !excluded.hits(set) && bare(set))
            })
        };
        self.compound_rules.iter().any(|rule| compound_matches(rule, &mut part, n))
    }

    /// Is `w` a stem with one suffix? With `need` set (a prefix was taken off), the prefix's flag
    /// must come from the stem or from the suffix's continuation classes (`leg` + `nagy` + `obb/A`)
    /// and the suffix must allow cross products. `circ` says whether that prefix is a circumfix
    /// one: the suffix must be one too, and otherwise must not be. `pfx_needs`: the prefix needs
    /// another affix (NEEDAFFIX), so the suffix may not need one too (Hunspell rejects `reworks`
    /// from `re/N` and `s/N`).
    /// `need` holds the flags of all the prefixes that could have been taken off (one walk for
    /// them all: a thousand prefix rules adding nothing cost one walk, not a thousand).
    fn suffixed(&self, w: &str, need: Option<&FlagProbe>, circ: bool, pfx_needs: bool, excluded: Excluded, steps: &Steps) -> bool {
        // Fits the suffix rule `r`: circumfix alike, and in need of another affix only when a
        // prefix that doesn't is there.
        let fits = |r: &Rule| {
            self.cont_holds(r, self.circumfix) == circ
                && (!self.cont_holds(r, self.need_affix) || (need.is_some() && !pfx_needs))
                && !self.cont_holds(r, self.only_in_compound)
        };
        let mut root = String::with_capacity(w.len() + 8);
        let mut flags: Vec<u16> = Vec::new();
        let mut met: Vec<u16> = Vec::new();
        // Each split point from the end: the appended string is `w[b..]`, at most the longest
        // suffix, and something must stay before it.
        let cuts = std::iter::once(w.len()).chain(w.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0 || self.full_strip));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (w.get(..b), w.get(b..)) else { continue };
            // Each split is a lookup of its appended text.
            if !steps.scan(add.len()) {
                return false;
            }
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups {
                if !steps.spend(g.rules.len() + GROUP_STEPS) {
                    return false;
                }
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                let rules = || g.rules.iter().filter_map(|&i| self.suffixes.get(i as usize));
                // The stem path: `root` is a stem carrying the rule's flag (and the prefix's,
                // when a cross product is being checked).
                let takes = |f: &[u16]| {
                    steps.spend(g.rules.len())
                        && rules().any(|r| {
                            has(f, r.flag)
                                && need.is_none_or(|n| {
                                    let cont = self.set(r.cont);
                                    steps.scan(f.len() + cont.len()) && (n.meets(f) || n.meets(cont))
                                })
                                && (need.is_none() || r.cross)
                                && fits(r)
                                && steps.cond(r)
                                && r.cond_matches(&root, true)
                        })
                };
                if if need.is_some() {
                    self.homonym_meets(&root, excluded, takes)
                } else {
                    self.stem_flags_except(&root, excluded).is_some_and(takes)
                } {
                    return true;
                }
                // Continuation classes: `root` need not be a stem carrying this rule's flag —
                // it may be a form that gained the flag from another rule's continuation. The
                // matching rules' flags are looked for together, in one walk. Without continuation
                // classes that walk would only repeat the stem path above.
                if self.has_cont && !circ {
                    flags.clear();
                    met.clear();
                    // Only a flag some continuation class holds can come from another rule. The
                    // outer suffix of a chain has another affix under it, so its NEEDAFFIX is met,
                    // and Hunspell ignores its CIRCUMFIX there (`workables` from `s/c`).
                    // After a prefix every rule must allow cross products, and a rule whose
                    // continuation classes hold the prefix's flag meets it for the whole chain.
                    for r in rules().filter(|r| {
                        self.is_cont_flag(r.flag)
                            && (need.is_none() || r.cross)
                            && !self.cont_holds(r, self.only_in_compound)
                            && steps.cond(r)
                            && r.cond_matches(&root, true)
                    }) {
                        let cont = self.set(r.cont);
                        if need.is_some_and(|n| steps.scan(cont.len()) && n.meets(cont)) { met.push(r.flag) } else { flags.push(r.flag) }
                    }
                    for (list, p) in [(&mut flags, need), (&mut met, None)] {
                        list.sort_unstable();
                        list.dedup();
                        if !list.is_empty() && self.form_has_flag(&root, list, INNER_CHAIN, p, excluded, steps) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Is `w` correct, Hunspell style: as written, or — for a capitalized word or one in capitals
    /// — in lower case (and capitalized), since a sentence may start with any word and headings
    /// may be in capitals. Lower-case input never matches a capitalized stem (`warszawa`), and a
    /// `KEEPCASE` stem matches only as written.
    pub fn check(&self, w: &str) -> bool {
        self.check_except(w, self.excluded(false))
    }

    /// Whether a suggestion is correct in its displayed case and has an eligible stem.
    pub fn can_suggest(&self, w: &str) -> bool {
        self.check_except(w, self.excluded(true))
    }

    /// Whether `w` is a word only through a `KEEPCASE` stem, so it is correct in this letter case
    /// alone (`km` from `km/K`, which `Km` and `KM` are not).
    pub(crate) fn case_locked(&self, w: &str) -> bool {
        self.keepcase.is_some() && self.contains(w) && !self.contains_except(&self.normalize(w), self.excluded(false).keeping(self.keepcase))
    }

    fn check_except(&self, w: &str, excluded: Excluded) -> bool {
        let w = self.normalize(w);
        if self.contains_except(&w, excluded) {
            return true;
        }
        // Over this no case variant can be a word either.
        if w.len() > MAX_INPUT_BYTES {
            return false;
        }
        let keep = excluded.keeping(self.keepcase);
        match casing(&w) {
            Casing::Lower => false,
            // A compound may join a capitalized stem to others, and the word start with a
            // capital (`FooQux` from `foo` and `Qux`: right in the CLI).
            Casing::Mixed => w.chars().next().is_some_and(char::is_uppercase) && self.flag_compound(&decapitalize(&w), keep, true),
            Casing::Initial => {
                let lower = w.to_lowercase();
                // CHECKSHARPS: a KEEPCASE word with `ß` may be capitalized (`Gruß` for `Gruß/k`).
                let ex = if self.check_sharps && lower.contains('ß') { excluded } else { keep };
                self.contains_in(&lower, ex, true)
            }
            Casing::Upper => {
                let lower = w.to_lowercase();
                self.contains_except(&capitalize(&lower), keep)
                    || self.contains_in(&lower, keep, true)
                    || self.mixed_case(&lower, keep)
                    || (self.check_sharps && self.sharps(&lower, excluded))
            }
        }
    }

    /// Is `lower` (a word in capitals, lowercased) a stem in mixed case, or one with an ending
    /// (`kbs` from `kB/S`)? Not a compound of one (`JQUERYKABEL` is wrong in the CLI).
    fn mixed_case(&self, lower: &str, excluded: Excluded) -> bool {
        if self.mixed.is_empty() {
            return false;
        }
        let ends = lower.char_indices().map(|(i, c)| i + c.len_utf8());
        // At most MAX_HOMONYMS stems in all are tried, whatever the word's length.
        ends.filter_map(|b| Some((self.mixed.get(lower.get(..b)?)?, lower.get(b..)?)))
            .flat_map(|(stems, rest)| stems.iter().filter_map(|&i| self.word(i as usize)).map(move |m| (m, rest)))
            .take(MAX_HOMONYMS)
            .any(|(m, rest)| self.simple_word(&format!("{m}{rest}"), excluded))
    }

    /// CHECKSHARPS: is `lower` (a word in capitals, lowercased) a word with some of its `ss` read
    /// as `ß`, in lower case or capitalized? Such a form may come from a `KEEPCASE` stem (in
    /// capitals a word with `ß` is written with `SS`). The first [`MAX_SHARPS`] `ss` pairs are
    /// tried, in every combination.
    fn sharps(&self, lower: &str, excluded: Excluded) -> bool {
        let at: Vec<usize> = lower.match_indices("ss").map(|(i, _)| i).take(MAX_SHARPS).collect();
        let mut variant = String::with_capacity(lower.len());
        for pick in 1u32..(1 << at.len()) {
            variant.clear();
            let mut from = 0;
            for (k, &i) in at.iter().enumerate() {
                if pick & (1 << k) != 0 {
                    variant.push_str(lower.get(from..i).unwrap_or_default());
                    variant.push('ß');
                    from = i + 2;
                }
            }
            variant.push_str(lower.get(from..).unwrap_or_default());
            if self.contains_except(&variant, excluded) || self.contains_except(&capitalize(&variant), excluded) {
                return true;
            }
        }
        false
    }
}
