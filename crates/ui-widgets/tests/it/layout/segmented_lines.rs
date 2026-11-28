//! A segmented strip in a column narrower than its one line moves the segments that do not fit to the next line, so
//! it stays inside the room; in a room it fits, and in a row or bar, it keeps one line.

use craft_i18n::Direction;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::buttons::Button;
use craft_ui::choice::Segmented;
use craft_ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft_ui::layout_components::Row;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Rect, Vec2};

const MODES: [&str; 5] = ["New", "Add", "Subtract", "Intersect", "Exclude"];

/// The strip of the five modes, shown in a room `width` wide, laid out in `direction`.
fn strip(width: f32, direction: Direction) -> Fixture {
    let mut fixture = Fixture::new(|ui| {
        let entries: Vec<Entry<u8>> = (0..).zip(MODES).map(|(at, mode)| Entry::new(at, mode)).collect();
        let mut picked = 0_u8;
        Segmented::new("Combine", Bind::bound(&mut picked), &entries).show(ui);
    });
    fixture.direction(direction).room(Vec2::new(width, 300.0)).settle();
    fixture
}

/// The strip's rect and each segment's, in reading order.
fn rects(fixture: &Fixture) -> (Rect, Vec<Rect>) {
    let group = fixture.by_label("Combine").expect("the strip").rect;
    let segments = MODES.iter().map(|mode| fixture.by_label(mode).expect("a segment").rect).collect();
    (group, segments)
}

/// The width the strip takes on one line, in a room much wider than it.
fn one_line_width() -> f32 {
    let (group, segments) = rects(&strip(2000.0, Direction::Ltr));
    let tops: Vec<f32> = segments.iter().map(|segment| segment.top()).collect();
    assert!(tops.iter().all(|top| (top - tops[0]).abs() < 0.5), "in a wide room the segments share one line: {tops:?}");
    group.width()
}

#[trial]
fn a_strip_wider_than_its_room_moves_segments_to_another_line() {
    let line = one_line_width();
    let room = line * 0.6;
    for direction in [Direction::Ltr, Direction::Rtl] {
        let fixture = strip(room, direction);
        let (group, segments) = rects(&fixture);
        assert!(group.right() <= room + 0.5, "{direction:?}: the strip ({group:?}) stays inside the room {room}");
        for (mode, segment) in MODES.iter().zip(&segments) {
            assert!(group.expand(0.5).contains_rect(*segment), "{direction:?}: {mode} ({segment:?}) is inside the strip ({group:?})");
        }
        assert!(segments[4].top() > segments[0].bottom() - 0.5, "{direction:?}: the last segment is on a later line than the first");
        for (at, pair) in segments.windows(2).enumerate() {
            let overlap = pair[0].intersect(pair[1]);
            assert!(overlap.width() <= 0.0 || overlap.height() <= 0.0, "{direction:?}: segments {at} and {} do not overlap", at + 1);
        }
        // Each line starts at the strip's start edge: the left one, or the right one right to left.
        let start = |segment: &Rect| if direction == Direction::Rtl { group.right() - segment.right() } else { segment.left() - group.left() };
        let first_of_later_line = segments.iter().find(|segment| segment.top() > segments[0].bottom() - 0.5).expect("a second line");
        assert!(start(&segments[0]).abs() < 0.5 && start(first_of_later_line).abs() < 0.5, "{direction:?}: every line starts at the start edge");
    }
}

#[trial]
fn a_strip_that_fits_its_room_keeps_one_line() {
    let line = one_line_width();
    let fixture = strip(line + 40.0, Direction::Ltr);
    let (group, segments) = rects(&fixture);
    assert!(segments.iter().all(|segment| (segment.top() - segments[0].top()).abs() < 0.5), "the segments share one line");
    assert!((group.width() - line).abs() < 0.5, "the strip is as wide as its one line ({} against {line})", group.width());
}

/// The strip of the five modes followed by three buttons, in a row (`bar` false) or an options bar, `width` wide.
fn strip_in_row(width: f32, bar: bool) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let entries: Vec<Entry<u8>> = (0..).zip(MODES).map(|(at, mode)| Entry::new(at, mode)).collect();
        let mut picked = 0_u8;
        let mut members = |ui: &mut egui::Ui| {
            Segmented::new("Combine", Bind::bound(&mut picked), &entries).show(ui);
            for at in 0..3 {
                Button::new(format!("Button {at}")).show(ui);
            }
        };
        if bar {
            Bar::new("Options", BarKind::Options).show(ui, |ui, _| members(ui));
        } else {
            Row::new().show(ui, |ui, _| members(ui));
        }
    });
    fixture.room(Vec2::new(width, 300.0)).settle();
    fixture
}

#[trial]
fn a_strip_in_a_row_or_bar_keeps_one_line_under_width_pressure() {
    let line = one_line_width();
    for bar in [false, true] {
        let (_, wide) = rects(&strip_in_row(2000.0, bar));
        let (_, narrow) = rects(&strip_in_row(line * 0.6, bar));
        assert!(
            narrow.iter().all(|segment| (segment.top() - narrow[0].top()).abs() < 0.5),
            "bar {bar}: the segments share one line in a room narrower than the strip"
        );
        for ((mode, wide), narrow) in MODES.iter().zip(&wide).zip(&narrow) {
            assert!(narrow.width() >= wide.width() - 0.5, "bar {bar}: {mode} keeps its whole face ({narrow:?} against {wide:?})");
        }
    }
}

#[trial]
fn arrows_move_across_the_lines_of_a_wrapped_strip() {
    let mut fixture = strip(one_line_width() * 0.6, Direction::Ltr);
    let focused = |fixture: &Fixture| fixture.focused_stop().and_then(|(_, part)| part).and_then(|part| part.index);
    fixture.key(Key::Tab, Modifiers::NONE);
    assert_eq!(focused(&fixture), Some(0), "Tab enters the strip on the picked segment");
    for to in 1..MODES.len() as u64 {
        fixture.key(Key::ArrowRight, Modifiers::NONE);
        assert_eq!(focused(&fixture), Some(to), "each arrow moves to the next segment, across a line break too");
    }
    fixture.key(Key::Home, Modifiers::NONE);
    assert_eq!(focused(&fixture), Some(0), "Home goes to the first segment");
}
