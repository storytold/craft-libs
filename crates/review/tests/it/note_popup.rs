//! The note pop-up's hover and selection, its corner grip, and when the comment composer closes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_review::ui::{CommentComposer, ComposerClose, ComposerDraft, NOTE_POPUP_LIMITS, NoteDrafts, NotePopup, NotePopupMessage};
use craft_ui::buttons::Button;
use craft_ui::contract::Show as _;
use craft_ui::contract::overlay::PlacementKind;
use craft_ui::testing::{Fixture, trial};
use egui::{Event, Rect, pos2, vec2};

/// How far a measured rect may sit from the expected one.
const SLACK: f32 = 1.5;

const NOTE: [NotePopupMessage<'static>; 1] =
    [NotePopupMessage { id: 1, title: "Sticky Note", author: "Ana", text: "Check the margin.", modified: None, depth: 0, editable: true }];

/// The title's close control reports the close to the host.
#[trial]
fn the_note_header_close_reports_to_the_host() {
    let closed = Rc::new(Cell::new(false));
    let result = closed.clone();
    let mut drafts = NoteDrafts::default();
    let mut fixture = Fixture::new(move |ui| {
        let out = NotePopup::new(1, "Note", Rect::from_min_size(pos2(100.0, 100.0), vec2(320.0, 200.0)), &NOTE, &mut drafts).show(ui);
        result.set(result.get() || out.closed);
    });
    fixture.room(vec2(1000.0, 800.0)).settle();
    let close = fixture.by_label("Close").expect("the header has a close control");
    fixture.click(&close).settle();
    assert!(closed.get(), "closing reaches the host");
}

/// Closing after editing reports the text and close together, so the host can undo the gesture once.
#[trial]
fn closing_an_edited_note_reports_text_with_the_close() {
    let reports = Rc::new(RefCell::new(Vec::new()));
    let result = reports.clone();
    let mut drafts = NoteDrafts::default();
    let mut fixture = Fixture::new(move |ui| {
        let out = NotePopup::new(1, "Note", Rect::from_min_size(pos2(100.0, 100.0), vec2(320.0, 200.0)), &NOTE, &mut drafts).show(ui);
        if out.closed || out.body.is_some() {
            result.borrow_mut().push((out.closed, out.body));
        }
    });
    fixture.room(vec2(1000.0, 800.0)).settle();
    let field = fixture.by_label("Add a comment").expect("the note's text field");
    fixture.click(&field).type_text(" More").settle();
    let close = fixture.by_label("Close").expect("the close control");
    fixture.press(&close).frame(vec![Event::PointerMoved(pos2(900.0, 700.0))]).release_at(pos2(900.0, 700.0)).settle();
    assert!(reports.borrow().is_empty(), "an aborted close keeps the draft editable");
    fixture.type_text(" still").settle();
    fixture.click(&close).settle();
    let reports = reports.borrow();
    assert_eq!(reports.len(), 1, "the edit and close are one report: {reports:?}");
    assert_eq!((reports[0].0, reports[0].1.as_deref()), (true, Some("Check the margin. More still")));
}

/// The host of one note pop-up: a button stands for the note's mark, and the pop-up sits at `rect`, which takes each
/// reported placement.
struct NoteHost {
    rect: Cell<Rect>,
    selected: Cell<bool>,
    mark: Cell<Rect>,
    window: Cell<Rect>,
    placed: RefCell<Vec<(Rect, PlacementKind)>>,
}

impl NoteHost {
    fn new(rect: Rect) -> Rc<Self> {
        Rc::new(NoteHost {
            rect: Cell::new(rect),
            selected: Cell::new(false),
            mark: Cell::new(Rect::NOTHING),
            window: Cell::new(Rect::NOTHING),
            placed: RefCell::new(Vec::new()),
        })
    }

    /// A settled fixture that shows the mark and the pop-up.
    fn fixture(host: &Rc<Self>) -> Fixture {
        let shown = Rc::clone(host);
        let mut drafts = NoteDrafts::default();
        let mut fixture = Fixture::new(move |ui| {
            let mark = Button::new("Mark").show(ui);
            let out = NotePopup::new(1, "Note", shown.rect.get(), &NOTE, &mut drafts)
                .anchor(mark.rect, mark.hovered())
                .selected(shown.selected.get())
                .show(ui);
            shown.mark.set(mark.rect);
            shown.window.set(out.response.rect);

            if let Some((rect, kind)) = out.placed {
                shown.rect.set(rect);
                shown.placed.borrow_mut().push((rect, kind));
            }
        });
        fixture.room(vec2(1000.0, 800.0)).settle();
        fixture
    }
}

/// Hovering the mark or the window and selecting the note leave the window where it is and report no placement; the
/// connector is painted by the window itself and is covered by the stories.
#[trial]
fn hovering_the_mark_or_window_or_selecting_the_note_reports_no_placement() {
    let host = NoteHost::new(Rect::from_min_size(pos2(400.0, 300.0), vec2(240.0, 160.0)));
    let mut fixture = NoteHost::fixture(&host);
    let (mark, window) = (host.mark.get(), host.window.get());
    assert!(window.is_positive() && !mark.intersects(window), "the window shows apart from the mark: {mark:?}, {window:?}");
    for at in [pos2(900.0, 40.0), mark.center(), window.center()] {
        fixture.frame(vec![Event::PointerMoved(at)]).settle();
        assert!((host.window.get().min - window.min).length() < SLACK, "the pointer at {at:?} leaves the window: {:?}", host.window.get());
    }
    host.selected.set(true);
    fixture.settle();
    assert!((host.window.get().min - window.min).length() < SLACK, "a selected note stays put: {:?}", host.window.get());
    // The window over the mark shows too.
    host.rect.set(Rect::from_min_size(mark.min, vec2(240.0, 160.0)));
    fixture.settle();
    assert!(host.window.get().intersects(mark), "the window lies over the mark: {:?}", host.window.get());
    assert!(host.placed.borrow().is_empty(), "no hover or selection is a placement: {:?}", host.placed.borrow());
}

/// The lower end corner resizes the window within `NOTE_POPUP_LIMITS`; a press that moves under 2 points reports
/// nothing, and a resize reports `placed` once, when the drag is released.
#[trial]
fn a_corner_drag_resizes_within_the_limits_and_reports_on_release() {
    let start = Rect::from_min_size(pos2(300.0, 200.0), vec2(240.0, 160.0));
    let host = NoteHost::new(start);
    let mut fixture = NoteHost::fixture(&host);
    let window = host.window.get();
    assert!((window.size() - start.size()).length() < SLACK, "the window shows at the host's size: {window:?}");
    // A press on the grip that hardly moves changes nothing.
    fixture.drag_at(window.max, vec2(1.0, 0.5)).settle();
    assert!(host.placed.borrow().is_empty(), "a move under 2 pt reports nothing: {:?}", host.placed.borrow());
    assert!((host.window.get().size() - start.size()).length() < SLACK, "the window keeps its size: {:?}", host.window.get());
    // The window follows the drag while it runs, and the report waits for the release.
    let grow = vec2(60.0, 40.0);
    fixture.drag_at_pausing(window.max, grow, |fixture| {
        fixture.idle(1);
        assert!(host.placed.borrow().is_empty(), "nothing is reported while the drag runs: {:?}", host.placed.borrow());
        assert!(host.window.get().width() > start.width() + grow.x / 2.0, "the window follows the drag: {:?}", host.window.get());
    });
    fixture.settle();
    let placed = host.placed.borrow().clone();
    assert_eq!(placed.len(), 1, "one report when the drag commits: {placed:?}");
    let (rect, kind) = placed[0];
    assert!(matches!(kind, PlacementKind::Size(_)), "the grip reports a resize: {kind:?}");
    assert!((rect.size() - (start.size() + grow)).length() < SLACK, "the window grew by the drag: {rect:?}");
    assert!((rect.min - start.min).length() < SLACK, "the top left stays: {rect:?}");
    assert!((host.window.get().size() - rect.size()).length() < SLACK, "the window takes the reported size: {:?}", host.window.get());
    // Past the least size the window stops there.
    host.placed.borrow_mut().clear();
    fixture.drag_at(host.window.get().max, vec2(-400.0, -400.0)).settle();
    let placed = host.placed.borrow().clone();
    assert_eq!(placed.len(), 1, "one report for the second drag: {placed:?}");
    let (rect, _) = placed[0];
    assert!((rect.size() - NOTE_POPUP_LIMITS.min).length() < SLACK, "the window stops at its least size: {rect:?}");
}

/// Each reply is a panel of its own, in thread order: an answer to a reply sits inside that reply's panel, inset on
/// both sides, and the replies to the note line up.
#[trial]
fn a_pop_up_nests_each_answer_in_the_panel_of_the_reply_it_answers() {
    let message = |id: u64, author: &'static str, text: &'static str, depth: usize| NotePopupMessage {
        id,
        title: "Sticky Note",
        author,
        text,
        modified: None,
        depth,
        editable: depth == 0,
    };
    let thread =
        [message(1, "Ana", "First", 0), message(2, "Bo", "Second", 1), message(3, "Cy", "Second, first answer", 2), message(4, "Di", "Third", 1)];
    let mut drafts = NoteDrafts::default();
    let mut fixture = Fixture::new(move |ui| {
        let _ = NotePopup::new(1, "Note", Rect::from_min_size(pos2(100.0, 100.0), vec2(320.0, 600.0)), &thread, &mut drafts).show(ui);
    });
    fixture.room(vec2(1000.0, 800.0)).settle();
    let panel = |author: &str| fixture.by_label(&format!("Comment by {author}")).unwrap_or_else(|| panic!("{author}'s reply panel")).rect;
    let (second, answer, third) = (panel("Bo"), panel("Cy"), panel("Di"));
    assert!(second.bottom() <= third.top(), "thread order: {second:?} then {third:?}");
    assert!(second.contains_rect(answer), "the answer sits inside its reply's panel: {second:?} holds {answer:?}");
    assert!(answer.left() > second.left() && answer.right() < second.right(), "inset on both sides: {second:?} {answer:?}");
    assert!((third.left() - second.left()).abs() < SLACK && (third.right() - second.right()).abs() < SLACK, "replies to the note line up");
}

/// The host of one composer: it shows the composer until it posts or cancels.
struct ComposerHost {
    open: Cell<bool>,
    posted: RefCell<Option<String>>,
    cancelled: Cell<bool>,
    draft: RefCell<String>,
}

/// Where a press lands outside the composer's card.
const OUTSIDE: egui::Pos2 = egui::Pos2::new(700.0, 550.0);

/// A composer's hint names its field; without one the field is "Add a comment".
#[trial]
fn a_composer_hint_names_its_field() {
    for hint in [Some("Text to insert"), None] {
        let mut draft = String::new();
        let mut fixture = Fixture::new(move |ui| {
            let composer = CommentComposer::new(Some(pos2(200.0, 200.0)), &mut draft);
            let _ = match hint {
                Some(hint) => composer.hint(hint),
                None => composer,
            }
            .show(ui);
        });
        fixture.room(vec2(800.0, 600.0)).settle();
        let name = hint.unwrap_or("Add a comment");
        assert!(fixture.by_label(name).is_some(), "the field is named {name}");
    }
}

/// `ComposerClose::OnPostOrCancel` keeps the card open on a press outside and closes it on Post or Cancel; the default
/// closes it on the press outside.
#[trial]
fn a_composer_kept_open_on_outside_press_closes_only_on_post_or_cancel() {
    for close in [ComposerClose::OnPostOrCancel, ComposerClose::OnOutsidePress] {
        let host = Rc::new(ComposerHost {
            open: Cell::new(true),
            posted: RefCell::new(None),
            cancelled: Cell::new(false),
            draft: RefCell::new(String::new()),
        });
        let shown = Rc::clone(&host);
        let mut fixture = Fixture::new(move |ui| {
            if !shown.open.get() {
                return;
            }
            let mut draft = shown.draft.borrow_mut();
            let out = CommentComposer::new(Some(pos2(200.0, 200.0)), &mut draft).close(close).show(ui);
            if out.cancelled {
                shown.cancelled.set(true);
                shown.open.set(false);
            }
            if let Some(text) = out.posted {
                *shown.posted.borrow_mut() = Some(text);
                shown.open.set(false);
            }
        });
        fixture.room(vec2(800.0, 600.0)).settle();
        assert!(fixture.by_label("Post").is_some(), "{close:?}: the composer shows");
        fixture.click_at(OUTSIDE).settle();
        if close != ComposerClose::OnPostOrCancel {
            assert!(host.cancelled.get() && !host.open.get(), "{close:?}: a press outside closes the composer");
            continue;
        }
        assert!(host.open.get() && !host.cancelled.get(), "{close:?}: a press outside keeps the composer open");
        assert!(fixture.by_label("Post").is_some(), "{close:?}: the card still shows");
        // The field has the focus as the card opens.
        fixture.type_text("Looks good  ").settle();
        let post = fixture.by_label("Post").expect("the Post button");
        fixture.click(&post).settle();
        assert_eq!(host.posted.borrow().as_deref(), Some("Looks good"), "{close:?}: Post sends the draft, trailing blanks trimmed");
        assert!(!host.open.get(), "{close:?}: Post closes the composer");
        host.open.set(true);
        fixture.settle();
        fixture.click_at(OUTSIDE).settle();
        assert!(host.open.get() && !host.cancelled.get(), "{close:?}: the reopened card stays open on a press outside");
        let cancel = fixture.by_label("Cancel").expect("the Cancel button");
        fixture.click(&cancel).settle();
        assert!(host.cancelled.get() && !host.open.get(), "{close:?}: Cancel closes the composer");
    }
}

/// Reply sits at the end edge of the body under the note's text, as production's small right-aligned button.
#[trial]
fn the_reply_button_sits_at_the_end_edge_of_the_window() {
    let host = NoteHost::new(Rect::from_min_size(pos2(100.0, 100.0), vec2(320.0, 200.0)));
    let fixture = NoteHost::fixture(&host);
    let reply = fixture.by_label("Reply").expect("the Reply button");
    let window = host.window.get();
    assert!(window.right() - reply.rect.right() < 24.0, "Reply ends near the window's end edge: {:?} in {window:?}", reply.rect);
    assert!(reply.rect.left() > window.center().x, "Reply sits in the end half: {:?} in {window:?}", reply.rect);
}

/// The title band is the note's colour across the window's whole width, Close included, as production's.
#[trial]
fn the_title_band_runs_under_the_close_control() {
    use craft_ui::testing::{Frames, install_spec};
    let mut frames = Frames::uninstalled();
    craft_ui::install(&frames.ctx, install_spec()).expect("the runtime installs");
    frames.record_paint();
    let mut drafts = NoteDrafts::default();
    let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(320.0, 200.0));
    for _ in 0..4 {
        frames.frame(vec![], |ui| {
            let _ = NotePopup::new(1, "Note", rect, &NOTE, &mut drafts).color(Some([10, 20, 200])).show(ui);
        });
    }
    let mut paint = frames.take_paint().expect("paint is recorded");
    paint.textures_delta.clear();
    fn band(shape: &egui::Shape, out: &mut Vec<Rect>) {
        match shape {
            egui::Shape::Rect(r) if r.fill == egui::Color32::from_rgb(10, 20, 200) => out.push(r.rect),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| band(s, out)),
            _ => {}
        }
    }
    let mut bands = Vec::new();
    paint.shapes.iter().for_each(|clipped| band(&clipped.shape, &mut bands));
    let widest = bands.iter().map(|r| r.width()).fold(0.0, f32::max);
    assert!((widest - rect.width()).abs() < 1.5, "the band covers the window's width, close control included: {bands:?}");
}

/// A composer that edits an existing comment saves it; one that starts a comment posts it.
#[trial]
fn an_edit_composer_reads_save_and_a_new_one_reads_post() {
    for (kind, word, other) in [(ComposerDraft::New, "Post", "Save"), (ComposerDraft::Edit, "Save", "Post")] {
        let mut draft = String::from("text");
        let mut fixture = Fixture::new(move |ui| {
            let _ = CommentComposer::new(Some(pos2(200.0, 200.0)), &mut draft).draft_kind(kind).show(ui);
        });
        fixture.room(vec2(800.0, 600.0)).settle();
        assert!(fixture.by_label(word).is_some(), "{kind:?}: the button reads {word}");
        assert!(fixture.by_label(other).is_none(), "{kind:?}: not {other}");
    }
}

/// An edit composer opens with the caret after the comment's text, so typing adds to it; a new comment's composer
/// opens on its field as the dialog rule has it (the whole text selected).
#[trial]
fn an_edit_composer_opens_with_the_caret_after_the_text() {
    for (kind, want) in [(ComposerDraft::Edit, "Original edited"), (ComposerDraft::EditVerbatim, "Original edited"), (ComposerDraft::New, " edited")]
    {
        let draft = Rc::new(RefCell::new(String::from("Original")));
        let shown = Rc::clone(&draft);
        let mut fixture = Fixture::new(move |ui| {
            let _ = CommentComposer::new(Some(pos2(200.0, 200.0)), &mut shown.borrow_mut()).draft_kind(kind).show(ui);
        });
        fixture.room(vec2(800.0, 600.0)).settle();
        fixture.type_text(" edited").settle();
        assert_eq!(draft.borrow().as_str(), want, "{kind:?}");
    }
}

/// The Reply variants a pop-up shows, with the app's `NoteReplyLook` when `look` names one.
fn reply_variants(look: Option<craft_review::ui::NoteReplyLook>) -> (Vec<craft_theme::Variant>, craft_theme::Tokens) {
    let mut drafts = NoteDrafts::default();
    let tokens = Rc::new(RefCell::new(None));
    let seen = Rc::clone(&tokens);
    let mut fixture = Fixture::new(move |ui| {
        let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(320.0, 200.0));
        let _ = NotePopup::new(1, "Note", rect, &NOTE, &mut drafts).show(ui);
        *seen.borrow_mut() = Some((**craft_ui::runtime::env::Env::of(ui).tokens.tokens()).clone());
    });
    if let Some(look) = look {
        fixture.root_value(look);
    }
    fixture.room(vec2(1000.0, 800.0)).settle();
    assert!(fixture.by_label("Reply").is_some(), "the pop-up shows Reply");
    let variants = fixture.styles_of(std::any::type_name::<Button<'static>>()).iter().map(|style| style.variant).collect();
    let tokens = tokens.borrow().clone().expect("the fixture ran");
    (variants, tokens)
}

/// Upstream's Reply is the suite's small outlined button; PdfCraft's (`NoteReplyLook::Flat`) is production's small flat
/// rectangle: a translucent white face with control corners, not a pill.
#[trial]
fn a_flat_reply_is_a_small_rectangle_and_the_default_an_outlined_button() {
    use craft_theme::variant::{RadiusStep, Variant};
    let (standard, _) = reply_variants(None);
    assert!(standard.iter().all(|variant| matches!(variant, Variant::Look { .. })), "the default Reply is a built-in look: {standard:?}");
    let (flat, tokens) = reply_variants(Some(craft_review::ui::NoteReplyLook::Flat));
    let custom = flat.iter().find_map(|variant| match variant {
        Variant::Custom(custom) => Some(*custom),
        _ => None,
    });
    let custom = custom.expect("the flat Reply takes its own look");
    let look = custom.look(&tokens);
    assert_eq!(look.radius, RadiusStep::Control, "control corners, not a pill");
    assert!(look.fill.rest.0[3] > 0 && look.fill.rest.0[3] < 255, "a translucent face at rest: {:?}", look.fill.rest);
    assert!(look.edge.is_some(), "a thin edge");
}

/// `CommentComposer::fixed_rows`: a card near the bottom of the window keeps its Post button inside it when its text
/// runs past the field's three rows, and Post posts.
#[trial]
fn a_composer_with_fixed_rows_keeps_post_inside_its_card_near_the_bottom() {
    let posted = Rc::new(RefCell::new(None));
    let seen = Rc::clone(&posted);
    let mut draft = String::new();
    let mut fixture = Fixture::new(move |ui| {
        let out = CommentComposer::new(Some(pos2(120.0, 470.0)), &mut draft).header("Text box", "Ada").fixed_rows().show(ui);
        if let Some(text) = out.posted {
            *seen.borrow_mut() = Some(text);
        }
    });
    fixture.room(vec2(800.0, 600.0)).settle();
    fixture.type_text(&"word ".repeat(20)).settle();
    let card = fixture.by_role(craft_ui::contract::Role::DIALOG).first().expect("the card").rect;
    let post = fixture.by_label("Post").expect("the Post button");
    assert!(card.contains_rect(post.rect), "Post {:?} lies inside the card {card:?}", post.rect);
    fixture.click(&post).settle();
    assert!(posted.borrow().as_deref().is_some_and(|text| text.starts_with("word word")), "Post posted: {:?}", posted.borrow());
}

/// Production's flat Reply takes its edge from the note's colour (thinned with white to seven tenths), not the light
/// palette's control edge: a blue note's Reply has a blue edge.
#[trial]
fn a_flat_reply_edge_takes_the_notes_colour() {
    let mut drafts = NoteDrafts::default();
    let mut fixture = Fixture::new(move |ui| {
        let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(320.0, 200.0));
        let _ = NotePopup::new(1, "Note", rect, &NOTE, &mut drafts).color(Some([10, 20, 200])).show(ui);
    });
    fixture.root_value(craft_review::ui::NoteReplyLook::Flat);
    fixture.room(vec2(1000.0, 800.0)).settle();
    let reply = fixture.by_label("Reply").expect("the pop-up shows Reply").rect;
    fn strokes(shape: &egui::Shape, out: &mut Vec<(Rect, egui::Color32)>) {
        match shape {
            egui::Shape::Rect(rect) if rect.stroke.width > 0.0 => out.push((rect.rect, rect.stroke.color)),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| strokes(shape, out)),
            _ => {}
        }
    }
    let mut seen = Vec::new();
    for shape in fixture.frame_shapes() {
        strokes(&shape, &mut seen);
    }
    let edge = egui::Color32::from_rgb(83, 90, 216);
    let around = seen.iter().filter(|(rect, _)| (rect.center() - reply.center()).length() < 2.0).collect::<Vec<_>>();
    assert!(around.iter().any(|(_, color)| *color == edge), "the Reply {reply:?} is edged in the note's colour {edge:?}: {around:?}");
}
