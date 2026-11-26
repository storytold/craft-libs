//! The page view and the page preview: the scroll anchor under a zoom change, the wheel's page turns, the pictures
//! the view asks for, the view's page and arrow keys and its zoom commands, and the preview's keys.

use craft_behavior::geometry::{CropBox, SnapKind, SnapOptions, SnapTarget};
use craft_behavior::{Axis, CancelReason, CanvasView, EditCommand};
use craft_behavior::{GESTURE_GAP, kurbo};
use craft_pages::layout::{Fit, LayoutOptions, PageDisplay, PagedLayout};
use craft_pages::ui::fixtures::{DemoPages, PAPER};
use craft_pages::ui::{
    GoTo, PageBody, PagePreview, PageRegion, PageRevision, PageSnap, PageSource, PageView, PageViewState, PageViewWords, PlacedPage, ReadingNav,
    ReadingState, ReadingView,
};
use craft_theme::ComponentSet;
use craft_ui::canvas::{
    CanvasTool, CropOverlay, HandTool, NavigationOptions, ToolAction, ToolContext, ToolHost, ToolKey, ToolPath, ToolPointer, WheelScheme,
};
use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::primitives::textures::{Revision, TextureState, ThumbRender, ThumbRequest, ThumbSource};

use craft_ui::primitives::textures::TextureStoreOptions;
use craft_ui::primitives::{Draw, EditorPurpose};
use craft_ui::runtime::commands::{CommandState, EditCommands, resolve};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::focus::EditSet;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Key, Modifiers, pos2, vec2};
use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

/// The default state with `change` applied.
fn state_with(change: impl FnOnce(&mut PageViewState)) -> PageViewState {
    let mut state = PageViewState::default();
    change(&mut state);
    state
}

type LayoutRig = (Fixture, Rc<RefCell<PageViewState>>, Rc<RefCell<Out>>);

struct RevisionPages {
    pictures: DemoPages,
    revision: Rc<Cell<u64>>,
}

impl ThumbSource<usize> for RevisionPages {
    fn revision(&self, key: &usize, request: &ThumbRequest) -> Revision {
        Revision::of(&(self.pictures.revision(key, request), self.revision.get()))
    }

    fn render(&self, key: &usize, request: &ThumbRequest) -> ThumbRender {
        self.pictures.render(key, request)
    }
}

impl PageSource for RevisionPages {
    fn page_count(&self) -> usize {
        self.pictures.page_count()
    }

    fn page_size(&self, page: usize) -> kurbo::Size {
        self.pictures.page_size(page)
    }

    fn page_revision(&self, _: usize) -> Option<PageRevision> {
        Some(PageRevision { document: 0, page: self.revision.get() })
    }
}

struct UnequalPages(DemoPages);
struct SmallPages(DemoPages);

impl ThumbSource<usize> for UnequalPages {
    fn revision(&self, key: &usize, request: &ThumbRequest) -> Revision {
        self.0.revision(key, request)
    }

    fn render(&self, key: &usize, request: &ThumbRequest) -> ThumbRender {
        self.0.render(key, request)
    }
}

impl PageSource for UnequalPages {
    fn page_count(&self) -> usize {
        2
    }

    fn page_size(&self, page: usize) -> kurbo::Size {
        if page == 0 { kurbo::Size::new(3000.0, 792.0) } else { kurbo::Size::new(300.0, 792.0) }
    }
}

impl ThumbSource<usize> for SmallPages {
    fn revision(&self, key: &usize, request: &ThumbRequest) -> Revision {
        self.0.revision(key, request)
    }
    fn render(&self, key: &usize, request: &ThumbRequest) -> ThumbRender {
        self.0.render(key, request)
    }
}

impl PageSource for SmallPages {
    fn page_count(&self) -> usize {
        self.0.page_count()
    }
    fn page_size(&self, _: usize) -> kurbo::Size {
        kurbo::Size::new(200.0, 300.0)
    }
}

/// Whether the view shows a horizontal scroll bar at `zoom` in a window 900 wide.
fn has_horizontal_bar(zoom: f64) -> bool {
    let (fixture, _, _) = layout_rig(
        3,
        state_with(|s| {
            s.fit = Fit::None;
            s.zoom = zoom;
        }),
    );
    horizontal_bar(&fixture).is_some()
}

#[trial]
fn the_horizontal_bar_shows_only_when_the_pages_are_wider_than_the_view() {
    assert!(!has_horizontal_bar(0.5), "pages that fit the width need no horizontal bar");
    assert!(has_horizontal_bar(3.0), "pages wider than the view scroll sideways");
}

/// The horizontal scroll bar the fixture shows, if any.
fn horizontal_bar(fixture: &Fixture) -> Option<egui::Rect> {
    fixture.by_role(Role::SCROLL_BAR).iter().map(|bar| bar.rect).find(|rect| rect.width() > rect.height())
}

#[trial]
fn a_horizontal_thumb_held_while_the_pages_come_to_fit_keeps_its_bar_until_the_release() {
    let (mut fixture, state, _) = layout_rig(
        3,
        state_with(|s| {
            s.fit = Fit::None;
            s.zoom = 3.0;
        }),
    );
    let bar = horizontal_bar(&fixture).expect("pages wider than the view have a horizontal bar");
    // The view starts at the left edge, so the thumb is at the start of the track.
    let from = pos2(bar.left() + 4.0, bar.center().y);
    fixture.drag_hold_at(from, vec2(20.0, 0.0));
    state.borrow_mut().zoom = 0.5;
    fixture.idle(3);
    assert!(horizontal_bar(&fixture).is_some(), "the held thumb keeps its bar, so its release reaches it");
    fixture.release_at(from + vec2(20.0, 0.0)).idle(2);
    assert!(horizontal_bar(&fixture).is_none(), "after the release, pages that fit have no horizontal bar");
}

#[trial]
fn a_horizontal_thumb_released_while_the_bars_are_not_shown_leaves_no_bar_behind() {
    let state = Rc::new(RefCell::new(state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 3.0;
    })));
    let full_screen = Rc::new(Cell::new(false));
    let (kept, edge_to_edge) = (Rc::clone(&state), Rc::clone(&full_screen));
    let pages = DemoPages::new(3);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept.borrow().clone();
        PageView::new("Pages", &pages, &mut now, PAPER).options(LayoutOptions::ACROBAT).fill_screen(edge_to_edge.get()).show(ui, |_, _| ());
        *kept.borrow_mut() = now;
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let bar = horizontal_bar(&fixture).expect("pages wider than the view have a horizontal bar");
    let from = pos2(bar.left() + 4.0, bar.center().y);
    fixture.drag_hold_at(from, vec2(20.0, 0.0));
    // Full screen mid-drag: the bars go, and the release can't reach them.
    full_screen.set(true);
    fixture.idle(2);
    fixture.release_at(from + vec2(20.0, 0.0)).idle(2);
    full_screen.set(false);
    {
        let mut state = state.borrow_mut();
        state.fit = Fit::None;
        state.zoom = 0.5;
    }
    fixture.idle(3);
    assert!(horizontal_bar(&fixture).is_none(), "a press that ended unseen leaves no horizontal bar over pages that fit");
}

#[trial]
fn go_to_a_narrow_page_brings_it_back_into_horizontal_view() {
    let Rig { mut fixture, state, out, .. } = rig_over(
        UnequalPages(DemoPages::new(2)),
        state_with(|s| {
            s.fit = Fit::None;
            s.zoom = 1.0;
        }),
        NavigationOptions::default(),
        true,
    );
    state.borrow_mut().offset.0 = 0.0;
    fixture.idle(3);
    state.borrow_mut().go_to(1);
    fixture.idle(3);
    let out = out.borrow();
    let page = out.rects[1];
    assert!(page.x1 > out.offset.0 && page.x0 < out.offset.0 + f64::from(out.view.width()), "target page must be visible horizontally");
}

#[trial]
fn wheel_zoom_keeps_a_point_on_a_wide_page_under_the_pointer() {
    for zoom in [1.0, 2.0, 4.0] {
        for fraction in [0.1, 0.5, 0.9] {
            let mut navigation = NavigationOptions::default();
            navigation.wheel = WheelScheme::Acrobat;
            let start = state_with(|s| {
                s.fit = Fit::None;
                s.zoom = zoom;
            });
            let Rig { mut fixture, state, out, .. } = rig_over(SmallPages(DemoPages::new(20)), start, navigation, true);
            let pointer = {
                let out = out.borrow();
                let visible = out.pages[0].rect.intersect(out.view).shrink(15.0);
                visible.min + visible.size() * fraction
            };
            let (before, wide) = {
                let out = out.borrow();
                (out.on_page(0, out.under(pointer)), out.pages[0].rect.width() > out.view.width())
            };
            let mods = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
            fixture.frame(vec![
                egui::Event::ModifiersChanged(mods),
                egui::Event::PointerMoved(pointer),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, 20.0),
                    modifiers: mods,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                },
            ]);
            fixture.settle();
            assert!(state.borrow().zoom > zoom, "the wheel zoomed");
            let out = out.borrow();
            let (now, under) = (out.point(0, before), out.under(pointer));
            assert!(
                (!wide || (now.x - under.x).abs() < 2.0) && (now.y - under.y).abs() < 2.0,
                "zoom {zoom} at {fraction} moved the point: {now:?} vs {under:?}"
            );
        }
    }
}

#[trial]
fn wheel_zoom_keeps_its_anchor_after_an_external_zoom_change() {
    for zoom in [1.0, 2.0, 4.0] {
        for fraction in [0.1, 0.5, 0.9] {
            let mut navigation = NavigationOptions::default();
            navigation.wheel = WheelScheme::Acrobat;
            navigation.eased_wheel = true;
            let Rig { mut fixture, state, out, .. } =
                rig_config(SmallPages(DemoPages::new(20)), PageViewState::default(), navigation, true, true, |_, _| ());
            fixture.room(vec2(1274.0, 900.0));
            fixture.idle(4);
            {
                let mut s = state.borrow_mut();
                s.fit = Fit::None;
                s.zoom = zoom;
            }
            fixture.idle(4);
            let pointer = {
                let out = out.borrow();
                let visible = out.pages[0].rect.intersect(out.view).shrink(15.0);
                visible.min + visible.size() * fraction
            };
            let (before, wide) = {
                let out = out.borrow();
                (out.on_page(0, out.under(pointer)), out.pages[0].rect.width() > out.view.width())
            };
            let initial = out.borrow().pages[0].rect;
            let fraction_on_page = (pointer - initial.min) / initial.size();
            let mods = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
            fixture.frame(vec![
                egui::Event::ModifiersChanged(mods),
                egui::Event::PointerMoved(pointer),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, 20.0),
                    modifiers: mods,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                },
            ]);
            fixture.idle(20);
            assert!(state.borrow().zoom > zoom, "wheel zoomed from {zoom} to {}", state.borrow().zoom);
            let out = out.borrow();
            let (now, under) = (out.point(0, before), out.under(pointer));
            assert!(
                (!wide || (now.x - under.x).abs() < 2.0) && (now.y - under.y).abs() < 2.0,
                "zoom {zoom} at {fraction} moved {now:?} from {under:?}"
            );
            let page = out.pages[0].rect;
            let screen = page.min + fraction_on_page * page.size();
            assert!(
                (!wide || (screen.x - pointer.x).abs() < 2.0) && (screen.y - pointer.y).abs() < 2.0,
                "zoom {zoom} at {fraction} moved {screen:?} from {pointer:?}"
            );
        }
    }
}

#[trial]
fn page_view_one_frame_middle_click_scrolls_from_the_press_position() {
    let mut navigation = NavigationOptions::default();
    navigation.middle_button = craft_behavior::MiddleButton::AutoScroll;
    let Rig { mut fixture, state, out, .. } = rig_over(SmallPages(DemoPages::new(20)), PageViewState::default(), navigation, true);
    let at = out.borrow().view.center();
    let below = at + vec2(0.0, 60.0);
    let before = state.borrow().offset.1;
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Middle, pressed, modifiers: Modifiers::NONE };
    fixture.frame(vec![egui::Event::PointerMoved(at), button(at, true), egui::Event::PointerMoved(below), button(below, false)]);
    fixture.idle(8);
    assert!(state.borrow().offset.1 > before, "the view scrolls from the original middle press");
}

#[trial]
fn placed_page_reports_only_the_revision_of_a_ready_picture() {
    let revision = Rc::new(Cell::new(1));
    let source = RevisionPages { pictures: DemoPages::new(1), revision: revision.clone() };
    let Rig { mut fixture, out, .. } = rig_over(source, PageViewState::default(), NavigationOptions::default(), false);
    fixture.settle();
    assert_eq!(out.borrow().pages[0].picture_revision, Some(PageRevision { document: 0, page: 1 }));
    revision.set(2);
    fixture.idle(1);
    let pages = out.borrow();
    let page = &pages.pages[0];
    if !matches!(page.picture, TextureState::Ready(_)) {
        assert_eq!(page.picture_revision, None, "a stale stand-in is not the new revision");
    } else {
        assert_eq!(page.picture_revision, Some(PageRevision { document: 0, page: 2 }));
    }
}

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
    /// Some pass since the trial last cleared it reported the reader's scroll.
    scrolled: bool,
}

impl Default for Out {
    fn default() -> Self {
        Out { offset: (0.0, 0.0), rects: Vec::new(), pages: Vec::new(), visible: 0..0, view: egui::Rect::NOTHING, scrolled: false }
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
    rig_with(source, start, navigation, settle, |_, _| ())
}

/// A rig over `source` whose page body runs `body` (the app's tool host), in a 900 × 600 room, settled when `settle`.
fn rig_with(
    source: impl PageSource + 'static,
    start: PageViewState,
    navigation: NavigationOptions,
    settle: bool,
    body: impl FnMut(&mut egui::Ui, &PageBody<'_>) + 'static,
) -> Rig {
    rig_config(source, start, navigation, settle, false, body)
}

fn rig_config(
    source: impl PageSource + 'static,
    start: PageViewState,
    navigation: NavigationOptions,
    settle: bool,
    host_passthrough: bool,
    mut body: impl FnMut(&mut egui::Ui, &PageBody<'_>) + 'static,
) -> Rig {
    let state = Rc::new(RefCell::new(start));
    let out: Rc<RefCell<Out>> = Rc::default();
    let keep: Rc<RefCell<Vec<usize>>> = Rc::default();
    let (kept, seen, held) = (Rc::clone(&state), Rc::clone(&out), Rc::clone(&keep));
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept.borrow().clone();
        let offset = now.offset;
        let keep = held.borrow().clone();
        let page_view = PageView::new("Pages", &source, &mut now, PAPER).options(LayoutOptions::ACROBAT).navigation(navigation).keep(&keep);
        let shown = if host_passthrough {
            let rect = egui::Rect::from_min_size(ui.max_rect().min + vec2(276.0, 68.0), vec2(998.0, 832.0));
            page_view.show_at_with(
                ui,
                rect,
                &mut body,
                |ui, page, body| {
                    body(ui, page);
                    page.pass.and_then(|pass| pass.view)
                },
                |_, _, _| (),
            )
        } else {
            page_view.show_with(
                ui,
                &mut body,
                |ui, page, body| {
                    body(ui, page);
                    None
                },
                |_, _, _| (),
            )
        };
        let scrolled = seen.borrow().scrolled || shown.scrolled;
        *seen.borrow_mut() =
            Out { offset, rects: shown.layout.rects.clone(), pages: shown.pages, visible: shown.visible, view: shown.canvas.response.rect, scrolled };
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

/// A written zoom keeps the point at the centre; a new room (a panel opened beside the view) refits and keeps the
/// current page's top where it was, as production did.
#[trial]
fn a_changed_zoom_keeps_the_point_at_the_centre_and_a_new_room_the_current_pages_top() {
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
    fitted.state.borrow_mut().go_to_fraction(2, 0.2);
    fitted.fixture.idle(3);
    let below_top = |out: &Out, page: usize| out.rects[page].y0 - out.offset.1;
    let (page, before) = (fitted.state.borrow().current, below_top(&fitted.out.borrow(), 2));
    assert_eq!(page, 2, "precondition: the third page is current");
    let scale = fitted.out.borrow().rects[2].height();
    fitted.fixture.room(vec2(700.0, 500.0));
    fitted.fixture.idle(3);
    let out = fitted.out.borrow();
    assert!(out.rects[2].height() < scale, "precondition: the fit shrank the pages");
    assert_eq!(fitted.state.borrow().current, 2, "the page stays current");
    let after = below_top(&out, 2);
    assert!((after - before).abs() < 1.0, "the current page's top stays put: {before} then {after}");
}

#[trial]
fn resizing_back_keeps_the_last_pages_top() {
    let mut view = rig(3, state_with(|s| s.fit = Fit::Width));
    view.state.borrow_mut().go_to(2);
    view.fixture.idle(4);
    let top = |out: &Out| out.rects[2].y0 - out.offset.1;
    let before = top(&view.out.borrow());
    view.fixture.room(vec2(1200.0, 600.0));
    view.fixture.idle(4);
    view.fixture.room(vec2(900.0, 600.0));
    view.fixture.idle(4);
    let after = top(&view.out.borrow());
    assert!((before - after).abs() < 1.0, "the last page stays at its top: {before} then {after}");
}

/// Turning the page gaps off keeps the scroll offset, as production did, so the pages below the top close up toward
/// it; also when the gaps change while the view is hidden (a page grid shown in its place) and it shows again.
#[trial]
fn a_page_gap_change_keeps_the_scroll_offset() {
    for fit in [Fit::None, Fit::Width] {
        for hidden in [false, true] {
            let source = DemoPages::new(5);
            let state = Rc::new(RefCell::new(state_with(|s| s.fit = fit)));
            let shows = Rc::new(Cell::new(true));
            let rects: Rc<RefCell<Vec<kurbo::Rect>>> = Rc::default();
            let (kept, seen, showing) = (Rc::clone(&state), Rc::clone(&rects), Rc::clone(&shows));
            let mut fixture = Fixture::new(move |ui| {
                if !showing.get() {
                    return;
                }
                let mut now = kept.borrow().clone();
                let shown = PageView::new("Pages", &source, &mut now, PAPER)
                    .options(LayoutOptions::ACROBAT)
                    .show(ui, &mut |_: &mut egui::Ui, _: &PageBody<'_>| ());
                *seen.borrow_mut() = shown.layout.rects.clone();
                *kept.borrow_mut() = now;
            });
            fixture.room(vec2(900.0, 600.0));
            fixture.settle();
            state.borrow_mut().go_to_fraction(1, 0.0);
            fixture.idle(3);
            let offset = state.borrow().offset;
            assert_eq!(state.borrow().current, 1, "precondition: the second page is current");
            shows.set(!hidden);
            fixture.idle(2);
            state.borrow_mut().page_gaps = false;
            fixture.idle(2);
            shows.set(true);
            fixture.idle(3);
            let after = state.borrow().offset;
            assert!(
                (after.0 - offset.0).abs() < 0.5 && (after.1 - offset.1).abs() < 0.5,
                "{fit:?}, hidden {hidden}: the offset stays: {offset:?} then {after:?}"
            );
        }
    }
}

#[trial]
fn the_pictures_a_view_holds_stay_bounded_as_it_scrolls_through_many_pages() {
    let budget = TextureStoreOptions::pages().budget.max_entries;
    // Past the budget by half again: the store has to evict for the bound to be tested.
    let count = budget + budget / 2;
    let state = Rc::new(RefCell::new(state_with(|s| {
        s.continuous = true;
        s.fit = Fit::None;
        s.zoom = 1.0;
    })));
    // The most pictures held and the most pages drawn in one pass.
    let most = Rc::new(Cell::new((0_usize, 0_usize)));
    let (kept, seen) = (Rc::clone(&state), Rc::clone(&most));
    let pages = DemoPages::new(count);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept.borrow().clone();
        let out = PageView::new("Pages", &pages, &mut now, PAPER).options(LayoutOptions::ACROBAT).show(ui, |_, _| ());
        let (held, drawn) = seen.get();
        seen.set((held.max(out.resident_pictures()), drawn.max(out.pages.len())));
        *kept.borrow_mut() = now;
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    for page in 0..count {
        state.borrow_mut().go_to(page);
        fixture.idle(2);
    }
    let (held, drawn) = most.get();
    assert!(drawn > 0, "the view drew pages");
    // The pages of the last two passes, in view and one row either side, are never evicted.
    let bound = budget + 2 * (drawn + 2);
    eprintln!("DIAG pictures_bounded: budget {budget}, count {count}, held {held}, drawn {drawn}");
    assert!(held > 0 && held <= bound, "{held} pictures held over {count} pages scrolled through, at most {bound} expected");
}

/// The most pictures a continuous view of 40 Letter pages at 100 % in a 900 × 600 room holds while it scrolls from
/// the top to the bottom 400 points a step, with `reach` (`None`: the default), and the most pages it drew in a pass.
fn most_held_scrolling(reach: Option<f32>) -> (usize, usize) {
    let state = Rc::new(RefCell::new(state_with(|s| {
        s.continuous = true;
        s.fit = Fit::None;
        s.zoom = 1.0;
    })));
    let most = Rc::new(Cell::new((0_usize, 0_usize)));
    let (kept, seen) = (Rc::clone(&state), Rc::clone(&most));
    let pages = DemoPages::new(40);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept.borrow().clone();
        let view = PageView::new("Pages", &pages, &mut now, PAPER).options(LayoutOptions::ACROBAT);
        let view = match reach {
            Some(screens) => view.picture_reach(screens),
            None => view,
        };
        let out = view.show(ui, |_, _| ());
        let (held, drawn) = seen.get();
        seen.set((held.max(out.resident_pictures()), drawn.max(out.pages.len())));
        *kept.borrow_mut() = now;
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let mut last = f64::NAN;
    while state.borrow().offset.1 != last {
        last = state.borrow().offset.1;
        state.borrow_mut().offset.1 += 400.0;
        fixture.idle(2);
    }
    assert!(last > 30_000.0, "the view scrolled through the pages: {last}");
    most.get()
}

#[trial]
fn pages_scrolled_out_of_reach_give_their_pictures_back() {
    // WordCraft (#811) kept the textures of the pages within three screens of the view: a 600-point view holds the
    // pages over 4'200 points, four or five 1'056-point pages, and the row either side; never the 40 it went past.
    let (held, drawn) = most_held_scrolling(None);
    assert!(drawn > 0, "the view drew pages");
    assert!(held > 0 && held <= 8, "{held} pictures held while scrolling 40 pages with the default reach");
    // With no reach only the pages drawn and the row either side keep theirs.
    let (held, drawn) = most_held_scrolling(Some(0.0));
    assert!(held > 0 && held <= drawn + 2, "{held} pictures held with no reach, {drawn} pages drawn");
}

/// What a pass of the zoom rig showed: the area the pages show in, the pages drawn and the screen points per page point.
#[derive(Clone)]
struct ZoomSeen {
    area: egui::Rect,
    pages: Vec<(usize, egui::Rect)>,
    scale: f64,
}

impl ZoomSeen {
    /// Pages across: those level with the first page drawn.
    fn columns(&self) -> usize {
        self.pages.first().map_or(0, |(_, first)| self.pages.iter().filter(|(_, r)| (r.top() - first.top()).abs() < 1.0).count())
    }

    fn rect(&self, page: usize) -> Option<egui::Rect> {
        self.pages.iter().find(|(p, _)| *p == page).map(|(_, r)| *r)
    }
}

/// WordCraft's Ctrl + wheel zoom out (#1340): the point under the pointer stays where it is in every frame, the
/// pages reflow into rows in the frame of the notch that makes room for them, and a frame without input moves
/// nothing. Pointer in the middle of the area inside the rulers, many pages down, a notch every 8 frames; the pages
/// pass through a gutter between columns under the pointer on the way.
#[trial]
fn a_ctrl_wheel_zoom_keeps_the_point_under_the_pointer_and_settles_in_its_own_frame() {
    // Far enough down that zooming out never meets the top of the document, whose clamp would move the place.
    ctrl_wheel_zoom_out(vec2(1280.0, 760.0), 40.4 * 1056.0, 30, egui::Vec2::ZERO, 5);
}

/// The WordCraft app's case (#1340): a few pages down, the pointer just inside page 3, right of the gutter in the
/// middle of the view, when the pages reflow from two columns to three, which moves page 3 down a row. The canvas
/// zooms about the pointer, then keeps the pages (no wider than the view) centred: the place under the pointer, not
/// the middle of the view, keeps its place, on page 3 and not page 2 across the gutter. Then three columns to four and
/// five, before the top of the document stops the view.
#[trial]
fn a_ctrl_wheel_zoom_keeps_the_place_when_the_page_under_the_pointer_moves_down_a_row() {
    ctrl_wheel_zoom_out(vec2(1280.0, 690.0), 13.2 * 1056.0, 3, vec2(7.0, 0.0), 4);
}

/// `notches` Ctrl + wheel notches out, 8 frames apart, in a page view `room` big from `top` (screen points at 100 %)
/// with the pointer `nudge` from the middle of the area: every frame keeps the point under the pointer at the notch,
/// which starts on page `least` or later.
fn ctrl_wheel_zoom_out(room: egui::Vec2, top: f64, least: usize, nudge: egui::Vec2, notches: usize) {
    let state = Rc::new(RefCell::new(state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 1.0;
        s.display = PageDisplay::Columns;
    })));
    let seen = Rc::new(RefCell::new(ZoomSeen { area: egui::Rect::NOTHING, pages: Vec::new(), scale: 0.0 }));
    let (kept, shown) = (Rc::clone(&state), Rc::clone(&seen));
    let pages = DemoPages::new(200);
    let mut navigation = craft_ui::canvas::NavigationOptions::default();
    navigation.wheel = craft_ui::canvas::WheelScheme::Document;
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept.borrow().clone();
        let mut area = egui::Rect::NOTHING;
        let out = PageView::new("Pages", &pages, &mut now, PAPER)
            .options(LayoutOptions::WORD)
            .navigation(navigation)
            .rulers(true)
            .show(ui, |_, body| area = body.view.rect);
        let scale = out.pages.first().map_or(0.0, |p| p.scale);
        *shown.borrow_mut() = ZoomSeen { area, pages: out.pages.iter().map(|p| (p.page, p.rect)).collect(), scale };
        *kept.borrow_mut() = now;
    });
    fixture.ctx().options_mut(|options| options.input_options.zoom_modifier = Modifiers::NONE);
    fixture.room(room);
    fixture.settle();
    state.borrow_mut().offset.1 = top;
    fixture.settle();
    let pointer = seen.borrow().area.center() + nudge;
    fixture.frame(vec![egui::Event::PointerMoved(pointer)]).settle();
    let start = seen.borrow().clone();
    // The page under the pointer, else the nearest; a reflow can put another page there, so each notch asks again.
    let page_under = |seen: &ZoomSeen| {
        let (page, _) =
            *seen.pages.iter().min_by(|(_, a), (_, b)| a.distance_to_pos(pointer).total_cmp(&b.distance_to_pos(pointer))).expect("pages show");
        page
    };
    let mut page = page_under(&start);
    assert!(page >= least, "pages down: page {page}");
    let under = |seen: &ZoomSeen, page: usize| seen.rect(page).map(|r| (pointer - r.min) / seen.scale as f32);
    let mut at = under(&start, page).expect("the page shows");
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    fixture.frame(vec![egui::Event::ModifiersChanged(ctrl)]);
    let mut last = seen.borrow().clone();
    for n in 0..notches * 8 {
        let notch = n % 8 == 0;
        if notch {
            page = page_under(&last);
            at = under(&last, page).expect("the page under the pointer shows");
        }
        let events = if notch {
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, -1.0),
                modifiers: ctrl,
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
            }]
        } else {
            Vec::new()
        };
        fixture.frame(events);
        let now = seen.borrow().clone();
        let here = under(&now, page).unwrap_or_else(|| panic!("frame {n}: page {page} shows"));
        let off = (here - at) * now.scale as f32;
        // Across, the place holds while the pages stay one a row; in rows they fit the window side by side.
        let across = now.columns() > start.columns() || off.x.abs() < 2.0;
        assert!(
            across && off.y.abs() < 2.0,
            "frame {n} at scale {:.3}: {here:?} on page {page}, was {at:?}; {:?} after {:?}",
            now.scale,
            now.pages,
            last.pages
        );
        if !notch {
            assert_eq!(now.columns(), last.columns(), "frame {n}: the pages reflow only in a notch's frame");
            assert_eq!(now.pages, last.pages, "frame {n}: a frame without input moves nothing");
        }
        last = now;
    }
    assert!(last.scale < start.scale * 0.6, "zoomed out: {} -> {}", start.scale, last.scale);
    assert!(
        last.columns() > start.columns(),
        "the pages reflowed into rows on the way: {} to {} at {} in {:?}: {:?}",
        start.columns(),
        last.columns(),
        last.scale,
        last.area,
        last.pages
    );
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

#[trial]
fn an_eased_notch_settles_at_the_same_page_position_as_an_immediate_notch() {
    let run = |eased| {
        let mut navigation = NavigationOptions::default();
        navigation.wheel_line = 100.0;
        navigation.eased_wheel = eased;
        let start = state_with(|s| {
            s.fit = Fit::None;
            s.zoom = 1.0;
        });
        let mut view = rig_over(DemoPages::new(3), start, navigation, true);
        wheel(&mut view.fixture, -1.0);
        view.fixture.idle(100);
        let state = view.state.borrow();
        let out = view.out.borrow();
        (state.offset.1, out.pages[0].rect.min.y)
    };
    let immediate = run(false);
    let eased = run(true);
    assert!((immediate.0 - eased.0).abs() < 0.0000001, "both notches settle at the same offset: {immediate:?} {eased:?}");
    assert!((immediate.1 - eased.1).abs() < 0.5, "both notches place the page alike: {immediate:?} {eased:?}");
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
fn the_preview_takes_page_keys_when_first_opened() {
    let at = Rc::new(RefCell::new(1_usize));
    let kept = Rc::clone(&at);
    let pages = DemoPages::new(5);
    let mut open = false;
    let mut fixture = Fixture::new(move |ui| {
        if craft_ui::contract::clicked(&craft_ui::buttons::Button::new("Preview").show(ui)) {
            open = true;
        }
        if open {
            PagePreview::new(&pages, &[0, 1, 2, 3, 4], &mut kept.borrow_mut(), PAPER).show(ui);
        }
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let button = fixture.by_label("Preview").expect("preview button");
    fixture.click(&button).settle();
    fixture.key(Key::PageDown, Modifiers::NONE);
    assert_eq!(*at.borrow(), 2, "Page Down steps the newly opened preview");
}

/// The memoized view that holds the Preview button and, once it was pressed, the preview.
#[derive(craft_ui::contract::MemoProps)]
struct PreviewRegion {}

thread_local! {
    /// What the preview region shows: whether the preview is open, and the page it is at.
    static PREVIEW_REGION: RefCell<(bool, usize)> = const { RefCell::new((false, 1)) };
}

fn preview_region(_: &mut PreviewRegion, ui: &mut egui::Ui) {
    if craft_ui::contract::clicked(&craft_ui::buttons::Button::new("Preview").show(ui)) {
        PREVIEW_REGION.with(|region| region.borrow_mut().0 = true);
    }
    let pages = DemoPages::new(5);
    PREVIEW_REGION.with(|region| {
        let (open, at) = &mut *region.borrow_mut();
        if *open {
            PagePreview::new(&pages, &[0, 1, 2, 3, 4], at, PAPER).show(ui);
        }
    });
}

/// A preview opened inside a settled memoized view (a docked panel's body) takes the page keys.
#[trial]
fn the_preview_in_a_settled_view_takes_page_keys_when_opened() {
    PREVIEW_REGION.with(|region| *region.borrow_mut() = (false, 1));
    let mut fixture = Fixture::new(move |ui| {
        craft_ui::contract::MemoView::new("region", PreviewRegion {}, preview_region).show(ui);
    });
    fixture.room(vec2(900.0, 600.0)).settle();
    let button = fixture.by_label("Preview").expect("preview button");
    fixture.click(&button).settle();
    // Focus entered this modal after the command board last recorded its vantage.
    craft_ui::runtime::commands::clear_focus_vantage(fixture.ctx());
    fixture.key(Key::PageDown, Modifiers::NONE);
    assert_eq!(PREVIEW_REGION.with(|region| region.borrow().1), 2, "Page Down steps the preview in a settled view");
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
    assert!(large.height() >= small.height() * 1.5, "the page takes the window: {small:?} then {large:?}");
    assert!(closed, "Close reports closed");
}

/// The preview is a framed dialog, as production's: the file's name in a header with the page's place and a close
/// x, the page inside the frame (not the whole window), and the arrows under it.
#[trial]
fn the_preview_is_a_framed_dialog_with_a_header_and_a_close_x() {
    let pages = DemoPages::new(3);
    let mut at = 0_usize;
    let mut fixture = Fixture::new(move |ui| {
        PagePreview::new(&pages, &[0, 1, 2], &mut at, PAPER).title("ten-pages.pdf").show(ui);
    });
    let room = vec2(1200.0, 900.0);
    fixture.room(room);
    fixture.settle();
    assert!(fixture.by_label("ten-pages.pdf").is_some(), "the header names the file");
    let picture = fixture.by_role(Role::IMAGE).first().expect("the page's picture is shown").rect;
    assert!(picture.width() < room.x * 0.9 && picture.height() < room.y * 0.9, "the page stands inside a frame: {picture:?}");
    assert!(fixture.by_label("Close").is_some(), "the header has a close x");
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

/// With nothing focused the window's document canvas is its key target: the page keys and the arrows no tool takes
/// page the view, as they do while it has the focus. They used to reach only the window, where no view read them.
#[trial]
fn page_keys_and_arrows_page_the_view_with_nothing_focused() {
    let (mut fixture, state, _) = layout_rig(
        5,
        state_with(|s| {
            s.continuous = false;
            s.fit = Fit::Page;
        }),
    );
    assert_eq!(fixture.focused_stop(), None, "nothing has the focus");
    fixture.key(Key::PageDown, Modifiers::NONE);
    fixture.idle(1);
    assert_eq!(state.borrow().current, 1, "Page Down turns one page");
    fixture.key(Key::ArrowRight, Modifiers::NONE);
    fixture.idle(1);
    assert_eq!(state.borrow().current, 2, "Right turns one page");
    fixture.key(Key::End, Modifiers::NONE);
    fixture.idle(1);
    assert_eq!(state.borrow().current, 4, "End goes to the last page");
    assert_eq!(fixture.focused_stop(), None, "the keys leave the focus where it was");
}

#[trial]
fn shift_and_command_with_the_page_keys_and_arrows_go_to_the_ends() {
    let both = Modifiers::COMMAND | Modifiers::SHIFT;
    for continuous in [false, true] {
        let (mut fixture, state, _) = layout_rig(
            5,
            state_with(|s| {
                s.continuous = continuous;
                s.fit = Fit::Page;
            }),
        );
        focus_canvas(&mut fixture);
        let keys = [(Key::PageDown, both, 4), (Key::PageUp, both, 0), (Key::ArrowDown, both, 4), (Key::ArrowUp, both, 0)];
        // Off macOS Ctrl+End and Ctrl+Home go to the ends too; macOS gives Cmd with Home and End no move.
        let mut ends = vec![(Key::End, Modifiers::SHIFT, 4), (Key::Home, Modifiers::SHIFT, 0)];
        if !cfg!(target_os = "macos") {
            ends.extend([(Key::End, Modifiers::COMMAND, 4), (Key::Home, Modifiers::COMMAND, 0)]);
        }
        for (key, mods, want) in keys.into_iter().chain(ends) {
            fixture.key(key, mods);
            fixture.idle(2);
            assert_eq!(state.borrow().current, want, "{key:?} with {mods:?}, continuous {continuous}");
        }
    }
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
    for (page, rect) in rects.iter().enumerate().skip(1) {
        state.borrow_mut().offset.1 = rect.y0;
        fixture.idle(2);
        assert_eq!(state.borrow().zoom, zoom, "page {page} at the top leaves the fitted zoom alone");
        assert_eq!(out.borrow().rects[page], *rect, "page {page} at the top keeps its place and size");
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

    type Change = (&'static str, fn(&mut PageViewState));
    let changes: [Change; 2] = [("display", |s| s.display = PageDisplay::TwoUp), ("fit", |s| s.fit = Fit::Page)];
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

#[trial]
fn the_view_reports_the_readers_wheel_and_scroll_bar_but_not_a_go_to_or_a_zoom() {
    let Rig { mut fixture, state, out, .. } = rig(5, state_with(|s| s.fit = Fit::Width));
    let scrolled_by = |fixture: &mut Fixture, what: &mut dyn FnMut(&mut Fixture)| {
        out.borrow_mut().scrolled = false;
        what(fixture);
        fixture.settle();
        out.borrow().scrolled
    };
    let go_to = scrolled_by(&mut fixture, &mut |fixture| {
        state.borrow_mut().go_to = Some(GoTo::Top { page: 3, fraction: 0.0 });
        fixture.idle(2);
    });
    assert!(!go_to, "the app's go-to is not the reader's scroll");
    assert_eq!(state.borrow().current, 3, "precondition: the go-to moved the view");
    let zoom = scrolled_by(&mut fixture, &mut |fixture| {
        fixture.key(Key::Equals, Modifiers::COMMAND);
    });
    assert!(!zoom, "a zoom is not a scroll");
    let wheeled = scrolled_by(&mut fixture, &mut |fixture| wheel(fixture, -1.0));
    assert!(wheeled, "the wheel's scroll is the reader's");
    let bar =
        fixture.by_role(Role::SCROLL_BAR).into_iter().max_by(|a, b| a.rect.height().total_cmp(&b.rect.height())).expect("the view has scroll bars");
    let dragged = scrolled_by(&mut fixture, &mut |fixture| {
        let grab = pos2(bar.rect.center().x, bar.rect.max.y - 4.0);
        fixture.frame(vec![craft_ui::testing::pointer::moved(grab)]);
        fixture.frame(vec![craft_ui::testing::pointer::press(grab)]);
        fixture.frame(vec![craft_ui::testing::pointer::release(grab)]);
    });
    assert!(dragged, "a press on the scroll bar's track is the reader's scroll");
}

/// A tool that takes the arrows while `takes` holds (a selected picture's nudge), and counts them.
struct Nudges {
    takes: Rc<Cell<bool>>,
    taken: Rc<Cell<usize>>,
}

/// A hand drag must move the pages in the pass that receives the movement.
#[trial]
fn hand_drag_places_pages_at_the_tools_view_in_the_same_pass() {
    let observed = Rc::new(RefCell::new(None));
    let seen = Rc::clone(&observed);
    let mut hand = HandTool::new(NavigationOptions::default());
    let start = state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 2.0;
        s.offset = (200.0, 200.0);
    });
    let mut state = start;
    let source = DemoPages::new(5);
    let (before_runs, after_runs) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (before_count, after_count) = (Rc::clone(&before_runs), Rc::clone(&after_runs));
    let mut fixture = Fixture::new(move |ui| {
        let mut target = None;
        PageView::new("Pages", &source, &mut state, PAPER).show_with(
            ui,
            &mut target,
            |ui, body, target| {
                before_count.set(before_count.get() + 1);
                let reply = body.pass.map(|pass| ToolHost::new().run(ui, pass, &mut hand));
                let view = reply.and_then(|reply| reply.view);
                *target = view.and_then(|view| body.pages.first().map(|page| (view, body.view.to_doc(page.rect.min))));
                view
            },
            |_, body, target| {
                after_count.set(after_count.get() + 1);
                if let (Some((view, point)), Some(page)) = (*target, body.pages.first()) {
                    *seen.borrow_mut() = Some((body.view, view, page.rect.min, point));
                }
            },
        );
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let at = pos2(450.0, 300.0);
    fixture.frame(vec![craft_ui::testing::pointer::moved(at)]);
    fixture.frame(vec![craft_ui::testing::pointer::press(at)]);
    *observed.borrow_mut() = None;
    fixture.frame(vec![craft_ui::testing::pointer::moved(at + vec2(0.0, -20.0))]);
    let (drawn, target, page_min, point) = observed.borrow().expect("the hand reports its moved view");
    let distance = (drawn.center - target.center).hypot() * target.zoom;
    assert!(distance < 1.0, "the pages use the hand's view in the movement pass: {distance} points apart");
    assert!(page_min.distance(target.to_screen(point)) < 1.0, "the page is placed at the hand's view in the movement pass");
    assert_eq!(before_runs.get(), after_runs.get(), "each placement hosts tools once before its overlay");
}

impl CanvasTool for Nudges {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: craft_behavior::CancelReason, _: &mut ToolContext<'_>) {}

    fn key(&mut self, key: &ToolKey, _: &mut ToolContext<'_>) -> bool {
        let take = matches!(key, ToolKey::Nudge { .. }) && self.takes.get();
        if take {
            self.taken.set(self.taken.get() + 1);
        }
        take
    }

    fn paint(&self, _: &craft_ui::primitives::Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the nudged object's position fields")
    }
}

/// The page view gives the window's arrows to its body's tool host first and turns the page only with what the tool
/// left, in the same pass. The view used to take them before its body ran, so a selected object never moved.
#[trial]
fn an_arrow_the_tool_takes_leaves_the_page_and_one_it_leaves_turns_it() {
    let (takes, taken) = (Rc::new(Cell::new(true)), Rc::new(Cell::new(0)));
    let mut tool = Nudges { takes: Rc::clone(&takes), taken: Rc::clone(&taken) };
    let start = state_with(|s| {
        s.continuous = false;
        s.fit = Fit::Page;
    });
    let Rig { mut fixture, state, .. } = rig_with(DemoPages::new(5), start, NavigationOptions::default(), true, move |ui, body| {
        if let Some(pass) = body.pass {
            let reply = ToolHost::new().run(ui, pass, &mut tool);
            assert!(reply.view.is_none(), "the tool moves no view");
        }
    });
    fixture.key(Key::ArrowRight, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!(taken.get(), 1, "the tool takes the arrow");
    assert_eq!(state.borrow().current, 0, "an arrow the tool took leaves the page");
    takes.set(false);
    fixture.key(Key::ArrowRight, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!((taken.get(), state.borrow().current), (1, 1), "an arrow the tool leaves turns the page");
}

/// With nothing focused the view takes only the arrows it acts on: a plain arrow turns the page and is the view's
/// alone, while Alt with an arrow, which it does not read, stays in egui's input for the app's shortcuts (a view
/// history's Alt+Left). The view used to hold every arrow, whatever was held with it.
#[trial]
fn with_nothing_focused_the_view_leaves_alt_and_an_arrow_to_the_app() {
    let reached: Rc<RefCell<Vec<Modifiers>>> = Rc::default();
    let seen = Rc::clone(&reached);
    let start = state_with(|s| {
        s.continuous = false;
        s.fit = Fit::Page;
    });
    let Rig { mut fixture, state, .. } = rig_with(DemoPages::new(5), start, NavigationOptions::default(), true, move |ui, _| {
        let pressed = ui.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Key { key: Key::ArrowRight, pressed: true, modifiers, .. } => Some(*modifiers),
                _ => None,
            })
        });
        seen.borrow_mut().extend(pressed);
    });
    fixture.key(Key::ArrowRight, Modifiers::ALT);
    fixture.idle(2);
    assert_eq!(state.borrow().current, 0, "Alt+Right turns no page");
    assert!(reached.borrow().iter().any(|mods| mods.alt), "Alt+Right reaches the app");
    reached.borrow_mut().clear();
    fixture.key(Key::ArrowRight, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!(state.borrow().current, 1, "Right turns the page");
    assert!(reached.borrow().is_empty(), "Right is the view's alone");
}

/// The page and end keys a focused control in the page body does not use (a button takes no moves) page the view,
/// as they do with the canvas focused: the control leaves them untaken and they go on to the view, its scroll owner.
#[trial]
fn page_keys_a_focused_control_in_the_body_leaves_page_the_view() {
    let start = state_with(|s| {
        s.continuous = false;
        s.fit = Fit::Page;
    });
    let Rig { mut fixture, state, .. } = rig_with(DemoPages::new(5), start, NavigationOptions::default(), true, |ui, _| {
        let _ = craft_ui::buttons::Button::new("Mark").show(ui);
    });
    for _ in 0..4 {
        if fixture.focused_stop().is_some() && fixture.focused_role().is_some_and(|role| !role.nudge) {
            break;
        }
        fixture.key(Key::Tab, Modifiers::NONE);
    }
    assert!(fixture.focused_role().is_some_and(|role| !role.nudge), "Tab reaches the button in the body");
    fixture.key(Key::PageDown, Modifiers::NONE);
    fixture.idle(1);
    assert_eq!(state.borrow().current, 1, "Page Down turns one page");
    fixture.key(Key::End, Modifiers::NONE);
    fixture.idle(1);
    assert_eq!(state.borrow().current, 4, "End goes to the last page");
}

/// Cmd with Page Down turns one page in a scrolling view, where plain Page Down scrolls a screen; Right turns a
/// spread in a scrolling two-page view, not to the other page of the spread already on screen.
#[trial]
fn command_page_down_turns_a_page_and_right_a_spread_in_a_scrolling_view() {
    let (mut fixture, state, out) = layout_rig(5, state_with(|s| s.fit = Fit::Page));
    focus_canvas(&mut fixture);
    fixture.key(Key::PageDown, Modifiers::COMMAND);
    fixture.idle(2);
    assert_eq!(state.borrow().current, 1, "Cmd+Page Down turns one page");
    let top = out.borrow().rects[1].y0 - out.borrow().offset.1;
    assert!((0.0..40.0).contains(&top), "the page's top comes to the view's top: {top}");
    fixture.key(Key::PageUp, Modifiers::COMMAND);
    fixture.idle(2);
    assert_eq!(state.borrow().current, 0, "Cmd+Page Up turns back");

    let (mut fixture, state, _) = layout_rig(
        6,
        state_with(|s| {
            s.fit = Fit::Page;
            s.display = PageDisplay::TwoUp;
        }),
    );
    focus_canvas(&mut fixture);
    fixture.key(Key::ArrowRight, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!(state.borrow().current, 2, "Right moves a spread");
}

/// Right on a page wider than the view scrolls it a line, turns at its right edge, and the next page comes from its
/// left edge, so Right scrolls across it rather than turning again. The view used to scroll the wide layout for ever.
#[trial]
fn right_scrolls_a_page_wider_than_the_view_then_turns_at_its_edge() {
    let (mut fixture, state, out) = layout_rig(
        3,
        state_with(|s| {
            s.continuous = false;
            s.fit = Fit::None;
            s.zoom = 3.0;
        }),
    );
    focus_canvas(&mut fixture);
    let start = state.borrow().offset.0;
    fixture.key(Key::ArrowRight, Modifiers::NONE);
    fixture.idle(2);
    assert_eq!(state.borrow().current, 0, "Right scrolls the wide page");
    assert!(state.borrow().offset.0 > start, "the page moved left");
    let mut presses = 1;
    while state.borrow().current == 0 {
        fixture.key(Key::ArrowRight, Modifiers::NONE);
        fixture.idle(2);
        presses += 1;
        assert!(presses < 300, "Right never reached the page's edge");
    }
    assert!(presses > 3, "Right scrolled across the page before turning ({presses} presses)");
    let shown = out.borrow();
    let (left, view) = (shown.rects[1].x0 - shown.offset.0, f64::from(shown.view.width()));
    assert!(left >= 0.0 && shown.rects[1].x1 - shown.offset.0 > view, "the next page shows from its left edge: {left}");
}

/// In a continuous view the current page is the one that shows the most of itself, the topmost of a tie; the page
/// gone to stays current while it shows wholly or as much as any other, as the reader scrolls (production's rule,
/// not the page under the view's middle).
#[trial]
fn the_current_page_shows_the_most_of_itself_and_a_page_gone_to_stays_while_it_shows_whole() {
    let Rig { mut fixture, state, out, .. } = rig(
        6,
        state_with(|s| {
            s.fit = Fit::None;
            s.zoom = 0.25;
        }),
    );
    let shown = |out: &Out, page: usize| {
        let r = out.rects[page];
        let (top, bottom) = (out.offset.1, out.offset.1 + f64::from(out.view.height()));
        (r.y1.min(bottom) - r.y0.max(top)).max(0.0)
    };
    // The last page, gone to, sits wholly at the bottom of the view; a line up the wheel leaves it current.
    state.borrow_mut().go_to(5);
    fixture.settle();
    wheel(&mut fixture, 1.0);
    fixture.settle();
    {
        let out = out.borrow();
        assert!(shown(&out, 5) >= out.rects[5].height() - 0.5, "precondition: the last page shows whole");
        assert_eq!(state.borrow().current, 5, "the page gone to stays current while it shows whole");
    }
    // From the top the wheel scrolls down until the second page shows more of itself than the first.
    state.borrow_mut().go_to(0);
    fixture.settle();
    for _ in 0..40 {
        let more = {
            let out = out.borrow();
            shown(&out, 1) > shown(&out, 0) + 1.0 && shown(&out, 1) > shown(&out, 2) + 1.0
        };
        if more {
            break;
        }
        wheel(&mut fixture, -1.0);
        fixture.settle();
    }
    let out = out.borrow();
    let most = (0..6).max_by(|&a, &b| shown(&out, a).total_cmp(&shown(&out, b)).then(b.cmp(&a))).unwrap_or(0);
    assert_eq!(most, 1, "precondition: the second page shows the most");
    assert_eq!(state.borrow().current, 1, "the page that shows the most of itself is current");
}

/// Pages shorter than the view sit in its middle, a page or the column of a scrolling view, and so does a page that
/// fits only without its margins. The scroll area used to be at least the view's height, from the top.
#[trial]
fn pages_shorter_than_the_view_sit_in_its_middle() {
    let middle = |out: &Out, rows: Range<usize>| {
        let column = rows.map(|page| out.rects[page]).reduce(|a, b| a.union(b)).expect("pages");
        let (top, bottom) = (column.y0 - out.offset.1, column.y1 - out.offset.1);
        let view = f64::from(out.view.height());
        assert!(bottom - top < view, "precondition: {top}..{bottom} is shorter than {view}");
        assert!((top - (view - bottom)).abs() < 1.0, "{top}..{bottom} sits in the middle of {view}");
    };
    let small = |continuous: bool, zoom: f64| {
        state_with(|s| {
            s.continuous = continuous;
            s.fit = Fit::None;
            s.zoom = zoom;
        })
    };
    let (_, _, out) = layout_rig(1, small(false, 0.25));
    middle(&out.borrow(), 0..1);
    let (_, _, out) = layout_rig(2, small(true, 0.25));
    middle(&out.borrow(), 0..2);

    // A page that fits the view only without its margins shows whole, in the middle.
    let height = out.borrow().rects[0].height();
    let room = f64::from(out.borrow().view.height());
    let (_, _, out) = layout_rig(1, small(false, 0.25 * (room - 8.0) / height));
    middle(&out.borrow(), 0..1);
}

/// The reader's wheel over the pages scrolls the view and reports it while a field above the view (a find bar's
/// query) has the focus.
#[trial]
fn the_wheel_over_the_pages_scrolls_and_reports_it_while_a_field_beside_them_has_the_focus() {
    let state = Rc::new(RefCell::new(state_with(|s| s.fit = Fit::Width)));
    let scrolled = Rc::new(Cell::new(false));
    let (kept, seen) = (Rc::clone(&state), Rc::clone(&scrolled));
    let source = DemoPages::new(5);
    let mut query = String::new();
    let mut fixture = Fixture::new(move |ui| {
        craft_ui::text::TextField::new("Find", Bind::bound(&mut query)).show(ui);
        let mut now = kept.borrow().clone();
        let shown = PageView::new("Pages", &source, &mut now, PAPER).show(ui, |_, _| ());
        seen.set(seen.get() || shown.scrolled);
        *kept.borrow_mut() = now;
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    assert!(fixture.focused_role().is_some_and(|role| role.text), "precondition: the field has the focus");
    let before = state.borrow().offset.1;
    fixture.frame(vec![egui::Event::PointerMoved(pos2(450.0, 350.0))]);
    scrolled.set(false);
    fixture.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -20.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
    }]);
    fixture.idle(4);
    assert!(state.borrow().offset.1 > before, "the wheel scrolls the pages");
    assert!(scrolled.get(), "the view reports the reader's scroll");
}

/// With `PageInsets::Theme` (PdfCraft's page view) a page fitted to the width stands the theme's page side margin from
/// each edge of the view, and the first page stands the theme's page margin below its top: the spacing is the theme's
/// (production's 70 and 28 points at the comfortable density), not a size derived from the tool rail.
#[trial]
fn a_page_fitted_to_the_width_keeps_the_theme_side_margin() {
    use craft_theme::{Contrast, Mode};
    let tokens = craft_ui::testing::style::tokens(Mode::Dark, Contrast::Standard);
    let (side, margin) = (f64::from(tokens.space.inset.page_side), f64::from(tokens.space.inset.page_margin));
    let mut view = rig_over(DemoPages::new(3), state_with(|s| s.fit = Fit::Width), NavigationOptions::default(), false);
    view.fixture.root_value(craft_pages::ui::PageInsets::Theme);
    view.fixture.settle();
    let out = view.out.borrow();
    let page = out.rects[0];
    let left = page.x0 - out.offset.0;
    let right = f64::from(out.view.width()) - (page.x1 - out.offset.0);
    assert!((left - side).abs() < 1.0 && (right - side).abs() < 1.0, "side margins {left} and {right}, not the theme's {side}");
    assert!((page.y0 - out.offset.1 - margin).abs() < 1.0, "the first page's margin: {} not {margin}", page.y0 - out.offset.1);
}

/// L-309: the page view's surround takes the colour the app sets: full screen shows the pages on black, as production.
#[trial]
fn the_pasteboard_colour_of_a_page_view_paints_the_surround() {
    use craft_ui::canvas::PasteboardColor;
    let paints_black = |colour: Option<PasteboardColor>| {
        let source = DemoPages::new(2);
        let mut state = state_with(|s| s.fit = Fit::Width);
        let mut fixture = Fixture::new(move |ui| {
            let mut view = PageView::new("Pages", &source, &mut state, PAPER);
            if let Some(colour) = colour {
                view = view.pasteboard(colour);
            }
            view.show(ui, |_, _| ());
        });
        fixture.idle(3);
        fixture.frame_shapes().iter().any(|shape| match shape {
            egui::Shape::Rect(r) => r.fill == egui::Color32::BLACK,
            egui::Shape::Path(p) => p.fill == egui::Color32::BLACK,
            egui::Shape::Mesh(m) => m.vertices.iter().any(|v| v.color == egui::Color32::BLACK),
            _ => false,
        })
    };
    assert!(!paints_black(None), "the default surround is the theme's");
    assert!(paints_black(Some(PasteboardColor::Black)), "a black pasteboard paints black round the pages");
}

/// The page whose tip text (one of `expected`, the words filled for pages 1, 2, ..) is painted in the latest frame.
fn page_tip(fixture: &Fixture, expected: &[String]) -> Option<usize> {
    fixture.texts().iter().find_map(|run| expected.iter().position(|text| text.as_str() == &*run.text).map(|at| at + 1))
}

/// The tip named while the vertical thumb is dragged `fraction` of the bar's height from its top, and after the release.
fn tips_around_a_thumb_drag(hide_white_space: bool, fraction: f32) -> (Option<usize>, Option<usize>) {
    let pages = DemoPages::new(8);
    let bodies = vec![(72.0, 720.0); 8];
    let mut state = state_with(|s| s.hide_white_space = hide_white_space);
    let expected: Rc<RefCell<Vec<String>>> = Rc::default();
    let shared = Rc::clone(&expected);
    let mut fixture = Fixture::new(move |ui| {
        let env = Env::of(ui);
        let words = env.words::<PageViewWords>();
        let total = env.format().integer(8);
        *shared.borrow_mut() = (1..=8).map(|page| words.page_of.fill(&[("page", page.to_string().as_str()), ("count", total.as_str())])).collect();
        let mut view = PageView::new("Pages", &pages, &mut state, PAPER).options(LayoutOptions::ACROBAT);
        if hide_white_space {
            view = view.bodies(&bodies);
        }
        view.show(ui, |_, _| ());
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let bar =
        fixture.by_role(Role::SCROLL_BAR).into_iter().find(|bar| bar.rect.height() > bar.rect.width()).expect("the view has a vertical scroll bar");
    assert_eq!(page_tip(&fixture, &expected.borrow()), None, "no tip while no thumb drags");
    let from = pos2(bar.rect.center().x, bar.rect.min.y + 3.0);
    let mut during = None;
    fixture.drag_at_pausing(from, vec2(0.0, bar.rect.height() * fraction), |held| during = page_tip(held, &expected.borrow()));
    // The tip leaves through its exit fade; the fixture's clock moves only when told to.
    fixture.advance(1.0).settle();
    let after = page_tip(&fixture, &expected.borrow());
    (during, after)
}

#[trial]
fn a_thumb_drag_names_the_page_at_the_top_of_the_view() {
    for hide_white_space in [false, true] {
        let (early, early_after) = tips_around_a_thumb_drag(hide_white_space, 0.02);
        let (late, late_after) = tips_around_a_thumb_drag(hide_white_space, 1.0);
        let (early, late) = (early.expect("the tip shows while the thumb drags"), late.expect("the tip shows while the thumb drags"));
        assert!(late > early, "dragging farther names a later page (hide white space {hide_white_space}): {early} then {late}");
        assert!(late <= 8, "the tip names a page of the document: {late}");
        assert_eq!((early_after, late_after), (None, None), "the tip is gone after the release (hide white space {hide_white_space})");
    }
}

/// The height under the columns kept for the page-turn row, screen points.
const NAV_ROW: f32 = 60.0;

/// The rects of the columns' paper and of the row's Next button, for a reading view over `room` above a page-turn row.
fn columns_above_the_nav(room: egui::Vec2) -> (Vec<egui::Rect>, egui::Rect) {
    let tall = DemoPages::new(1).page_size(0).height as f32 * ReadingView::screen_scale(1.0);
    let pages = DemoPages::new(6);
    let mut state = ReadingState::default();
    let nav_bar = Rc::new(Cell::new(egui::Rect::NOTHING));
    let bar_seen = Rc::clone(&nav_bar);
    let mut fixture = Fixture::new(move |ui| {
        let all = ui.available_rect_before_wrap();
        let (view, bar) = all.split_top_bottom_at_y(all.max.y - NAV_ROW);
        bar_seen.set(bar);
        ReadingView::new("Document", &pages, &mut state, PAPER).show_at(ui, view);
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
            ReadingNav::new(&mut state, 6).show(ui);
        });
    });
    fixture.room(room);
    fixture.settle();
    let papers = fixture.shape_bounds().iter().copied().filter(|rect| (rect.height() - tall).abs() < 1.0).collect();
    // The view's own page-turn parts carry the same names, so the row's Next is the last button inside the row.
    let bar = nav_bar.get();
    let next = fixture.by_role(Role::BUTTON).into_iter().map(|target| target.rect).rfind(|rect| bar.contains_rect(*rect)).expect("the row has Next");
    (papers, next)
}

#[trial]
fn reading_columns_fit_above_the_nav_bar_and_the_viewport_shows_whole_columns() {
    let (narrow, narrow_next) = columns_above_the_nav(vec2(900.0, 1400.0));
    let (wide, wide_next) = columns_above_the_nav(vec2(1800.0, 1400.0));
    assert!(!narrow.is_empty(), "the columns are painted");
    for (papers, next) in [(&narrow, narrow_next), (&wide, wide_next)] {
        for paper in papers {
            assert!(paper.max.y <= next.min.y, "a column ends above the nav row: {paper:?} and {next:?}");
        }
    }
    assert!(wide.len() >= narrow.len(), "a wider room shows at least as many columns: {} then {}", narrow.len(), wide.len());
}

#[trial]
fn the_reading_view_reports_resizing_until_the_size_holds() {
    let resized = Rc::new(Cell::new(false));
    let seen = Rc::clone(&resized);
    let mut state = ReadingState::default();
    let pages = DemoPages::new(4);
    let mut fixture = Fixture::new(move |ui| {
        if ReadingView::new("Document", &pages, &mut state, PAPER).show(ui).resizing {
            seen.set(true);
        }
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    resized.set(false);
    fixture.idle(2);
    assert!(!resized.get(), "a size that holds is not resizing");
    fixture.room(vec2(700.0, 500.0));
    fixture.idle(1);
    assert!(resized.get(), "a changed size is resizing");
    resized.set(false);
    fixture.idle(3);
    assert!(!resized.get(), "resizing ends once the size holds");
}

#[trial]
fn to_doc_and_from_doc_round_trip_and_page_at_doc_takes_the_nearest_page_outside_all() {
    #[derive(Default)]
    struct Probe {
        pages: Vec<usize>,
        gap: f32,
        round_trip: f64,
        hit: Option<(usize, f64)>,
        near_first: Option<usize>,
        near_second: Option<usize>,
        above: Option<usize>,
        below: Option<usize>,
    }
    let probe = Rc::new(RefCell::new(Probe::default()));
    let kept = Rc::clone(&probe);
    let pages = DemoPages::new(3);
    let mut state = state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 0.2;
    });
    let mut fixture = Fixture::new(move |ui| {
        PageView::new("Pages", &pages, &mut state, PAPER).options(LayoutOptions::ACROBAT).show(ui, |_, body| {
            let [first, second, ..] = body.pages else { return };
            let last = &body.pages[body.pages.len() - 1];
            let at = kurbo::Point::new(100.0, 150.0);
            let doc = second.to_doc(&body.view, at);
            let back = second.from_doc(&body.view, doc);
            let gap = second.rect.min.y - first.rect.max.y;
            let x = first.rect.center().x;
            let page_at = |y: f32| body.page_at_doc(body.view.to_doc(pos2(x, y))).map(|(placed, _)| placed.page);
            *kept.borrow_mut() = Probe {
                pages: body.pages.iter().map(|placed| placed.page).collect(),
                gap,
                round_trip: (back.x - at.x).abs().max((back.y - at.y).abs()),
                hit: body.page_at_doc(doc).map(|(placed, point)| (placed.page, (point.x - at.x).abs().max((point.y - at.y).abs()))),
                near_first: page_at(first.rect.max.y + gap * 0.25),
                near_second: page_at(second.rect.min.y - gap * 0.25),
                above: page_at(first.rect.min.y - 400.0),
                below: page_at(last.rect.max.y + 400.0),
            };
        });
    });
    fixture.room(vec2(900.0, 700.0));
    fixture.settle();
    let probe = probe.borrow();
    assert!(probe.pages.len() >= 2, "two pages are in view: {:?}", probe.pages);
    assert!(probe.gap > 0.0, "pages are apart: {}", probe.gap);
    assert!(probe.round_trip < 1e-3, "to_doc then from_doc returns the page point: {}", probe.round_trip);
    let (first, second, last) = (probe.pages[0], probe.pages[1], probe.pages[probe.pages.len() - 1]);
    assert_eq!(probe.hit.map(|(page, _)| page), Some(second), "a point on a page finds that page");
    assert!(probe.hit.is_some_and(|(_, error)| error < 1e-3), "the page point is the one mapped: {:?}", probe.hit);
    assert_eq!(probe.near_first, Some(first), "between pages the nearer page: nearer the first");
    assert_eq!(probe.near_second, Some(second), "between pages the nearer page: nearer the second");
    assert_eq!(probe.above, Some(first), "above all pages: the first");
    assert_eq!(probe.below, Some(last), "below all pages: the last in view");
}

#[trial]
fn a_reading_column_turn_eases_and_lands_exactly_on_the_column_start() {
    let state = Rc::new(RefCell::new(ReadingState::default()));
    let kept = Rc::clone(&state);
    let tall = DemoPages::new(1).page_size(0).height as f32 * ReadingView::screen_scale(1.0);
    let pages = DemoPages::new(6);
    let mut fixture = Fixture::new(move |ui| {
        // Shown over the whole room, as the app shows it, so the viewport is as tall as the room.
        let all = ui.available_rect_before_wrap();
        ReadingView::new("Document", &pages, &mut kept.borrow_mut(), PAPER).show_at(ui, all);
    });
    // Tall enough that the pages' paper is not cut by the room, so each paper's bounds keep the page's height.
    fixture.room(vec2(900.0, 1400.0));
    fixture.settle();
    // The leading edge of the leftmost painted page: with the column at the start, page 0 sits there; once on column 1, page 1 does.
    let leading =
        |fixture: &Fixture| fixture.shape_bounds().iter().filter(|rect| (rect.height() - tall).abs() < 1.0).map(|rect| rect.min.x).reduce(f32::min);
    let start = leading(&fixture).expect("page 0's paper is painted");
    state.borrow_mut().turn(1, 6);
    let mut seen = Vec::new();
    for _ in 0..60 {
        fixture.idle(1);
        seen.extend(leading(&fixture));
    }
    assert!(seen.iter().filter(|x| **x < start - 0.5).count() >= 2, "the turn takes several passes, the columns part way between: {seen:?}");
    let landed = seen.last().copied().expect("the paper is painted after the turn");
    assert!((landed - start).abs() < 1e-3, "the new column lands exactly on the column start: {landed} against {start}");
}

#[trial]
fn a_view_with_an_editor_purpose_takes_no_plain_keys_for_moving() {
    /// Page Down, the arrows and End in a focused view with or without an editor purpose: the state before and after.
    fn keyed(editor: bool) -> (PageViewState, PageViewState) {
        let state = Rc::new(RefCell::new(state_with(|s| s.fit = Fit::Page)));
        let kept = Rc::clone(&state);
        let pages = DemoPages::new(5);
        let mut fixture = Fixture::new(move |ui| {
            let mut now = kept.borrow().clone();
            let mut view = PageView::new("Pages", &pages, &mut now, PAPER).options(LayoutOptions::ACROBAT);
            if editor {
                view = view.editor(EditorPurpose::Text);
            }
            view.show(ui, |_, _| ());
            *kept.borrow_mut() = now;
        });
        fixture.room(vec2(900.0, 600.0));
        fixture.settle();
        if editor {
            for _ in 0..4 {
                if fixture.focused().is_some() {
                    break;
                }
                fixture.key(Key::Tab, Modifiers::NONE);
            }
            assert!(fixture.focused().is_some(), "Tab reaches the editor view");
        } else {
            focus_canvas(&mut fixture);
        }
        let before = state.borrow().clone();
        for key in [Key::PageDown, Key::ArrowDown, Key::ArrowRight, Key::End] {
            fixture.key(key, Modifiers::NONE);
            fixture.idle(1);
        }
        let after = state.borrow().clone();
        (before, after)
    }
    let (before, after) = keyed(false);
    assert!(after.current != before.current || after.offset != before.offset, "the plain view moves on its keys");
    let (before, after) = keyed(true);
    assert_eq!((after.current, after.offset), (before.current, before.offset), "the editor view leaves moving to the app");
}

/// Where a snap line lies on page 2, in that page's own points.
const LINE_X: f64 = 120.0;

/// A tool that snaps the document point under each drag sample and keeps the last one, and counts its presses.
#[derive(Default)]
struct Snapper {
    snapped: Option<kurbo::Point>,
    presses: usize,
}

impl CanvasTool for Snapper {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.presses += 1;
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        if let Some(space) = cx.space() {
            self.snapped = Some(cx.snap(space.view.to_doc(pointer.pos)));
        }
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Snap command")
    }
}

/// A drag a few points beside the snap line at `LINE_X` on page 2, in a view of `state` with the line handed over in
/// page points: where the drag snapped to on screen, and where the line shows on page 2.
fn drag_beside_the_line_on_page_two(state: PageViewState) -> (f32, f32) {
    let tool = Rc::new(RefCell::new(Snapper::default()));
    let two: Rc<RefCell<Option<(PlacedPage, CanvasView)>>> = Rc::default();
    let (shown_tool, seen) = (Rc::clone(&tool), Rc::clone(&two));
    let pages = DemoPages::new(4);
    let mut state = state;
    let mut fixture = Fixture::new(move |ui| {
        let snap = PageSnap::new(SnapOptions::default()).page(1, vec![SnapTarget::new(Axis::Y, LINE_X, SnapKind::Guide)]);
        PageView::new("Pages", &pages, &mut state, PAPER).options(LayoutOptions::ACROBAT).snap(Some(snap)).show(ui, |ui, body| {
            if let Some(page) = body.pages.iter().find(|placed| placed.page == 1) {
                *seen.borrow_mut() = Some((page.clone(), body.view));
            }
            if let Some(pass) = body.pass {
                let _ = ToolHost::new().run(ui, pass, &mut *shown_tool.borrow_mut());
            }
        });
    });
    fixture.room(vec2(900.0, 700.0));
    fixture.settle();
    let (page, _) = two.borrow().clone().expect("page 2 shows");
    let line = page.to_screen(kurbo::Point::new(LINE_X, 200.0));
    let from = line + vec2(30.0, 0.0);
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::press(from)]);
    for step in 1..=3 {
        fixture.frame(vec![pointer::moved(from - vec2(9.0 * step as f32, 0.0))]);
    }
    let (_, view) = two.borrow().clone().expect("page 2 shows");
    let snapped = tool.borrow().snapped.expect("the tool dragged");
    (view.to_screen(snapped).x, line.x)
}

/// Guards the view's placement of page geometry: a target in page points snaps where its page shows, not at the
/// same numbers in document units, in a continuous view and in a two-up spread.
#[trial]
fn a_snap_target_on_page_two_snaps_a_drag_where_page_two_shows() {
    let continuous = state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 0.4;
        s.continuous = true;
    });
    let spread = state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 0.4;
        s.continuous = false;
        s.display = PageDisplay::TwoUp;
    });
    for (name, state) in [("continuous", continuous), ("two-up", spread)] {
        let (snapped, line) = drag_beside_the_line_on_page_two(state);
        assert!((snapped - line).abs() < 0.5, "{name}: the drag snaps onto page 2's line: {snapped} against {line}");
    }
}

/// Guards the view's placement of tool regions: a region in page points on page 1 takes a press over a canvas
/// control there.
#[trial]
fn a_tool_region_on_page_one_takes_a_press_over_a_canvas_control_there() {
    let tool = Rc::new(RefCell::new(Snapper::default()));
    let crop: Rc<RefCell<Option<CropBox>>> = Rc::default();
    let (shown_tool, shown_crop) = (Rc::clone(&tool), Rc::clone(&crop));
    let pages = DemoPages::new(2);
    let mut state = state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 0.4;
    });
    // The region covers the crop frame's top left corner, at page 1's (100, 80) points.
    let corner = vec![kurbo::Point::new(80.0, 60.0), kurbo::Point::new(120.0, 60.0), kurbo::Point::new(120.0, 100.0), kurbo::Point::new(80.0, 100.0)];
    let mut fixture = Fixture::new(move |ui| {
        let regions = vec![PageRegion::polygon(0, corner.clone())];
        PageView::new("Pages", &pages, &mut state, PAPER).options(LayoutOptions::ACROBAT).tool_regions(regions).show(ui, |ui, body| {
            let Some(first) = body.pages.iter().find(|placed| placed.page == 0) else { return };
            let mut crop = shown_crop.borrow_mut();
            let frame = crop.get_or_insert_with(|| {
                let (a, b) = (first.to_doc(&body.view, kurbo::Point::new(100.0, 80.0)), first.to_doc(&body.view, kurbo::Point::new(300.0, 220.0)));
                CropBox::upright(kurbo::Rect::from_points(a, b))
            });
            let _ = CropOverlay::new("Crop", Bind::bound(frame)).show_at(ui, body.rect);
            if let Some(pass) = body.pass {
                let _ = ToolHost::new().run(ui, pass, &mut *shown_tool.borrow_mut());
            }
        });
    });
    fixture.room(vec2(900.0, 700.0));
    fixture.settle();
    let start = crop.borrow().as_ref().map(|frame| frame.rect).expect("the crop shows on page 1");
    let at = fixture.by_label("Crop corner, top left").expect("the crop shows its corners").rect.center();
    let by = vec2(20.0, 15.0);
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::press(at)]);
    fixture.frame(vec![pointer::moved(at + by * 0.5)]);
    fixture.frame(vec![pointer::moved(at + by)]);
    fixture.frame(vec![pointer::release(at + by)]);
    assert_eq!(tool.borrow().presses, 1, "a press in the region is the tool's");
    assert_eq!(crop.borrow().as_ref().map(|frame| frame.rect), Some(start), "the corner under the region does not move");
}

/// Wheel up with the page view at the top, `rulers` on or off: the first page's top edge on screen and the state's
/// offset in every frame, after the settled frame. (#7149 item 7)
fn wheel_up_at_the_top(rulers: bool) -> Vec<(f32, f64)> {
    let state = Rc::new(RefCell::new(state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 1.0;
        s.display = PageDisplay::Columns;
        s.continuous = true;
    })));
    let top = Rc::new(Cell::new(0.0f32));
    let (kept, shown) = (Rc::clone(&state), Rc::clone(&top));
    let pages = DemoPages::new(20);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept.borrow().clone();
        let out = PageView::new("Pages", &pages, &mut now, PAPER).options(LayoutOptions::WORD).rulers(rulers).show(ui, |_, _| ());
        shown.set(out.pages.first().map_or(f32::NAN, |p| p.rect.min.y));
        *kept.borrow_mut() = now;
    });
    fixture.room(vec2(1280.0, 760.0));
    fixture.settle();
    fixture.frame(vec![egui::Event::PointerMoved(pos2(640.0, 400.0))]).settle();
    let mut seen = vec![(top.get(), state.borrow().offset.1)];
    for _ in 0..6 {
        fixture.frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, 40.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        }]);
        seen.push((top.get(), state.borrow().offset.1));
        for _ in 0..3 {
            fixture.frame(Vec::new());
            seen.push((top.get(), state.borrow().offset.1));
        }
    }
    seen
}

#[trial]
fn a_wheel_up_at_the_top_never_moves_the_page_with_rulers_on_or_off() {
    for rulers in [false, true] {
        let seen = wheel_up_at_the_top(rulers);
        let rest = seen[0];
        for (n, frame) in seen.iter().enumerate() {
            assert_eq!(*frame, rest, "rulers {rulers}: frame {n} moved the page off its top: {seen:?}");
        }
    }
}

/// Go To `fraction` of page `page` in a continuous view of 20 pages at 100 %, `rulers` on or off: the first page the
/// view places and the offset, with where the page before ends and where that fraction of the page lies.
fn gone_to(page: usize, fraction: f64, rulers: bool) -> (Option<usize>, f64, f64, f64) {
    let state = Rc::new(RefCell::new(state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 1.0;
        s.continuous = true;
    })));
    let seen = Rc::new(RefCell::new((None, 0.0, 0.0)));
    let (kept, shown) = (Rc::clone(&state), Rc::clone(&seen));
    let pages = DemoPages::new(20);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept.borrow().clone();
        let out = PageView::new("Pages", &pages, &mut now, PAPER).options(LayoutOptions::ACROBAT).rulers(rulers).show(ui, |_, _| ());
        let before = page.checked_sub(1).map_or(0.0, |before| out.layout.rects[before].y0 + out.layout.bands[before].1);
        let (top, bottom) = out.layout.bands[page];
        let at = out.layout.rects[page].y0 + top + fraction * (bottom - top);
        *shown.borrow_mut() = (out.pages.first().map(|p| p.page), before, at);
        *kept.borrow_mut() = now;
    });
    fixture.room(vec2(1280.0, 760.0));
    fixture.settle();
    state.borrow_mut().go_to = Some(GoTo::Top { page, fraction });
    fixture.idle(3);
    let (first, before, at) = *seen.borrow();
    (first, state.borrow().offset.1, before, at)
}

/// Go To puts the view's top where the page before ends, so the gap above the page shows and nothing of the page
/// before it, with rulers on or off.
#[trial]
fn go_to_shows_the_page_at_the_top_and_nothing_of_the_page_before() {
    for rulers in [false, true] {
        let (first, offset, before, _) = gone_to(5, 0.0, rulers);
        assert_eq!(first, Some(5), "rulers {rulers}: the first page in view is the one gone to");
        assert!((offset - before).abs() < 0.01, "rulers {rulers}: the view's top {offset} is where the page before ends, {before}");
    }
}

/// Go To a point inside a page puts that point at the view's top, not the gap above it.
#[trial]
fn go_to_a_point_inside_a_page_puts_it_at_the_top() {
    let (first, offset, _, at) = gone_to(5, 0.5, false);
    assert_eq!(first, Some(5), "the first page in view is the one gone to");
    assert!((offset - at).abs() < 0.01, "the view's top {offset} is the point gone to, {at}");
}

/// Lays out three pages under `set` with `start`, returning the layout and the scope's `space.gap.section` token.
fn layout_in_set(set: ComponentSet, options: LayoutOptions, start: PageViewState, bodies: Option<&'static [(f64, f64)]>) -> (PagedLayout, f32) {
    let state = Rc::new(RefCell::new(start));
    let seen: Rc<RefCell<(PagedLayout, f32)>> = Rc::default();
    let (kept, recorded) = (Rc::clone(&state), Rc::clone(&seen));
    let pages = DemoPages::new(3);
    let mut fixture = Fixture::new(move |ui| {
        let section_gap = Env::of(ui).tokens.space.gap.section;
        let mut now = kept.borrow().clone();
        let mut view = PageView::new("Pages", &pages, &mut now, PAPER).options(options);
        if let Some(bodies) = bodies {
            view = view.bodies(bodies);
        }
        let out = view.show(ui, |_, _| ());
        *recorded.borrow_mut() = (out.layout, section_gap);
        *kept.borrow_mut() = now;
    });
    fixture.set(set);
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    seen.borrow().clone()
}

/// The productivity component set gives pages a wider inter-page gap, no tool-rail side gutter under Fit Width (without
/// overflowing the viewport), and wider Hide White Space padding than the creative set, while leaving `space.gap.section`
/// untouched for other components.
#[trial]
fn the_component_set_picks_the_page_spacing_without_changing_section_gap() {
    let fixed = state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 1.0;
        s.display = PageDisplay::Columns;
    });
    let (creative, creative_section) = layout_in_set(ComponentSet::Creative, LayoutOptions::WORD, fixed.clone(), None);
    let (productivity, productivity_section) = layout_in_set(ComponentSet::Productivity, LayoutOptions::WORD, fixed, None);
    let creative_gap = creative.rects[1].y0 - creative.rects[0].y1;
    let productivity_gap = productivity.rects[1].y0 - productivity.rects[0].y1;
    assert_eq!(productivity_section, creative_section, "space.gap.section is unchanged by the component set");
    assert!(productivity_gap > creative_gap, "productivity inter-page gap ({productivity_gap}) is wider than creative ({creative_gap})");
    assert!(
        productivity.rects[0].y0 > creative.rects[0].y0,
        "productivity top margin ({}) is wider than creative ({})",
        productivity.rects[0].y0,
        creative.rects[0].y0
    );

    let fit_width = state_with(|s| {
        s.fit = Fit::Width;
        s.display = PageDisplay::Columns;
    });
    let (creative_fit, _) = layout_in_set(ComponentSet::Creative, LayoutOptions::WORD, fit_width.clone(), None);
    let (productivity_fit, _) = layout_in_set(ComponentSet::Productivity, LayoutOptions::WORD, fit_width, None);
    assert!(
        productivity_fit.rects[0].width() > creative_fit.rects[0].width(),
        "productivity Fit Width fills more of the viewport ({}) than creative with its rail gutter ({})",
        productivity_fit.rects[0].width(),
        creative_fit.rects[0].width()
    );
    assert!(
        productivity_fit.size.width <= 900.0 + 0.5,
        "productivity Fit Width stays within the viewport width without horizontal overflow: {}",
        productivity_fit.size.width
    );

    const BODIES: [(f64, f64); 3] = [(72.0, 720.0); 3];
    let hidden = state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 1.0;
        s.display = PageDisplay::Single;
        s.hide_white_space = true;
    });
    let (creative_hidden, _) = layout_in_set(ComponentSet::Creative, LayoutOptions::ACROBAT, hidden.clone(), Some(&BODIES));
    let (productivity_hidden, _) = layout_in_set(ComponentSet::Productivity, LayoutOptions::ACROBAT, hidden, Some(&BODIES));
    assert!(
        productivity_hidden.bands[1].0 < creative_hidden.bands[1].0 && productivity_hidden.bands[1].1 > creative_hidden.bands[1].1,
        "productivity Hide White Space keeps more pad around the text body: {:?} vs {:?}",
        productivity_hidden.bands[1],
        creative_hidden.bands[1]
    );
}

/// Switching the component set updates the page view's inter-page spacing on the frame of the change and settles.
#[trial]
fn a_component_set_change_updates_the_page_view_spacing() {
    let state = Rc::new(RefCell::new(state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 1.0;
        s.display = PageDisplay::Columns;
    })));
    let gap = Rc::new(Cell::new(0.0_f64));
    let (kept, seen) = (Rc::clone(&state), Rc::clone(&gap));
    let pages = DemoPages::new(3);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept.borrow().clone();
        let out = PageView::new("Pages", &pages, &mut now, PAPER).options(LayoutOptions::WORD).show(ui, |_, _| ());
        seen.set(out.layout.rects[1].y0 - out.layout.rects[0].y1);
        *kept.borrow_mut() = now;
    });
    fixture.set(ComponentSet::Creative);
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let creative_gap = gap.get();

    fixture.set(ComponentSet::Productivity);
    fixture.frame(Vec::new());
    let productivity_gap = gap.get();
    assert!(
        productivity_gap > creative_gap,
        "switching to productivity widens the page gap on the frame of the change: {productivity_gap} vs {creative_gap}"
    );
    fixture.idle(1);
    assert_eq!(gap.get(), productivity_gap, "an idle frame after the switch keeps the new gap");
}

/// `PageView::inactive_edits` forwards to its inner canvas editor: Cut resolves disabled while inactive and enabled
/// once active, and Undo/Redo fall through to app command states while inactive.
#[trial]
fn page_view_forwards_inactive_edits_to_its_editor_canvas() {
    let state = Rc::new(RefCell::new(PageViewState::default()));
    let inactive = Rc::new(Cell::new(EditSet::NONE.with(EditCommand::Cut).with(EditCommand::Undo).with(EditCommand::Redo)));
    let (kept_state, kept_inactive) = (Rc::clone(&state), Rc::clone(&inactive));
    let pages = DemoPages::new(2);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept_state.borrow().clone();
        PageView::new("Pages", &pages, &mut now, PAPER)
            .options(LayoutOptions::ACROBAT)
            .editor(EditorPurpose::Text)
            .inactive_edits(kept_inactive.get())
            .show(ui, |_, _| ());
        *kept_state.borrow_mut() = now;
    });
    fixture.command_state(EditCommands::Cut, CommandState::new().enabled(true));
    fixture.command_state(EditCommands::Undo, CommandState::new().enabled(false));
    fixture.command_state(EditCommands::Redo, CommandState::new().enabled(false));
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();

    let canvas_rect = fixture.by_role(Role::CANVAS).into_iter().next().expect("a canvas").rect;
    fixture.click_at(canvas_rect.center());
    fixture.settle();
    assert!(fixture.focused().is_some(), "clicking the page view focuses its editor canvas");

    assert!(!resolve(fixture.ctx(), EditCommands::Cut).enabled, "inactive Cut resolves disabled on focused PageView");
    assert!(!resolve(fixture.ctx(), EditCommands::Undo).enabled, "inactive Undo falls through to disabled app state");
    assert!(!resolve(fixture.ctx(), EditCommands::Redo).enabled, "inactive Redo falls through to disabled app state");

    fixture.command_state(EditCommands::Undo, CommandState::new().enabled(true));
    fixture.command_state(EditCommands::Redo, CommandState::new().enabled(true));
    fixture.settle();
    assert!(resolve(fixture.ctx(), EditCommands::Undo).enabled, "inactive Undo follows enabled app state");
    assert!(resolve(fixture.ctx(), EditCommands::Redo).enabled, "inactive Redo follows enabled app state");

    inactive.set(EditSet::NONE.with(EditCommand::Undo).with(EditCommand::Redo));
    fixture.settle();
    assert!(resolve(fixture.ctx(), EditCommands::Cut).enabled, "active Cut resolves enabled on focused PageView");
}

/// The layout of three demo pages in a 900 × 600 room of the creative set, at 100 % in one column, with the app's
/// `PageInsets` when `insets` names one, and the tokens' roomy pad and theme page margin.
fn creative_layout(insets: Option<craft_pages::ui::PageInsets>) -> (PagedLayout, f64, f64) {
    let seen: Rc<RefCell<(PagedLayout, f64, f64)>> = Rc::default();
    let recorded = Rc::clone(&seen);
    let pages = DemoPages::new(3);
    let mut state = state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 1.0;
    });
    let mut fixture = Fixture::new(move |ui| {
        let tokens = Env::of(ui).tokens.clone();
        let out = PageView::new("Pages", &pages, &mut state, PAPER).options(LayoutOptions::ACROBAT).show(ui, |_, _| ());
        *recorded.borrow_mut() = (out.layout, f64::from(tokens.space.pad.roomy), f64::from(tokens.space.inset.page_margin));
    });
    fixture.set(ComponentSet::Creative);
    if let Some(insets) = insets {
        fixture.root_value(insets);
    }
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    seen.borrow().clone()
}

/// Upstream's creative spacing by default: the first page sits the roomy pad down; with `PageInsets::Theme`
/// (PdfCraft's page view) it sits the theme's page margin down.
#[trial]
fn the_creative_page_spacing_is_upstreams_by_default_and_the_themes_insets_when_asked() {
    let (standard, roomy, _) = creative_layout(None);
    assert!((standard.rects[0].y0 - roomy).abs() < 0.5, "the first page sits the roomy pad ({roomy}) down: {}", standard.rects[0].y0);
    let (themed, _, page_margin) = creative_layout(Some(craft_pages::ui::PageInsets::Theme));
    assert!(
        (themed.rects[0].y0 - page_margin).abs() < 0.5,
        "the first page sits the theme's page margin ({page_margin}) down: {}",
        themed.rects[0].y0
    );
}

/// The view's offset after a Go To a quarter into page 2 of five, continuous at 100 %, with Go To placed as `place`
/// says; and the page's top and the layout's page gap.
fn go_to_quarter(place: craft_pages::ui::GoToPlace) -> (f64, f64, f64) {
    let state = Rc::new(RefCell::new(state_with(|s| {
        s.fit = Fit::None;
        s.zoom = 1.0;
    })));
    let seen: Rc<RefCell<Option<PagedLayout>>> = Rc::default();
    let (kept, recorded) = (Rc::clone(&state), Rc::clone(&seen));
    let pages = DemoPages::new(5);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept.borrow().clone();
        let out = PageView::new("Pages", &pages, &mut now, PAPER).options(LayoutOptions::ACROBAT).go_to_place(place).show(ui, |_, _| ());
        *recorded.borrow_mut() = Some(out.layout);
        *kept.borrow_mut() = now;
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    state.borrow_mut().go_to_fraction(2, 0.25);
    fixture.settle();
    let layout = seen.borrow().clone().expect("a pass ran");
    let top = layout.rects[2].y0 + 0.25 * layout.rects[2].height();
    let gap = layout.rects[2].y0 - layout.rects[1].y1;
    let offset = state.borrow().offset.1;
    (offset, top, gap)
}

/// By default Go To puts the quarter of the page at the view's top; with `GoToPlace::GapAbove` (production's place)
/// the page gap stands above it.
#[trial]
fn go_to_puts_the_point_at_the_top_by_default_and_a_gap_above_it_when_asked() {
    let (offset, top, _) = go_to_quarter(craft_pages::ui::GoToPlace::AfterRowAbove);
    assert!((offset - top).abs() < 0.5, "the quarter is at the view's top: offset {offset}, point {top}");
    let (offset, top, gap) = go_to_quarter(craft_pages::ui::GoToPlace::GapAbove);
    assert!(gap > 0.0, "the layout keeps a page gap");
    assert!((offset - (top - gap)).abs() < 0.5, "the page gap ({gap}) stands above the quarter: offset {offset}, point {top}");
}

/// The page rects of `count` demo pages shown one row at a time at Fit Page in `display`, with `fill_screen`, and the
/// view's rect.
fn filled(display: PageDisplay, count: usize, fill: bool) -> (Vec<kurbo::Rect>, (f64, f64), egui::Rect) {
    let state = Rc::new(RefCell::new(state_with(|s| {
        s.fit = Fit::Page;
        s.continuous = false;
        s.display = display;
    })));
    let seen = Rc::new(RefCell::new((Vec::<kurbo::Rect>::new(), egui::Rect::NOTHING)));
    let (kept, recorded) = (Rc::clone(&state), Rc::clone(&seen));
    let pages = DemoPages::new(count);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept.borrow().clone();
        let out = PageView::new("Pages", &pages, &mut now, PAPER).options(LayoutOptions::ACROBAT).fill_screen(fill).show(ui, |_, _| ());
        *recorded.borrow_mut() = (out.layout.rects.clone(), out.canvas.response.rect);
        *kept.borrow_mut() = now;
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    let (rects, view) = seen.borrow().clone();
    let offset = state.borrow().offset;
    (rects, offset, view)
}

/// By default a page shown alone at Fit Page keeps the view's margin above and below it; with `fill_screen` (full
/// screen's edge-to-edge view) it spans the view's height, its top at the view's top, centred across.
#[trial]
fn full_screen_fills_the_screen_edge_to_edge() {
    let (rects, offset, view) = filled(PageDisplay::Single, 3, false);
    let height = f64::from(view.height());
    assert!(rects[0].height() < height - 1.0, "a margin stays above and below the page: {} in {height}", rects[0].height());
    let (rects, offset_filled, view) = filled(PageDisplay::Single, 3, true);
    assert!((rects[0].height() - height).abs() < 1.0, "the page spans the view's height: {} in {height}", rects[0].height());
    assert!((rects[0].y0 - offset_filled.1).abs() < 1.0, "the page's top is the view's top: {} at offset {}", rects[0].y0, offset_filled.1);
    let centre = offset_filled.0 + f64::from(view.width()) / 2.0;
    assert!((rects[0].center().x - centre).abs() < 1.0, "the page is centred across: {} against {centre}", rects[0].center().x);
    let _ = offset;
}

/// Filling the screen in Two-up one spread at a time, the spread fits by its two pages, side by side and centred.
#[trial]
fn full_screen_two_page_view_fits_the_spread_it_shows() {
    let (rects, offset, view) = filled(PageDisplay::TwoUp, 4, true);
    let spread = rects[0].union(rects[1]);
    let (width, height) = (f64::from(view.width()), f64::from(view.height()));
    assert!(spread.width() <= width + 1.0 && spread.height() <= height + 1.0, "the spread fits the view: {spread:?} in {width} × {height}");
    assert!(
        (spread.width() - width).abs() < 1.0 || (spread.height() - height).abs() < 1.0,
        "the spread meets the view on one axis: {spread:?} in {width} × {height}"
    );
    let centre = offset.0 + width / 2.0;
    assert!((spread.center().x - centre).abs() < 1.0, "the spread is centred across: {} against {centre}", spread.center().x);
}

/// Full screen's edge-to-edge view (`fill_screen`) has no scroll bars, so a press along the window's edges reaches the
/// pages; by default the bars show.
#[trial]
fn full_screen_shows_no_scroll_bars() {
    let bars = |fill: bool| {
        let state = Rc::new(RefCell::new(state_with(|s| s.fit = Fit::Width)));
        let pages = DemoPages::new(3);
        let mut fixture = Fixture::new(move |ui| {
            let mut now = state.borrow().clone();
            let _ = PageView::new("Pages", &pages, &mut now, PAPER).options(LayoutOptions::ACROBAT).fill_screen(fill).show(ui, |_, _| ());
            *state.borrow_mut() = now;
        });
        fixture.room(vec2(900.0, 600.0));
        fixture.settle();
        fixture.by_label("Vertical scroll bar").is_some()
    };
    assert!(bars(false), "the view shows its scroll bars by default");
    assert!(!bars(true), "full screen shows none");
}
