//! `Bar`: a body too wide for the bar shrinks its members before it scrolls, a captioned field among them too; a
//! bar gives way from its end, a status bar's trailing group keeps its width, and a row of labelled controls in it
//! lays out under right-to-left.

use craft_behavior::CanvasView;
use craft_behavior::IconId;
use craft_behavior::kurbo::{Point, Size};
use craft_theme::variant::Tone;
use craft_ui::bars::{Bar, BarKind, BreadcrumbBar, StatusItem, StatusMessage, StatusProgress, StatusReadout, StatusValues, path_crumbs};
use craft_ui::buttons::{Button, IconButton, Link};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::pickers::Select;
use craft_ui::style::GlyphRole;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::layout::Block;
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
            let select = Select::new("Sample", Bind::given(1u8), &entries).sizing(Sizing::Content).show(ui).response.rect;
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
    let (select, button) = bar(200.0, 1);
    assert!(select.width() < wide.width() - SLACK, "the select kept its width {} in a narrow bar", select.width());
    assert!(button.max.x <= 200.0 + SLACK, "the last member ends at {} past the bar's end", button.max.x);
    // Buttons do not shrink: when even the select's least leaves no room for them, the select keeps its width (at most
    // the bar's room) and the rest scrolls.
    let (crowded, button) = bar(200.0, 6);
    assert!(button.max.x > 200.0, "six buttons fit a 200-point bar: {button:?}");
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
    let narrow = captioned(260.0, 1);
    assert!(narrow.select.width() < wide.select.width() - SLACK, "the select kept its width {} in a narrow bar", narrow.select.width());
    assert!(narrow.last.max.x <= 260.0 + SLACK, "the last member ends at {} past the bar's end", narrow.last.max.x);
    assert!(
        narrow.caption.max.x <= narrow.select.min.x + SLACK && narrow.caption.center().y > narrow.select.min.y,
        "the caption {:?} is not beside the select {:?}",
        narrow.caption,
        narrow.select
    );
    // Once the field is at its least, the rest scrolls.
    let crowded = captioned(260.0, 6);
    assert!(crowded.last.max.x > 260.0, "six buttons fit a 260-point bar: {:?}", crowded.last);
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
            out.set(Select::new("Sample Size", Bind::given(0u8), &entries).sizing(Sizing::Content).show(ui).response.rect);
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
    // The widths at which the buttons after it, at their least, leave the select less than a field's own least.
    let narrowed = (200..=400).step_by(10).map(|width| select_in(width as f32)).find(|(rect, _)| rect.width() < 96.0);
    let Some((narrow, elided)) = narrowed else { panic!("the select stayed at least 96 wide at every width") };
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

fn status_fixture(message: &'static str, seen: Rc<Cell<Status>>) -> Fixture {
    Fixture::new(move |ui| {
        let (mut progress, mut link) = (Rect::NOTHING, Rect::NOTHING);
        let shown = Bar::new("Status", BarKind::Status)
            .trailing(|ui, _| {
                let output = StatusProgress::new("Blur", Some(0.45)).cancel("Cancel").show(ui);
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
    status_fixture(message, Rc::clone(&seen)).room(Vec2::new(width, 80.0)).settle();
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

/// What a tool bar of two search fields and a button shows: the bar's rect and the parts of the fields and the button its view
/// shows.
#[derive(Clone, Copy)]
struct Filling {
    bar: Rect,
    fields: [Rect; 2],
    button: Rect,
}

/// A content-sized column `room` wide holding a block `column` wide and a tool bar of two search fields sized
/// `fields` and a button, after `pages` pages of the bar toward its end.
fn filling_bar(room: f32, column: f32, fields: Sizing, pages: usize) -> Filling {
    let empty = Filling { bar: Rect::NOTHING, fields: [Rect::NOTHING; 2], button: Rect::NOTHING };
    let seen = Rc::new(Cell::new(empty));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let (mut name, mut kind) = (String::new(), String::new());
        Stack::new().show(ui, |ui, _| {
            Block::new(Vec2::new(column, 20.0)).show(ui);
            let mut shown = empty;
            let bar = Bar::new("Filters", BarKind::Tool).show(ui, |ui, _| {
                shown.fields[0] = TextField::search("Name", Bind::bound(&mut name)).sizing(fields).show(ui).response.interact_rect;
                shown.fields[1] = TextField::search("Kind", Bind::bound(&mut kind)).sizing(fields).show(ui).response.interact_rect;
                shown.button = Button::new("Apply").show(ui).interact_rect;
            });
            shown.bar = bar.output.response.rect;
            out.set(shown);
        });
    });
    fixture.room(Vec2::new(room, 200.0)).settle();
    for _ in 0..pages {
        let target = fixture.by_label("Filters").expect("the bar is in the tree");
        fixture.access(AccessActionKind::ScrollRight, &target).settle();
    }
    seen.get()
}

#[trial]
fn filling_bar_members_keep_their_natural_width_and_the_bar_scrolls() {
    // A bar's row scrolls along x, so a content-sized field there measures at its natural width.
    let natural = filling_bar(1400.0, 1000.0, Sizing::Content, 0).fields;
    let held = filling_bar(1400.0, 1000.0, Sizing::FillWidth, 0);
    let dump = format!("bar {:?}, fields {:?}, button {:?}", held.bar, held.fields, held.button);
    assert!(held.bar.width() >= 1000.0 - SLACK, "the bar is held across its column: {dump}");
    assert!(held.fields[0].width() > natural[0].width() + SLACK, "the fields fill the bar held at the column's width: {dump}");
    let (lead, trail) = (held.fields[0].min.x - held.bar.min.x, held.bar.max.x - held.button.max.x);
    assert!((lead - trail).abs() < 1.0, "the button reaches the bar's end, as the first field its start: {dump}");
    // Narrowed, the members keep their widths and the bar scrolls. A member's shown rect is the part the bar's view
    // shows (its interact rect), so the bar is paged toward its end: on some page each field shows at least its natural width and the
    // button shows whole, and on no page do two members overlap.
    let button = held.button.width();
    let (mut widest, mut button_whole) = ([0.0_f32; 2], Vec::new());
    for pages in 0..=12 {
        let page = filling_bar(200.0, 100.0, Sizing::FillWidth, pages);
        let dump = format!("page {pages}: bar {:?}, fields {:?}, button {:?}", page.bar, page.fields, page.button);
        let shown: Vec<Rect> = page.fields.iter().chain([&page.button]).copied().filter(|rect| rect.is_positive()).collect();
        for (at, first) in shown.iter().enumerate() {
            for second in &shown[at + 1..] {
                assert!(first.intersect(*second).width() <= SLACK, "two members overlap: {dump}");
            }
        }
        for (widest, field) in widest.iter_mut().zip(page.fields) {
            if field.is_positive() {
                *widest = widest.max(field.width());
            }
        }
        button_whole.push(page.button.is_positive() && (page.button.width() - button).abs() < SLACK);
    }
    for (widest, natural) in widest.iter().zip(natural) {
        assert!(*widest >= natural.width() - SLACK, "a field never showed its natural width {}: at most {widest}", natural.width());
    }
    assert!(!button_whole[0], "the members fit the narrow bar unscrolled");
    assert!(button_whole.contains(&true), "the bar does not scroll to show its last member whole: {button_whole:?}");
}

/// The rects of `count` buttons in a tool bar `width` wide that wraps or scrolls.
fn tools(width: f32, wrap: bool, count: usize) -> Vec<Rect> {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let bar = Bar::new("Tools", BarKind::Tool);
        let bar = if wrap { bar.wrap() } else { bar };
        bar.show(ui, |ui, _| {
            *out.borrow_mut() = (0..count).map(|at| Button::new(format!("Tool {at}")).show(ui).rect).collect();
        });
    });
    fixture.room(Vec2::new(width, 300.0)).settle();
    seen.borrow().clone()
}

/// The rect of a select with a long entry in a tool bar `width` wide that wraps.
fn wide_member(width: f32) -> Rect {
    let seen = Rc::new(Cell::new(Rect::NOTHING));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(0u8, "Current and Below, No Adjustments"), Entry::new(1, "All Layers")];
        Bar::new("Tools", BarKind::Tool).wrap().show(ui, |ui, _| {
            out.set(Select::new("Sample", Bind::given(0u8), &entries).sizing(Sizing::Content).show(ui).response.rect);
        });
    });
    fixture.room(Vec2::new(width, 300.0)).settle();
    seen.get()
}

#[trial]
fn a_wrapping_bar_continues_on_a_new_line() {
    let plain = tools(200.0, false, 6);
    assert!(plain.last().is_some_and(|last| last.max.x > 200.0), "six tools fit a 200-point bar that scrolls: {plain:?}");
    let wrapped = tools(200.0, true, 6);
    assert_eq!(wrapped.len(), 6, "every member shows");
    for rect in &wrapped {
        assert!(rect.min.x >= -SLACK && rect.max.x <= 200.0 + SLACK, "{rect:?} lies past the bar's edge");
    }
    let first_line = wrapped[0].center().y;
    assert!(wrapped.iter().any(|rect| rect.center().y > first_line + SLACK), "no member continued on a new line: {wrapped:?}");
    for pair in wrapped.windows(2) {
        let (before, after) = (pair[0], pair[1]);
        let same_line = (before.center().y - after.center().y).abs() < SLACK;
        assert!(
            (same_line && before.max.x <= after.min.x + SLACK) || after.center().y > before.center().y + SLACK,
            "{after:?} is out of reading order after {before:?}"
        );
    }
    let select = wide_member(120.0);
    assert!(select.max.x <= 120.0 + SLACK, "a member wider than the bar kept its width: {select:?}");
}

/// What a bar of three rows, ranked 1 and 2 and unranked, `width` wide shows: whether each of their buttons is there.
fn ranked(width: f32) -> [bool; 3] {
    let mut fixture = Fixture::new(|ui| {
        Bar::new("Tools", BarKind::Tool).show(ui, |ui, _| {
            Row::new().fit_rank(1).show(ui, |ui, _| {
                Button::new("Low").show(ui);
            });
            Row::new().fit_rank(2).show(ui, |ui, _| {
                Button::new("Mid").show(ui);
            });
            Row::new().show(ui, |ui, _| {
                Button::new("Keep").show(ui);
            });
        });
    });
    fixture.room(Vec2::new(width, 80.0)).settle();
    ["Low", "Mid", "Keep"].map(|label| fixture.by_label(label).is_some())
}

#[trial]
fn ranked_rows_leave_a_narrow_bar_by_rank_and_take_their_rule() {
    let widths: Vec<f32> = (60..=600).step_by(20).map(|width| width as f32).collect();
    let shown: Vec<[bool; 3]> = widths.iter().map(|width| ranked(*width)).collect();
    assert_eq!(shown[shown.len() - 1], [true; 3], "a wide bar shows every row");
    assert!(!shown[0][0], "the lowest rank is still there in the narrowest bar");
    for (width, [low, mid, keep]) in widths.iter().zip(&shown) {
        assert!(*keep, "the unranked row left a bar {width} wide");
        assert!(!(*low && !*mid), "the higher rank left before the lower at {width}");
    }
    assert!(shown.iter().any(|[low, mid, _]| !low && *mid), "the lowest rank never left alone: {shown:?}");
}

/// The texts a status bar `width` wide paints, each with whether it was cut: the readout of the document sizes, a long
/// message and the render-path link in its trailing group.
fn readout_texts(width: f32) -> Vec<(String, bool)> {
    let mut fixture = Fixture::new(|ui| {
        let mut values = StatusValues::default();
        values.document_sizes = Some((26_100_000, 31_400_000));
        let canvas = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(800.0, 600.0));
        let view = CanvasView::new(canvas, Point::new(1000.0, 750.0), 0.6667);
        Bar::new("Status", BarKind::Status)
            .trailing(|ui, _| {
                Link::new("CPU Rendering").tone(Tone::Warn).show(ui);
            })
            .show(ui, |ui, _| {
                StatusReadout::new(Bind::given(view), Size::new(2000.0, 1500.0), StatusItem::DocumentSizes, &values).show(ui);
                StatusMessage::new("Exported 14 layers to the folder Documents/Exports/2026").show(ui);
            });
    });
    fixture.room(Vec2::new(width, 80.0)).settle();
    fixture.texts().iter().map(|text| (text.text.clone(), text.elided)).collect()
}

#[trial]
fn a_narrow_bar_gives_way_from_its_end_so_the_message_elides_before_the_readout() {
    let cut = |texts: &[(String, bool)], start: &str| texts.iter().find(|(text, _)| text.starts_with(start)).expect(start).1;
    let wide = readout_texts(1200.0);
    assert!(!cut(&wide, "Doc") && !cut(&wide, "Exported"), "a wide bar cuts nothing");
    let narrow = readout_texts(480.0);
    assert!(cut(&narrow, "Exported"), "the message kept its whole text in a 480-point bar");
    assert!(!cut(&narrow, "Doc"), "the readout's text was cut while the message after it could still give way");
}

fn status_zoom(zoom: f64, comma: bool) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let size = Size::new(2000.0, 1500.0);
        let canvas = Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(800.0, 600.0));
        let scale = craft_ui::runtime::env::Env::of(ui).scale.0;
        let view = CanvasView::new(canvas, Point::new(1000.0, 750.0), zoom / f64::from(scale));
        Bar::new("Status", BarKind::Status).show(ui, |ui, _| {
            StatusReadout::new(Bind::given(view), size, StatusItem::DocumentSizes, &StatusValues::default()).show(ui);
        });
    });
    if comma {
        use craft_host::{AppInfo, Host};
        use std::sync::Arc;
        let (i18n, problems) = craft_i18n::I18n::from_owned([("de".to_owned(), String::new(), String::new())], &["de"]);
        assert!(problems.is_empty(), "{problems:?}");
        i18n.set_format(craft_i18n::Format::from_tag("de"));
        fixture.host(Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).i18n(Arc::new(i18n)).build()));
    }
    fixture.room(Vec2::new(600.0, 80.0)).settle();
    fixture
}

#[trial]
fn the_status_zoom_shows_its_complete_value_and_percent() {
    for (zoom, comma, expected) in [(0.7969, false, "79.69%"), (0.7969, true, "79,69%"), (32.0, false, "3200%"), (31.9999, false, "3199.99%")] {
        let fixture = status_zoom(zoom, comma);
        let number = fixture.texts().iter().find(|text| text.text == expected).expect(expected);
        assert!(
            !number.elided && number.rect.min.x + SLACK >= number.clip.min.x && number.rect.max.x <= number.clip.max.x + SLACK,
            "{expected} is clipped: {:?} outside {:?}",
            number.rect,
            number.clip,
        );
    }
}

#[trial]
fn the_status_zoom_shows_the_complete_dash_without_a_view() {
    use craft_behavior::Mixed;
    use craft_ui::runtime::env::Scale;

    let mut fixture = Fixture::new(|ui| {
        let mut values = StatusValues::default();
        values.document_sizes = Some((1024, 4096));
        Bar::new("Status", BarKind::Status).show(ui, |ui, _| {
            StatusReadout::new(Bind::given(Mixed::<CanvasView>::Mixed), Size::new(2000.0, 1500.0), StatusItem::DocumentSizes, &values).show(ui);
        });
    });
    fixture.scale(Scale::new(1.25)).room(Vec2::new(600.0, 80.0)).settle();

    let zoom = fixture.by_label("Zoom").expect("the status zoom field");
    assert!(fixture.tree().is_disabled_key(zoom.node));
    let dash = fixture.texts().iter().find(|text| text.text == "—" && zoom.rect.contains(text.rect.center())).expect("a dash inside the zoom field");
    assert!(
        !dash.elided && dash.rect.min.x + SLACK >= dash.clip.min.x && dash.rect.max.x <= dash.clip.max.x + SLACK,
        "the empty zoom dash is clipped: {:?} outside {:?}",
        dash.rect,
        dash.clip,
    );
}

#[trial]
fn the_status_zoom_still_scrubs_and_applies_edited_text() {
    use egui::{Event, Key, Modifiers};

    let view = Rc::new(Cell::new(CanvasView::new(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0)), Point::ZERO, 1.0)));
    let shown = view.clone();
    let committed = Rc::new(Cell::new(None));
    let seen = committed.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut view = shown.get();
        Bar::new("Status", BarKind::Status).show(ui, |ui, _| {
            let output =
                StatusReadout::new(Bind::bound(&mut view), Size::new(2000.0, 1500.0), StatusItem::DocumentSizes, &StatusValues::default()).show(ui);
            if let Some(view) = output.view.committed() {
                seen.set(Some(view.zoom));
            }
        });
        shown.set(view);
    });
    fixture.room(Vec2::new(600.0, 80.0)).settle();
    let zoom = fixture.by_label("Zoom").expect("the status zoom field");
    fixture.drag(&zoom, Vec2::new(12.0, 0.0)).settle();
    let scrubbed = view.get().zoom;
    assert!(scrubbed > 1.0, "the status bar still scrubs the zoom to the right");

    fixture.click(&zoom).settle();
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.frame(vec![Event::Paste("079.69%".to_owned())]).settle();
    let before_tab = view.get().zoom;
    committed.set(None);
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(view.get().zoom, before_tab, "Tab does not change the current zoom");
    assert_eq!(committed.get(), None, "Tab does not commit the draft");
    let zoom = fixture.by_label("Zoom").expect("the status zoom field");
    assert_eq!(fixture.tree().value_of(zoom.node), Some("79.69%"), "Tab rewrites the draft with its suffix");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert!(committed.get().is_some_and(|value| (value - 0.7969).abs() < 1e-7), "Enter commits the percent-bearing zoom");
}

/// A Row in a Bar under right-to-left measures in a room unbounded at its start edge, the left one: a labelled check
/// box there lays out with finite rects inside the bar, after the button before it in reading order. Guards against a
/// label row built from the room's start and width, which was NaN there.
#[trial]
fn a_labelled_check_box_in_a_row_in_a_bar_lays_out_under_rtl() {
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let mut alpha = Rect::NOTHING;
        let bar = Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                alpha = Button::new("Alpha").show(ui).rect;
                Checkbox::new(Bind::given(true)).label("Delete Cropped Pixels").show(ui);
            });
        });
        out.set((bar.output.response.rect, alpha));
    });
    fixture.direction(craft_i18n::Direction::Rtl).room(Vec2::new(400.0, 80.0)).settle();
    let (bar, alpha) = seen.get();
    let label = fixture.texts().iter().find(|text| text.text == "Delete Cropped Pixels").map(|text| text.rect).expect("the label is painted");
    let control = fixture.by_label("Delete Cropped Pixels").expect("the check box is in the tree").rect;
    for rect in [label, control] {
        assert!(rect.is_finite() && bar.contains_rect(rect), "{rect:?} lies outside the bar {bar:?}");
    }
    assert!(label.max.x <= alpha.min.x + SLACK, "the check box {label:?} follows Alpha {alpha:?} from the right");
}

/// A text tab strip in a tool bar's row, as a window's mode strip: the row gives it unbounded room, so it measures
/// the natural length of its tabs and every tab shows, none in the overflow menu, and the strip ends before the
/// bar's trailing group.
#[trial]
fn a_tab_strip_in_a_bar_row_takes_its_tabs_natural_length() {
    use craft_behavior::keyed;
    use craft_ui::contract::ShowCollection as _;
    use craft_ui::tabs::{Tab, TabKind, TabStrip};
    const MODES: [&str; 4] = ["All tools", "Edit", "Convert", "E-Sign"];
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        Row::new().show(ui, |ui, _| {
            let mut trailing = Rect::NOTHING;
            let mut strip = Rect::NOTHING;
            Bar::new("Modes", BarKind::Tool)
                .trailing(|ui, _| {
                    trailing = IconButton::new(IconId::new("printer")).label("Print").show(ui).rect;
                })
                .show(ui, |ui, _| {
                    let keys = [0_usize, 1, 2, 3];
                    let source = keyed(&keys, |key: &usize| *key);
                    let tabs = TabStrip::new("Modes").kind(TabKind::Text).natural_height(true).shown(1);
                    strip = tabs.show_items(ui, &source, &mut |key, _| Tab::new(MODES[*key])).response.response.rect;
                });
            out.set((strip, trailing));
        });
    });
    fixture.room(Vec2::new(1274.0, 200.0)).settle();
    let (strip, trailing) = seen.get();
    let shown: Vec<Rect> = MODES.iter().filter_map(|title| fixture.by_label(title)).map(|tab| tab.rect).collect();
    let dump = format!("strip {strip:?}, trailing {trailing:?}, tabs {shown:?}");
    assert_eq!(shown.len(), MODES.len(), "every tab shows: {dump}");
    let length: f32 = shown.iter().map(Rect::width).sum();
    assert!(strip.width() >= length - SLACK, "the strip is as long as its tabs: {dump}");
    assert!(strip.max.x <= trailing.min.x + SLACK, "and ends before the trailing group: {dump}");
}

/// An icon-only button in an options bar draws its icon at the tool size, as the align strip's buttons do.
#[trial]
fn an_icon_button_in_an_options_bar_draws_its_icon_at_the_tool_size() {
    let mut fixture = Fixture::new(|ui| {
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            IconButton::new(IconId::new("settings")).label("Plain").show(ui);
            IconButton::new(IconId::new("settings")).label("Tool").glyphs(GlyphRole::Tool).show(ui);
        });
    });
    fixture.room(Vec2::new(400.0, 80.0)).settle();
    let sides: Vec<f32> = fixture
        .styles_of(std::any::type_name::<IconButton<'static>>())
        .iter()
        .flat_map(|seen| seen.looks.icons.iter().map(|icon| icon.side))
        .collect();
    // Buttons that resolve the same looks share one recorded style, so a single side means they agree.
    assert!(!sides.is_empty(), "the buttons resolved a style");
    assert!(sides.iter().all(|side| (side - sides[0]).abs() <= 0.01), "the options bar's button and the tool button differ: {sides:?}");
}

/// Each pass's stops, each with the roving group it registered in.
type StopGroups = Rc<std::cell::RefCell<Vec<Vec<(egui::Id, Option<egui::Id>)>>>>;

/// A tool bar's buttons are members of its roving group from the first pass that shows them: that pass registers the
/// stops in the groups every later pass registers them in.
#[trial]
fn a_tool_bar_is_one_roving_group_from_its_first_pass() {
    let seen: StopGroups = Rc::default();
    let passes = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        Button::new("Before").show(ui);
        Bar::new("Tools", BarKind::Tool).show(ui, |ui, _| {
            IconButton::new(IconId::new("brush")).label("Brush").show(ui);
            IconButton::new(IconId::new("eraser")).label("Eraser").show(ui);
        });
        passes.borrow_mut().push(craft_ui::testing::focus::pass_stop_groups(ui.ctx()));
    });
    fixture.settle();
    let seen = seen.borrow();
    let (first, settled) = (seen.first().expect("a pass ran"), seen.last().expect("a pass ran"));
    let grouped = settled.iter().filter(|(_, group)| group.is_some()).count();
    assert!(grouped >= 2, "the tools are in the bar's roving group: {settled:?}");
    assert_eq!(first, settled, "the first pass registers the stops as a settled pass does");
}

/// A tool bar's tools report their place in the set from the pass's members: in the first frame, and in the frame a
/// tool joins the bar.
#[trial]
fn a_tool_bar_reports_set_positions_from_its_first_pass() {
    const TOOLS: [&str; 4] = ["Brush", "Eraser", "Smudge", "Blur"];
    let count = Rc::new(Cell::new(3_usize));
    let ids: Rc<std::cell::RefCell<Vec<egui::Id>>> = Rc::default();
    let (shown, seen) = (count.clone(), ids.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut now = Vec::new();
        Bar::new("Tools", BarKind::Tool).show(ui, |ui, _| {
            for name in &TOOLS[..shown.get()] {
                // The bar's body runs in its measuring run and its real run: each tool once.
                let id = IconButton::new(IconId::new("brush")).label(*name).show(ui).id;
                if !now.contains(&id) {
                    now.push(id);
                }
            }
        });
        *seen.borrow_mut() = now;
    });
    let places = |fixture: &Fixture| -> Vec<(usize, usize)> {
        ids.borrow().iter().map(|id| fixture.tree().position(*id).expect("a tool's place in the set")).collect()
    };
    fixture.idle(1);
    let first = places(&fixture);
    let size = first[0].1;
    assert!(size >= 3, "the set holds the three tools: {first:?}");
    assert!(first.windows(2).all(|pair| pair[1].0 == pair[0].0 + 1 && pair[1].1 == size), "the tools follow each other in one set: {first:?}");
    count.set(4);
    fixture.idle(1);
    let joined = places(&fixture);
    assert!(joined.len() == 4 && joined.iter().all(|(_, held)| *held == size + 1), "the set counts the new tool in the frame it joins: {joined:?}");
}

/// The painted rects of a status bar `width` wide that ends as production's did: the zoom and the message in the body,
/// then the pointer's position (rank 2) and the timings (rank 1) in the trailing group. In that order, `None` for one
/// not shown, and the bar's rect last.
fn end_group(width: f32) -> ([Option<Rect>; 4], Rect) {
    const TEXTS: [&str; 4] = ["93.5 %", "Saved recovery data for Untitled-1", "X: 120 pt  Y: 48 pt", "render 0.6 ms"];
    let seen = Rc::new(Cell::new((None, Rect::NOTHING)));
    let shown = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let bar = Bar::new("Status", BarKind::Status)
            .trailing(|ui, _| {
                Row::new().fit_rank(2).show(ui, |ui, _| {
                    Text::new(TEXTS[2]).show(ui);
                });
                Row::new().fit_rank(1).show(ui, |ui, _| {
                    Text::new(TEXTS[3]).show(ui);
                });
            })
            .show(ui, |ui, _| {
                Text::new(TEXTS[0]).show(ui);
                StatusMessage::new(TEXTS[1]).show(ui).map(|message| message.rect)
            });
        shown.set((bar.inner.flatten(), bar.output.response.rect));
    });
    fixture.room(Vec2::new(width, 80.0)).settle();
    let rect_of = |text: &str| fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect);
    let (message, bar) = seen.get();
    ([rect_of(TEXTS[0]), message.filter(|rect| rect.width() > 0.0), rect_of(TEXTS[2]), rect_of(TEXTS[3])], bar)
}

/// A status bar's trailing group joins the body's fit: on a wide bar the timings end at the bar's end with the position
/// before them; a narrowing bar drops the timings first, then the position, while the message keeps its whole width,
/// and only then cuts the message.
#[trial]
fn a_narrow_status_bar_drops_its_ranked_trailing_members_before_it_cuts_its_message() {
    let ([zoom, message, position, timings], bar) = end_group(1200.0);
    let [zoom, message, position, timings] = [zoom, message, position, timings].map(|rect| rect.expect("a wide bar shows every member"));
    assert!(bar.right() - timings.right() < bar.width() / 20.0, "the timings end at the bar's end: {timings:?} in {bar:?}");
    assert!(position.right() <= timings.left() && message.right() <= position.left() && zoom.right() <= message.left(), "in reading order");
    let whole = message.width();
    let shown: Vec<(u32, [Option<Rect>; 4])> = (150..=1200).step_by(10).map(|width| (width, end_group(width as f32).0)).collect();
    for (width, [zoom, message, position, timings]) in &shown {
        assert!(zoom.is_some() && message.is_some(), "the body's members stay at {width}: {zoom:?} {message:?}");
        assert!(!(timings.is_some() && position.is_none()), "the timings leave before the position at {width}");
        if position.is_some() || timings.is_some() {
            let message = message.expect("the message shows");
            assert!(message.width() > whole - SLACK, "the message is cut to {} of {whole} at {width} while a ranked member stays", message.width());
        }
    }
    assert!(shown.iter().any(|(_, [.., position, timings])| position.is_some() && timings.is_none()), "the timings leave alone first");
    assert!(
        shown.iter().any(|(_, [_, message, position, _])| position.is_none() && message.is_some_and(|rect| rect.width() < whole - SLACK)),
        "a narrow bar drops the position, then cuts the message"
    );
}
