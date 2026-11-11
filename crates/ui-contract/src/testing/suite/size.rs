//! Size cases: text size, density and scale grow the layout without cutting text; a room of one point does not break
//! it; a narrow room elides with the full text as the accessible name.

use super::{ANY, Case, Subject};
use crate::runtime::egui::widgets::{self, ScrollRegion};
use crate::runtime::env::Scale;
use crate::testing::fixture::Fixture;
use crate::testing::relations::{grows_with, inside};
use crate::testing::story::Story;
use craft_theme::{Size, TextSize};
use egui::{Rect, Vec2};

pub(super) const CASES: &[Case] = &[
    Case::new("text-size", ANY, stories_grow_with_text_density_and_scale),
    Case::new("tiny-room", ANY, a_tiny_room_breaks_nothing),
    Case::new("elision", ANY, elided_text_keeps_its_full_name),
];

/// A settled fixture of the story after `change`: its shown rect, painted text runs and scroll regions.
struct Shot {
    shown: Rect,
    runs: Vec<crate::runtime::egui::fixture::PaintedText>,
    scrolls: Vec<ScrollRegion>,
}

fn shot(story: &Story, change: &dyn Fn(&mut Fixture)) -> Shot {
    let mut fixture = Fixture::story(story);
    change(&mut fixture);
    fixture.settle();
    Shot::of(&fixture)
}

impl Shot {
    /// What the settled `fixture` shows.
    fn of(fixture: &Fixture) -> Shot {
        Shot { shown: fixture.shown(), runs: fixture.texts().to_vec(), scrolls: widgets::scroll_regions(fixture.ctx()) }
    }

    /// The texts this shot shows whole, in paint order, with their rects.
    fn whole_texts(&self) -> Vec<(&str, Rect)> {
        self.runs.iter().filter(|run| !run.elided && !run.text.trim().is_empty()).map(|run| (run.text.as_str(), run.rect)).collect()
    }

    /// The text run at `run` stays in its part: it lies in the shown rect, or a scroll region's view cuts it. A run
    /// in a scroll region's content shows through the region's view, so it stays in its part when the whole run lies
    /// in the content, which scrolls, and what the view shows of it in the shown rect.
    fn keeps(&self, run: Rect) -> bool {
        inside(run, self.shown)
            || self
                .scrolls
                .iter()
                .filter(|region| region.content.intersects(run))
                .min_by(|a, b| a.content.area().total_cmp(&b.content.area()))
                .is_some_and(|region| {
                    let seen = run.intersect(region.view);
                    inside(run, region.content) && (!seen.is_positive() || inside(seen, self.shown))
                })
    }
}

/// Whether the layout reflowed from `base` to `grown`: two texts lie beside each other in `base` and one above the
/// other in `grown`, as a label that moves above its control. The texts are `(text, rect)` in paint order; a text
/// shown several times pairs its nth occurrence in one with its nth in the other, and a text one of them lacks pairs
/// with nothing. A reflow can shorten the layout, so the growth check leaves the height out.
pub fn reflowed(base: &[(&str, Rect)], grown: &[(&str, Rect)]) -> bool {
    let stacked = |a: Rect, b: Rect| a.max.y <= b.min.y || b.max.y <= a.min.y;
    let occurrence = |runs: &[(&str, Rect)], at: usize| runs[..at].iter().filter(|(text, _)| *text == runs[at].0).count();
    let there = |at: usize| grown.iter().filter(|(text, _)| *text == base[at].0).nth(occurrence(base, at)).map(|(_, rect)| *rect);
    (0..base.len()).any(|first| {
        (first + 1..base.len()).any(|second| match (there(first), there(second)) {
            (Some(a), Some(b)) => !stacked(base[first].1, base[second].1) && stacked(a, b),
            _ => false,
        })
    })
}

/// Text layout at scale 2 may shave a point off a wrapped measure (68 becomes 67); real growth failures are larger.
const SCALE_LAYOUT_SLACK: f32 = 1.0;

fn stories_grow_with_text_density_and_scale(subject: &Subject) -> Result<(), String> {
    subject.stories.iter().try_for_each(|story| text_size_from(story, Shot::of(&subject.base(story).fixture)))
}

/// The suite's text-size case for one story: under a larger text size, either density and scale 2 the story grows
/// and no text run that lay in its part leaves it.
pub fn text_size_case(story: &Story) -> Result<(), String> {
    text_size_from(story, shot(story, &|_| {}))
}

/// The text-size case for `story`, whose settled default shot is `base`.
fn text_size_from(story: &Story, base: Shot) -> Result<(), String> {
    let variants: [(&str, Shot); 4] = [
        (
            "text size",
            shot(story, &|f| {
                f.text_size(TextSize::Large);
            }),
        ),
        (
            "compact density",
            shot(story, &|f| {
                f.density(Size::Compact);
            }),
        ),
        (
            "comfortable density",
            shot(story, &|f| {
                f.density(Size::Comfortable);
            }),
        ),
        (
            "scale 2",
            shot(story, &|f| {
                f.scale(Scale(2.0));
            }),
        ),
    ];
    let (before, after) = (base.shown.size(), |shot: &Shot| shot.shown.size());
    for (name, grown) in &variants {
        let size = after(grown);
        let slack = if *name == "scale 2" { SCALE_LAYOUT_SLACK } else { 0.0 };
        // A label that moves from beside to above its control reflows the field, and that can shorten it.
        let moved = reflowed(&base.whole_texts(), &grown.whole_texts());
        if !(grows_with(before.x - slack, size.x) && (moved || grows_with(before.y - slack, size.y))) {
            return Err(format!("{}: under {name} the size went from {before:?} to {size:?}", story.name));
        }
        for run in base.runs.iter().filter(|run| !run.elided && !run.text.trim().is_empty() && inside(run.rect, base.shown)) {
            for same in grown.runs.iter().filter(|other| other.text == run.text) {
                // A run the component painted elided is shortened by design and keeps its full text in the access name
                // (the elision rule checks that), so it is not cut; it must still stay inside its part.
                if !grown.keeps(same.rect) {
                    return Err(format!("{}: under {name} the text `{}` leaves its part", story.name, run.text));
                }
            }
        }
    }
    Ok(())
}

/// A room of one point and a room of half the settled width show without a panic, and everything the access tree
/// reports has finite bounds. The `paints-inside` case shows each story in the same room of one point and checks the
/// same bounds there, so this case leaves that room to it when it runs.
fn a_tiny_room_breaks_nothing(subject: &Subject) -> Result<(), String> {
    let point = (!subject.runs("paints-inside")).then_some(Vec2::splat(1.0));
    for story in &subject.stories {
        let width = subject.base(story).fixture.shown().width();
        for room in point.into_iter().chain([Vec2::new((width / 2.0).max(1.0), 300.0)]) {
            let mut fixture = Fixture::story(story);
            fixture.room(room);
            fixture.settle();
            let tree = fixture.tree();
            if tree.keys().into_iter().filter_map(|key| tree.bounds_of(key)).any(|bounds| !(bounds.min.is_finite() && bounds.max.is_finite())) {
                return Err(format!("{}: a node has non-finite bounds in a room of {room:?}", story.name));
            }
        }
    }
    Ok(())
}

/// In a room narrower than the story, every text run that the layout cut has its full text in an accessible name or
/// description: a cut label is the node's name, a cut description its description.
fn elided_text_keeps_its_full_name(subject: &Subject) -> Result<(), String> {
    for story in &subject.stories {
        let width = subject.base(story).fixture.shown().width();
        if width < 40.0 {
            continue;
        }
        let mut fixture = Fixture::story(story);
        fixture.room(Vec2::new(width / 3.0, 300.0));
        fixture.settle();
        let tree = fixture.tree();
        let read: Vec<&str> = tree.keys().into_iter().flat_map(|key| [tree.name_of(key), tree.description_of(key)]).flatten().collect();
        for run in fixture.texts().iter().filter(|run| run.elided) {
            if !read.iter().any(|text| text.contains(run.text.trim())) {
                return Err(format!("{}: the elided text `{}` is in no accessible name or description", story.name, run.text));
            }
        }
    }
    Ok(())
}
