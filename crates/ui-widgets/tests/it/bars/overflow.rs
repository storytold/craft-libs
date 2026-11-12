//! A `Bar` too short for its members: it lays out under right-to-left as the mirror of left-to-right, and its end
//! region shares the members' line; a member that fills takes the room the bar leaves.

use craft_behavior::IconId;
use craft_behavior::geometry::Constraint;
use craft_i18n::Direction;
use craft_ui::bars::{Bar, BarItem, BarItems, BarKind, ConstraintFields, ItemDisplay};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::{Divider, Text};
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::runtime::commands::EditCommands;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use craft_ui::value_drag::Slider;
use egui::{Rect, Vec2};
use std::cell::Cell;
use std::num::NonZeroU8;
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

/// The rects of a seek slider that fills and of the readout after it in a tool bar `width` wide, and of a search
/// field that fills a fill-width row in the second row of a two-row bar.
fn filled(width: f32) -> (Rect, Rect, Rect) {
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING, Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let mut search = String::new();
        let (mut slider, mut readout, mut field) = (Rect::NOTHING, Rect::NOTHING, Rect::NOTHING);
        Bar::new("Media", BarKind::Tool).show(ui, |ui, _| {
            IconButton::new(IconId::new("play")).label("Play").show(ui);
            slider = Slider::new(Bind::given(2.0), 0.0..=10.0).fill_width(true).label("Seek").show(ui).response.rect;
            readout = Text::new("0:12.50").show(ui).rect;
        });
        Bar::new("Bin", BarKind::Tool).rows(NonZeroU8::new(2).expect("two")).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                IconButton::new(IconId::new("arrow_up")).label("Up").show(ui);
            });
            Row::new().fill_width().show(ui, |ui, _| {
                field = TextField::search("Search", Bind::bound(&mut search)).fill_width(true).show(ui).response.rect;
                Text::new("12 items").show(ui);
            });
        });
        out.set((slider, readout, field));
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
    assert!(wide.2.width() - narrow.2.width() >= grown, "the search field grew from {:?} to {:?} in a 300 pt wider bar", narrow.2, wide.2);
}
