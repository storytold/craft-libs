//! `Bar`: a body too wide for the bar shrinks its members before it scrolls, a captioned field among them too; a
//! status bar's message gives way while its trailing group keeps its width.

use craft_behavior::IconId;
use craft_theme::variant::Tone;
use craft_ui::bars::{Bar, BarKind, BreadcrumbBar, StatusMessage, StatusProgress, path_crumbs};
use craft_ui::buttons::{Button, IconButton, Link};
use craft_ui::contract::{Bind, Entry, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::pickers::Select;
use craft_ui::testing::{Fixture, shrink_least_runs, trial};
use craft_ui::text::TextField;
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
    // Buttons do not shrink: when even the select's least leaves no room for them, the select keeps its width (at most
    // the bar's room) and the rest scrolls.
    let (crowded, button) = bar(240.0, 6);
    assert!(button.max.x > 240.0, "six buttons fit a 240-point bar: {button:?}");
    assert!(crowded.width() > select.width() + SLACK, "the select narrowed to {} in a bar that scrolls anyway", crowded.width());
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

/// What a select in an options bar `width` wide shows, with three buttons after it: its rect and whether its text
/// was elided.
fn select_in(width: f32) -> (Rect, bool) {
    let seen = Rc::new(Cell::new(Rect::NOTHING));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(0u8, "Point Sample"), Entry::new(1, "5 by 5 Average")];
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            out.set(Select::new("Sample Size", Bind::given(0u8), &entries).show(ui).response.rect);
            for at in 0..3 {
                Button::new(format!("Button {at}")).show(ui);
            }
        });
    });
    fixture.room(Vec2::new(width, 80.0)).settle();
    let elided = fixture.texts().iter().any(|text| text.text == "Point Sample" && text.elided);
    (seen.get(), elided)
}

#[trial]
fn a_select_in_a_short_bar_narrows_past_a_fields_own_least_and_elides() {
    let (wide, _) = select_in(900.0);
    let (narrow, elided) = select_in(200.0);
    assert!(narrow.width() < 96.0, "the select stayed {} wide", narrow.width());
    assert!(narrow.width() < wide.width() - SLACK && elided, "the select {narrow:?} did not elide its pick");
}

/// What a status bar shows: the message's rect (none when it shows nothing), the readout before it, and the trailing
/// group's progress and render-path link.
struct Status {
    message: Option<Rect>,
    readout: Rect,
    progress: Rect,
    link: Rect,
}

impl Default for Status {
    fn default() -> Self {
        Status { message: None, readout: Rect::NOTHING, progress: Rect::NOTHING, link: Rect::NOTHING }
    }
}

fn status_fixture(message: &'static str, cancelled: Rc<Cell<bool>>, seen: Rc<Cell<Status>>) -> Fixture {
    Fixture::new(move |ui| {
        let (mut progress, mut link) = (Rect::NOTHING, Rect::NOTHING);
        let shown = Bar::new("Status", BarKind::Status)
            .trailing(|ui| {
                let output = StatusProgress::new("Blur", Some(0.45)).cancel("Cancel").show(ui);
                if output.cancelled {
                    cancelled.set(true);
                }
                progress = output.response.rect;
                link = Link::new("CPU Rendering").tone(Tone::Warn).show(ui).rect;
            })
            .show(ui, |ui, _| {
                let readout = Text::new("1200 px x 800 px (72 ppi)").show(ui).rect;
                (readout, StatusMessage::new(message).tone(Tone::Bad).show(ui).map(|shown| shown.rect))
            });
        if let Some((readout, message)) = shown.inner {
            seen.set(Status { message, readout, progress, link });
        }
    })
}

fn status(width: f32, message: &'static str) -> Status {
    let seen = Rc::new(Cell::new(Status::default()));
    status_fixture(message, Rc::new(Cell::new(false)), Rc::clone(&seen)).room(Vec2::new(width, 80.0)).settle();
    seen.take()
}

#[trial]
fn a_narrow_status_bar_cuts_its_message_and_keeps_its_trailing_group_whole() {
    const LONG: &str = "Exported 14 layers to the folder Documents/Exports/2026";
    let wide = status(1200.0, LONG);
    let narrow = status(520.0, LONG);
    let (wide_message, narrow_message) = (wide.message.expect("a message shows"), narrow.message.expect("a cut message shows"));
    assert!(narrow_message.width() < wide_message.width() - SLACK, "the message kept its width {} in a narrow bar", narrow_message.width());
    assert!(narrow_message.max.x <= narrow.progress.min.x + SLACK, "the message runs under the trailing group: {narrow_message:?}");
    assert!((narrow.progress.width() - wide.progress.width()).abs() < SLACK, "the progress narrowed with the bar");
    assert!((narrow.link.width() - wide.link.width()).abs() < SLACK, "the render-path label narrowed with the bar");
    assert!(narrow.link.max.x <= 520.0 + SLACK, "the trailing group ends past the bar at {}", narrow.link.max.x);
    let empty = status(1200.0, "");
    assert!(empty.message.is_none(), "an empty message shows");
    assert!((empty.readout.width() - wide.readout.width()).abs() < SLACK, "an empty message changed the members before it");
}

#[trial]
fn a_status_progress_is_named_by_its_task_and_reports_cancel() {
    let cancelled = Rc::new(Cell::new(false));
    let mut fixture = status_fixture("", Rc::clone(&cancelled), Rc::new(Cell::new(Status::default())));
    fixture.room(Vec2::new(900.0, 80.0)).settle();
    let progress = fixture.by_role(Role::PROGRESS_INDICATOR);
    assert_eq!(progress.len(), 1, "one progress indicator");
    assert_eq!(craft_ui::testing::access::name(fixture.tree(), progress[0].node), Some("Blur"), "the task's name labels the progress");
    let cancel = fixture.by_label("Cancel").expect("the cancel button");
    fixture.click(&cancel).settle();
    assert!(cancelled.get(), "a click on Cancel reports it");
}
