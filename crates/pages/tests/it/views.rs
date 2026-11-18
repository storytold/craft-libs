//! The page view and the page preview: the scroll anchor under a zoom change, wheel input at the end of the document,
//! the view's page and arrow keys and its zoom commands, and the preview's keys.

use craft_behavior::kurbo;
use craft_pages::layout::{Fit, LayoutOptions};
use craft_pages::ui::fixtures::{DemoPages, PAPER};
use craft_pages::ui::{PagePreview, PageSource, PageView, PageViewState, PageViewWords, ReadingNav, ReadingState, ReadingView};
use craft_ui::contract::{Role, Show as _};

use craft_ui::primitives::EditorPurpose;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
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
    let bar = fixture
        .by_role(Role::SCROLL_BAR)
        .into_iter()
        .find(|bar| bar.rect.height() > bar.rect.width())
        .expect("the view has a vertical scroll bar");
    assert_eq!(page_tip(&fixture, &expected.borrow()), None, "no tip while no thumb drags");
    let from = pos2(bar.rect.center().x, bar.rect.min.y + 3.0);
    let mut during = None;
    fixture.drag_at_pausing(from, vec2(0.0, bar.rect.height() * fraction), |held| during = page_tip(held, &expected.borrow()));
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
    let mut fixture = Fixture::new(move |ui| {
        let all = ui.available_rect_before_wrap();
        let (view, bar) = all.split_top_bottom_at_y(all.max.y - NAV_ROW);
        ReadingView::new("Document", &pages, &mut state, PAPER).show_at(ui, view);
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
            ReadingNav::new(&mut state, 6).show(ui);
        });
    });
    fixture.room(room);
    fixture.settle();
    let papers = fixture.shape_bounds().iter().copied().filter(|rect| (rect.height() - tall).abs() < 1.0).collect();
    let next = fixture.by_label("Next Screen").expect("the row has Next").rect;
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
        ReadingView::new("Document", &pages, &mut kept.borrow_mut(), PAPER).show(ui);
    });
    // Tall enough that the pages' paper is not cut by the room, so each paper's bounds keep the page's height.
    fixture.room(vec2(900.0, 1400.0));
    fixture.settle();
    // The leading edge of the leftmost painted page: with the column at the start, page 0 sits there; once on column 1, page 1 does.
    let leading = |fixture: &Fixture| {
        fixture.shape_bounds().iter().filter(|rect| (rect.height() - tall).abs() < 1.0).map(|rect| rect.min.x).reduce(f32::min)
    };
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
