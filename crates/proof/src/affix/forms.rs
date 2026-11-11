//! Form generation: every word a stem accepts, within a work budget.

use super::*;

impl AffixDict {
    /// Every form of the stem `stem` (itself included), unsorted, possibly with repeats; at most
    /// [`MAX_FORMS`], and at most `MAX_FORMS_WORK` rules tried along the continuation chains and
    /// cross products, so a dictionary with thousands of self-continuing rules can't make one stem
    /// cost millions of strings (#3613).
    pub fn forms(&self, stem: &str) -> Vec<String> {
        let mut budget = MAX_FORMS_WORK;
        self.forms_spending(stem, &mut budget)
    }

    /// [`AffixDict::forms`], with the rules it tries taken off `budget` (several stems' forms
    /// share one budget: the suggestions' scan of 48 stems in a dictionary of 12,000 rules).
    pub(crate) fn forms_spending(&self, stem: &str, budget: &mut usize) -> Vec<String> {
        let Some(f) = self.stem_flags(stem) else { return Vec::new() };
        let start = *budget;
        let out = self.forms_within(stem, f, (*budget).min(MAX_FORMS_WORK));
        *budget = start.saturating_sub(out.1);
        out.0
    }

    /// The forms of `stem` (flags `f`) in at most `limit` rules tried, and the rules tried.
    fn forms_within(&self, stem: &str, f: &[u16], limit: usize) -> (Vec<String>, usize) {
        let mut out = Vec::new();
        if !self.need_affix.is_some_and(|n| has(f, n)) {
            out.push(stem.to_string());
        }
        // Suffix forms a prefix may be added to: the form, whether its suffix is a circumfix one,
        // whether it needs another affix, and its continuation classes (which may allow a prefix
        // the stem doesn't).
        let mut cross_sfx: Vec<(String, bool, bool, u32)> = Vec::new();
        // Suffix forms queued with the continuation classes they carry, for chained expansion;
        // each (form, classes) pair is expanded once.
        let mut chain: Vec<(String, u32)> = Vec::new();
        let mut queued: std::collections::HashSet<(String, u32)> = std::collections::HashSet::new();
        let mut work = 0usize;
        for r in &self.suffixes {
            if !has(f, r.flag) {
                continue;
            }
            work += 1;
            if work > limit {
                return (out, work);
            }
            if !r.cond_matches(stem, true) || self.cont_holds(r, self.only_in_compound) {
                continue;
            }
            let Some(base) = stem.strip_suffix(&*r.strip) else { continue };
            if base.is_empty() && !self.full_strip {
                continue;
            }
            let form = format!("{base}{}", r.add);
            let circ = self.cont_holds(r, self.circumfix);
            if r.cross {
                cross_sfx.push((form.clone(), circ, self.cont_holds(r, self.need_affix), r.cont));
            }
            // A circumfix suffix needs its prefix, so what it ends is never chained without one.
            if r.cont != 0 && !circ && queued.insert((form.clone(), r.cont)) {
                chain.push((form.clone(), r.cont));
            }
            // A circumfix suffix or one that needs another affix is no word on its own.
            // (An empty form still feeds the cross products and chains above; it is no word.)
            if !circ && !self.cont_holds(r, self.need_affix) {
                out.extend(non_empty(form));
            }
            if out.len() >= MAX_FORMS {
                return (out, work);
            }
        }
        // Continuation classes chain suffixes (`workable/B` → `workables`), up to [`MAX_CHAIN`]
        // suffixes in all — the same bound the checker walks.
        'chain: for _ in 0..INNER_CHAIN {
            let level = std::mem::take(&mut chain);
            if level.is_empty() {
                break;
            }
            for (form, flags) in level {
                for r in self.suffixes_with(flags).into_iter().filter_map(|i| self.suffixes.get(i as usize)) {
                    work += 1;
                    if work > limit {
                        break 'chain;
                    }
                    if !r.cond_matches(&form, true) || self.cont_holds(r, self.only_in_compound) {
                        continue;
                    }
                    let Some(base) = form.strip_suffix(&*r.strip) else { continue };
                    if base.is_empty() && !self.full_strip {
                        continue;
                    }
                    let next = format!("{base}{}", r.add);
                    if r.cont != 0 && queued.insert((next.clone(), r.cont)) {
                        chain.push((next.clone(), r.cont));
                    }
                    // The outer suffix of a chain has an affix under it, so its NEEDAFFIX is met,
                    // and Hunspell ignores its CIRCUMFIX there (`workables` from `s/c`).
                    out.extend(non_empty(next));
                    if out.len() >= MAX_FORMS {
                        return (out, work);
                    }
                }
            }
        }
        for r in self.prefixes.iter().filter(|r| !self.cont_holds(r, self.only_in_compound)) {
            let on_stem = has(f, r.flag);
            let circ = self.cont_holds(r, self.circumfix);
            let needs = self.cont_holds(r, self.need_affix);
            if on_stem && !circ && !needs {
                out.extend(self.prefixed(r, stem));
            }
            if r.cross {
                // Every suffix form looked at counts against the work budget, matching or not.
                for (s, s_circ, s_needs, s_cont) in &cross_sfx {
                    work += 1;
                    if out.len() >= MAX_FORMS || work > limit {
                        return (out, work);
                    }
                    // The prefix's flag comes from the stem or the suffix's continuation classes;
                    // circumfix alike; not both in need of another affix.
                    if !(on_stem || has(self.set(*s_cont), r.flag)) || *s_circ != circ || (needs && *s_needs) {
                        continue;
                    }
                    out.extend(self.prefixed(r, s));
                }
            }
            if out.len() >= MAX_FORMS {
                return (out, work);
            }
        }
        (out, work)
    }

    /// `root` with the prefix rule `r` added, when the checker takes it back off: the rule's
    /// condition holds for `root` (the stem, or a suffix form in a cross product), and something
    /// stays after the strip unless `FULLSTRIP`.
    fn prefixed(&self, r: &Rule, root: &str) -> Option<String> {
        let rest = root.strip_prefix(&*r.strip)?;
        if (rest.is_empty() && !self.full_strip) || !r.cond_matches(root, false) {
            return None;
        }
        non_empty(format!("{}{rest}", r.add))
    }

    /// The suffix rules carrying any flag of the set `set`, in rule order.
    fn suffixes_with(&self, set: u32) -> Vec<u32> {
        let mut rules: Vec<u32> = Vec::new();
        let mut lists = 0;
        for &flag in self.set(set) {
            if let Some(list) = self.sfx_by_flag.get(usize::from(flag))
                && !list.is_empty()
            {
                rules.extend_from_slice(list);
                lists += 1;
            }
        }
        if lists > 1 {
            rules.sort_unstable();
        }
        rules
    }
}
