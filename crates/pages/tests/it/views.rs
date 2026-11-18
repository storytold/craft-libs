//! The page view and the page preview: the scroll anchor under a zoom change, the wheel's page turns, the pictures
//! the view asks for, the view's page and arrow keys and its zoom commands, and the preview's keys.

use craft_behavior::{GESTURE_GAP, kurbo};
use craft_pages::layout::{Fit, LayoutOptions, PageDisplay};
use craft_pages::ui::fixtures::{DemoPages, PAPER};
use craft_pages::ui::{GoTo, PagePreview, PageSource, PageView, PageViewState, PlacedPage, ReadingState, ReadingView};
use craft_ui::canvas::NavigationOptions;
use craft_ui::contract::{Role, Show as _};
use craft_ui::primitives::textures::{Revision, TextureState, ThumbRender, ThumbRequest, ThumbSource};

use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, pos2, vec2};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

/// The default state with `change` applied.
fn state_with(change: impl FnOnce(&mut PageViewState)) -> PageViewState {
    let mut state = PageViewState::default();
    change(&mut state);
    state
}

type LayoutRig = (Fixture, Rc<RefCell<PageViewState>>, Rc<RefCell<Out>>);

/// What a trial reads of the last pass.
#[derive(Clone, Debug)]
struct Out {
    /// The offset the pass started from, which the layout below is of.
    offset: (f64, f64),
    /// The page rects of the layout the pass drew.
    rects: Vec<kurbo::Rect>,
    /// The pages on screen.
    pages: Vec<PlacedPage>,
    /// The pages at least partly in view.
    visible: Range<usize>,
    /// The view on screen.
    view: egui::Rect,
}

impl Default for Out {
    fn default() -> Self {
        Out { offset: (0.0, 0.0), rects: Vec::new(), pages: Vec::new(), visible: 0..0, view: egui::Rect::NOTHING }
    }
}

impl Out {
    /// The layout point at the centre of the view.
    fn centre(&self) -> kurbo::Point {
        kurbo::Point::new(self.offset.0 + f64::from(self.view.width()) / 2.0, self.offset.1 + f64::from(self.view.height()) / 2.0)
    }

    /// The layout point of `at` (page points) on `page`, a US Letter page.
    fn point(&self, page: usize, at: kurbo::Point) -> kurbo::Point {
        let r = self.rects[page];
        kurbo::Point::new(r.x0 + at.x * r.width() / 612.0, r.y0 + at.y * r.height() / 792.0)
    }

    /// The layout point under `screen`.
    fn under(&self, screen: egui::Pos2) -> kurbo::Point {
        kurbo::Point::new(self.offset.0 + f64::from(screen.x - self.view.min.x), self.offset.1 + f64::from(screen.y - self.view.min.y))
    }

    /// The page point (page points) of the layout point `p` on `page`, a US Letter page.
    fn on_page(&self, page: usize, p: kurbo::Point) -> kurbo::Point {
        let r = self.rects[page];
        kurbo::Point::new((p.x - r.x0) * 612.0 / r.width(), (p.y - r.y0) * 792.0 / r.height())
    }
}

/// A page view over `source` with the state it starts from and `navigation`, the pages it keeps, and what it showed.
struct Rig {
    fixture: Fixture,
    state: Rc<RefCell<PageViewState>>,
    out: Rc<RefCell<Out>>,
    keep: Rc<RefCell<Vec<usize>>>,
}

/// A rig over `source` in a 900 × 600 room, settled when `settle`.
fn rig_over(source: impl PageSource + 'static, start: PageViewState, navigation: NavigationOptions, settle: bool) -> Rig {
    let state = Rc::new(RefCell::new(start));
    let out: Rc<RefCell<Out>> = Rc::default();
    let keep: Rc<RefCell<Vec<usize>>> = Rc::default();
    let (kept, seen, held) = (Rc::clone(&state), Rc::clone(&out), Rc::clone(&keep));
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept.borrow().clone();
        let offset = now.offset;
        let keep = held.borrow().clone();
        let shown =
            PageView::new("Pages", &source, &mut now, PAPER).options(LayoutOptions::ACROBAT).navigation(navigation).keep(&keep).show(ui, |_, _| ());
        *seen.borrow_mut() =
            Out { offset, rects: shown.layout.rects.clone(), pages: shown.pages, visible: shown.visible, view: shown.canvas.response.rect };
        *kept.borrow_mut() = now;
    });
    fixture.room(vec2(900.0, 600.0));
    if settle {
        fixture.settle();
    }
    Rig { fixture, state, out, keep }
}

/// A rig over `count` demo pages with the default navigation.
fn rig(count: usize, start: PageViewState) -> Rig {
    rig_over(DemoPages::new(count), start, NavigationOptions::default(), true)
}

/// A page view over `count` demo pages with the state it starts from, and what its last pass showed.
fn layout_rig(count: usize, start: PageViewState) -> LayoutRig {
    let Rig { fixture, state, out, .. } = rig(count, start);
    (fixture, state, out)
}

/// The page under the centre of the last pass's view and the page point there.
fn centre_point(out: &Out) -> (usize, kurbo::Point) {
    let centre = out.centre();
    let page = out.rects.iter().position(|r| r.y0 <= centre.y && centre.y < r.y1).expect("a page is under the centre");
    (page, out.on_page(page, centre))
}

/// Asserts the same page point is under the centre as `before`, within a screen point.
fn assert_centre_kept(out: &Out, before: (usize, kurbo::Point), what: &str) {
    let (page, point) = before;
    let now = out.point(page, point);
    let centre = out.centre();
    assert!(
        (now.x - centre.x).abs() < 1.0 && (now.y - centre.y).abs() < 1.0,
        "{what}: {point:?} of page {page} is at {now:?}, the centre at {centre:?}"
    );
}

#[trial]
fn a_changed_zoom_keeps_the_point_at_the_centre_in_place() {
    // The app's zoom.
    let mut zoomed = rig(
        5,
        state_with(|s| {
            s.fit = Fit::None;
            s.zoom = 1.0;
        }),
    );
    let third = zoomed.out.borrow().rects[2];
    zoomed.state.borrow_mut().offset.1 = third.y0 + 0.4 * third.height();
    zoomed.fixture.idle(2);
    let before = centre_point(&zoomed.out.borrow());
    assert_eq!(before.0, 2, "precondition: the centre is on the third page");
    zoomed.state.borrow_mut().zoom = 2.0;
    zoomed.fixture.idle(3);
    assert_centre_kept(&zoomed.out.borrow(), before, "a written zoom");

    // A fit over a new room.
    let mut fitted = rig(5, state_with(|s| s.fit = Fit::Width));
    let third = fitted.out.borrow().rects[2];
    fitted.state.borrow_mut().offset.1 = third.y0 + 0.4 * third.height();
    fitted.fixture.idle(2);
    let before = centre_point(&fitted.out.borrow());
    fitted.fixture.room(vec2(700.0, 500.0));
    fitted.fixture.idle(3);
    assert_centre_kept(&fitted.out.borrow(), before, "a fit after a new room");
}

/// A wheel step of `lines` over the middle of the room.
fn wheel(fixture: &mut Fixture, lines: f32) {
    let middle = pos2(450.0, 300.0);
    fixture.frame(vec![egui::Event::PointerMoved(middle)]);
    fixture.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, lines),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
    }]);
}

/// A scroll of `dy` points over the middle of the room from `source`, in device phase `phase`.
fn swipe(fixture: &mut Fixture, dy: f32, phase: egui::TouchPhase, source: egui::MouseWheelSource) {
    fixture.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, dy),
        modifiers: Modifiers::NONE,
        phase,
        source,
    }]);
}

/// Frames until the wheel service has ended the running gesture.
fn end_gesture(fixture: &mut Fixture) {
    fixture.advance(GESTURE_GAP);
    fixture.idle(2);
}

/// A view that is not continuous, with each page fitted, so the wheel is at both edges of every row.
fn paged(count: usize, change: impl FnOnce(&mut PageViewState)) -> Rig {
    rig(
        count,
        state_with(|s| {
            s.continuous = false;
            s.fit = Fit::Page;
            change(s);
        }),
    )
}

#[trial]
fn half_notches_past_the_bottom_turn_once_they_add_up_within_one_gesture_and_never_across_two() {
    let Rig { mut fixture, state, .. } = paged(4, |_| ());
    wheel(&mut fixture, -0.5);
    assert_eq!(state.borrow().current, 0, "half a notch is not a turn");
    wheel(&mut fixture, -0.5);
    assert_eq!(state.borrow().current, 1, "two halves of one gesture add up to a turn");

    end_gesture(&mut fixture);
    wheel(&mut fixture, -0.5);
    end_gesture(&mut fixture);
    wheel(&mut fixture, -0.5);
    end_gesture(&mut fixture);
    assert_eq!(state.borrow().current, 1, "halves of two gestures never add up");

    wheel(&mut fixture, -1.0);
    assert_eq!(state.borrow().current, 2, "a whole notch turns");
}

#[trial]
fn the_preview_steps_with_the_arrow_and_page_keys_from_a_button_of_its_row() {
    let at = Rc::new(RefCell::new(1_usize));
    let kept = Rc::clone(&at);
    let pages = DemoPages::new(5);
    let mut fixture = Fixture::new(move |ui| {
        let shown = [0, 1, 2, 3, 4];
        PagePreview::new(&pages, &shown, &mut kept.borrow_mut(), PAPER).show(ui);
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    let button = fixture.focused_stop();
    assert!(button.is_some(), "Tab focuses a button of the row");
    fixture.key(Key::ArrowRight, Modifiers::NONE);
    assert_eq!(*at.borrow(), 2, "Right steps forward");
    fixture.key(Key::PageDown, Modifiers::NONE);
    assert_eq!(*at.borrow(), 3, "Page Down steps forward");
    fixture.key(Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(*at.borrow(), 2, "Left steps back");
    assert_eq!(fixture.focused_stop(), button, "the keys leave the focus where it was");
}

/// The page's picture in a preview shown in a room of `room`, and whether Close was reported.
fn preview_in(room: egui::Vec2, close: bool) -> (egui::Rect, bool) {
    let closed = Rc::new(RefCell::new(false));
    let seen = Rc::clone(&closed);
    let pages = DemoPages::new(3);
    let mut at = 0_usize;
    let mut fixture = Fixture::new(move |ui| {
        if PagePreview::new(&pages, &[0, 1, 2], &mut at, PAPER).show(ui).closed {
            *seen.borrow_mut() = true;
        }
    });
    fixture.room(room);
    fixture.settle();
    let picture = fixture.by_role(Role::IMAGE).first().expect("the page's picture is shown").rect;
    if close {
        let button = fixture.by_label("Close").expect("the row has Close");
        fixture.click(&button).settle();
    }
    let closed = *closed.borrow();
    (picture, closed)
}

#[trial]
fn the_preview_page_grows_with_the_window_and_close_reports_closed() {
    let (small, _) = preview_in(vec2(600.0, 400.0), false);
    let (large, closed) = preview_in(vec2(1200.0, 900.0), true);
    assert!(large.height() > small.height() * 1.5, "the page takes the window: {small:?} then {large:?}");
    assert!(closed, "Close reports closed");
}

/// Tabs to the view's canvas.
fn focus_canvas(fixture: &mut Fixture) {
    for _ in 0..4 {
        if fixture.focused_role().is_some_and(|role| role.nudge) {
            return;
        }
        fixture.key(Key::Tab, Modifiers::NONE);
    }
    assert!(fixture.focused_role().is_some_and(|role| role.nudge), "Tab reaches the view's canvas");
}

#[trial]
fn page_keys_turn_and_go_to_the_ends_in_a_view_that_is_not_continuous() {
    let (mut fixture, state, _) = layout_rig(
        5,
        state_with(|s| {
            s.continuous = false;
            s.fit = Fit::Page;
        }),
    );
    focus_canvas(&mut fixture);
    fixture.key(Key::PageDown, Modifiers::NONE);
    fixture.idle(1);
    assert_eq!(state.borrow().current, 1, "Page Down turns one page");
    fixture.key(Key::End, Modifiers::NONE);
    fixture.idle(1);
    assert_eq!(state.borrow().current, 4, "End goes to the last page");
    fixture.key(Key::Home, Modifiers::NONE);
    fixture.idle(1);
    assert_eq!(state.borrow().current, 0, "Home goes to the first page");
}

#[trial]
fn arrows_go_to_the_next_page_and_scroll_a_line_in_a_continuous_view() {
    // Right, then Down, in a view whose key line is `key_line`: the page reached and how far Down scrolled.
    let down = |key_line: Option<f32>| {
        let mut navigation = NavigationOptions::default();
        if let Some(line) = key_line {
            navigation.key_line = line;
        }
        let Rig { mut fixture, state, .. } = rig_over(DemoPages::new(5), state_with(|s| s.fit = Fit::Page), navigation, true);
        focus_canvas(&mut fixture);
        fixture.key(Key::ArrowRight, Modifiers::NONE);
        fixture.idle(1);
        let page = state.borrow().current;
        let before = state.borrow().offset.1;
        fixture.key(Key::ArrowDown, Modifiers::NONE);
        fixture.idle(1);
        let after = state.borrow().offset.1;
        (page, after - before)
    };
    let (page, plain) = down(None);
    assert_eq!(page, 1, "Right goes to the next page while the page fits the width");
    assert!(plain > 0.0, "Down scrolls forward: {plain}");
    let (_, longer) = down(Some(NavigationOptions::default().key_line * 4.0));
    assert!(longer > plain, "a longer key line scrolls farther: {longer} against {plain}");
}

#[trial]
fn zoom_in_zooms_a_focused_view_with_animated_zoom() {
    let (mut fixture, state, _) = layout_rig(3, PageViewState::default());
    focus_canvas(&mut fixture);
    let before = state.borrow().zoom;
    fixture.key(Key::Equals, Modifiers::COMMAND);
    fixture.settle();
    assert!(state.borrow().zoom > before, "Zoom In zooms: {before} -> {}", state.borrow().zoom);
}

#[trial]
fn zoom_keys_under_an_open_preview_leave_the_view_alone() {
    let state = Rc::new(RefCell::new(PageViewState::default()));
    let open = Rc::new(RefCell::new(false));
    let (kept, opened) = (Rc::clone(&state), Rc::clone(&open));
    let pages = DemoPages::new(3);
    let mut at = 0_usize;
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept.borrow().clone();
        PageView::new("Pages", &pages, &mut now, PAPER).options(LayoutOptions::ACROBAT).show(ui, |_, _| ());
        *kept.borrow_mut() = now;
        if *opened.borrow() {
            PagePreview::new(&pages, &[0, 1, 2], &mut at, PAPER).show(ui);
        }
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    focus_canvas(&mut fixture);
    *open.borrow_mut() = true;
    fixture.settle();
    let before = state.borrow().zoom;
    for (key, modifiers) in [
        (Key::Equals, Modifiers::COMMAND),
        (Key::Plus, Modifiers::COMMAND),
        (Key::Minus, Modifiers::COMMAND),
        (Key::Num0, Modifiers::COMMAND),
        (Key::Plus, Modifiers::NONE),
        (Key::Minus, Modifiers::NONE),
    ] {
        fixture.key(key, modifiers);
        fixture.settle();
        assert_eq!(state.borrow().zoom, before, "{key:?} with {modifiers:?} under the preview");
    }
}

/// A wheel step of `lines` with `modifiers` held, near the top of the room, where a reading view is.
fn wheel_with(fixture: &mut Fixture, lines: f32, modifiers: Modifiers) {
    fixture.frame(vec![egui::Event::PointerMoved(pos2(450.0, 40.0))]);
    fixture.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, lines),
        modifiers,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
    }]);
    fixture.idle(1);
}

#[trial]
fn the_reading_view_turns_on_the_wheel_but_leaves_cmd_or_ctrl_with_the_wheel_to_the_app_zoom() {
    let state = Rc::new(RefCell::new(ReadingState::default()));
    let kept = Rc::clone(&state);
    let pages = DemoPages::new(6);
    let mut fixture = Fixture::new(move |ui| {
        ReadingView::new("Document", &pages, &mut kept.borrow_mut(), PAPER).show(ui);
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    wheel_with(&mut fixture, -1.0, Modifiers::COMMAND);
    assert_eq!(state.borrow().column, 0, "Cmd or Ctrl with the wheel turns no column");
    wheel_with(&mut fixture, -1.0, Modifiers::NONE);
    assert!(state.borrow().column > 0, "the wheel alone turns");
}

/// A US Letter page, points.
const LETTER: kurbo::Size = kurbo::Size::new(612.0, 792.0);

/// Demo pictures over pages of `sizes`, noting each page the view asks it to render: the public traits, as an app
/// implements them.
struct Recording {
    pictures: DemoPages,
    sizes: Vec<kurbo::Size>,
    asked: Rc<RefCell<Vec<usize>>>,
}

impl Recording {
    fn new(pictures: DemoPages, sizes: Vec<kurbo::Size>, asked: &Rc<RefCell<Vec<usize>>>) -> Self {
        Recording { pictures, sizes, asked: Rc::clone(asked) }
    }
}

impl ThumbSource<usize> for Recording {
    fn revision(&self, key: &usize, request: &ThumbRequest) -> Revision {
        self.pictures.revision(key, request)
    }

    fn render(&self, key: &usize, request: &ThumbRequest) -> ThumbRender {
        self.asked.borrow_mut().push(*key);
        self.pictures.render(key, request)
    }
}

impl PageSource for Recording {
    fn page_count(&self) -> usize {
        self.sizes.len()
    }

    fn page_size(&self, page: usize) -> kurbo::Size {
        self.sizes.get(page).copied().unwrap_or(LETTER)
    }
}

#[trial]
fn a_fit_holds_the_page_at_the_top_still_while_pages_of_other_sizes_scroll_past() {
    let (small, large) = (kurbo::Size::new(300.0, 400.0), kurbo::Size::new(900.0, 1200.0));
    let sizes = vec![LETTER, small, large, LETTER, small, large];
    let source = Recording::new(DemoPages::new(sizes.len()), sizes, &Rc::default());
    let Rig { mut fixture, state, out, .. } = rig_over(source, state_with(|s| s.fit = Fit::Width), NavigationOptions::default(), true);
    let (zoom, rects) = (state.borrow().zoom, out.borrow().rects.clone());
    for page in 1..rects.len() {
        state.borrow_mut().offset.1 = rects[page].y0;
        fixture.idle(2);
        assert_eq!(state.borrow().zoom, zoom, "page {page} at the top leaves the fitted zoom alone");
        assert_eq!(out.borrow().rects[page], rects[page], "page {page} at the top keeps its place and size");
    }
}

#[trial]
fn prefetch_asks_for_the_rows_ahead_of_the_motion_and_none_behind() {
    // The pages outside the view the source was asked to render since the last call.
    let outside =
        |asked: &RefCell<Vec<usize>>, out: &Out| -> Vec<usize> { asked.borrow_mut().drain(..).filter(|page| !out.visible.contains(page)).collect() };
    for two_up in [false, true] {
        let asked: Rc<RefCell<Vec<usize>>> = Rc::default();
        let source = Recording::new(DemoPages::new(24), vec![LETTER; 24], &asked);
        let start = state_with(|s| {
            s.fit = Fit::Width;
            if two_up {
                s.display = PageDisplay::TwoUp;
                s.cover = true;
            }
        });
        let Rig { mut fixture, state, out, .. } = rig_over(source, start, NavigationOptions::default(), true);
        asked.borrow_mut().clear();

        let mut ahead = 0;
        for _ in 0..6 {
            state.borrow_mut().offset.1 += 250.0;
            fixture.frame(Vec::new());
            let out = out.borrow();
            let asked = outside(&asked, &out);
            assert!(
                asked.iter().all(|page| *page >= out.visible.end),
                "two-up {two_up}: forward asks ahead only: {asked:?} beside {:?}",
                out.visible
            );
            ahead += asked.len();
        }
        assert!(ahead > 0, "two-up {two_up}: forward travel asks for the pages ahead");

        // No travel is kept: a view at rest asks one row either side, which the travel already passed or asked for.
        fixture.idle(3);
        let still = outside(&asked, &out.borrow());
        assert!(still.is_empty(), "two-up {two_up}: a still view asks for nothing it had not asked for: {still:?}");

        // To the end of the document, far from every page the run forward and its rests asked for: a page the source
        // rendered before is cached, so only pages never asked for show what the back travel asks for.
        state.borrow_mut().offset.1 = 1.0e6;
        fixture.idle(3);
        asked.borrow_mut().clear();
        let mut behind = 0;
        for _ in 0..6 {
            state.borrow_mut().offset.1 -= 250.0;
            fixture.frame(Vec::new());
            let out = out.borrow();
            let asked = outside(&asked, &out);
            assert!(asked.iter().all(|page| *page < out.visible.start), "two-up {two_up}: back asks behind only: {asked:?} beside {:?}", out.visible);
            behind += asked.len();
        }
        assert!(behind > 0, "two-up {two_up}: travel back asks for the pages before");
    }
}

#[trial]
fn every_notch_turns_one_page_and_a_swipe_with_its_momentum_turns_once() {
    use egui::MouseWheelSource::{Momentum, Trackpad, Unknown};
    use egui::TouchPhase::{End, Move, Start};
    let Rig { mut fixture, state, .. } = paged(8, |_| ());
    let current = || state.borrow().current;

    for _ in 0..3 {
        wheel(&mut fixture, -1.0);
    }
    assert_eq!(current(), 3, "each line notch of a fast run turns a page");
    wheel(&mut fixture, 1.0);
    assert_eq!(current(), 2, "a whole notch back turns back");
    end_gesture(&mut fixture);

    // A swipe on a pad that reports phases, then the system's momentum run.
    swipe(&mut fixture, -10.0, Start, Trackpad);
    for _ in 0..5 {
        swipe(&mut fixture, -10.0, Move, Trackpad);
    }
    swipe(&mut fixture, 0.0, End, Trackpad);
    for _ in 0..5 {
        swipe(&mut fixture, -10.0, Move, Momentum);
    }
    end_gesture(&mut fixture);
    assert_eq!(current(), 3, "a swipe and its momentum turn once");

    // A pad that reports no phases: its inertia is one gesture with the swipe by the gap alone.
    for _ in 0..10 {
        swipe(&mut fixture, -10.0, Move, Unknown);
    }
    end_gesture(&mut fixture);
    assert_eq!(current(), 4, "a phaseless swipe turns once");

    // The ends.
    state.borrow_mut().go_to(7);
    fixture.idle(2);
    wheel(&mut fixture, -1.0);
    end_gesture(&mut fixture);
    assert_eq!(current(), 7, "no turn past the last page");
    state.borrow_mut().go_to(0);
    fixture.idle(2);
    wheel(&mut fixture, 1.0);
    end_gesture(&mut fixture);
    assert_eq!(current(), 0, "no turn before the first page");

    // Spreads with a cover: the cover alone, then two pages at a time.
    let Rig { mut fixture, state, .. } = paged(6, |s| {
        s.display = PageDisplay::TwoUp;
        s.cover = true;
    });
    wheel(&mut fixture, -1.0);
    assert_eq!(state.borrow().current, 1, "from the cover to the first spread");
    wheel(&mut fixture, -1.0);
    assert_eq!(state.borrow().current, 3, "a notch turns a whole spread");
}

#[trial]
fn an_edge_turns_only_after_a_gesture_that_did_not_pan_and_a_go_to_drops_the_gathered_motion() {
    // A page taller than the view: the gesture that pans it to its edge does not run on into the next page.
    let Rig { mut fixture, state, .. } = rig(
        4,
        state_with(|s| {
            s.continuous = false;
            s.fit = Fit::None;
            s.zoom = 2.0;
        }),
    );
    let top = state.borrow().offset.1;
    for _ in 0..12 {
        wheel(&mut fixture, -20.0);
    }
    assert!(state.borrow().offset.1 > top, "precondition: the wheel panned the page");
    assert_eq!(state.borrow().current, 0, "the gesture that panned the page to its edge does not turn");
    end_gesture(&mut fixture);
    wheel(&mut fixture, -1.0);
    assert_eq!(state.borrow().current, 1, "a new gesture at the edge turns");

    // A go-to during a gesture: the motion gathered before it does not add up to a turn after it.
    let Rig { mut fixture, state, .. } = paged(6, |_| ());
    wheel(&mut fixture, -0.5);
    state.borrow_mut().go_to(3);
    fixture.idle(1);
    wheel(&mut fixture, -0.5);
    assert_eq!(state.borrow().current, 3, "the go-to dropped the half notch gathered before it");
}

#[trial]
fn a_placed_page_reports_loading_until_its_picture_lands() {
    let asked: Rc<RefCell<Vec<usize>>> = Rc::default();
    let source = Recording::new(DemoPages::rendering(10), vec![LETTER; 10], &asked);
    let Rig { mut fixture, out, keep, .. } = rig_over(source, PageViewState::default(), NavigationOptions::default(), false);
    *keep.borrow_mut() = vec![9];
    fixture.frame(Vec::new());
    let first = out.borrow().pages.clone();
    assert!(!first.is_empty(), "the first pass places pages");
    assert!(
        first.iter().all(|page| matches!(page.picture, TextureState::Loading { .. } | TextureState::Missing)),
        "a page reports its picture on its way: {first:?}"
    );
    fixture.settle();
    let landed = out.borrow().pages.clone();
    assert!(landed.iter().all(|page| matches!(page.picture, TextureState::Ready(_))), "each page reports its picture once it lands: {landed:?}");
    assert!(
        !out.borrow().visible.contains(&9) && asked.borrow().contains(&9),
        "a page the app keeps is asked for while out of view, so the store sees it"
    );
}

#[trial]
fn a_pointer_zoom_while_a_go_to_is_pending_anchors_on_it_and_the_next_one_anchors_on_the_pointer() {
    let Rig { mut fixture, state, out, .. } = rig(
        5,
        state_with(|s| {
            s.fit = Fit::None;
            s.zoom = 1.0;
        }),
    );
    // The middle of the third page, which a view narrower than the page can centre on.
    let at = kurbo::Point::new(306.0, 396.0);
    let pointer = pos2(200.0, 150.0);
    let zoom_in = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, 1.0),
        modifiers: Modifiers::COMMAND,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
    };
    fixture.frame(vec![egui::Event::PointerMoved(pointer)]);
    let before = state.borrow().zoom;
    state.borrow_mut().go_to_point(2, at, (0.5, 0.5));
    fixture.frame(vec![zoom_in.clone()]);
    assert!(state.borrow().go_to.is_some(), "the go-to stays pending while the zoom runs");
    let mut passes = 0;
    while state.borrow().go_to.is_some() && passes < 120 {
        fixture.frame(Vec::new());
        // This pass drew the layout of the state the last pass ended with, where the pending go-to was placed.
        let out = out.borrow();
        let (point, centre) = (out.point(2, at), out.centre());
        assert!((point.x - centre.x).abs() < 1.0 && (point.y - centre.y).abs() < 1.0, "pass {passes}: {at:?} at {point:?}, the centre at {centre:?}");
        passes += 1;
    }
    assert!(passes > 0 && state.borrow().go_to.is_none(), "the go-to is let go once a pass runs with no zoom");
    assert!(state.borrow().zoom != before, "the wheel zoomed");

    // The next zoom anchors on the pointer, not on the go-to's point.
    fixture.settle();
    let (page, held) = {
        let out = out.borrow();
        let under = out.under(pointer);
        let page = out.rects.iter().position(|r| r.contains(under)).expect("a page is under the pointer");
        (page, out.on_page(page, under))
    };
    fixture.frame(vec![zoom_in]);
    fixture.settle();
    let out = out.borrow();
    let (now, under) = (out.point(page, held), out.under(pointer));
    assert!((now.x - under.x).abs() < 1.0 && (now.y - under.y).abs() < 1.0, "{held:?} stays under the pointer: {now:?} against {under:?}");
    let (point, centre) = (out.point(2, at), out.centre());
    assert!((point.x - centre.x).abs() > 1.0 || (point.y - centre.y).abs() > 1.0, "the go-to's point leaves the centre: {point:?}");
}

#[trial]
fn reselecting_the_display_moves_nothing_and_a_change_keeps_the_current_page() {
    let into_third = |rig: &mut Rig| {
        let third = rig.out.borrow().rects[2];
        rig.state.borrow_mut().offset.1 = third.y0 + 0.3 * third.height();
        rig.fixture.idle(2);
    };
    let mut view = rig(6, state_with(|s| s.fit = Fit::Width));
    into_third(&mut view);
    let current = view.state.borrow().current;
    let offset = view.state.borrow().offset;
    let display = view.state.borrow().display;
    view.state.borrow_mut().display = display;
    view.fixture.idle(2);
    assert_eq!(view.state.borrow().offset, offset, "writing the display it has moves nothing");

    let changes: [(&str, fn(&mut PageViewState)); 2] = [("display", |s| s.display = PageDisplay::TwoUp), ("fit", |s| s.fit = Fit::Page)];
    for (what, change) in changes {
        // Where a go-to to the current page puts the view, from the same state.
        let mut reference = rig(6, view.state.borrow().clone());
        {
            let mut state = reference.state.borrow_mut();
            change(&mut state);
            state.go_to(current);
        }
        reference.fixture.idle(2);
        change(&mut view.state.borrow_mut());
        view.fixture.idle(2);
        let (got, want) = (view.state.borrow().offset, reference.state.borrow().offset);
        assert!(
            (got.0 - want.0).abs() < 1.0 && (got.1 - want.1).abs() < 1.0,
            "a new {what} keeps the current page in view: {got:?} against {want:?}"
        );
        assert_eq!(view.state.borrow().current, current, "a new {what} keeps the current page");
    }
}

#[trial]
fn a_go_to_rect_fits_and_centres_the_rect_and_a_written_zoom_is_brought_into_range() {
    let Rig { mut fixture, state, out, .. } = rig(5, PageViewState::default());
    let rect = kurbo::Rect::new(100.0, 200.0, 300.0, 300.0);
    state.borrow_mut().go_to = Some(GoTo::Rect { page: 2, rect, margin: 10.0 });
    fixture.idle(2);
    {
        let out = out.borrow();
        let (min, max) = (out.point(2, rect.origin()), out.point(2, kurbo::Point::new(rect.x1, rect.y1)));
        let centre = out.centre();
        let middle = min.midpoint(max);
        assert!((middle.x - centre.x).abs() < 1.0 && (middle.y - centre.y).abs() < 1.0, "the rect is centred: {middle:?} against {centre:?}");
        let (w, h) = (max.x - min.x, max.y - min.y);
        let (vw, vh) = (f64::from(out.view.width()), f64::from(out.view.height()));
        assert!(w <= vw && h <= vh, "the rect fits: {w} × {h} in {vw} × {vh}");
        assert!(w > 0.8 * vw || h > 0.8 * vh, "and fills the view: {w} × {h} in {vw} × {vh}");
    }
    assert_eq!((state.borrow().current, state.borrow().fit), (2, Fit::None), "the rect's page is current at a zoom of its own");

    for written in [1000.0, f64::NAN] {
        state.borrow_mut().zoom = written;
        fixture.idle(2);
        let zoom = state.borrow().zoom;
        assert!((0.08..=64.0).contains(&zoom), "a written zoom of {written} is brought into range: {zoom}");
    }
}

/// Demo pages that count the renders of a part of a page, which only the detail store asks for.
struct Regions {
    pictures: DemoPages,
    parts: Rc<RefCell<usize>>,
}

impl ThumbSource<usize> for Regions {
    fn revision(&self, key: &usize, request: &ThumbRequest) -> Revision {
        self.pictures.revision(key, request)
    }

    fn render(&self, key: &usize, request: &ThumbRequest) -> ThumbRender {
        if request.region.is_some() {
            *self.parts.borrow_mut() += 1;
        }
        self.pictures.render(key, request)
    }
}

impl PageSource for Regions {
    fn page_count(&self) -> usize {
        self.pictures.page_count()
    }

    fn page_size(&self, page: usize) -> kurbo::Size {
        self.pictures.page_size(page)
    }
}

#[trial]
fn a_scroll_bar_drag_holds_the_sharp_renders_of_the_part_in_view_in_its_own_pass() {
    let parts: Rc<RefCell<usize>> = Rc::default();
    let source = Regions { pictures: DemoPages::new(3), parts: Rc::clone(&parts) };
    let start = state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 8.0;
    });
    let Rig { mut fixture, state, .. } = rig_over(source, start, NavigationOptions::default(), true);
    assert!(*parts.borrow() > 0, "precondition: a page past the texture side asks for the part in view");
    let bar =
        fixture.by_role(Role::SCROLL_BAR).into_iter().max_by(|a, b| a.rect.height().total_cmp(&b.rect.height())).expect("the view has scroll bars");
    let grab = pos2(bar.rect.center().x, bar.rect.min.y + 6.0);
    let (before, offset) = (*parts.borrow(), state.borrow().offset.1);
    fixture.frame(vec![craft_ui::testing::pointer::moved(grab)]);
    fixture.frame(vec![craft_ui::testing::pointer::press(grab)]);
    let mut at = grab;
    for _ in 0..4 {
        at += vec2(0.0, 12.0);
        fixture.frame(vec![craft_ui::testing::pointer::moved(at)]);
    }
    assert!(state.borrow().offset.1 > offset, "precondition: the bar moved the view");
    assert_eq!(*parts.borrow(), before, "the part in view is not rendered while a bar is dragged");
    fixture.frame(vec![craft_ui::testing::pointer::release(at)]);
    fixture.settle();
    assert!(*parts.borrow() > before, "the part in view is rendered once the drag ends");
}
