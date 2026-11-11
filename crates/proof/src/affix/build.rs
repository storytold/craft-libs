//! Building an [`AffixDict`] from `.aff` and `.dic` text: stem and flag-set interning and the lookup index.

use super::*;

/// A dictionary's stems as `(word, start, end, is a word)` ranges of one buffer of flag keys.
type StemFlags = (Vec<(String, usize, usize, bool)>, Vec<FlagKey>);

/// Flags a stem's entries hold in all from which its sets are worked out once per distinct
/// entry list ([`StemSets`]); real stems have a few dozen.
const MEMO_STEM_FLAGS: usize = 256;

/// A stem's flag sets as [`AffixDict::build`] works them out from its entries.
struct StemSets {
    id: u32,
    cpd: Option<u32>,
    rule: Option<u32>,
    homonyms: Option<Box<[u32]>>,
    dropped: usize,
    compound: bool,
}

/// Note a homonym's flags among its group's (`group`), unless an equal one is there; false past
/// [`MAX_HOMONYMS`] (then the merged flags answer: hostile input can't make it costly).
fn note_homonym(group: &mut Vec<Vec<u16>>, fresh: &[u16]) -> bool {
    if group.iter().any(|g| g == fresh) {
        return true;
    }
    if group.len() >= MAX_HOMONYMS {
        return false;
    }
    group.push(fresh.to_vec());
    true
}

/// Add a homonym's sorted flags `fresh` to the sorted set `set`; `false` (and nothing added) when
/// that would take it past [`MAX_FLAGS_PER_STEM`].
fn merge_homonym(set: &mut Vec<u16>, fresh: &[u16]) -> bool {
    let before = set.len();
    let add: Vec<u16> = fresh.iter().copied().filter(|&f| !has(set, f)).collect();
    if add.is_empty() {
        // A repeated homonym adds nothing: no sort (200,000 duplicate lines of a stem with
        // thousands of flags would each pay one).
        return true;
    }
    if before.saturating_add(add.len()) > MAX_FLAGS_PER_STEM {
        return false;
    }
    set.extend(add);
    // A sorted run and a sorted tail: the stable sort merges them in linear time.
    set.sort();
    true
}

/// Put the exclusion flags `carried` (FORBIDDENWORD, NOSUGGEST, SUBSTANDARD of any homonym) back
/// into the sorted set `set`, making room by dropping a flag that excludes nothing (there are at
/// most three exclusion flags, so a full set always has one).
fn carry_exclusions(set: &mut Vec<u16>, carried: &[u16], exclusions: &[u16]) {
    for &f in carried {
        if has(set, f) {
            continue;
        }
        if set.len() >= MAX_FLAGS_PER_STEM
            && let Some(at) = set.iter().rposition(|g| !exclusions.contains(g))
        {
            set.remove(at);
        }
        if let Err(at) = set.binary_search(&f) {
            set.insert(at, f);
        }
    }
}

/// Do two sorted flag sets share a flag? (A binary search per flag of the smaller one.)
pub(super) fn intersects(a: &[u16], b: &[u16]) -> bool {
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    small.iter().any(|f| has(large, *f))
}

/// A running count of the stems' flag text while a dictionary is read: past
/// [`FLAG_TEXT_PER_FLAG`] bytes for each of `max_flags` flags, reading stops with an error, before
/// the rest of a huge `.dic` is held ([`MAX_STEM_FLAGS`]).
pub(super) struct FlagText {
    bytes: usize,
    cap: usize,
}

impl FlagText {
    pub(super) fn new(max_flags: usize) -> Self {
        FlagText { bytes: 0, cap: max_flags.saturating_mul(FLAG_TEXT_PER_FLAG) }
    }

    pub(super) fn add(&mut self, flags: &str) -> Result<(), String> {
        self.bytes = self.bytes.saturating_add(flags.len());
        if self.bytes > self.cap {
            return Err(format!("the stems' flags take more than {} bytes", self.cap));
        }
        Ok(())
    }
}

impl AffixDict {
    /// Build from Hunspell `.aff` and `.dic` text (decoded to UTF-8; the `.dic` count line and
    /// morphological fields are ignored). `comments` (provenance, licence) are kept with the data.
    pub fn from_hunspell(aff: &str, dic: &str, comments: &[&str]) -> Result<AffixDict, String> {
        AffixDict::from_hunspell_in(aff, dic, comments, false)
    }

    /// [`AffixDict::from_hunspell`] for an English dictionary: flag-based compounding
    /// (`COMPOUNDFLAG`…), which this engine doesn't implement and English spelling doesn't need,
    /// is ignored rather than refused. `COMPOUNDRULE`, `ONLYINCOMPOUND` and `COMPOUNDMIN`
    /// (ordinals: `21st`) and `OCONV` are read, as for every language.
    pub fn from_hunspell_english(aff: &str, dic: &str, comments: &[&str]) -> Result<AffixDict, String> {
        AffixDict::from_hunspell_in(aff, dic, comments, true)
    }

    fn from_hunspell_in(aff: &str, dic: &str, comments: &[&str], english: bool) -> Result<AffixDict, String> {
        AffixDict::from_hunspell_capped(aff, dic, comments, english, MAX_STEM_FLAGS)
    }

    /// [`AffixDict::from_hunspell_in`] with at most `max_flags` stem flags in all
    /// ([`MAX_STEM_FLAGS`]; tests lower it).
    pub(super) fn from_hunspell_capped(aff: &str, dic: &str, comments: &[&str], english: bool, max_flags: usize) -> Result<AffixDict, String> {
        // Some dictionaries open with a UTF-8 BOM; it must not glue itself to the first
        // directive or turn the `.dic` count line into a stem.
        let aff = aff.strip_prefix('\u{feff}').unwrap_or(aff);
        let dic = dic.strip_prefix('\u{feff}').unwrap_or(dic);
        let rules = parse_aff(aff, english)?;
        let mut stems: Vec<(String, String)> = Vec::new();
        let mut flag_text = FlagText::new(max_flags);
        for (n, line) in dic.lines().enumerate() {
            // The entry may hold spaces (`ad hoc/B\tpo:adv`); it ends where Hunspell ends it.
            // Leading blanks belong to it, as in Hunspell (`  lab` is no `lab`).
            let tok = line.get(..crate::hunspell::dic_entry_end(line.as_bytes())).unwrap_or_default();
            if tok.is_empty() {
                continue;
            }
            if n == 0 && tok.parse::<usize>().is_ok() {
                continue;
            }
            // `\/` is a literal slash in a word; the first plain `/` starts the flags.
            let mut word = String::new();
            let mut flags = String::new();
            let mut chars = tok.chars().peekable();
            let mut in_flags = false;
            while let Some(c) = chars.next() {
                if in_flags {
                    if c != EMOJI_SELECTOR {
                        flags.push(c);
                    }
                } else if c == '\\' && chars.peek() == Some(&'/') {
                    word.push('/');
                    chars.next();
                } else if c == '/' {
                    in_flags = true;
                } else {
                    word.push(c);
                }
            }
            let word = rules.strip_ignored(&word);
            if word.is_empty() || word.len() > MAX_WORD_BYTES || word.contains(['\n', '/']) {
                continue;
            }
            // With an `AF` table the field stays an alias number here: build() reads each alias
            // once, and every stem using it shares those flags.
            flag_text.add(&flags)?;
            stems.push((word, flags));
        }
        AffixDict::build(rules, stems, comments.iter().map(ToString::to_string).collect(), MAX_FLAGS_PER_ENTRY, max_flags)
    }

    /// Read each stem's flag field ([`AffixDict::build`] explains the rules) into ranges of one key
    /// buffer, within the `max_flags` budget: the stems as `(word, start, end, is a word)`, and the keys.
    fn read_stem_flags(rules: &Rules, stems: Vec<(String, String)>, entry_cap: usize, max_flags: usize) -> Result<StemFlags, String> {
        let mode = rules.mode;
        // A `.dic` entry's malformed flag items read as Hunspell reads them ([`FlagMode`]); an
        // entry with more than MAX_FLAGS_PER_STEM flags, or compact data whose flags don't read,
        // is skipped. One with more than `entry_cap` flags is not a word either, but its
        // FORBIDDENWORD and NOSUGGEST flags still apply to its homonyms: an exclusion is never lost.
        let mut skipped = 0usize;
        let exclusion_keys = [rules.forbidden, rules.no_suggest, rules.substandard];
        let is_exclusion_key = |k: &FlagKey| exclusion_keys.contains(&Some(*k));
        // Each stem's flags are a range of `stem_keys`: one buffer, not one allocation per stem.
        // The flag says whether the entry is a word (false: it only carries exclusions).
        let mut stem_keys: Vec<FlagKey> = Vec::new();
        // `.dic` entries (not compact data) read malformed flag items as Hunspell does.
        let lenient = entry_cap == MAX_FLAGS_PER_ENTRY;
        // With an `AF` table: each alias read once, on first use, as its range of `stem_keys`
        // (and whether it is a word's flags); the stems using it share the range, so a big alias
        // on many stems costs its flags once (no copy per stem).
        let mut alias_ranges: FxMap<usize, Option<(usize, usize, bool)>> = FxMap::default();
        let read = |f: &str, stem_keys: &mut Vec<FlagKey>| -> Option<(usize, usize, bool)> {
            let keys = mode.parse_at_most(f, MAX_FLAGS_PER_STEM.max(entry_cap), lenient).ok()?;
            let start = stem_keys.len();
            let word = keys.len() <= entry_cap;
            if word {
                stem_keys.extend(keys);
            } else {
                stem_keys.extend(keys.into_iter().filter(is_exclusion_key));
            }
            Some((start, stem_keys.len(), word))
        };
        let mut read_stems: Vec<(String, usize, usize, bool)> = Vec::with_capacity(stems.len());
        for (w, f) in stems {
            let range = match rules.alias_index(&f) {
                Some(i) => *alias_ranges.entry(i).or_insert_with(|| read(rules.aliases.get(i).map_or("", String::as_str), &mut stem_keys)),
                None if rules.aliases_on => Some((0, 0, true)),
                None => read(&f, &mut stem_keys),
            };
            if stem_keys.len() > max_flags {
                return Err(format!("the stems carry more than {max_flags} flags in all"));
            }
            if range.is_none_or(|r| !r.2) {
                skipped += 1;
            }
            if let Some((a, b, word)) = range {
                read_stems.push((w, a, b, word));
            }
        }
        if skipped > 0 {
            log::debug!("affix dictionary: {skipped} entries with unreadable or too many flags skipped");
        }
        Ok((read_stems, stem_keys))
    }

    /// Build from parsed rules and stems whose flag fields carry at most `entry_cap` flags each
    /// ([`MAX_FLAGS_PER_ENTRY`] for `.dic` entries, [`MAX_FLAGS_PER_STEM`] for compact data).
    /// The stems' flags may number `max_flags` in all ([`MAX_STEM_FLAGS`]), counted as they are read.
    pub(super) fn build(
        rules: Rules,
        stems: Vec<(String, String)>,
        comments: Vec<String>,
        entry_cap: usize,
        max_flags: usize,
    ) -> Result<AffixDict, String> {
        let mode = rules.mode;
        let exclusion_keys = [rules.forbidden, rules.no_suggest, rules.substandard];
        let (mut stems, stem_keys) = AffixDict::read_stem_flags(&rules, stems, entry_cap, max_flags)?;
        let conts = |rs: &[RawRule]| rs.iter().map(|r| mode.parse(&r.4)).collect::<Result<Vec<_>, _>>();
        let (pfx_cont, sfx_cont) = (conts(&rules.prefixes)?, conts(&rules.suffixes)?);
        // Flags: every one a rule, a stem or a directive uses gets an id, in key order.
        let mut distinct: FxMap<FlagKey, u16> = FxMap::default();
        let all = rules.prefixes.iter().chain(&rules.suffixes).map(|r| r.0);
        let all = all.chain(pfx_cont.iter().chain(&sfx_cont).flatten().copied()).chain(stem_keys.iter().copied());
        // NEEDAFFIX and CIRCUMFIX flags may only ever appear in rules' continuation classes.
        let directives =
            [rules.forbidden, rules.no_suggest, rules.substandard, rules.keepcase, rules.only_in_compound, rules.need_affix, rules.circumfix];
        let cpd_keys = rules.cpd.keys(mode)?;
        for k in all.chain(directives.into_iter().flatten()).chain(cpd_keys) {
            distinct.entry(k).or_insert(0);
            if distinct.len() > MAX_DISTINCT_FLAGS {
                return Err(format!("more than {MAX_DISTINCT_FLAGS} distinct flags"));
            }
        }
        let mut flag_keys: Vec<FlagKey> = distinct.keys().copied().collect();
        flag_keys.sort_unstable();
        // At most 2^16 keys, so every index fits a u16.
        for (i, k) in flag_keys.iter().enumerate() {
            if let (Some(id), Ok(i)) = (distinct.get_mut(k), u16::try_from(i)) {
                *id = i;
            }
        }
        let id_of = |k: FlagKey| distinct.get(&k).copied();
        // Stems sorted by bytes; homonyms (the same word twice) are merged.
        stems.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut d = AffixDict {
            words: String::with_capacity(stems.iter().map(|s| s.0.len()).sum()),
            offs: Vec::with_capacity(stems.len() + 1),
            set_offs: vec![0, 0],
            forbidden: rules.forbidden.and_then(id_of),
            no_suggest: rules.no_suggest.and_then(id_of),
            substandard: rules.substandard.and_then(id_of),
            keepcase: rules.keepcase.and_then(id_of),
            need_affix: rules.need_affix.and_then(id_of),
            circumfix: rules.circumfix.and_then(id_of),
            full_strip: rules.full_strip,
            check_sharps: rules.check_sharps,
            flag_mode: mode,
            iconv: Conv::new(rules.iconv),
            oconv: Conv::new(rules.oconv),
            ignore: rules.ignore.into_boxed_slice(),
            try_chars: rules.try_chars,
            key: rules.key,
            map: rules.map,
            rep: rules.rep,
            comments,
            // A compound rule's flag no stem carries gets no id: its token matches no part.
            compound_rules: rules
                .compound_rules
                .iter()
                .map(|p| parse_compound_rule(p, mode).map(|t| t.into_iter().map(|(k, rep)| (id_of(k), rep)).collect()))
                .collect::<Result<_, _>>()?,
            compound_text: rules.compound_rules,
            only_in_compound: rules.only_in_compound.and_then(id_of),
            compound_min: rules.compound_min.unwrap_or(3),
            cpd: rules.cpd.build(mode, &id_of)?,
            ..AffixDict::default()
        };
        d.offs.push(0);
        let compound_flags: Vec<u16> = d.compound_rules.iter().flatten().filter_map(|(f, _)| *f).collect();
        let mut set_ids: FxMap<Vec<u16>, u32> = FxMap::default();
        set_ids.insert(Vec::new(), 0);
        let exclusions: Vec<u16> = exclusion_keys.into_iter().flatten().filter_map(id_of).collect();
        let mut i = 0;
        let mut dropped = 0usize;
        let mut merged: Vec<u16> = Vec::new();
        let mut solo: Vec<u16> = Vec::new();
        let mut only_set: Vec<u16> = Vec::new();
        let mut fresh: Vec<u16> = Vec::new();
        let mut carried: Vec<u16> = Vec::new();
        // Each homonym's flags (ONLYINCOMPOUND ones apart), and whether all fit.
        let mut each: Vec<Vec<u16>> = Vec::new();
        let mut each_only: Vec<Vec<u16>> = Vec::new();
        // A stem's sets depend only on its entries' flag ranges: stems whose entries share big
        // `AF` aliases are worked out once (16 homonyms of 8,000 flags on 17,576 stems loaded in
        // 55 s when each stem merged them again).
        let mut memo: FxMap<Vec<(usize, usize, bool)>, Option<StemSets>> = FxMap::default();
        while i < stems.len() {
            let Some((word, ..)) = stems.get(i) else { break };
            let end = stems.get(i..).and_then(|rest| rest.iter().position(|s| s.0 != *word)).map_or(stems.len(), |k| i + k);
            let entries: Vec<(usize, usize, bool)> = stems.get(i..end).unwrap_or_default().iter().map(|s| (s.1, s.2, s.3)).collect();
            let heavy = entries.iter().map(|e| e.1.saturating_sub(e.0)).sum::<usize>() >= MEMO_STEM_FLAGS;
            let at = u32::try_from(d.flags.len()).ok();
            if heavy && let Some(known) = memo.get(&entries) {
                if let Some(m) = known {
                    dropped += m.dropped;
                    d.push_known(word, m)?;
                }
                i = end;
                continue;
            }
            let dropped_before = dropped;
            merged.clear();
            solo.clear();
            only_set.clear();
            carried.clear();
            each.clear();
            each_only.clear();
            let mut fits = true;
            let mut is_word = false;
            // NEEDAFFIX belongs to its homonym: the word stands alone if any homonym does
            // (`foo/NA` and `foo/B`: `foo` is right).
            let mut all_need = true;
            // So does ONLYINCOMPOUND, with its flags: beside homonyms without it, the homonyms
            // with it are kept apart (`cpd_sets`), only for compound parts (German
            // `schulen/DIXY` beside `schulen/Sozm`: `schulen` alone, but not `schulens`), as in
            // the CLI.
            let (mut solo_need, mut only_need, mut any_solo, mut any_only) = (true, true, false, false);
            let mut j = i;
            while let Some((w, a, b, entry_is_word)) = stems.get(j)
                && w == word
            {
                j += 1;
                fresh.clear();
                fresh.extend(stem_keys.get(*a..*b).unwrap_or_default().iter().filter_map(|&k| id_of(k)));
                for &f in &fresh {
                    if exclusions.contains(&f) && !carried.contains(&f) {
                        carried.push(f);
                    }
                }
                if !entry_is_word {
                    continue;
                }
                is_word = true;
                let need = d.need_affix.is_some_and(|n| fresh.contains(&n));
                let only = d.only_in_compound.is_some_and(|o| fresh.contains(&o));
                all_need &= need;
                fresh.sort_unstable();
                fresh.dedup();
                let group_fits = if only {
                    any_only = true;
                    only_need &= need;
                    merge_homonym(&mut only_set, &fresh)
                } else {
                    any_solo = true;
                    solo_need &= need;
                    merge_homonym(&mut solo, &fresh)
                };
                // Homonyms can add up to more flags than one entry holds; past MAX_FLAGS_PER_STEM
                // an entry's other flags are dropped (its exclusions are carried below), so the
                // compact data never holds a set its reader refuses.
                if !merge_homonym(&mut merged, &fresh) || !group_fits {
                    dropped += 1;
                    fits = false;
                }
                fits &= note_homonym(if only { &mut each_only } else { &mut each }, &fresh);
            }
            if !is_word {
                if heavy {
                    memo.insert(entries, None);
                }
                i = j;
                continue;
            }
            if !all_need && let Some(n) = d.need_affix {
                merged.retain(|&f| f != n);
            }
            let mixed = any_solo && any_only;
            // The homonyms whose flags `flags` holds merged, if they differ.
            let apart = if mixed { std::mem::take(&mut each) } else { each.drain(..).chain(each_only.drain(..)).collect() };
            if fits {
                d.keep_apart(&mut set_ids, apart, &carried, &exclusions)?;
            }
            let cpd_id = if mixed {
                for (set, all_need) in [(&mut solo, solo_need), (&mut only_set, only_need)] {
                    if !all_need && let Some(n) = d.need_affix {
                        set.retain(|&f| f != n);
                    }
                    carry_exclusions(set, &carried, &exclusions);
                }
                // `COMPOUNDRULE` parts read all the homonyms' flags.
                if !d.compound_rules.is_empty() {
                    carry_exclusions(&mut merged, &carried, &exclusions);
                    let all = d.intern(&mut set_ids, &mut merged)?;
                    if let Ok(at) = u32::try_from(d.flags.len()) {
                        d.rule_sets.insert(at, all);
                    }
                }
                std::mem::swap(&mut merged, &mut solo);
                Some(d.intern(&mut set_ids, &mut only_set)?)
            } else {
                carry_exclusions(&mut merged, &carried, &exclusions);
                None
            };
            if let Some(c) = cpd_id
                && let Ok(at) = u32::try_from(d.flags.len())
            {
                d.cpd_sets.insert(at, c);
            }
            let id = d.intern(&mut set_ids, &mut merged)?;
            let compound = [Some(id), cpd_id].into_iter().flatten().any(|s| d.set(s).iter().any(|f| compound_flags.contains(f)));
            d.note_first(word, compound);
            if heavy && let Some(at) = at {
                let homonyms = d.homonyms.get(&at).cloned();
                let rule = d.rule_sets.get(&at).copied();
                memo.insert(entries, Some(StemSets { id, cpd: cpd_id, rule, homonyms, dropped: dropped - dropped_before, compound }));
            }
            d.words.push_str(word);
            d.offs.push(u32::try_from(d.words.len()).map_err(|_| "stems too large".to_string())?);
            d.flags.push(id);
            i = j;
        }
        if dropped > 0 {
            log::debug!("affix dictionary: {dropped} homonym entries past {MAX_FLAGS_PER_STEM} merged flags kept only their exclusions");
        }
        let mut build_rules = |raw: Vec<RawRule>, conts: Vec<Vec<FlagKey>>, d: &mut AffixDict| -> Result<Vec<Rule>, String> {
            raw.into_iter()
                .zip(conts)
                .map(|((flag, cross, strip, add, _, cond), cont)| {
                    let flag = id_of(flag).ok_or("affix flag without an id")?;
                    let cont = d.intern(&mut set_ids, &mut cont.iter().filter_map(|&k| id_of(k)).collect())?;
                    Ok(Rule { flag, cross, strip: strip.into(), add: add.into(), cont, cond: parse_cond(&cond)?, cond_text: cond.into() })
                })
                .collect()
        };
        d.prefixes = build_rules(rules.prefixes, pfx_cont, &mut d)?;
        d.suffixes = build_rules(rules.suffixes, sfx_cont, &mut d)?;
        d.flag_keys = flag_keys;
        d.index();
        Ok(d)
    }

    /// Note the first letter of `word` among the compound parts' when it is one (`compound`).
    fn note_first(&mut self, word: &str, compound: bool) {
        if compound
            && let Some(c) = word.chars().next()
            && !self.compound_firsts.contains(&c)
        {
            self.compound_firsts.push(c);
        }
    }

    /// Push the stem `word` with the sets `m` another stem with the same entries got.
    fn push_known(&mut self, word: &str, m: &StemSets) -> Result<(), String> {
        if let Ok(at) = u32::try_from(self.flags.len()) {
            if let Some(h) = &m.homonyms {
                self.homonyms.insert(at, h.clone());
            }
            if let Some(c) = m.cpd {
                self.cpd_sets.insert(at, c);
            }
            if let Some(r) = m.rule {
                self.rule_sets.insert(at, r);
            }
        }
        self.note_first(word, m.compound);
        self.words.push_str(word);
        self.offs.push(u32::try_from(self.words.len()).map_err(|_| "stems too large".to_string())?);
        self.flags.push(m.id);
        Ok(())
    }

    /// The id of the flag set `flags` (sorted and deduplicated here), adding it if it is new.
    fn intern(&mut self, ids: &mut FxMap<Vec<u16>, u32>, flags: &mut Vec<u16>) -> Result<u32, String> {
        flags.sort_unstable();
        flags.dedup();
        if let Some(&id) = ids.get(flags.as_slice()) {
            return Ok(id);
        }
        let id = u32::try_from(self.set_offs.len().saturating_sub(1)).map_err(|_| "too many distinct flag sets".to_string())?;
        if self.set_data.len().saturating_add(flags.len()) > MAX_SET_DATA {
            return Err(format!("distinct flag sets hold more than {MAX_SET_DATA} flags in all"));
        }
        self.set_data.extend_from_slice(flags);
        self.set_offs.push(u32::try_from(self.set_data.len()).map_err(|_| "flag sets too large".to_string())?);
        ids.insert(flags.clone(), id);
        Ok(id)
    }

    /// Keep the homonyms `apart` of the stem about to be pushed, each with the exclusions
    /// `carried` (see [`AffixDict::homonyms`]), when there are two or more.
    fn keep_apart(&mut self, ids: &mut FxMap<Vec<u16>, u32>, mut apart: Vec<Vec<u16>>, carried: &[u16], exclusions: &[u16]) -> Result<(), String> {
        if apart.len() < 2 {
            return Ok(());
        }
        let mut sets = Vec::with_capacity(apart.len());
        for set in &mut apart {
            carry_exclusions(set, carried, exclusions);
            sets.push(self.intern(ids, set)?);
        }
        if let Ok(at) = u32::try_from(self.flags.len()) {
            self.homonyms.insert(at, sets.into());
        }
        Ok(())
    }

    /// Build the suffix index and the stem hash table.
    fn index(&mut self) {
        let mut idx: FxMap<Box<str>, Vec<SfxGroup>> = FxMap::default();
        // Each (appended, stripped) pair's group, found by hash rather than by scanning the
        // appended string's groups: a dictionary with many strips per suffix stays linear.
        let mut group_of: FxMap<(&str, &str), usize> = FxMap::default();
        let mut by_flag: Vec<Vec<u32>> = vec![Vec::new(); self.flag_keys.len()];
        for (i, r) in self.suffixes.iter().enumerate() {
            let groups = idx.entry(r.add.clone()).or_default();
            let Ok(i) = u32::try_from(i) else { break };
            let g = match group_of.get(&(&*r.add, &*r.strip)).copied() {
                Some(g) => g,
                None => {
                    group_of.insert((&r.add, &r.strip), groups.len());
                    groups.push(SfxGroup { strip: r.strip.clone(), ..SfxGroup::default() });
                    groups.len() - 1
                }
            };
            if let Some(g) = groups.get_mut(g) {
                g.rules.push(i);
                if r.cont != 0 {
                    g.cont_rules.push(i);
                }
            }
            if let Some(list) = by_flag.get_mut(usize::from(r.flag)) {
                list.push(i);
            }
        }
        self.sfx_by_flag = by_flag;
        let mut pfx_index: FxMap<Box<str>, Vec<u32>> = FxMap::default();
        for (i, r) in self.prefixes.iter().enumerate() {
            if let Ok(i) = u32::try_from(i) {
                pfx_index.entry(r.add.clone()).or_default().push(i);
            }
        }
        self.pfx_index = pfx_index;
        let mut mixed: FxMap<Box<str>, Vec<u32>> = FxMap::default();
        for i in 0..self.len() {
            if let (Some(w), Ok(i)) = (self.word(i), u32::try_from(i))
                && casing(w) == Casing::Mixed
            {
                mixed.entry(w.to_lowercase().into()).or_default().push(i);
            }
        }
        self.mixed = mixed;
        self.max_pfx_chars = self.prefixes.iter().map(|r| r.add.chars().count()).max().unwrap_or(0);
        let mut cont_flag = vec![false; self.flag_keys.len()];
        for r in self.prefixes.iter().chain(&self.suffixes) {
            for &f in self.set(r.cont) {
                if let Some(c) = cont_flag.get_mut(usize::from(f)) {
                    *c = true;
                }
            }
        }
        self.has_cont = cont_flag.contains(&true);
        for g in idx.values_mut().flatten() {
            let outer = |i: &&u32| self.suffixes.get(**i as usize).is_some_and(|r| cont_flag.get(usize::from(r.flag)).copied().unwrap_or(false));
            g.outer_rules = g.rules.iter().filter(outer).copied().collect();
        }
        self.cont_flag = cont_flag;
        self.max_sfx_chars = self.suffixes.iter().map(|r| r.add.chars().count()).max().unwrap_or(0);
        self.sfx_index = idx;
        let n = self.len();
        let size = (n.max(1) * 2).next_power_of_two();
        let mut table = vec![EMPTY; size];
        let mask = size - 1;
        for i in 0..n {
            let Some(w) = self.word(i) else { continue };
            let mut slot = hash_str(w) as usize & mask;
            while table.get(slot).is_some_and(|&t| t != EMPTY) {
                slot = (slot + 1) & mask;
            }
            if let (Some(t), Ok(i)) = (table.get_mut(slot), u32::try_from(i)) {
                *t = i;
            }
        }
        self.table = table;
    }
}
