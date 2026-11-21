//! The page view and the page preview: the scroll anchor under a zoom change, wheel input at the end of the document,
//! the view's page and arrow keys and its zoom commands, and the preview's keys.

use craft_behavior::geometry::{CropBox, SnapKind, SnapOptions, SnapTarget};
use craft_behavior::kurbo;
use craft_behavior::{Axis, CancelReason, CanvasView, EditCommand};
use craft_pages::layout::{Fit, LayoutOptions, PageDisplay};
use craft_pages::ui::fixtures::{DemoPages, PAPER};
use craft_pages::ui::{
    PagePreview, PageRegion, PageSnap, PageSource, PageView, PageViewState, PageViewWords, PlacedPage, ReadingNav, ReadingState, ReadingView,
};
use craft_ui::canvas::{CanvasTool, CropOverlay, ToolAction, ToolContext, ToolHost, ToolPath, ToolPointer};
use craft_ui::contract::{Bind, Role, Show as _};

use craft_ui::primitives::textures::TextureStoreOptions;
use craft_ui::primitives::{Draw, EditorPurpose};
use craft_ui::runtime::commands::{CommandState, EditCommands, resolve};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::focus::EditSet;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Key, Modifiers, pos2, vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The default state with `change` applied.
fn state_with(change: impl FnOnce(&mut PageViewState)) -> PageViewState {
    let mut state = PageViewState::default();
    change(&mut state);
    state
}

type LayoutRig = (Fixture, Rc<RefCell<PageViewState>>, Rc<RefCell<Vec<kurbo::Rect>>>);

/// A page view over `count` demo pages with the state it starts from, and the rects of the pages it laid out.
fn layout_rig(count: usize, start: PageViewState) -> LayoutRig {
    let state = Rc::new(RefCell::new(start));
    let rects: Rc<RefCell<Vec<kurbo::Rect>>> = Rc::default();
    let (kept, seen) = (Rc::clone(&state), Rc::clone(&rects));
    let pages = DemoPages::new(count);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = kept.borrow().clone();
        let out = PageView::new("Pages", &pages, &mut now, PAPER).options(LayoutOptions::ACROBAT).show(ui, |_, _| ());
        seen.borrow_mut().clone_from(&out.layout.rects);
        *kept.borrow_mut() = now;
    });
    fixture.room(vec2(900.0, 600.0));
    fixture.settle();
    (fixture, state, rects)
}

#[trial]
fn a_changed_zoom_keeps_the_page_at_the_top_in_place() {
    let (mut fixture, state, rects) = layout_rig(
        5,
        state_with(|s| {
            s.fit = Fit::None;
            s.zoom = 1.0;
        }),
    );
    let third = rects.borrow()[2];
    state.borrow_mut().offset.1 = third.y0 + 0.4 * third.height();
    fixture.idle(2);
    state.borrow_mut().zoom = 2.0;
    fixture.idle(3);
    let third = rects.borrow()[2];
    let offset = state.borrow().offset.1;
    assert!((offset - (third.y0 + 0.4 * third.height())).abs() < 1.0, "offset {offset}, page {third:?}");
}

#[trial]
fn the_pictures_a_view_holds_stay_bounded_as_it_scrolls_through_many_pages() {
    let budget = TextureStoreOptions::pages().budget.max_entries;
    let count = 4 * budget;
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
fn wheel_gathered_past_the_bottom_is_forgotten_by_a_pass_without_wheel_input() {
    let (mut fixture, state, _) = layout_rig(
        4,
        state_with(|s| {
            s.continuous = false;
            s.fit = Fit::None;
            s.zoom = 2.0;
        }),
    );
    state.borrow_mut().offset.1 = 1e9;
    fixture.idle(2);
    wheel(&mut fixture, -0.5);
    fixture.idle(1);
    wheel(&mut fixture, -0.5);
    fixture.idle(1);
    assert_eq!(state.borrow().current, 0, "each half notch alone is not a turn");
    wheel(&mut fixture, -3.0);
    assert_eq!(state.borrow().current, 1, "a whole notch at once is");
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
    let (mut fixture, state, _) = layout_rig(5, state_with(|s| s.fit = Fit::Page));
    focus_canvas(&mut fixture);
    fixture.key(Key::ArrowRight, Modifiers::NONE);
    fixture.idle(1);
    assert_eq!(state.borrow().current, 1, "Right goes to the next page while the page fits the width");
    let before = state.borrow().offset.1;
    fixture.key(Key::ArrowDown, Modifiers::NONE);
    fixture.idle(1);
    assert!(state.borrow().offset.1 > before, "Down scrolls forward");
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
            let last = body.pages[body.pages.len() - 1];
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
    let two: Rc<Cell<Option<(PlacedPage, CanvasView)>>> = Rc::default();
    let (shown_tool, seen) = (Rc::clone(&tool), Rc::clone(&two));
    let pages = DemoPages::new(4);
    let mut state = state;
    let mut fixture = Fixture::new(move |ui| {
        let snap = PageSnap::new(SnapOptions::default()).page(1, vec![SnapTarget::new(Axis::Y, LINE_X, SnapKind::Guide)]);
        PageView::new("Pages", &pages, &mut state, PAPER).options(LayoutOptions::ACROBAT).snap(Some(snap)).show(ui, |ui, body| {
            if let Some(page) = body.pages.iter().find(|placed| placed.page == 1) {
                seen.set(Some((*page, body.view)));
            }
            if let Some(pass) = body.pass {
                let _ = ToolHost::new().run(ui, pass, &mut *shown_tool.borrow_mut());
            }
        });
    });
    fixture.room(vec2(900.0, 700.0));
    fixture.settle();
    let (page, _) = two.get().expect("page 2 shows");
    let line = page.to_screen(kurbo::Point::new(LINE_X, 200.0));
    let from = line + vec2(30.0, 0.0);
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::press(from)]);
    for step in 1..=3 {
        fixture.frame(vec![pointer::moved(from - vec2(9.0 * step as f32, 0.0))]);
    }
    let (_, view) = two.get().expect("page 2 shows");
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
    state.borrow_mut().go_to = Some((page, fraction));
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
