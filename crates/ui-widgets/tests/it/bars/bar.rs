//! `Bar`: a body too wide for the bar shrinks its members before it scrolls, a captioned field among them too.

use craft_ui::bars::{Bar, BarKind, BreadcrumbBar, path_crumbs};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::pickers::Select;
use craft_ui::testing::{Fixture, shrink_least_runs, trial};
use craft_ui::text::TextField;
use craft_behavior::IconId;
use egui::{Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

const SLACK: f32 = 0.5;

/// The rects of a select and of the last of `buttons` buttons after it, in an options bar `width` wide.
fn bar(width: f32, buttons: usize) -> (Rect, Rect) {
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(0u8, "Current and Below, No Adjustments"), Entry::new(1, "All Layers")];
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            let select = Select::new("Sample", Bind::given(1u8), &entries).show(ui).response.rect;
            let mut last = Rect::NOTHING;
            for at in 0..buttons {
                last = Button::new(format!("Button {at}")).show(ui).rect;
            }
            out.set((select, last));
        });
    });
    fixture.room(Vec2::new(width, 80.0)).settle();
    seen.get()
}

#[trial]
fn a_bar_too_narrow_for_its_members_elides_a_select_before_it_scrolls() {
    let (wide, _) = bar(900.0, 1);
    let (select, button) = bar(240.0, 1);
    assert!(select.width() < wide.width() - SLACK, "the select kept its width {} in a narrow bar", select.width());
    assert!(button.max.x <= 240.0 + SLACK, "the last member ends at {} past the bar's end", button.max.x);
    // Buttons do not shrink: once the select is at its least, the rest scrolls.
    let (_, button) = bar(240.0, 6);
    assert!(button.max.x > 240.0, "six buttons fit a 240-point bar: {button:?}");
}

/// What a captioned select in an options bar `width` wide shows, with `buttons` buttons after it.
struct Captioned {
    select: Rect,
    caption: Rect,
    last: Rect,
    /// Whether a row asked its body units for their least widths.
    probed: bool,
}

fn captioned(width: f32, buttons: usize) -> Captioned {
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(0u8, "Current and Below, No Adjustments"), Entry::new(1, "All Layers")];
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            let select = Field::new("Sample").show(ui, |ui, _| Select::new("Sample", Bind::given(0u8), &entries).show(ui).response.rect);
            let mut last = Rect::NOTHING;
            for at in 0..buttons {
                last = Button::new(format!("Button {at}")).show(ui).rect;
            }
            out.set((select.inner.unwrap_or(Rect::NOTHING), last));
        });
    });
    fixture.room(Vec2::new(width, 80.0)).settle();
    let (select, last) = seen.get();
    let caption = fixture.texts().iter().find(|text| text.text == "Sample").map_or(Rect::NOTHING, |text| text.rect);
    Captioned { select, caption, last, probed: !shrink_least_runs(fixture.ctx()).is_empty() }
}

#[trial]
fn a_captioned_select_in_a_narrow_bar_narrows_with_its_caption_beside_it_before_the_bar_scrolls() {
    let wide = captioned(900.0, 1);
    assert!(!wide.probed, "a bar its members fit asked a body for its least width");
    let narrow = captioned(300.0, 1);
    assert!(narrow.select.width() < wide.select.width() - SLACK, "the select kept its width {} in a narrow bar", narrow.select.width());
    assert!(narrow.last.max.x <= 300.0 + SLACK, "the last member ends at {} past the bar's end", narrow.last.max.x);
    assert!(
        narrow.caption.max.x <= narrow.select.min.x + SLACK && narrow.caption.center().y > narrow.select.min.y,
        "the caption {:?} is not beside the select {:?}",
        narrow.caption,
        narrow.select
    );
    // Once the field is at its least, the rest scrolls.
    let crowded = captioned(300.0, 6);
    assert!(crowded.last.max.x > 300.0, "six buttons fit a 300-point bar: {:?}", crowded.last);
}

/// The rects of an up button, a path, a search field and a count, in that order, in a tool bar `width` wide.
fn header(width: f32) -> Vec<Rect> {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let crumbs = path_crumbs("Project");
        let mut search = String::new();
        Bar::new("Header", BarKind::Tool).show(ui, |ui, _| {
            let up = IconButton::new(IconId::new("chevron_up")).label("Up").show(ui).rect;
            let path = BreadcrumbBar::new("Path", &crumbs).show(ui).response.rect;
            let field = TextField::search("Search", Bind::bound(&mut search)).show(ui).response.rect;
            let count = Text::new("1 of 9 items selected").show(ui).rect;
            *out.borrow_mut() = vec![up, path, field, count];
        });
    });
    fixture.room(Vec2::new(width, 80.0)).settle();
    seen.borrow().clone()
}

#[trial]
fn the_members_of_a_tool_bar_never_overlap_as_it_narrows() {
    for width in [900.0, 342.0, 260.0] {
        let rects = header(width);
        for pair in rects.windows(2) {
            assert!(pair[0].max.x <= pair[1].min.x + SLACK, "at {width} points {:?} runs into {:?}", pair[0], pair[1]);
        }
    }
}
