//! Breaker tests on a monospaced model: letters are 6 units wide, spaces 3, hyphens 4. Parameter
//! sets mirror vectorcraft's and designcraft's composers.

use super::*;
use craft_ui::testing::trial;

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

/// Lines are contiguous and cover every item.
fn assert_covers(items: &[Item], lines: &[Line]) {
    assert_eq!(lines.first().map(|l| l.start), Some(0));
    for pair in lines.windows(2) {
        assert_eq!(pair[0].next, pair[1].start, "{lines:?}");
    }
    assert_eq!(lines.last().map(|l| l.next), Some(items.len()));
}

#[trial]
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

/// Two consecutive breaks: the line after the first starts past the second, which must not end a
/// line `4..3` from there.
#[trial]
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
#[trial]
fn an_overlong_last_word_adds_no_empty_line() {
    let fill = [Item::boxed(20.0), Item::fill(), Item::forced()];
    assert_eq!(spans(&break_paragraph(&fill, &|_| 11.0, &Params::default(), &[2.5, f64::INFINITY]).lines), [(0, 2)]);
    let forced = [Item::boxed(20.0), Item::forced(), Item::forced()];
    assert_eq!(spans(&break_paragraph(&forced, &|_| 11.0, &Params::default(), &[2.5, f64::INFINITY]).lines), [(0, 1), (2, 2)]);
}

/// Overfull lines keep the hyphen limit too: with a limit of 1, no two consecutive lines end at a
/// hyphen even when every line is overfull.
#[trial]
fn overfull_lines_keep_the_hyphen_limit() {
    let items =
        [Item::boxed(10.0), Item::penalty(1.0, 0.0, true), Item::boxed(10.0), Item::penalty(1.0, 0.0, true), Item::boxed(10.0), Item::forced()];
    let p = Params { hyphen_limit: 1, ..Params::default() };
    let lines = break_paragraph(&items, &|_| 5.0, &p, &[2.5, f64::INFINITY]).lines;
    assert!(lines.windows(2).all(|w| !(w[0].flagged && w[1].flagged)), "{lines:?}");
    assert_eq!(spans(&lines), [(0, 1), (2, 5)]);
}

/// Tolerances of the passes, as the composers use them.
const PASSES: [f64; 3] = [2.5, 12.0, f64::INFINITY];

/// Past its work limit the paragraph is broken greedily, with every item laid out and no error or
/// cut: a limit of a few units stands for the real one, which a hostile paragraph of hundreds of
/// thousands of items would reach (and the test never builds).
#[trial]
fn a_paragraph_past_the_work_limit_falls_back_to_greedy_with_every_item() {
    let p = vectorcraft(false);
    let para = Para::new(COPY, &p, Some(50.0));
    let w = |_| 150.0;
    let within = break_paragraph_within(&para.items, &w, &p, &PASSES, MAX_WORK);
    assert!(matches!(within.method, Method::KnuthPlass { .. }), "{:?}", within.method);
    let limited = break_paragraph_within(&para.items, &w, &p, &PASSES, 20);
    assert_eq!(limited.method, Method::Greedy);
    assert_eq!(limited.lines, greedy(&para.items, &w, &p));
    assert_covers(&para.items, &limited.lines);
    // The work is shared by the passes: what one spends the next doesn't have.
    let mut work = 30;
    assert!(knuth_plass_within(&para.items, &w, &Params { tolerance: 1.0, ..p }, &mut work).is_none());
}
