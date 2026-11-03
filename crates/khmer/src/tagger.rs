//! Aksara feature-set v3 averaged perceptron, ported without changing features.
use std::{collections::HashMap, fmt::Write, sync::OnceLock};
fn weights() -> &'static HashMap<&'static str, f64> {
    static WEIGHTS: OnceLock<HashMap<&'static str, f64>> = OnceLock::new();
    WEIGHTS.get_or_init(|| {
        super::assets::weights()
            .lines()
            .filter_map(|line| {
                let (feature, weight) = line.rsplit_once('\t')?;
                Some((feature, weight.parse().ok()?))
            })
            .collect()
    })
}
fn band(f: u64) -> &'static str {
    match f {
        0 => "0",
        1 => "demoted",
        2..100 => "rare",
        100..2000 => "mid",
        2000..20000 => "high",
        _ => "top",
    }
}
fn bucket(n: usize) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        2 => "2",
        3..=4 => "3-4",
        5..=6 => "5-6",
        _ => "7+",
    }
}
// Reuse one feature buffer instead of allocating a string for every feature.
// Keep additions in exactly the original order: rounding near zero can change
// the boundary decision even when the mathematical sum is the same.
struct Score {
    weights: &'static HashMap<&'static str, f64>,
    scratch: String,
    total: f64,
}
impl Score {
    fn add(&mut self, feature: std::fmt::Arguments<'_>) {
        self.scratch.clear();
        // Writing to String is infallible; no input-derived indexing is used.
        let _ = self.scratch.write_fmt(feature);
        if let Some(weight) = self.weights.get(self.scratch.as_str()) {
            self.total += weight;
        }
    }
}
// Each neighboring gap asks about many of the same contiguous word spans.
// A fixed seven-row ring keeps those exact frequency results, bounded regardless
// of paragraph length. A tagger belongs to one run, so offsets never alias text.
struct ProbeRow {
    start: usize,
    values: [Option<u64>; 12],
}
impl Default for ProbeRow {
    fn default() -> Self {
        Self { start: usize::MAX, values: [None; 12] }
    }
}
#[derive(Default)]
struct Probes {
    rows: [ProbeRow; 7],
}
impl Probes {
    fn frequency(&mut self, text: &str, edges: &[usize], start: usize, end: usize) -> u64 {
        let Some(width) = end.checked_sub(start).filter(|n| (1..=12).contains(n)) else { return 0 };
        let Some(row) = self.rows.get_mut(start % 7) else { return 0 };
        if row.start != start {
            *row = ProbeRow { start, ..Default::default() };
        }
        let Some(slot) = row.values.get_mut(width - 1) else { return 0 };
        *slot.get_or_insert_with(|| edges.get(start).zip(edges.get(end)).and_then(|(&from, &to)| text.get(from..to)).map_or(0, super::frequency))
    }
}

pub(super) struct Tagger {
    score: Score,
    probes: Probes,
}
impl Tagger {
    pub(super) fn new() -> Self {
        Self { score: Score { weights: weights(), scratch: String::new(), total: -0.0 }, probes: Probes::default() }
    }
    pub(super) fn boundary(&mut self, text: &str, edges: &[usize], clusters: &[&str], gap: usize) -> bool {
        self.score(text, edges, clusters, gap) > 0.0
    }
    fn score(&mut self, text: &str, edges: &[usize], clusters: &[&str], gap: usize) -> f64 {
        let mut frequency = |start: usize, end: usize| self.probes.frequency(text, edges, start, end);
        let at = |offset: isize| gap.checked_add_signed(offset).and_then(|i| clusters.get(i)).copied().unwrap_or("<edge>");
        let (m3, m2, m1, p0, p1, p2) = (at(-3), at(-2), at(-1), at(0), at(1), at(2));
        let score = &mut self.score;
        score.total = -0.0;
        score.add(format_args!("bias"));
        score.add(format_args!("a-3={m3}"));
        score.add(format_args!("a-2={m2}"));
        score.add(format_args!("a-1={m1}"));
        score.add(format_args!("a+0={p0}"));
        score.add(format_args!("a+1={p1}"));
        score.add(format_args!("a+2={p2}"));
        score.add(format_args!("b-2={m2}|{m1}"));
        score.add(format_args!("bx={m1}|{p0}"));
        score.add(format_args!("b+1={p0}|{p1}"));
        score.add(format_args!("t-={m2}|{m1}|{p0}"));
        score.add(format_args!("t+={m1}|{p0}|{p1}"));
        score.add(format_args!("q={m2}|{m1}|{p0}|{p1}"));
        score.add(format_args!("pos={}|{}", bucket(gap), bucket(clusters.len().saturating_sub(gap))));
        let (mut left, mut right) = (0, 0);
        let (mut left_band, mut right_band) = ("0", "0");
        for n in 1..=6 {
            if let Some(start) = gap.checked_sub(n) {
                let f = frequency(start, gap);
                if f > 0 {
                    left = n;
                    left_band = band(f);
                    score.add(format_args!("L{n}"));
                    score.add(format_args!("L{n}f={left_band}"));
                }
            }
            if gap.saturating_add(n) <= clusters.len() {
                let f = frequency(gap, gap.saturating_add(n));
                if f > 0 {
                    right = n;
                    right_band = band(f);
                    score.add(format_args!("R{n}"));
                    score.add(format_args!("R{n}f={right_band}"));
                }
            }
        }
        score.add(format_args!("Lf={left_band}"));
        score.add(format_args!("Rf={right_band}"));
        score.add(format_args!("LRf={left_band}|{right_band}"));
        score.add(format_args!("Lmax={left}"));
        score.add(format_args!("Rmax={right}"));
        score.add(format_args!("LR={left}|{right}"));
        if left > 0 && right > 0 {
            score.add(format_args!("both"));
        }
        if left == 0 && right == 0 {
            score.add(format_args!("neither"));
        }
        let mut straddles = false;
        'search: for l in 1..=6 {
            let Some(start) = gap.checked_sub(l) else { break };
            for r in 1..=6 {
                if gap.saturating_add(r) > clusters.len() {
                    break;
                }
                let f = frequency(start, gap.saturating_add(r));
                if f > 0 {
                    let band = band(f);
                    score.add(format_args!("X={l}|{r}"));
                    score.add(format_args!("Xf={band}"));
                    score.add(format_args!("Xf={band}|{left_band}|{right_band}"));
                    straddles = true;
                    break 'search;
                }
            }
        }
        if straddles {
            score.add(format_args!("straddled"));
        }
        if straddles && left > 0 && right > 0 {
            score.add(format_args!("contested"));
        }
        score.total
    }
}

#[cfg(all(test, craft_khmer_data))]
fn boundary_score(text: &str, edges: &[usize], clusters: &[&str], gap: usize) -> f64 {
    Tagger::new().score(text, edges, clusters, gap)
}

#[cfg(all(test, craft_khmer_data))]
mod tests {
    #[test]
    fn every_weight_preserves_its_exact_f64_bits() {
        let weights = super::weights();
        assert_eq!(weights.len(), 39_127);
        for line in include_str!(concat!(env!("OUT_DIR"), "/kcc-weights.tsv")).lines() {
            let (feature, number) = line.rsplit_once('\t').unwrap();
            assert_eq!(weights.get(feature).unwrap().to_bits(), number.parse::<f64>().unwrap().to_bits(), "{feature}");
        }
    }
}

#[cfg(all(test, craft_khmer_data))]
#[path = "tagger_baseline.rs"]
mod baseline;
#[cfg(all(test, craft_khmer_data))]
#[test]
fn boundaries_match_frozen_baseline() {
    for text in ["ភាសាខ្មែរ", "សួស្តីពិភពលោក", "កម្ពុជាប្រទេសខ្ញុំ", "abc", ""]
    {
        let edges = crate::kcc_edges(text);
        let clusters: Vec<_> = edges.windows(2).filter_map(|w| text.get(*w.first()?..*w.get(1)?)).collect();
        for gap in 1..clusters.len() {
            assert_eq!(boundary_score(text, &edges, &clusters, gap).to_bits(), baseline::boundary_score(&clusters, gap).to_bits());
        }
    }
}

#[cfg(all(test, craft_khmer_data))]
#[test]
fn score_bits_match_baseline_on_deterministic_dictionary_sample() {
    let mut words: Vec<_> = crate::frequencies().keys().copied().collect();
    words.sort_unstable();
    for word in words.into_iter().step_by(73) {
        let text = format!("ខ្មែរ{word}ភាសា");
        let edges = crate::kcc_edges(&text);
        let clusters: Vec<_> = edges.windows(2).filter_map(|w| text.get(*w.first()?..*w.get(1)?)).collect();
        for gap in 1..clusters.len() {
            assert_eq!(
                boundary_score(&text, &edges, &clusters, gap).to_bits(),
                baseline::boundary_score(&clusters, gap).to_bits(),
                "{text} at {gap}"
            );
        }
    }
}

#[cfg(all(test, craft_khmer_data))]
#[test]
fn score_bits_match_every_frequency_word_and_context() {
    let mut words: Vec<_> = crate::frequencies().keys().copied().collect();
    words.sort_unstable();
    for word in words {
        for text in [word.to_owned(), format!("ខ្មែរ{word}ភាសា")] {
            let edges = crate::kcc_edges(&text);
            let clusters: Vec<_> = edges.windows(2).filter_map(|w| text.get(*w.first()?..*w.get(1)?)).collect();
            let mut tagger = Tagger::new();
            for gap in 1..clusters.len() {
                assert_eq!(
                    tagger.score(&text, &edges, &clusters, gap).to_bits(),
                    baseline::boundary_score(&clusters, gap).to_bits(),
                    "{text} at {gap}"
                );
            }
        }
    }
}
