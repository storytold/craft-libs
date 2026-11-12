//! A `Bar` too short for its members: it lays out under right-to-left as the mirror of left-to-right, and its end
//! region shares the members' line; a member that fills takes the room the bar leaves; a select fits its widest entry
//! up to a field length; a rule before a member the bar hides is not drawn, and a settled bar replays it so.

use craft_behavior::IconId;
use craft_behavior::geometry::Constraint;
use craft_i18n::Direction;
use craft_theme::ControlSize;
use craft_ui::bars::{Bar, BarItem, BarItems, BarKind, ConstraintFields, ItemDisplay};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{Bind, Entry, Lazy, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::{Divider, Text};
use craft_ui::form::Field;
use craft_ui::pickers::Select;
use craft_ui::runtime::commands::EditCommands;
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Fixture, Frames, trial};
use craft_ui::value_drag::Slider;
use egui::{Pos2, Rect, Shape, Ui, Vec2};
use std::cell::Cell;
use std::rc::Rc;

const SLACK: f32 = 0.5;
const ROOM: Vec2 = Vec2::new(400.0, 80.0);
const NAMES: &[&str] = &["Style", "Width", "Height", "Undo"];

/// An options bar with the Style, Width and Height cells, a rule and a text item strip, in `direction`.
fn options_bar(direction: Direction) -> Fixture {
    let mut fixture = Fixture::new(|ui| {
        let mut constraint = Constraint::Free;
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            ConstraintFields::new(Bind::bound(&mut constraint)).show(ui);
            Divider::vertical().show(ui);
            let items = [EditCommands::Undo, EditCommands::Redo].map(|command| BarItem::new(command).display(ItemDisplay::Text));
            BarItems::new(items.to_vec()).label("Edit").show(ui);
        });
    });
    fixture.direction(direction).room(ROOM).settle();
    fixture
}

fn rects(fixture: &Fixture) -> Vec<Option<Rect>> {
    NAMES.iter().map(|name| fixture.by_label(name).map(|target| target.rect)).collect()
}

#[trial]
fn a_bar_too_short_for_its_members_lays_out_under_rtl_as_the_mirror_of_ltr() {
    let (ltr, rtl) = (rects(&options_bar(Direction::Ltr)), rects(&options_bar(Direction::Rtl)));
    for ((name, a), b) in NAMES.iter().zip(&ltr).zip(&rtl) {
        let (Some(a), Some(b)) = (a, b) else { panic!("{name}: shown in one direction only: {a:?} / {b:?}") };
        let mirror = (ROOM.x - a.max.x, ROOM.x - a.min.x);
        assert!(
            (b.min.x - mirror.0).abs() <= SLACK && (b.max.x - mirror.1).abs() <= SLACK,
            "{name} at {a:?} under left-to-right is at {b:?} under right-to-left, not at x {mirror:?}"
        );
    }
}

/// The centre lines of a body member and of the cancel button in an options bar `width` wide with a commit and a
/// cancel button, a select and four buttons.
fn centre_lines(width: f32) -> (f32, f32) {
    let mut fixture = Fixture::new(|ui| {
        let entries = [Entry::new(0u8, "Normal"), Entry::new(1, "Fixed Ratio")];
        Bar::new("Options", BarKind::Options).commit_cancel("Commit", "Cancel").show(ui, |ui, _| {
            Field::new("Style").show(ui, |ui, _| Select::new("Style", Bind::given(0u8), &entries).show(ui));
            for at in 0..4 {
                Button::new(format!("Button {at}")).show(ui);
            }
        });
    });
    fixture.room(Vec2::new(width, 80.0)).settle();
    let centre = |name: &str| fixture.by_label(name).unwrap_or_else(|| panic!("no {name}")).rect.center().y;
    (centre("Button 0"), centre("Cancel"))
}

#[trial]
fn the_end_region_shares_the_line_of_the_members_whether_the_bar_overflows_or_not() {
    for width in [900.0, 300.0] {
        let (member, cancel) = centre_lines(width);
        assert!((member - cancel).abs() <= SLACK, "at {width}: the members' centre line is {member}, the cancel button's {cancel}");
    }
}

/// The rects of a seek slider that fills and of the readout after it in a tool bar `width` wide.
fn filled(width: f32) -> (Rect, Rect) {
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let (mut slider, mut readout) = (Rect::NOTHING, Rect::NOTHING);
        Bar::new("Media", BarKind::Tool).show(ui, |ui, _| {
            IconButton::new(IconId::new("play")).label("Play").show(ui);
            slider = Slider::new(Bind::given(2.0), 0.0..=10.0).fill_width(true).label("Seek").show(ui).response.rect;
            readout = Text::new("0:12.50").show(ui).rect;
        });
        out.set((slider, readout));
    });
    fixture.room(Vec2::new(width, 160.0)).settle();
    seen.get()
}

#[trial]
fn a_member_that_fills_takes_the_room_its_bar_leaves() {
    let (narrow, wide) = (filled(400.0), filled(700.0));
    let grown = 300.0 - SLACK;
    assert!(wide.0.width() - narrow.0.width() >= grown, "the seek slider grew from {:?} to {:?} in a 300 pt wider bar", narrow.0, wide.0);
    assert!(
        (700.0 - wide.1.max.x - (400.0 - narrow.1.max.x)).abs() <= SLACK,
        "the readout keeps its place at the bar's end: {:?} / {:?}",
        narrow.1,
        wide.1
    );
}

/// An options bar `width` wide with eight buttons, and the clicks each button reported.
fn buttons(width: f32) -> (Fixture, Rc<Cell<[u32; 8]>>) {
    let clicks = Rc::new(Cell::new([0u32; 8]));
    let seen = Rc::clone(&clicks);
    let mut fixture = Fixture::new(move |ui| {
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            for at in 0..8 {
                if Button::new(format!("Button {at}")).show(ui).clicked() {
                    let mut counts = seen.get();
                    counts[at] += 1;
                    seen.set(counts);
                }
            }
        });
    });
    fixture.room(Vec2::new(width, 80.0)).settle();
    (fixture, clicks)
}

#[trial]
fn a_member_the_bar_shows_in_part_is_not_shown_until_it_is_whole() {
    let mut checked = 0;
    for width in (200..=420).step_by(9) {
        let width = width as f32;
        let (fixture, _) = buttons(width);
        // A button that starts well inside the bar and ends past it.
        let straddles = (0..8).find_map(|at| {
            let rect = fixture.by_label(&format!("Button {at}"))?.rect;
            (rect.min.x + 2.0 < width - 20.0 && rect.max.x > width).then_some((at, rect))
        });
        let Some((at, rect)) = straddles else { continue };
        let (mut fixture, clicks) = buttons(width);
        fixture.click_at(egui::pos2(rect.min.x + 2.0, rect.center().y));
        assert_eq!(clicks.get()[at], 0, "at {width}: Button {at} at {rect:?}, shown in part, took a click");
        let first = fixture.by_label("Button 0").expect("the first button");
        fixture.click(&first);
        assert_eq!(clicks.get()[0], 1, "at {width}: the first button, shown whole, took no click");
        checked += 1;
    }
    assert!(checked > 0, "no width put a button across the bar's end");
}

const SHORT: &[&str] = &["Normal", "Fixed"];
const LONG: &[&str] = &["Normal", "A select entry far longer than any field is wide, so the box elides it"];

/// The width of a small select of `entries`, in an options bar or in a plain column.
fn select_width(entries: &[&str], in_bar: bool) -> f32 {
    let entries: Vec<Entry<u8>> = (0u8..).zip(entries).map(|(at, text)| Entry::new(at, *text)).collect();
    let mut fixture = Fixture::new(move |ui| {
        let select = |ui: &mut Ui| {
            Select::new("Pick", Bind::given(0u8), &entries).size(ControlSize::Sm).show(ui);
        };
        if in_bar {
            Bar::new("Options", BarKind::Options).show(ui, |ui, _| select(ui));
        } else {
            select(ui);
        }
    });
    fixture.room(Vec2::new(900.0, 80.0)).settle();
    fixture.by_label("Pick").expect("the select").rect.width()
}

#[trial]
fn a_select_in_a_bar_fits_its_widest_entry_up_to_the_field_length() {
    let (short_bar, short_plain) = (select_width(SHORT, true), select_width(SHORT, false));
    let (long_bar, long_plain) = (select_width(LONG, true), select_width(LONG, false));
    assert!(short_bar < short_plain - SLACK, "short entries: {short_bar} in a bar, {short_plain} at the field length outside one");
    assert!(long_bar <= short_plain + SLACK, "a long entry widens the select in a bar past the field length: {long_bar} against {short_plain}");
    assert!(long_plain > long_bar + SLACK, "outside a bar the long entry sets the width: {long_plain}, in a bar {long_bar}");
}

/// A bar `width` wide holding a button, a rule and a wider button; `rects` gets the two buttons' rects.
fn rule_bar(ui: &mut Ui, width: f32, rects: &std::cell::Cell<(Rect, Rect)>) {
    in_region(ui, Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 80.0)), |ui| {
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            let first = Button::new("First").show(ui).response.rect;
            Divider::vertical().show(ui);
            let second = Button::new("A second, wider button").show(ui).response.rect;
            rects.set((first, second));
        });
    });
}

/// Whether `shapes` draw anything between the two buttons of `rects`: the rule, there being nothing else.
fn draws_rule(shapes: &[Shape], (first, second): (Rect, Rect)) -> bool {
    let between = Rect::from_min_max(Pos2::new(first.max.x, first.min.y - 4.0), Pos2::new(second.min.x, first.max.y + 4.0));
    shapes.iter().map(Shape::visual_bounding_rect).any(|bounds| bounds.is_positive() && between.contains_rect(bounds))
}

/// The shapes a bar `width` wide paints once settled, and its buttons' rects.
fn rule_bar_shapes(width: f32) -> (Vec<Shape>, (Rect, Rect)) {
    let rects = std::cell::Cell::new((Rect::NOTHING, Rect::NOTHING));
    let mut frames = Frames::new();
    frames.idle(4, |ui| rule_bar(ui, width, &rects));
    let shapes = frames.frame_shapes(|ui| rule_bar(ui, width, &rects));
    (shapes, rects.get())
}

/// A width at which the bar shows the second button only in part: it starts inside the bar and ends past it.
fn cutting_width() -> f32 {
    (180..=520)
        .step_by(11)
        .map(|width| width as f32)
        .find(|&width| {
            let (_, (_, second)) = rule_bar_shapes(width);
            second.min.x + 2.0 < width - 20.0 && second.max.x > width && second.width() < width - 40.0
        })
        .expect("no width put the second button across the bar's end")
}

#[trial]
fn a_rule_before_a_member_the_bar_hides_is_not_drawn() {
    let (shown, rects) = rule_bar_shapes(900.0);
    assert!(draws_rule(&shown, rects), "with both buttons whole the rule between them is drawn");
    let (cut, rects) = rule_bar_shapes(cutting_width());
    assert!(!draws_rule(&cut, rects), "the rule before the hidden button is drawn");
}

/// A settled region holding a bar whose rule was taken back replays the bar without the rule.
#[trial]
fn a_settled_bar_replays_the_rule_it_took_back() {
    let width = cutting_width();
    let runs = std::cell::Cell::new(0);
    let rects = std::cell::Cell::new((Rect::NOTHING, Rect::NOTHING));
    let scene = |ui: &mut Ui| {
        Lazy::new("bar", 0.0).settles(true).show(ui, |ui| {
            runs.set(runs.get() + 1);
            rule_bar(ui, width, &rects);
        });
    };
    let mut frames = Frames::new();
    frames.frame(vec![], scene);
    assert!(runs.get() > 0, "the first frame runs the region");
    frames.idle(30, scene);
    for frame in 0..4 {
        runs.set(0);
        let shapes = frames.frame_shapes(scene);
        assert_eq!(runs.get(), 0, "settled frame {frame} ran the region");
        assert!(!draws_rule(&shapes, rects.get()), "settled frame {frame} replays the rule the run took back");
    }
}
