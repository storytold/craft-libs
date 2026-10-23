//! Breaker tests on a monospaced model: letters are 6 units wide, spaces 3, hyphens 4. Parameter
//! sets mirror vectorcraft's and designcraft's composers.

use super::*;

const CHAR: f64 = 6.0;
const SPACE: f64 = 3.0;
const HYPHEN: f64 = 4.0;

const COPY: &str = "Typography is the craft of arranging type to make written language legible, readable and appealing when displayed. \
The arrangement of type involves selecting typefaces, point sizes, line lengths, line spacing and letter spacing, and adjusting the space \
between pairs of letters.";

/// A paragraph's items and the text each item stands for.
struct Para {
    items: Vec<Item>,
    text: Vec<String>,
}

impl Para {
    /// A box per word and glue per space (elastic unless ragged); with `hyphen_cost`, a
    /// hyphenation point every two letters inside words of six letters or more. Ends with fill and
    /// a forced break.
    fn new(text: &str, p: &Params, hyphen_cost: Option<f64>) -> Para {
        let mut para = Para { items: Vec::new(), text: Vec::new() };
        for (i, word) in text.split(' ').enumerate() {
            if i > 0 {
                let glue = if p.rag.is_some() { Item::glue(SPACE, 0.0, 0.0) } else { Item::glue(SPACE, SPACE / 2.0, SPACE / 5.0) };
                para.push(glue, " ");
            }
            let chars: Vec<char> = word.chars().collect();
            let points: Vec<usize> = match hyphen_cost {
                Some(_) if chars.len() >= 6 => (2..chars.len() - 2).step_by(2).collect(),
                _ => Vec::new(),
            };
            let mut from = 0;
            for &at in points.iter().chain([chars.len()].iter()) {
                let piece: String = chars[from..at].iter().collect();
                para.push(Item::boxed(width_of(&piece)), &piece);
                if at < chars.len() {
                    para.push(Item { hyphenation: true, ..Item::penalty(HYPHEN, hyphen_cost.unwrap_or(0.0), true) }, "");
                }
                from = at;
            }
        }
        para.push(Item::fill(), "");
        para.push(Item::forced(), "");
        para
    }

    fn push(&mut self, item: Item, text: &str) {
        self.items.push(item);
        self.text.push(text.to_string());
    }

    fn lines(&self, lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                let s: String = self.text[l.start..l.end.min(self.text.len())].concat();
                let s = s.trim_end().to_string();
                if l.flagged { s + "-" } else { s }
            })
            .collect()
    }
}

fn width_of(s: &str) -> f64 {
    s.chars().map(|c| if c == ' ' { SPACE } else { CHAR }).sum()
}

fn vectorcraft(ragged: bool) -> Params {
    Params {
        fitness_demerits: if ragged { 1000.0 } else { 10_000.0 },
        fitness: if ragged { [0.15, 0.4, 0.75] } else { [-0.5, 0.5, 1.0] },
        first_fitness: None,
        max_badness: 10_000.0,
        rag: ragged.then_some(Rag {
            stretch: 0.0,
            width_fraction: 1.0 / 6.0,
            short_last: 1.0 / 3.0,
            short_last_unit: 1.0 / 3.0,
            hyphenation_zone: 0.0,
        }),
        ..Params::default()
    }
}

/// vectorcraft's every-line composer: a strict pass, then a loose one, then single-line.
fn every_line(items: &[Item], width: f64, ragged: bool) -> Vec<Line> {
    let p = vectorcraft(ragged);
    let w = |_| width;
    let loose = if ragged { f64::INFINITY } else { 4.0 };
    knuth_plass(items, &w, &Params { tolerance: 1.0, ..p })
        .or_else(|| knuth_plass(items, &w, &Params { tolerance: loose, ..p }))
        .unwrap_or_else(|| greedy(items, &w, &p))
}

fn designcraft(ragged: bool) -> Params {
    Params {
        hyphen_limit: 3,
        overrun: 0.01,
        uniform_from: 0,
        rag: ragged.then_some(Rag { stretch: 24.0, width_fraction: 0.0, ..Rag::default() }),
        ..Params::default()
    }
}

const PASSES: [f64; 3] = [2.5, 12.0, f64::INFINITY];

/// Lines are contiguous and cover every item.
fn assert_covers(items: &[Item], lines: &[Line]) {
    assert_eq!(lines.first().map(|l| l.start), Some(0));
    for pair in lines.windows(2) {
        assert_eq!(pair[0].next, pair[1].start, "{lines:?}");
    }
    assert_eq!(lines.last().map(|l| l.next), Some(items.len()));
}

#[test]
fn tiers_are_used_in_order() {
    let cost = Params::default().tier_cost;
    // Word stretch alone.
    let (r, used, _, ok) = tiered(5.0, &[10.0, 10.0, 10.0], 10.0, &cost);
    assert!(ok && (r - 0.5).abs() < 1e-9 && used == [0.5, 0.0, 0.0]);
    // Word saturated, half the letter tier.
    let (r, used, _, _) = tiered(15.0, &[10.0, 10.0, 10.0], 10.0, &cost);
    assert!((r - (1.0 + 0.5 * cost[1])).abs() < 1e-9 && used == [1.0, 0.5, 0.0]);
    // Beyond every tier: unbounded word spacing.
    let (r, _, excess, _) = tiered(40.0, &[10.0, 10.0, 10.0], 10.0, &cost);
    assert!(r > 2.0 && (excess - 10.0).abs() < 1e-9);
    // Shrink beyond the tiers is infeasible.
    assert!(!tiered(-25.0, &[10.0, 10.0, 0.0], 0.0, &cost).3);
    assert!(tiered(-15.0, &[10.0, 10.0, 0.0], 0.0, &cost).3);
}

#[test]
fn lines_fit_and_cover_the_paragraph() {
    let text = [COPY; 2].join(" ");
    for ragged in [false, true] {
        for hyphen in [None, Some(50.0)] {
            let p = vectorcraft(ragged);
            let para = Para::new(&text, &p, hyphen);
            for (name, lines) in [("every-line", every_line(&para.items, 180.0, ragged)), ("greedy", greedy(&para.items, &|_| 180.0, &p))] {
                assert!(lines.len() > 10, "{name}");
                assert_covers(&para.items, &lines);
                for l in &lines {
                    assert!(l.excess >= -p.overrun, "{name} ragged={ragged} {l:?}");
                    if ragged {
                        assert!(l.width <= 180.0 + 1e-6, "{name} {l:?}");
                    }
                }
                assert!(lines.last().is_some_and(|l| l.forced));
            }
        }
    }
}

/// Every-line breaking spreads the spacing more evenly than greedy breaking.
#[test]
fn every_line_is_more_even_than_greedy() {
    let text = [COPY; 2].join(" ");
    let p = vectorcraft(false);
    let para = Para::new(&text, &p, None);
    for width in [150.0, 190.0, 240.0] {
        let worst = |lines: &[Line]| lines[..lines.len() - 1].iter().map(|l| l.ratio).fold(0.0, f64::max);
        let (kp, gr) = (worst(&every_line(&para.items, width, false)), worst(&greedy(&para.items, &|_| width, &p)));
        assert!(kp <= gr + 1e-9, "{width}: every-line worst ratio {kp} vs greedy {gr}");
    }
}

/// Ragged: every-line breaks before a word that would fit, so the last line is not a lone short
/// word; greedy keeps filling the first line.
#[test]
fn every_line_balances_ragged_lines() {
    const S: &str = "Destroy all creatures with toughness X or less.";
    let width = width_of("Destroy all creatures with toughness X or") + 2.0;
    let p = vectorcraft(true);
    let para = Para::new(S, &p, None);
    assert_eq!(para.lines(&greedy(&para.items, &|_| width, &p)), ["Destroy all creatures with toughness X or", "less."]);
    assert_eq!(para.lines(&every_line(&para.items, width, true)), ["Destroy all creatures with toughness X", "or less."]);
}

/// Every-line evens the rag of a long ragged paragraph: the leftover space is no worse than greedy
/// breaking, with at most one line more. Hyphenated, it trades some rag for fewer hyphens.
#[test]
fn every_line_evens_the_rag() {
    let text = [COPY; 2].join(" ");
    let p = vectorcraft(true);
    for (width, hyphen) in [(150.0, None), (190.0, None), (240.0, Some(25.0)), (310.0, None)] {
        let para = Para::new(&text, &p, hyphen);
        let rag = |lines: &[Line]| {
            for l in lines {
                assert!(l.width <= width + 1e-6, "{width}: {l:?}");
            }
            let slack = lines[..lines.len() - 1].iter().map(|l| (width - l.width).powi(2)).sum::<f64>();
            (slack, lines.len(), lines.iter().filter(|l| l.flagged).count())
        };
        let ((kp, kp_n, kp_hy), (gr, gr_n, gr_hy)) = (rag(&every_line(&para.items, width, true)), rag(&greedy(&para.items, &|_| width, &p)));
        if hyphen.is_some() {
            assert!(kp_hy < gr_hy, "{width}: {kp_hy} hyphens vs greedy {gr_hy}");
        } else {
            assert!(kp <= gr + 1e-6, "{width}: rag {kp} vs greedy {gr}");
        }
        assert!(kp_n <= gr_n + 1, "{width}: {kp_n} lines vs {gr_n}");
    }
}

/// Greedy: each line takes as many words as fit.
#[test]
fn greedy_fills_each_line() {
    let width = 190.0;
    let p = vectorcraft(true);
    let para = Para::new(COPY, &p, None);
    let lines = para.lines(&greedy(&para.items, &|_| width, &p));
    for pair in lines.windows(2) {
        let next_word = pair[1].split(' ').next().unwrap_or_default();
        assert!(width_of(&format!("{} {next_word}", pair[0])) > width, "{pair:?}");
    }
}

#[test]
fn hyphen_limit_is_a_hard_constraint() {
    let text = [COPY; 4].join(" ");
    for limit in [1, 2, 9] {
        let p = Params { hyphen_limit: limit, ..designcraft(false) };
        let para = Para::new(&text, &p, Some(0.0));
        for lines in [break_paragraph(&para.items, &|_| 130.0, &p, &PASSES).lines, greedy(&para.items, &|_| 130.0, &p)] {
            let mut run = 0;
            for l in &lines {
                run = if l.flagged { run + 1 } else { 0 };
                assert!(run <= limit, "{run} consecutive hyphens with limit {limit}");
            }
            assert!(lines.iter().any(|l| l.flagged));
        }
    }
}

#[test]
fn hyphenation_zone_limits_ragged_hyphens() {
    let text = [COPY; 4].join(" ");
    let measure = 110.0;
    let count = |zone: f64, every_line: bool| {
        let base = designcraft(true);
        let p = Params { rag: base.rag.map(|r| Rag { hyphenation_zone: zone, ..r }), ..base };
        let para = Para::new(&text, &p, Some(0.0));
        let lines = if every_line { break_paragraph(&para.items, &|_| measure, &p, &PASSES).lines } else { greedy(&para.items, &|_| measure, &p) };
        // A hyphenated line leaves at least the zone after the last space before its last word.
        for l in lines.iter().filter(|l| l.flagged) {
            if let Some(space) = (l.start..l.end).rfind(|&k| para.items[k].kind == Kind::Glue) {
                let before: f64 = para.items[l.start..space].iter().filter(|i| i.kind != Kind::Penalty).map(|i| i.width).sum();
                assert!(measure - before >= zone, "every_line={every_line} zone {zone}: {:?}", para.lines(&[*l]));
            }
        }
        lines.iter().filter(|l| l.flagged).count()
    };
    for every_line in [true, false] {
        let (free, zoned, huge) = (count(0.0, every_line), count(36.0, every_line), count(1000.0, every_line));
        // A huge zone leaves only words that start a line hyphenable.
        assert!(free > 0 && zoned > 0 && huge * 4 <= free, "every_line={every_line}: {free} {zoned} {huge}");
    }
}

#[test]
fn balanced_lines_even_a_headline() {
    let p = Params { rag: Some(Rag { stretch: 24.0, width_fraction: 0.0, ..Rag::default() }), ..designcraft(true) };
    let para = Para::new("Balanced headlines read better than a stub", &p, None);
    let w = |_| 150.0;
    let plain = break_paragraph(&para.items, &w, &p, &PASSES).lines;
    let bal = balanced(&para.items, &w, &p, &PASSES);
    assert!(bal.balanced);
    let bal = bal.lines;
    assert!(plain.len() >= 2 && plain.len() == bal.len(), "{:?} vs {:?}", para.lines(&plain), para.lines(&bal));
    let spread = |lines: &[Line]| lines.iter().map(|l| l.width).fold(f64::MIN, f64::max) - lines.iter().map(|l| l.width).fold(f64::MAX, f64::min);
    assert!(spread(&bal) < spread(&plain), "{:?} vs {:?}", para.lines(&bal), para.lines(&plain));
    assert!(bal.iter().all(|l| l.measure == 150.0));
}

/// A word wider than the measure breaks at its emergency points, or, for greedy without them,
/// before the box that overflows.
#[test]
fn an_overlong_word_breaks_at_the_measure() {
    let build = |emergency: bool, ragged: bool| {
        let p = designcraft(ragged);
        let glue = if ragged { Item::glue(SPACE, 0.0, 0.0) } else { Item::glue(SPACE, SPACE / 2.0, SPACE / 5.0) };
        let mut items = vec![Item::boxed(4.0 * CHAR), glue];
        for i in 0..60 {
            if i > 0 && emergency {
                items.push(Item::emergency());
            }
            items.push(Item::boxed(CHAR));
        }
        items.extend([glue, Item::boxed(3.0 * CHAR), glue, Item::boxed(2.0 * CHAR), Item::fill(), Item::forced()]);
        (items, p)
    };
    let (plain, _) = build(false, false);
    assert_eq!(overlong(&plain, 72.0), vec![2..62]);
    for ragged in [false, true] {
        let (items, p) = build(true, ragged);
        let (no_points, _) = build(false, ragged);
        for (name, lines) in [
            ("every-line", break_paragraph(&items, &|_| 72.0, &p, &PASSES).lines),
            ("greedy", greedy(&items, &|_| 72.0, &p)),
            ("greedy, no emergency points", greedy(&no_points, &|_| 72.0, &p)),
        ] {
            assert!(lines.len() >= 6, "{name} ragged={ragged}: {lines:?}");
            for l in &lines {
                assert!(l.width <= 72.0 + 0.01, "{name} ragged={ragged}: {l:?}");
            }
        }
    }
}

#[test]
fn forced_breaks_end_lines_and_the_paragraph_end_is_one() {
    let p = designcraft(false);
    // No forced penalty at the end: the paragraph end is the last break.
    let items = [Item::boxed(30.0), Item::glue(SPACE, 1.5, 0.6), Item::boxed(30.0), Item::fill(), Item::forced(), Item::boxed(30.0)];
    for lines in [break_paragraph(&items, &|_| 100.0, &p, &PASSES).lines, greedy(&items, &|_| 100.0, &p)] {
        assert_eq!(lines.iter().map(|l| (l.start, l.end, l.forced)).collect::<Vec<_>>(), [(0, 4, true), (5, 6, true)]);
        // The last line is justified: it has no fill and nothing to stretch.
        assert!(lines[1].ratio.is_infinite());
    }
    // Empty paragraph: one empty line.
    for lines in [break_paragraph(&[], &|_| 100.0, &p, &PASSES).lines, greedy(&[], &|_| 100.0, &p)] {
        assert_eq!(lines.iter().map(|l| (l.start, l.end, l.next)).collect::<Vec<_>>(), [(0, 0, 0)]);
    }
}

/// The measure is per line: a first-line indent narrows line 0 only.
#[test]
fn each_line_gets_its_own_measure() {
    let p = Params { uniform_from: 1, ..vectorcraft(true) };
    let para = Para::new(COPY, &p, None);
    let w = |k: usize| if k == 0 { 120.0 } else { 200.0 };
    for lines in [break_paragraph(&para.items, &w, &p, &[1.0, f64::INFINITY]).lines, greedy(&para.items, &w, &p)] {
        for (k, l) in lines.iter().enumerate() {
            assert_eq!(l.measure, w(k));
            assert!(l.width <= w(k) + 1e-6, "{k}: {l:?}");
        }
        assert!(lines[1].width > 120.0);
    }
}

/// Punctuation hanging into the margin gives its width back: the line fits with the word after it.
#[test]
fn a_hanging_break_gives_its_width_back() {
    let p = designcraft(true);
    let items = |hang: f64| {
        [
            Item::boxed(60.0),
            Item::glue(5.0, 0.0, 0.0),
            Item::boxed(40.0),
            Item { hang, ..Item::glue(5.0, 0.0, 0.0) },
            Item::boxed(30.0),
            Item::fill(),
            Item::forced(),
        ]
    };
    for (hang, end) in [(0.0, 1), (5.0, 3)] {
        let items = items(hang);
        for lines in [break_paragraph(&items, &|_| 100.0, &p, &PASSES).lines, greedy(&items, &|_| 100.0, &p)] {
            assert_eq!(lines[0].end, end, "hang {hang}: {lines:?}");
        }
    }
}

/// Justification uses word spacing first, then letter spacing.
#[test]
fn justification_uses_tiers_in_order() {
    let p = designcraft(false);
    let letters = Item { stretch: [0.0, 10.0, 0.0], ..Item::boxed(40.0) };
    let items = [letters, Item::glue(10.0, 6.0, 0.0), letters, Item::forced()];
    let lines = break_paragraph(&items, &|_| 100.0, &p, &PASSES).lines;
    let l = lines[0];
    assert_eq!(l.tiers, [1.0, 0.2, 0.0]);
    assert!((l.ratio - (1.0 + 0.2 * p.tier_cost[1])).abs() < 1e-9, "{l:?}");
}

/// Letter spacing and glyph scaling absorb what word spacing alone cannot in a narrow column.
#[test]
fn letter_and_glyph_tiers_ease_a_narrow_column() {
    let worst = |box_stretch: f64| {
        let p = designcraft(false);
        let mut para = Para::new(COPY, &p, Some(275.0));
        for it in para.items.iter_mut().filter(|i| i.kind == Kind::Box) {
            it.stretch = [0.0, it.width * box_stretch, it.width * box_stretch];
            it.shrink = [0.0, it.width * box_stretch / 2.0, it.width * box_stretch / 2.0];
        }
        let lines = break_paragraph(&para.items, &|_| 100.0, &p, &PASSES).lines;
        lines[..lines.len() - 1].iter().map(|l| l.ratio).fold(0.0, f64::max)
    };
    let (words, all) = (worst(0.0), worst(0.03));
    assert!(all < words, "{all} vs {words}");
}

/// A long ragged paragraph with a constant measure breaks by Knuth–Plass under the default
/// parameters and effectcraft's infinitely tolerant rag, without reaching the active-break limit.
#[test]
fn a_long_ragged_paragraph_stays_on_knuth_plass() {
    let mut seed = 1u32;
    let mut items = Vec::new();
    for i in 0..5000 {
        if i > 0 {
            items.push(Item::glue(1.0, 0.0, 0.0));
        }
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        items.push(Item::boxed(f64::from(3 + (seed >> 16) % 5)));
    }
    items.extend([Item::fill(), Item::forced()]);
    let default = Params { rag: Some(Rag::default()), ..Params::default() };
    let effectcraft = Params { rag: Some(Rag { width_fraction: 1.0, ..Rag::default() }), ..Params::default() };
    for (p, tolerances) in [(default, &[2.5][..]), (effectcraft, &[f64::INFINITY][..])] {
        let b = break_paragraph(&items, &|_| 60.0, &p, tolerances);
        assert_eq!(b.method, Method::KnuthPlass { pass: 0 }, "{p:?}");
        assert_covers(&items, &b.lines);
    }
}

/// A NaN measure counts as 0 and negative fitness demerits as 0.
#[test]
fn invalid_params_are_clamped() {
    let p = designcraft(false);
    let para = Para::new(COPY, &p, None);
    let lines = break_paragraph(&para.items, &|_| f64::NAN, &p, &PASSES).lines;
    assert_eq!(lines, break_paragraph(&para.items, &|_| 0.0, &p, &PASSES).lines);
    assert!(lines.iter().all(|l| l.measure == 0.0 && !l.excess.is_nan()));
    let negative = Params { fitness_demerits: -5000.0, ..p };
    let zero = Params { fitness_demerits: 0.0, ..p };
    assert_eq!(break_paragraph(&para.items, &|_| 150.0, &negative, &PASSES), break_paragraph(&para.items, &|_| 150.0, &zero, &PASSES));
}

/// Two consecutive breaks: the line after the first starts past the second, which must not end a
/// line `4..3` from there.
#[test]
fn lines_never_end_before_they_start() {
    let items = [
        Item::boxed(10.0),
        Item::glue(1.0, 0.0, 0.0),
        Item::penalty(0.0, -1000.0, false),
        Item::penalty(0.0, -1000.0, false),
        Item::boxed(10.0),
        Item::fill(),
        Item::forced(),
    ];
    let p = Params { rag: Some(Rag { width_fraction: 1.0, ..Rag::default() }), ..Params::default() };
    let lines = break_paragraph(&items, &|_| 11.0, &p, &[2.5, f64::INFINITY]).lines;
    assert!(lines.iter().all(|l| l.start <= l.end && l.end <= l.next), "{lines:?}");
    assert_covers(&items, &lines);
}

fn spans(lines: &[Line]) -> Vec<(usize, usize)> {
    lines.iter().map(|l| (l.start, l.end)).collect()
}

/// A final word wider than the measure is one overfull line: the fill before the forced break
/// doesn't end it and leave an empty line. Consecutive forced breaks still make an empty line.
#[test]
fn an_overlong_last_word_adds_no_empty_line() {
    let fill = [Item::boxed(20.0), Item::fill(), Item::forced()];
    assert_eq!(spans(&break_paragraph(&fill, &|_| 11.0, &Params::default(), &[2.5, f64::INFINITY]).lines), [(0, 2)]);
    let forced = [Item::boxed(20.0), Item::forced(), Item::forced()];
    assert_eq!(spans(&break_paragraph(&forced, &|_| 11.0, &Params::default(), &[2.5, f64::INFINITY]).lines), [(0, 1), (2, 2)]);
}

/// Overfull lines keep the hyphen limit too: with a limit of 1, no two consecutive lines end at a
/// hyphen even when every line is overfull.
#[test]
fn overfull_lines_keep_the_hyphen_limit() {
    let items =
        [Item::boxed(10.0), Item::penalty(1.0, 0.0, true), Item::boxed(10.0), Item::penalty(1.0, 0.0, true), Item::boxed(10.0), Item::forced()];
    let p = Params { hyphen_limit: 1, ..Params::default() };
    let lines = break_paragraph(&items, &|_| 5.0, &p, &[2.5, f64::INFINITY]).lines;
    assert!(lines.windows(2).all(|w| !(w[0].flagged && w[1].flagged)), "{lines:?}");
    assert_eq!(spans(&lines), [(0, 1), (2, 5)]);
}
