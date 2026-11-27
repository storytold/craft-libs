//! The compound search: analyses of the pieces of a word and the checks on the parts chosen.

use super::{AffixDict, Compounding, Excluded, MAX_PARTS, MAX_SUGGEST_WORK, MAX_WORK, Rule, has};

/// Most ways one piece of a word may be a part (distinct flags and places).
const MAX_INFOS: usize = 256;

/// The part may come first, in the middle, last.
const FIRST: u8 = 1;

const MIDDLE: u8 = 2;

const LAST: u8 = 4;

/// One way a piece of the word can be a compound part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Info {
    /// Where it may sit: [`FIRST`], [`MIDDLE`], [`LAST`].
    pos: u8,
    /// Which of [`Compounding::marks`] its flags hold.
    marks: u64,
    /// No suffix (`CHECKCOMPOUNDPATTERN`'s `0`).
    unmodified: bool,
    /// The stem carries `FORCEUCASE`.
    force: bool,
    /// The stem's index (`CHECKCOMPOUNDDUP` compares stems).
    stem: u32,
    /// How many words it counts as (`COMPOUNDROOT`: two).
    words: u8,
    /// Syllables of its suffix the Hungarian rule leaves out (as the last part): a suffix with no
    /// continuation classes doesn't count.
    tail: u8,
}

/// A stem found for a part, and the affixes that make the part from it.
#[derive(Clone, Copy)]
pub(super) struct Analysis<'a> {
    stem: u32,
    set: &'a [u16],
    pfx: Option<&'a Rule>,
    sfx: Option<&'a Rule>,
    /// The inner suffix of two in a row (`sfx` is the outer one).
    inner: Option<&'a Rule>,
}

/// Most pieces a [`Pieces`] cache keeps.
const MAX_CACHED_PIECES: usize = 1 << 14;

/// The infos of pieces, kept across the compound checks of one suggestion call: its candidates
/// differ from the word by a letter or two, so most of their pieces are the same. Indexed by
/// whether the piece starts the word (KEEPCASE holds there).
#[derive(Default)]
pub(crate) struct Pieces([crate::patterns::FxMap<Box<str>, Box<[Info]>>; 2], usize);

/// A terminal piece's bare rule flags and suffix analyses, shared across requested rule flags.
#[derive(Clone)]
pub(in crate::affix) struct TerminalFlags<'a> {
    bare: Option<&'a [u16]>,
    derived: Option<Vec<&'a [u16]>>,
}

/// One compound check: the word, its parts' analyses (each piece looked up once), the parts
/// chosen so far and the work done.
pub(super) struct Search<'a> {
    pub(super) c: &'a Compounding,
    pub(super) w: &'a str,
    /// Byte offset of each char, and `w.len()`.
    pub(super) cuts: Vec<usize>,
    pub(super) chars: Vec<char>,
    pub(super) excluded: Excluded,
    /// The first part's (KEEPCASE holds there).
    pub(super) first_excluded: Excluded,
    pub(super) capitalized: bool,
    /// Per piece `(start, end)` in chars: its infos as a range of `infos`.
    pub(super) memo: Vec<Option<(u32, u32)>>,
    pub(super) infos: Vec<Info>,
    /// The parts so far: start and end char, and the info.
    pub(super) path: Vec<(usize, usize, Info)>,
    pub(super) work: usize,
    /// [`AffixDict::near_word`], once answered.
    pub(super) near: Option<bool>,
    /// Pieces analysed by earlier checks with the same exclusions.
    pub(super) cache: Option<&'a mut Pieces>,
}

impl AffixDict {
    /// Is `w` a compound by the flag-based rules (see the module docs)? `capitalized`: the word
    /// as the user wrote it starts with a capital (`FORCEUCASE`).
    pub(in crate::affix) fn flag_compound(&self, w: &str, excluded: Excluded, capitalized: bool) -> bool {
        // A suggestion candidate's search gets MAX_SUGGEST_WORK steps.
        let mut budget = if excluded.suggesting() { MAX_SUGGEST_WORK } else { MAX_WORK };
        self.flag_compound_spending(w, excluded, capitalized, &mut budget, None)
    }

    /// [`AffixDict::flag_compound`] in at most `budget` steps (and [`MAX_WORK`]), taking the
    /// steps it spends off `budget`. `cache`: pieces analysed before, with the same `excluded`.
    pub(in crate::affix) fn flag_compound_spending(
        &self,
        w: &str,
        excluded: Excluded,
        capitalized: bool,
        budget: &mut usize,
        cache: Option<&mut Pieces>,
    ) -> bool {
        let Some(c) = self.cpd.as_deref() else { return false };
        if excluded.no_compounds() || w.is_empty() || w.len() > super::super::MAX_WORD_BYTES || *budget == 0 {
            return false;
        }
        if self.is_forbidden(w) {
            return false;
        }
        // KEEPCASE holds for the first part only (from `baz/K`, `Foobaz` and `FOOBAZ` are right
        // in the CLI, `Bazfoo` and `BAZFOO` wrong).
        let first_excluded = excluded;
        let excluded = excluded.without_keep();
        let cuts: Vec<usize> = w.char_indices().map(|(i, _)| i).chain(std::iter::once(w.len())).collect();
        let chars: Vec<char> = w.chars().collect();
        let n = cuts.len();
        let mut s = Search {
            c,
            w,
            cuts,
            chars,
            excluded,
            first_excluded,
            capitalized,
            memo: vec![None; n * n],
            infos: Vec::new(),
            path: Vec::with_capacity(MAX_PARTS),
            work: MAX_WORK - (*budget).min(MAX_WORK),
            near: None,
            cache,
        };
        let start = s.work;
        let found = self.parts_from(&mut s, 0);
        *budget = budget.saturating_sub(s.work.saturating_sub(start).max(1));
        found
    }

    /// Where a slip in `w` (no compound) likely is, in chars: from the furthest point its start
    /// reaches as compound parts to the nearest its end does, if anything reaches. A candidate
    /// changed there is checked as a compound first. `capitalized`: the same with a capital
    /// first letter counts too.
    pub(crate) fn compound_gap(&self, w: &str, capitalized: bool, budget: &mut usize, pieces: &mut Pieces) -> Option<(usize, usize)> {
        let c = self.cpd.as_deref()?;
        if w.is_empty() || w.len() > super::super::MAX_WORD_BYTES || *budget == 0 {
            return None;
        }
        let excluded = self.excluded(true);
        let n = w.chars().count();
        let min = self.compound_min.max(1);
        let mut reach = vec![false; n + 1];
        let mut back = vec![false; n + 1];
        let cap = capitalized.then(|| super::super::capitalize(w));
        for word in std::iter::once(w).chain(cap.as_deref()) {
            let mut s = Search {
                c,
                w: word,
                cuts: word.char_indices().map(|(i, _)| i).chain(std::iter::once(word.len())).collect(),
                chars: word.chars().collect(),
                excluded: excluded.without_keep(),
                first_excluded: excluded,
                capitalized,
                memo: vec![None; (n + 1) * (n + 1)],
                infos: Vec::new(),
                path: Vec::new(),
                work: MAX_WORK - (*budget).min(MAX_WORK),
                near: None,
                cache: Some(&mut *pieces),
            };
            let start = s.work;
            let fits = |s: &mut Search, a: usize, b: usize, place: u8| {
                let (x, y) = self.infos_of(s, a, b);
                s.infos.get(x as usize..y as usize).is_some_and(|infos| infos.iter().any(|i| i.pos & place != 0))
            };
            for a in 0..n {
                if a > 0 && !reach.get(a).copied().unwrap_or(false) {
                    continue;
                }
                for b in a.saturating_add(min)..n {
                    if s.work <= MAX_WORK
                        && fits(&mut s, a, b, if a == 0 { FIRST } else { MIDDLE })
                        && let Some(r) = reach.get_mut(b)
                    {
                        *r = true;
                    }
                }
            }
            for b in (1..=n).rev() {
                if b < n && !back.get(b).copied().unwrap_or(false) {
                    continue;
                }
                for a in 1..=b.saturating_sub(min) {
                    if s.work <= MAX_WORK
                        && fits(&mut s, a, b, if b == n { LAST } else { MIDDLE })
                        && let Some(r) = back.get_mut(a)
                    {
                        *r = true;
                    }
                }
            }
            *budget = budget.saturating_sub(s.work.saturating_sub(start));
        }
        let from = reach.iter().rposition(|&r| r).unwrap_or(0);
        let to = back.iter().position(|&r| r).unwrap_or(n);
        Some((from.min(to), from.max(to)))
    }

    /// Can the rest of the word, from char `start`, be split into parts after `s.path`?
    fn parts_from(&self, s: &mut Search, start: usize) -> bool {
        let total = s.chars.len();
        let count = s.path.len();
        let min = self.compound_min.max(1);
        for end in start.saturating_add(min)..=total {
            s.work += 1;
            if s.work > MAX_WORK {
                return false;
            }
            let last = end == total;
            if !last && count + 1 >= MAX_PARTS {
                break;
            }
            let place = if last {
                LAST
            } else if count == 0 {
                FIRST
            } else {
                MIDDLE
            };
            let (a, b) = self.infos_of(s, start, end);
            for k in a..b {
                // Each way of the piece costs a step, and a step per pattern at the boundary.
                s.work += 1 + s.c.patterns.len();
                if s.work > MAX_WORK {
                    return false;
                }
                let Some(info) = s.infos.get(k as usize).copied() else { continue };
                if info.pos & place == 0 || !self.boundary_ok(s, start, &info) {
                    continue;
                }
                s.path.push((start, end, info));
                let ok = if last { self.whole_ok(s) } else { self.words_ok(s, false) && self.next_parts(s, end) };
                s.path.pop();
                if ok {
                    return true;
                }
            }
        }
        false
    }

    /// After a part ending at char `end`: the next part starts there, or (`SIMPLIFIEDTRIPLE`)
    /// one char earlier when the part ends in a double letter.
    fn next_parts(&self, s: &mut Search, end: usize) -> bool {
        if self.parts_from(s, end) {
            return true;
        }
        let double = end >= 2 && s.chars.get(end - 1).is_some_and(|c| s.chars.get(end - 2) == Some(c));
        s.c.simplified && double && self.parts_from(s, end - 1)
    }

    /// The analyses of the piece from char `a` to char `b`, computed once: a range of `s.infos`.
    fn infos_of(&self, s: &mut Search, a: usize, b: usize) -> (u32, u32) {
        let n = s.cuts.len();
        let slot = a.saturating_mul(n).saturating_add(b);
        if let Some(Some(r)) = s.memo.get(slot) {
            return *r;
        }
        let from = u32::try_from(s.infos.len()).unwrap_or(u32::MAX);
        let cached = s.cuts.get(a).zip(s.cuts.get(b)).and_then(|(&x, &y)| s.w.get(x..y)).and_then(|piece| {
            let map = s.cache.as_deref()?.0.get(usize::from(a == 0))?;
            map.get(piece).map(|v| v.to_vec())
        });
        if let Some(v) = cached {
            s.work += 1;
            s.infos.extend(v);
        } else if let (Some(&x), Some(&y)) = (s.cuts.get(a), s.cuts.get(b))
            && let Some(piece) = s.w.get(x..y)
        {
            // Deduplicated by hashing, and at most MAX_INFOS per piece (each one is tried at
            // every split, against every pattern).
            let mut seen: std::collections::HashSet<Info, std::hash::BuildHasherDefault<crate::patterns::FxHasher>> = Default::default();
            let mut found: Vec<Info> = Vec::new();
            let mut work = s.work;
            let excluded = if a == 0 { s.first_excluded } else { s.excluded };
            self.analyses(piece, excluded, &mut work, &mut |an: Analysis| {
                if found.len() < MAX_INFOS
                    && let Some(info) = self.info(s.c, &an)
                    && seen.insert(info)
                {
                    found.push(info);
                }
            });
            s.work = work;
            // Only a piece analysed in full is kept (one cut short by the budget isn't).
            if work <= MAX_WORK
                && let Some(c) = s.cache.as_deref_mut()
                && c.1 < MAX_CACHED_PIECES
                && let Some(map) = c.0.get_mut(usize::from(a == 0))
            {
                map.insert(piece.into(), found.clone().into());
                c.1 += 1;
            }
            s.infos.extend(found);
        }
        let to = u32::try_from(s.infos.len()).unwrap_or(u32::MAX);
        if let Some(m) = s.memo.get_mut(slot) {
            *m = Some((from, to));
        }
        (from, to)
    }

    /// Where the analysis `an` lets its piece sit in a compound, if anywhere.
    fn info(&self, c: &Compounding, an: &Analysis) -> Option<Info> {
        let sets = [Some(an.set), an.pfx.map(|r| self.set(r.cont)), an.sfx.map(|r| self.set(r.cont)), an.inner.map(|r| self.set(r.cont))];
        let carries = |f: Option<u16>| f.is_some_and(|f| sets.iter().flatten().any(|s| has(s, f)));
        // COMPOUNDFORBIDFLAG on a prefix: no part anywhere (`bar|defoo` is wrong in the CLI).
        if an.pfx.is_some_and(|r| self.cont_holds(r, c.forbid)) {
            return None;
        }
        let mut pos = 0;
        if carries(c.flag) {
            pos = FIRST | MIDDLE | LAST;
        }
        for (flag, p) in [(c.begin, FIRST), (c.middle, MIDDLE), (c.end, LAST)] {
            if carries(flag) {
                pos |= p;
            }
        }
        // A prefix without COMPOUNDPERMITFLAG: any part but the last (`foo|unfoo|bar` is right
        // in the CLI, `foo|unfoo` wrong).
        if an.pfx.is_some_and(|r| !self.cont_holds(r, c.permit)) {
            pos &= !LAST;
        }
        if let Some(r) = an.sfx {
            if !self.cont_holds(r, c.permit) {
                pos &= LAST;
            }
            if self.cont_holds(r, self.only_in_compound) {
                pos &= !LAST;
            }
        }
        if an.inner.is_some() {
            pos &= LAST;
        }
        if carries(c.forbid) {
            pos &= LAST;
        }
        if pos == 0 {
            return None;
        }
        let mut marks = 0u64;
        for (i, &m) in c.marks.iter().enumerate().take(64) {
            if carries(Some(m)) {
                marks |= 1 << i;
            }
        }
        // COMPOUNDROOT and FORCEUCASE count on the stem only (an affix's continuation class
        // holding them is ignored);
        // a pattern's `0` means no suffix (a prefix doesn't count): the CLI's answers.
        let words = if c.root.is_some_and(|f| has(an.set, f)) { 2 } else { 1 };
        let force = c.force_ucase.is_some_and(|f| has(an.set, f));
        // The Hungarian syllable rule leaves out a suffix with no continuation classes (the
        // CLI's answers: `bab|bab|baata` from `ata` is right, from `ata/Z` wrong).
        let tail = match (&c.syllables, an.sfx) {
            (Some((_, vowels)), Some(r)) if self.set(r.cont).is_empty() => r.add.chars().filter(|ch| vowels.contains(ch)).count(),
            _ => 0,
        };
        let tail = u8::try_from(tail).unwrap_or(u8::MAX);
        Some(Info { pos, marks, unmodified: an.sfx.is_none(), force, stem: an.stem, words, tail })
    }

    /// Query one terminal flag, retaining bare flags and computing suffix analyses at most once per piece.
    pub(in crate::affix) fn rule_final_has_flag<'a>(
        &'a self,
        part: &str,
        excluded: Excluded,
        flag: u16,
        work: &mut usize,
        cache: &mut Option<TerminalFlags<'a>>,
    ) -> bool {
        let excluded = excluded.without_keep();
        let terminal = cache.get_or_insert_with(|| {
            if self.is_forbidden(part) {
                return TerminalFlags { bare: None, derived: Some(Vec::new()) };
            }
            let bare = self
                .stem_lookup(part)
                .map(|(stem, set)| self.rule_sets.get(&stem).map_or(set, |&id| self.set(id)))
                .filter(|set| !excluded.hits(set) && !self.need_affix.is_some_and(|f| has(set, f)) && !self.forbidden.is_some_and(|f| has(set, f)));
            TerminalFlags { bare, derived: None }
        });
        // A suffix scan for another flag may exhaust the budget. Bare flags are already
        // known and remain usable without doing any more analysis work.
        if terminal.bare.is_some_and(|set| has(set, flag)) {
            return true;
        }
        let sets = terminal.derived.get_or_insert_with(|| {
            if *work >= MAX_WORK {
                return Vec::new();
            }
            let derived = excluded.allowing_forbidden(self.forbidden, self.no_suggest, self.substandard);
            let mut sets = Vec::new();
            // Prefix-only analyses cannot license a terminal COMPOUNDRULE part.
            self.suffix_analyses(part, None, derived, work, &mut |an| {
                if (an.sfx.is_none() && excluded.hits(an.set))
                    || an.sfx.is_some_and(|r| self.cont_holds(r, self.only_in_compound) || excluded.hits(self.set(r.cont)))
                {
                    return;
                }
                sets.push(an.set);
            });
            sets
        });
        sets.iter().any(|set| has(set, flag))
    }

    /// Every way `p` is a stem, or a stem with a prefix, one or two suffixes, or a prefix and a
    /// suffix, as a compound part may be (`ONLYINCOMPOUND` affixes and stems included).
    pub(super) fn analyses<'a>(&'a self, p: &str, excluded: Excluded, work: &mut usize, emit: &mut impl FnMut(Analysis<'a>)) {
        for (stem, set) in self.part_stems(p, excluded) {
            if !self.need_affix.is_some_and(|n| has(set, n)) {
                emit(Analysis { stem, set, pfx: None, sfx: None, inner: None });
            }
        }
        self.suffix_analyses(p, None, excluded, work, emit);
        let mut root = String::with_capacity(p.len() + 8);
        let ends = std::iter::once(0).chain(p.char_indices().map(|(i, c)| i + c.len_utf8()));
        for b in ends.take(self.max_pfx_chars + 1) {
            let (Some(add), Some(rest)) = (p.get(..b), p.get(b..)) else { continue };
            if rest.is_empty() && !self.full_strip {
                continue;
            }
            let Some(ids) = self.pfx_index.get(add) else { continue };
            for r in ids.iter().filter_map(|&i| self.prefixes.get(i as usize)) {
                *work += 1;
                if *work > MAX_WORK {
                    return;
                }
                root.clear();
                root.push_str(&r.strip);
                root.push_str(rest);
                if !r.cond_matches(&root, false) {
                    continue;
                }
                let (circ, needs) = (self.cont_holds(r, self.circumfix), self.cont_holds(r, self.need_affix));
                if !circ && !needs {
                    for (stem, set) in self.part_stems(&root, excluded) {
                        if has(set, r.flag) {
                            emit(Analysis { stem, set, pfx: Some(r), sfx: None, inner: None });
                        }
                    }
                }
                if r.cross {
                    self.suffix_analyses(&root, Some(r), excluded, work, emit);
                }
            }
        }
    }

    /// The analyses of `p` that end in a suffix: on a stem (after the prefix `pfx`, if any), or
    /// (without a prefix) on a form of another suffix.
    fn suffix_analyses<'a>(&'a self, p: &str, pfx: Option<&'a Rule>, excluded: Excluded, work: &mut usize, emit: &mut impl FnMut(Analysis<'a>)) {
        let mut root = String::with_capacity(p.len() + 8);
        let cuts = std::iter::once(p.len()).chain(p.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0 || self.full_strip));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (p.get(..b), p.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups {
                *work += 1;
                if *work > MAX_WORK {
                    return;
                }
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                let rules = || g.rules.iter().filter_map(|&i| self.suffixes.get(i as usize));
                for (stem, set) in self.part_stems(&root, excluded) {
                    for r in rules() {
                        *work += 1;
                        if *work > MAX_WORK {
                            return;
                        }
                        if self.suffix_fits(r, set, pfx) && r.cond_matches(&root, true) {
                            emit(Analysis { stem, set, pfx, sfx: Some(r), inner: None });
                        }
                    }
                }
                // Two suffixes in a row: the outer ones that another suffix's continuation
                // classes can license, looked for together under one inner scan.
                if pfx.is_none() && !g.outer_rules.is_empty() {
                    // The condition checks cost work even when they reject every candidate.
                    *work = work.saturating_add(g.outer_rules.len());
                    if *work > MAX_WORK {
                        return;
                    }
                    let outers: Vec<&Rule> = g
                        .outer_rules
                        .iter()
                        .filter_map(|&i| self.suffixes.get(i as usize))
                        .filter(|r| !self.cont_holds(r, self.only_in_compound) && r.cond_matches(&root, true))
                        .collect();
                    if !outers.is_empty() {
                        self.inner_analyses(&root, &outers, excluded, work, emit);
                    }
                }
            }
        }
    }

    /// Does the suffix `r` apply to a stem with the flags `set`, after the prefix `pfx` (if any)?
    /// Circumfix alike; a suffix that needs another affix only after a prefix that doesn't.
    fn suffix_fits(&self, r: &Rule, set: &[u16], pfx: Option<&Rule>) -> bool {
        if !has(set, r.flag) {
            return false;
        }
        let circ = self.cont_holds(r, self.circumfix);
        let needs = self.cont_holds(r, self.need_affix);
        match pfx {
            None => !circ && !needs,
            Some(p) => {
                r.cross
                    && (has(set, p.flag) || self.cont_holds(r, Some(p.flag)))
                    && circ == self.cont_holds(p, self.circumfix)
                    && !(needs && self.cont_holds(p, self.need_affix))
            }
        }
    }

    /// `form` (what an outer suffix of `outers` was taken off) as a stem with an inner suffix
    /// whose continuation classes hold that outer suffix's flag.
    fn inner_analyses<'a>(&'a self, form: &str, outers: &[&'a Rule], excluded: Excluded, work: &mut usize, emit: &mut impl FnMut(Analysis<'a>)) {
        let mut root = String::with_capacity(form.len() + 8);
        let cuts = std::iter::once(form.len()).chain(form.char_indices().rev().map(|(i, _)| i).filter(|&i| i > 0 || self.full_strip));
        for b in cuts.take(self.max_sfx_chars + 1) {
            let (Some(base), Some(add)) = (form.get(..b), form.get(b..)) else { continue };
            let Some(groups) = self.sfx_index.get(add) else { continue };
            for g in groups.iter().filter(|g| !g.cont_rules.is_empty()) {
                *work += 1;
                if *work > MAX_WORK {
                    return;
                }
                root.clear();
                root.push_str(base);
                root.push_str(&g.strip);
                for (stem, set) in self.part_stems(&root, excluded) {
                    for r in g.cont_rules.iter().filter_map(|&i| self.suffixes.get(i as usize)) {
                        *work += 1;
                        if *work > MAX_WORK {
                            return;
                        }
                        if !has(set, r.flag)
                            || self.cont_holds(r, self.circumfix)
                            || self.cont_holds(r, self.only_in_compound)
                            || !r.cond_matches(&root, true)
                        {
                            continue;
                        }
                        for &outer in outers {
                            *work += 1;
                            if *work > MAX_WORK {
                                return;
                            }
                            if self.cont_holds(r, Some(outer.flag)) {
                                emit(Analysis { stem, set, pfx: None, sfx: Some(outer), inner: Some(r) });
                            }
                        }
                    }
                }
            }
        }
    }

    /// The stem `w` as a compound part: its index and flags, unless it is forbidden or excluded.
    /// A stem with `ONLYINCOMPOUND` homonyms beside others is two: each group's flags apply on
    /// their own (`foo/XOS` and `foo/T`: `foos` is a part, `foot` is not, as in the CLI).
    ///
    /// Homonyms kept apart ([`AffixDict::homonyms`]) are each a part of their own: a suffix and
    /// the compound flag must come from one (Dutch `poeder` the noun compounds, `poederden` the
    /// verb form doesn't: `schuurpoederden` is wrong in the CLI).
    fn part_stems(&self, w: &str, excluded: Excluded) -> impl Iterator<Item = (u32, &[u16])> {
        let found = self.stem_lookup(w);
        let apart = found.and_then(|(stem, _)| self.homonyms.get(&stem).map(|h| (stem, h)));
        let main = found.filter(|_| apart.is_none());
        let each = apart.into_iter().flat_map(move |(stem, h)| h.iter().map(move |&id| (stem, self.set(id))));
        let only = found.and_then(|(stem, _)| self.cpd_sets.get(&stem).map(|&c| (stem, self.set(c))));
        main.into_iter()
            .chain(each)
            .chain(only)
            .filter(move |(_, set)| !excluded.hits(set) && (excluded.forbidden_allowed() || !self.forbidden.is_some_and(|f| has(set, f))))
    }

    /// May a part with `info` start at char `start`, after the last part of `s.path`
    /// (`CHECKCOMPOUNDCASE`, `CHECKCOMPOUNDTRIPLE`, `CHECKCOMPOUNDPATTERN`)?
    fn boundary_ok(&self, s: &Search, start: usize, info: &Info) -> bool {
        let Some(&(ps, pe, prev)) = s.path.last() else { return true };
        let c = s.c;
        let (Some(&a), Some(&b)) = (s.chars.get(pe.saturating_sub(1)), s.chars.get(start)) else { return true };
        if c.case && (a.is_uppercase() || b.is_uppercase()) && a != '-' && b != '-' {
            return false;
        }
        if c.triple && start == pe && a == b {
            let before = pe >= 2 && s.chars.get(pe - 2) == Some(&a);
            let after = s.chars.get(start + 1) == Some(&b);
            if before || after {
                return false;
            }
        }
        // A SIMPLIFIEDTRIPLE join (the parts share a letter) still may not leave three equal
        // letters in the word (`buss|ssen` as `busssen`, `boo|oob` as `booob`).
        if c.triple && start + 1 == pe && s.chars.get(pe) == Some(&a) {
            return false;
        }
        if c.patterns.is_empty() {
            return true;
        }
        let text = |x: usize, y: usize| s.cuts.get(x).zip(s.cuts.get(y)).and_then(|(&x, &y)| s.w.get(x..y)).unwrap_or_default();
        let (before, after) = (text(ps, pe), text(start, s.chars.len()));
        !c.patterns.iter().any(|p| {
            before.ends_with(&*p.end)
                && after.starts_with(&*p.begin)
                && (!p.unmodified || prev.unmodified)
                && p.end_need.met(prev.marks)
                && p.begin_need.met(info.marks)
        })
    }

    /// Are the parts of `s.path` few enough (`COMPOUNDWORDMAX`, `COMPOUNDROOT`)? `whole`: the
    /// word is complete (the Hungarian syllable rule counts its vowels).
    fn words_ok(&self, s: &Search, whole: bool) -> bool {
        let Some(max) = s.c.word_max else { return true };
        let words: usize = s.path.iter().map(|p| usize::from(p.2.words)).sum();
        if words <= max {
            return true;
        }
        match &s.c.syllables {
            None => false,
            Some(_) if !whole => true,
            Some((most, vowels)) => {
                let tail = s.path.last().map_or(0, |p| usize::from(p.2.tail));
                s.chars.iter().filter(|c| vowels.contains(c)).count().saturating_sub(tail) <= *most
            }
        }
    }

    /// The checks on a complete split: two parts at least, `COMPOUNDWORDMAX`, `CHECKCOMPOUNDDUP`,
    /// `FORCEUCASE`, the three-part rule and `CHECKCOMPOUNDREP`.
    fn whole_ok(&self, s: &mut Search) -> bool {
        let n = s.path.len();
        if n < 2 || !self.words_ok(s, true) {
            return false;
        }
        let (Some(&(_, _, last)), Some(&(_, _, prev))) = (s.path.last(), s.path.get(n - 2)) else { return false };
        if s.c.dup && last.stem == prev.stem {
            return false;
        }
        // FORCEUCASE: a word with no capital at all is wrong (`fooQuxbar` is right in the CLI).
        if last.force && !s.capitalized && !s.chars.iter().any(|c| c.is_uppercase()) {
            return false;
        }
        let span = |s: &Search, i: usize, j: usize| -> String {
            let (Some(&(a, ..)), Some(&(_, b, _))) = (s.path.get(i), s.path.get(j)) else { return String::new() };
            s.cuts.get(a).zip(s.cuts.get(b)).and_then(|(&x, &y)| s.w.get(x..y)).unwrap_or_default().to_string()
        };
        if n >= 3 && self.near_word(s) {
            return false;
        }
        if s.c.rep && !self.rep.is_empty() {
            let pairs = (0..n - 1).map(|i| (i, i + 1));
            let tails = (0..n - 1).map(|i| (i, n - 1));
            let spans: Vec<String> = pairs.chain(tails).map(|(i, j)| span(s, i, j)).collect();
            if spans.iter().any(|sp| self.rep_fault(sp, s)) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod terminal_rule_review_tests {
    use super::{AffixDict, Excluded, MAX_WORK, Pieces};
    /// A tiny suffix chain supplies both matching and nonmatching outer candidates.
    fn suffix_budget_fixture() -> AffixDict {
        AffixDict::from_hunspell(
            "SET UTF-8\nFLAG num\nSFX 1 Y 1\nSFX 1 0 s/2 .\nSFX 2 Y 1\nSFX 2 0 t .\nSFX 3 Y 1\nSFX 3 0 t .\n",
            "2\nroot/1\nother/3\n",
            &[],
        )
        .unwrap()
    }

    /// Even when every condition fails, the outer candidates consume the remaining work.
    #[test]
    fn rejected_outer_conditions_consume_the_compound_budget() {
        let dict =
            AffixDict::from_hunspell("SET UTF-8\nFLAG num\nSFX 1 Y 1\nSFX 1 0 s/2 .\nSFX 2 Y 2\nSFX 2 0 t a\nSFX 2 0 t b\n", "1\nroot/1\n", &[])
                .unwrap();
        let mut work = MAX_WORK - 2;
        let mut emitted = 0;
        dict.suffix_analyses("rootst", None, Excluded::default(), &mut work, &mut |_| emitted += 1);
        assert!(work > MAX_WORK, "rejected conditions must exhaust the work budget");
        assert_eq!(emitted, 0);
    }

    /// Nonmatching outer candidates still consume the shared work budget.
    #[test]
    fn inner_suffix_comparisons_charge_nonmatching_outer_rules() {
        let dict = suffix_budget_fixture();
        let flag = dict.stem_lookup("other").unwrap().1[0];
        let outer = dict.suffixes.iter().find(|r| r.flag == flag).unwrap();
        let outers = vec![outer; 32];
        let (mut work, mut emitted) = (0, 0);
        dict.inner_analyses("roots", &outers, Excluded::default(), &mut work, &mut |_| emitted += 1);
        assert_eq!(emitted, 0);
        assert_eq!(work, outers.len() + 2);
    }

    /// Matching candidates stop at the budget before any additional result is emitted.
    #[test]
    fn inner_suffix_comparisons_stop_before_over_budget_emissions() {
        let dict = suffix_budget_fixture();
        let inner = dict.suffixes.iter().find(|r| !dict.set(r.cont).is_empty()).unwrap();
        let outer = dict.suffixes.iter().find(|r| dict.cont_holds(inner, Some(r.flag))).unwrap();
        let outers = vec![outer; 32];
        let (mut work, mut emitted) = (MAX_WORK - 3, 0);
        dict.inner_analyses("roots", &outers, Excluded::default(), &mut work, &mut |_| emitted += 1);
        assert_eq!(work, MAX_WORK + 1, "an interrupted scan must not look complete to the cache");
        assert_eq!(emitted, 1);
    }

    /// Earlier absent terminal flags must not repeatedly scan suffixes or hide a later bare match.
    #[test]
    fn terminal_rule_order_keeps_late_bare_and_derived_matches() {
        let count = 1500;
        for bare in [false, true] {
            let mut aff = String::from("SET UTF-8\nFLAG num\nCOMPOUNDMIN 1\nCOMPOUNDRULE 64\n");
            for flag in 100..163 {
                aff.push_str(&format!("COMPOUNDRULE (1)({flag})\n"));
            }
            aff.push_str("COMPOUNDRULE (1)(3)\n");
            aff.push_str(&format!("SFX 9 Y {count}\n"));
            for continuation in 1000..1000 + count {
                aff.push_str(&format!("SFX 9 0 s/{continuation} .\n"));
            }
            // The flags must occur on a stem, otherwise the reader correctly removes
            // their impossible COMPOUNDRULE tokens before terminal matching.
            let decoy = (100..163).map(|flag| flag.to_string()).collect::<Vec<_>>().join(",");
            let words = if bare { format!("4\nx/1\nys/3\ny/9\ndecoy/{decoy}\n") } else { format!("3\nx/1\ny/9,3\ndecoy/{decoy}\n") };
            let dict = AffixDict::from_hunspell(&aff, &words, &[]).unwrap();
            assert!(dict.check("xys"), "bare={bare}: a later matching rule must reuse terminal work");
            assert!(!dict.check("xz"));
        }
    }

    /// A terminal's suffix analyses are shared, while budget exhaustion never hides a cached bare flag.
    #[test]
    fn terminal_analyses_are_reused_across_flag_queries() {
        let dict = AffixDict::from_hunspell("SET UTF-8\nFLAG num\nSFX 9 Y 1\nSFX 9 0 s .\n", "3\ny/9,2\nys/3\nx/4\n", &[]).unwrap();
        let absent = dict.stem_lookup("x").unwrap().1[0];
        let bare_flag = dict.stem_lookup("ys").unwrap().1[0];
        let derived_flag = *dict.stem_lookup("y").unwrap().1.iter().find(|&&f| dict.suffixes.iter().all(|r| r.flag != f)).unwrap();
        let (mut work, mut cache) = (0, None);
        assert!(!dict.rule_final_has_flag("ys", Excluded::default(), absent, &mut work, &mut cache));
        let spent = work;
        assert!(spent > 0);
        assert!(dict.rule_final_has_flag("ys", Excluded::default(), derived_flag, &mut work, &mut cache));
        assert_eq!(work, spent);
        work = MAX_WORK + 1;
        assert!(dict.rule_final_has_flag("ys", Excluded::default(), bare_flag, &mut work, &mut cache));
        assert_eq!(work, MAX_WORK + 1);
    }

    /// An interrupted inner scan must not cache one of a piece's two valid syllable analyses.
    #[test]
    fn incomplete_compound_piece_analyses_are_not_cached() {
        let aff = "SET UTF-8\nFLAG num\nCOMPOUNDMIN 1\nLANG hu_HU\nCOMPOUNDFLAG 4\nCOMPOUNDSYLLABLE 6 aeiou\nSFX 1 Y 1\nSFX 1 0 s/2,3 .\nSFX 2 Y 1\nSFX 2 0 ta .\nSFX 3 Y 1\nSFX 3 0 ta/7 .\n";
        let dict = AffixDict::from_hunspell(aff, "2\nroot/1,4\nother/4\n", &[]).unwrap();
        let mut complete = Pieces::default();
        let mut full_budget = MAX_WORK;
        assert!(dict.flag_compound_spending("otherrootsta", Excluded::default(), false, &mut full_budget, Some(&mut complete)));
        assert_eq!(complete.0[0]["rootsta"].len(), 2, "positive control: both syllable analyses are reachable");
        for mut budget in 1..128 {
            let mut partial = Pieces::default();
            let _ = dict.flag_compound_spending("otherrootsta", Excluded::default(), false, &mut budget, Some(&mut partial));
            for (index, map) in partial.0.iter().enumerate() {
                for (piece, infos) in map {
                    assert_eq!(Some(infos), complete.0[index].get(piece), "incomplete cached analyses for {piece}");
                }
            }
        }
    }

    /// Regression: bare terminal homonyms use merged rule flags.
    #[test]
    fn bare_terminal_homonyms_use_merged_rule_flags() {
        let aff = "SET UTF-8\nNEEDAFFIX N\nCOMPOUNDMIN 1\nCOMPOUNDRULE 1\nCOMPOUNDRULE AB\n";
        let dict = AffixDict::from_hunspell(aff, "3\nx/A\ny/BN\ny/C\n", &[]).unwrap();
        assert!(dict.check("xy"));
        let only_needs_affix = AffixDict::from_hunspell(aff, "2\nx/A\ny/BN\n", &[]).unwrap();
        assert!(!only_needs_affix.check("xy"));
    }
    /// Regression: bare terminal match leaves work for later rule flags.
    #[test]
    fn bare_terminal_match_leaves_work_for_later_rule_flags() {
        let mut aff = String::from("SET UTF-8\nFLAG num\nCOMPOUNDMIN 1\nCOMPOUNDRULE 2\nCOMPOUNDRULE (1)(2)(4)\nCOMPOUNDRULE (1)(3)\nSFX 9 Y 100\n");
        for _ in 0..100 {
            aff.push_str("SFX 9 0 s .\n");
        }
        let dict = AffixDict::from_hunspell(&aff, "3\nx/1\nys/2,3\ny/9\n", &[]).unwrap();
        // The reader interns written flag names; query its actual IDs.
        let (_, flags) = dict.stem_lookup("ys").unwrap();
        assert_eq!(flags.len(), 2);
        let mut work = MAX_WORK - 1;
        for &flag in flags {
            assert!(dict.rule_final_has_flag("ys", Excluded::default(), flag, &mut work, &mut None));
            assert_eq!(work, MAX_WORK - 1);
        }
    }

    /// Regression: terminal flag queries do not spend budget on prefixes.
    #[test]
    fn terminal_flag_queries_do_not_spend_budget_on_prefixes() {
        let count = 100;
        let mut aff = format!("SET UTF-8\nFLAG num\nPFX 9 Y {count}\n");
        for _ in 0..count {
            aff.push_str("PFX 9 0 p .\n");
        }
        aff.push_str("SFX 10 Y 1\nSFX 10 0 s .\n");
        let dict = AffixDict::from_hunspell(&aff, "2\nroot/9,10,2\npword/3\n", &[]).unwrap();
        // Source keys are interned; use IDs resolved from stems carrying each key.
        let other_flag = dict.stem_lookup("pword").unwrap().1[0];
        let terminal_flag = dict.stem_lookup("root").unwrap().1[0];
        let mut work = MAX_WORK - 10;
        assert!(!dict.rule_final_has_flag("proots", Excluded::default(), other_flag, &mut work, &mut None));
        assert!(work < MAX_WORK, "irrelevant prefixes must not consume the remaining work");
        assert!(dict.rule_final_has_flag("roots", Excluded::default(), terminal_flag, &mut work, &mut None));
    }
}
