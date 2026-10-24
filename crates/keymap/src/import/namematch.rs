//! Fuzzy matching of a name another application's file writes (a shortcut set's item label)
//! against an app's commands, by their ids and labels only. A match is proposed when one candidate
//! scores well and clearly ahead of the next; otherwise the best few are offered to choose from.
//!
//! From VectorCraft `crates/engine/src/namematch.rs`: Dice coefficients over words and over
//! character pairs.

use std::collections::HashMap;

/// A command a name can stand for.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// A command id such as `select.all`; a `tool:` or `panel:` prefix is ignored.
    pub id: String,
    /// Its labels: the English one, the one in the interface language, other names.
    pub labels: Vec<String>,
}

/// What a name stands for, as far as [`propose`] can tell.
#[derive(Clone, Debug, PartialEq)]
pub enum Proposal {
    /// The candidate at this index of the list, with its score (0 to 1).
    Match(usize, f64),
    /// The best candidates, best first, when none stands out.
    Choice(Vec<usize>),
}

/// The lowest score [`Proposal::Match`] proposes.
pub const THRESHOLD: f64 = 0.6;
/// How far the best score must lead the next one to be proposed.
pub const MARGIN: f64 = 0.1;
/// The most candidates a [`Proposal::Choice`] lists.
pub const CHOICES: usize = 5;
/// The lowest score a [`Proposal::Choice`] lists.
pub const CHOICE_FLOOR: f64 = 0.3;
/// How many characters of a name or label are compared; the rest is ignored, so a long name
/// costs no more than this.
pub const MAX_NAME_CHARS: usize = 200;

/// The first [`MAX_NAME_CHARS`] characters of `s`.
fn head(s: &str) -> &str {
    s.char_indices().nth(MAX_NAME_CHARS).and_then(|(end, _)| s.get(..end)).unwrap_or(s)
}

/// `s` with only its letters and digits, lowercased.
fn folded(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The words of `s`, lowercased: runs of letters and digits, split where a lowercase letter or a digit
/// meets an uppercase one (`selectAll`: `select`, `all`).
fn words(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut word = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if !c.is_alphanumeric() {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// The words of a candidate's id: its segments after any `tool:` or `panel:`, split as [`words`].
fn id_words(id: &str) -> Vec<String> {
    let id = id.strip_prefix("tool:").or_else(|| id.strip_prefix("panel:")).unwrap_or(id);
    id.split('.').flat_map(words).collect()
}

/// How many words `a` and `b` share, each word counted once, as a Dice coefficient (0 to 1).
fn word_overlap(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut rest: Vec<&String> = b.iter().collect();
    let mut shared = 0usize;
    for w in a {
        if let Some(i) = rest.iter().position(|x| *x == w) {
            rest.swap_remove(i);
            shared += 1;
        }
    }
    (2 * shared) as f64 / (a.len() + b.len()) as f64
}

/// The pairs of neighboring characters in `s`.
fn pairs(s: &str) -> Vec<(char, char)> {
    let c: Vec<char> = s.chars().collect();
    c.windows(2).filter_map(|w| Some((*w.first()?, *w.get(1)?))).collect()
}

/// How many character pairs `a` and `b` (each sorted) share, as a Dice coefficient (0 to 1): for
/// names written without spaces. Linear in their lengths, so long names in a file stay quick.
fn pair_similarity(a: &[(char, char)], b: &[(char, char)]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let (mut i, mut j, mut shared) = (0usize, 0usize, 0usize);
    while let (Some(p), Some(q)) = (a.get(i), b.get(j)) {
        match p.cmp(q) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
        }
    }
    (2 * shared) as f64 / (a.len() + b.len()) as f64
}

/// A name or label as the scores compare it: its letters and digits (lowercased), its words and the
/// character pairs of the former (sorted).
struct Text {
    folded: String,
    words: Vec<String>,
    pairs: Vec<(char, char)>,
}

impl Text {
    fn of(s: &str) -> Self {
        let s = head(s);
        let folded = folded(s);
        let mut pairs = pairs(&folded);
        pairs.sort_unstable();
        Self { pairs, words: words(s), folded }
    }
}

/// A candidate as the scores compare it.
struct Prepared {
    /// Its id's words, and those joined.
    id_words: Vec<String>,
    id: String,
    labels: Vec<Text>,
}

impl Prepared {
    fn of(c: &Candidate) -> Self {
        let id_words = id_words(&c.id);
        Self { id: id_words.concat(), id_words, labels: c.labels.iter().map(|l| Text::of(l)).collect() }
    }

    /// See [`score`].
    fn score(&self, name: &Text) -> f64 {
        if name.folded.is_empty() {
            return 0.0;
        }
        if name.folded == self.id || self.labels.iter().any(|l| l.folded == name.folded) {
            return 1.0;
        }
        let by_words = self.labels.iter().map(|l| word_overlap(&name.words, &l.words)).fold(word_overlap(&name.words, &self.id_words), f64::max);
        let by_pairs = self.labels.iter().map(|l| pair_similarity(&name.pairs, &l.pairs)).fold(0.0, f64::max);
        by_words.max(by_pairs * 0.9)
    }
}

/// How well `name` fits `candidate` (0 to 1): 1 when it equals a label or the id, its letters and
/// digits compared without case; else the best of word overlap and character-pair similarity, the
/// latter counted a little lower. Only the first [`MAX_NAME_CHARS`] characters of the name and
/// of each label count.
pub fn score(name: &str, candidate: &Candidate) -> f64 {
    Prepared::of(candidate).score(&Text::of(name))
}

/// The candidate `name` stands for, when one scores at least 0.6 and 0.1 ahead of every other; else
/// the best five that score at least 0.3, to choose from.
pub fn propose(name: &str, candidates: &[Candidate]) -> Proposal {
    Matcher::new(candidates).propose(name)
}

/// [`propose`] for many names against the same candidates: each candidate's words and character
/// pairs are worked out once, and a name is scored only against the candidates that share a word or
/// a character pair with it (the others score 0).
pub struct Matcher {
    prepared: Vec<Prepared>,
    /// Candidates by their joined id words and their labels' letters and digits.
    exact: HashMap<String, Vec<usize>>,
    /// Candidates by a word of their id or labels.
    by_word: HashMap<String, Vec<usize>>,
    /// Candidates by a character pair of their labels.
    by_pair: HashMap<(char, char), Vec<usize>>,
}

impl Matcher {
    /// Prepares `candidates` once for many [`Matcher::propose`] calls.
    pub fn new(candidates: &[Candidate]) -> Self {
        let prepared: Vec<Prepared> = candidates.iter().map(Prepared::of).collect();
        let (mut exact, mut by_word, mut by_pair) = (HashMap::new(), HashMap::new(), HashMap::new());
        fn add<K: std::hash::Hash + Eq>(map: &mut HashMap<K, Vec<usize>>, key: K, i: usize) {
            let list = map.entry(key).or_default();
            if list.last() != Some(&i) {
                list.push(i);
            }
        }
        for (i, p) in prepared.iter().enumerate() {
            add(&mut exact, p.id.clone(), i);
            for l in &p.labels {
                add(&mut exact, l.folded.clone(), i);
            }
            for w in p.id_words.iter().chain(p.labels.iter().flat_map(|l| &l.words)) {
                add(&mut by_word, w.clone(), i);
            }
            for pair in p.labels.iter().flat_map(|l| &l.pairs) {
                add(&mut by_pair, *pair, i);
            }
        }
        Self { prepared, exact, by_word, by_pair }
    }

    /// See [`propose`].
    pub fn propose(&self, name: &str) -> Proposal {
        let text = Text::of(name);
        let mut seen = vec![false; self.prepared.len()];
        let lists = self
            .exact
            .get(&text.folded)
            .into_iter()
            .chain(text.words.iter().filter_map(|w| self.by_word.get(w)))
            // Each pair looked up once (they are sorted).
            .chain(text.pairs.chunk_by(|a, b| a == b).filter_map(|c| self.by_pair.get(c.first()?)));
        let mut scored: Vec<(usize, f64)> = vec![];
        for &i in lists.flatten() {
            if let Some(s) = seen.get_mut(i)
                && !*s
            {
                *s = true;
                let score = self.prepared.get(i).map_or(0.0, |p| p.score(&text));
                if score >= CHOICE_FLOOR {
                    scored.push((i, score));
                }
            }
        }
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        match (scored.first(), scored.get(1)) {
            (Some(&(i, best)), next) if best >= THRESHOLD && next.is_none_or(|n| best - n.1 >= MARGIN) => Proposal::Match(i, best),
            _ => Proposal::Choice(scored.iter().take(CHOICES).map(|(i, _)| *i).collect()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(list: &[(&str, &str)]) -> Vec<Candidate> {
        list.iter().map(|(id, label)| Candidate { id: id.to_string(), labels: vec![label.to_string()] }).collect()
    }

    fn proposed<'a>(name: &str, list: &'a [Candidate]) -> Option<&'a str> {
        match propose(name, list) {
            Proposal::Match(i, _) => list.get(i).map(|c| c.id.as_str()),
            Proposal::Choice(_) => None,
        }
    }

    #[test]
    fn extra_words_still_find_the_tool() {
        let tools = candidates(&[
            ("tool:rectangle", "Rectangle Tool"),
            ("tool:roundedRectangle", "Rounded Rectangle Tool"),
            ("tool:rectangularGrid", "Rectangular Grid Tool"),
            ("tool:ellipse", "Ellipse Tool"),
        ]);
        assert_eq!(proposed("Acme Rectangle Tool", &tools), Some("tool:rectangle"));
        assert_eq!(proposed("Rounded Rectangle", &tools), Some("tool:roundedRectangle"));
        assert_eq!(proposed("ellipse tool", &tools), Some("tool:ellipse"), "case aside, the label itself");
    }

    #[test]
    fn letters_only_names_match_ids() {
        let commands = candidates(&[("acme.copyAll", "Everything"), ("acme.copyAllOnPage", "Everything on This Page"), ("acme.clear", "Clear Out")]);
        assert_eq!(proposed("acmecopyall", &commands), Some("acme.copyAll"));
        assert_eq!(proposed("Acme Copy All On Page", &commands), Some("acme.copyAllOnPage"));
        assert_eq!(proposed("clear out", &commands), Some("acme.clear"));
    }

    #[test]
    fn ties_need_a_choice() {
        let tools = candidates(&[("tool:rectangle", "Rectangle Tool"), ("tool:ellipse", "Ellipse Tool"), ("tool:star", "Star Tool")]);
        assert_eq!(propose("Tool", &tools), Proposal::Choice(vec![0, 1, 2]));
        assert_eq!(propose("Nothing Alike", &tools), Proposal::Choice(vec![]));
        assert_eq!(propose("", &tools), Proposal::Choice(vec![]));
    }

    #[test]
    fn names_without_spaces_match_by_character_pairs() {
        let list = vec![
            Candidate { id: "tool:rectangle".into(), labels: vec!["Rectangle Tool".into(), "長方形ツール".into()] },
            Candidate { id: "tool:ellipse".into(), labels: vec!["Ellipse Tool".into(), "楕円形ツール".into()] },
        ];
        assert_eq!(proposed("長方形ツール", &list), Some("tool:rectangle"));
        assert_eq!(proposed("楕円形 ツール", &list), Some("tool:ellipse"), "spaces and punctuation aside");
        assert_eq!(proposed("楕円形ツール2", &list), Some("tool:ellipse"), "most character pairs shared");
    }

    #[test]
    fn the_index_finds_what_scoring_every_candidate_finds() {
        let list = candidates(&[
            ("tool:rectangle", "Rectangle Tool"),
            ("tool:roundedRectangle", "Rounded Rectangle Tool"),
            ("tool:ellipse", "Ellipse Tool"),
            ("select.all", "All"),
            ("select.none", "Deselect"),
            ("edit.undo", "Undo"),
            ("foo.bar", "Zzz"),
            ("panel:layers", "Layers"),
        ]);
        let matcher = Matcher::new(&list);
        for name in ["Acme Rectangle Tool", "rounded", "acmecopyall", "foobar", "zz", "Lay", "Undo Redo", "ツール", "", "e"] {
            let mut scored: Vec<(usize, f64)> =
                list.iter().enumerate().map(|(i, c)| (i, score(name, c))).filter(|(_, s)| *s >= CHOICE_FLOOR).collect();
            scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            let every = match (scored.first(), scored.get(1)) {
                (Some(&(i, best)), next) if best >= THRESHOLD && next.is_none_or(|n| best - n.1 >= MARGIN) => Proposal::Match(i, best),
                _ => Proposal::Choice(scored.iter().take(CHOICES).map(|(i, _)| *i).collect()),
            };
            assert_eq!(matcher.propose(name), every, "{name}");
        }
        assert_eq!(proposed("foobar", &list), Some("foo.bar"), "the id alone, sharing no word or pair with the label");
    }

    #[test]
    fn only_the_head_of_a_long_name_counts() {
        let label: String = "ä€𝄞b".repeat(50);
        let candidate = Candidate { id: "acme.long".into(), labels: vec![label.clone()] };
        assert_eq!(head(&label).chars().count(), MAX_NAME_CHARS);
        assert_eq!(score(&format!("{label}and more words after the head"), &candidate), 1.0);
        assert_eq!(head("short"), "short");
    }

    #[test]
    fn words_split_camel_case_and_separators() {
        assert_eq!(words("selectAll_onArtboard 2x"), ["select", "all", "on", "artboard", "2x"]);
        assert_eq!(words("Acme\u{a0}Lasso-Tool"), ["acme", "lasso", "tool"]);
        assert_eq!(id_words("tool:roundedRectangle"), ["rounded", "rectangle"]);
    }
}
