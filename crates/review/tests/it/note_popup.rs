//! The note pop-up's hover and selection, its corner grip, and when the comment composer closes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_review::ui::{CommentComposer, ComposerClose, NOTE_POPUP_LIMITS, NoteDrafts, NotePopup, NotePopupMessage};
use craft_ui::buttons::Button;
use craft_ui::contract::Show as _;
use craft_ui::contract::overlay::PlacementKind;
use craft_ui::testing::{Fixture, trial};
use egui::{Event, Rect, pos2, vec2};

/// How far a measured rect may sit from the expected one.
const SLACK: f32 = 1.5;

const NOTE: [NotePopupMessage<'static>; 1] =
    [NotePopupMessage { id: 1, title: "Sticky Note", author: "Ana", text: "Check the margin.", modified: None, depth: 0, editable: true }];

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

/// The host of one composer: it shows the composer until it posts or cancels.
struct ComposerHost {
    open: Cell<bool>,
    posted: RefCell<Option<String>>,
    cancelled: Cell<bool>,
    draft: RefCell<String>,
}

/// Where a press lands outside the composer's card.
const OUTSIDE: egui::Pos2 = egui::Pos2::new(700.0, 550.0);

/// `ComposerClose::OnPostOrCancel` keeps the card open on a press outside and closes it on Post or Cancel; the default
/// closes it on the press outside.
#[trial]
fn a_composer_kept_open_on_outside_press_closes_only_on_post_or_cancel() {
    for close in [ComposerClose::OnPostOrCancel, ComposerClose::OnOutsidePress] {
        let host = Rc::new(ComposerHost { open: Cell::new(true), posted: RefCell::new(None), cancelled: Cell::new(false), draft: RefCell::new(String::new()) });
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
