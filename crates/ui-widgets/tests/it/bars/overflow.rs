//! A `Bar` too short for its members: it lays out under right-to-left as the mirror of left-to-right, and its end
//! region shares the members' line; a member that fills takes the room the bar leaves; a select fits its widest entry
//! up to a field length; a rule before a member the bar hides is not drawn, and a settled bar replays it so; a button
//! label elides no further than its readable least.

use craft_behavior::IconId;
use craft_behavior::geometry::Constraint;
use craft_i18n::Direction;
use craft_theme::ControlSize;
use craft_ui::bars::{Bar, BarItem, BarItems, BarKind, ConstraintFields, ItemDisplay};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, Lazy, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::display::{Divider, Text};
use craft_ui::form::Field;
use craft_ui::pickers::Select;
use craft_ui::popover::Popover;
use craft_ui::primitives::TextRun;
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
            slider = Slider::new(Bind::given(2.0), 0.0..=10.0).sizing(Sizing::FillWidth).label("Seek").show(ui).response.rect;
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
fn select_width(entries: &'static [&'static str], in_bar: bool) -> f32 {
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

/// A layer opened from a bar is not the bar's line: a select in a popover a bar's button opens keeps the field length.
#[trial]
fn a_select_in_a_popover_opened_from_a_bar_keeps_the_field_length() {
    let mut fixture = Fixture::new(|ui| {
        let entries = [Entry::new(0u8, SHORT[0]), Entry::new(1, SHORT[1])];
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            let opener = Button::new("More").show(ui).response;
            Popover::new("More options", Bind::given(true)).opened_by(&opener).show(ui, |ui, _| {
                Select::new("Inside", Bind::given(0u8), &entries).size(ControlSize::Sm).show(ui);
            });
        });
    });
    fixture.room(Vec2::new(900.0, 300.0)).settle();
    let inside = fixture.by_label("Inside").expect("the select in the popover").rect.width();
    let plain = select_width(SHORT, false);
    assert!((inside - plain).abs() <= SLACK, "the select in the popover is {inside} wide, a select outside any bar {plain}");
}

/// A bar `width` wide holding an icon button, a rule and a second icon button; `rects` gets the two buttons' rects.
/// Icon buttons keep their width, so nothing in the line narrows to make room.
fn rule_bar(ui: &mut Ui, width: f32, rects: &std::cell::Cell<(Rect, Rect)>) {
    in_region(ui, Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 80.0)), |ui| {
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            let first = IconButton::new(IconId::new("plus")).label("First").show(ui).rect;
            Divider::vertical().show(ui);
            let second = IconButton::new(IconId::new("minus")).label("Second").show(ui).rect;
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

/// A width at which the bar's view ends inside the second button of `rects`, as a wide bar lays it out.
fn cutting_width((_, second): (Rect, Rect)) -> f32 {
    (second.min.x + second.max.x) / 2.0
}

#[trial]
fn a_rule_before_a_member_the_bar_hides_is_not_drawn() {
    let (shown, rects) = rule_bar_shapes(900.0);
    assert!(draws_rule(&shown, rects), "with both buttons whole the rule between them is drawn");
    let (cut, (_, hidden)) = rule_bar_shapes(cutting_width(rects));
    assert!(!hidden.is_positive(), "the second button shows at {hidden:?} in a bar that ends inside it");
    assert!(!draws_rule(&cut, rects), "the rule before the hidden button is drawn");
}

/// A settled region holding a bar whose rule was taken back replays the bar without the rule.
#[trial]
fn a_settled_bar_replays_the_rule_it_took_back() {
    let (_, wide) = rule_bar_shapes(900.0);
    let width = cutting_width(wide);
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
        assert!(!draws_rule(&shapes, wide), "settled frame {frame} replays the rule the run took back");
    }
}

/// A bar `width` wide holding an icon button, a rule, a labelled checkbox, a rule and a second icon button; `rects`
/// gets the first button's and the checkbox's rects.
fn labelled_rule_bar(ui: &mut Ui, width: f32, rects: &std::cell::Cell<(Rect, Rect)>) {
    in_region(ui, Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 80.0)), |ui| {
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            let first = IconButton::new(IconId::new("plus")).label("First").show(ui).rect;
            Divider::vertical().show(ui);
            let checkbox = Checkbox::new(Bind::given(false)).label("Snap").show(ui).response.rect;
            Divider::vertical().show(ui);
            IconButton::new(IconId::new("minus")).label("Last").show(ui);
            rects.set((first, checkbox));
        });
    });
}

/// The rule before a labelled control the bar shows is drawn, though a member after it is hidden.
#[trial]
fn a_rule_before_a_labelled_control_the_bar_shows_is_drawn() {
    let shapes_at = |width: f32| {
        let rects = std::cell::Cell::new((Rect::NOTHING, Rect::NOTHING));
        let mut frames = Frames::new();
        frames.idle(4, |ui| labelled_rule_bar(ui, width, &rects));
        let shapes = frames.frame_shapes(|ui| labelled_rule_bar(ui, width, &rects));
        (shapes, rects.get())
    };
    let (shown, rects) = shapes_at(900.0);
    assert!(draws_rule(&shown, rects), "with every member whole the rule before the checkbox is drawn");
    let mut wide = Fixture::new(|ui| labelled_rule_bar(ui, 900.0, &std::cell::Cell::new((Rect::NOTHING, Rect::NOTHING))));
    wide.room(Vec2::new(900.0, 80.0)).settle();
    let last = wide.by_label("Last").expect("the last button").rect;
    let (cut, (_, checkbox)) = shapes_at(last.center().x);
    assert!(checkbox.is_positive() && checkbox.max.x < last.min.x, "the checkbox shows at {checkbox:?}");
    assert!(draws_rule(&cut, rects), "the rule before the shown checkbox is not drawn when the last button is hidden");
}

const VIEW_BUTTONS: &[&str] = &["Actual Pixels", "Fit Screen", "Fill Screen"];

/// The characters each label a bar `width` wide of `VIEW_BUTTONS` shows once settled before its ellipsis, for each
/// label shown cut, with the characters of the whole label.
fn cut_labels(width: f32) -> Vec<(usize, usize)> {
    let scene = |ui: &mut Ui| {
        in_region(ui, Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 80.0)), |ui| {
            Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
                for label in VIEW_BUTTONS {
                    Button::new(*label).show(ui);
                }
            });
        });
    };
    let mut frames = Frames::new();
    frames.idle(4, scene);
    let mut cut = Vec::new();
    for shape in frames.frame_shapes(scene) {
        let Shape::Text(text) = shape else { continue };
        let Some(kept) = text.galley.job.text.strip_suffix('…') else { continue };
        if let Some(label) = VIEW_BUTTONS.iter().find(|label| label.starts_with(kept)) {
            cut.push((kept.chars().count(), label.chars().count()));
        }
    }
    cut
}

/// A bar too short for its buttons narrows their labels to their readable least, never to the ellipsis alone, and
/// scrolls past that.
#[trial]
fn a_button_label_in_a_bar_elides_no_further_than_its_readable_least() {
    let mut elided = 0;
    for width in (80..=400).step_by(8) {
        for (kept, characters) in cut_labels(width as f32) {
            elided += 1;
            assert!(kept >= TextRun::READABLE_CHARACTERS, "at {width}: a label of {characters} characters keeps {kept} before its ellipsis");
        }
    }
    assert!(elided > 0, "no width elided a label");
}
