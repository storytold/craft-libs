//! Behavior tests for the pane host, driven by egui pointer and key events in an 800 × 600
//! window. The harness's root ui has an 8 pt margin, so the host spans x 8..792, y 8..592.

use std::collections::HashMap;

use egui::{Event, Id, Key, Modifiers, PointerButton, Pos2, Rect, Ui, pos2, vec2};
use egui_kittest::{Harness, kittest::NodeT, kittest::Queryable};

use super::*;

const L: f32 = 8.0;
const R: f32 = 792.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pane {
    Nav,
    Styles,
    Comments,
    /// A context pane, like PivotTable Fields: not closable, not movable.
    Pivot,
}

struct Content;

impl PaneContent<Pane> for Content {
    fn title(&self, pane: &Pane) -> String {
        format!("{pane:?}")
    }
    fn body(&mut self, ui: &mut Ui, pane: &Pane) {
        ui.label(format!("{pane:?} body"));
    }
    /// Default docked pane state: end for Comments and Pivot, start otherwise,
    /// with a 250 pt size.
    fn default_state(&self, pane: &Pane) -> PaneState {
        let edge = if matches!(pane, Pane::Comments | Pane::Pivot) { Edge::End } else { Edge::Start };
        PaneState::docked(edge, 250.0)
    }
    fn movable(&self, pane: &Pane) -> bool {
        *pane != Pane::Pivot
    }
    fn closable(&self, pane: &Pane) -> bool {
        *pane != Pane::Pivot
    }
}

/// [`Content`] with the title bar on or off.
struct Titled(bool);

impl PaneContent<Pane> for Titled {
    fn title(&self, pane: &Pane) -> String {
        Content.title(pane)
    }
    fn body(&mut self, ui: &mut Ui, pane: &Pane) {
        Content.body(ui, pane);
    }
    fn default_state(&self, pane: &Pane) -> PaneState {
        Content.default_state(pane)
    }
    fn movable(&self, pane: &Pane) -> bool {
        Content.movable(pane)
    }
    fn closable(&self, pane: &Pane) -> bool {
        Content.closable(pane)
    }
    fn title_bar(&self, _: &Pane) -> bool {
        self.0
    }
}

/// The app side: open list, persisted states and every request it received.
struct App {
    open: Vec<Pane>,
    states: HashMap<Pane, PaneState>,
    log: Vec<PaneRequest<Pane>>,
    focused: Option<Pane>,
    to_document: usize,
    rtl: bool,
    policy: SidePolicy,
    title_bar: bool,
    /// The document is one drag-sensing widget, like a canvas.
    drag_document: bool,
    document_drags: usize,
}

impl App {
    /// Create an app with `open` panes, empty state and log, no focused pane, no
    /// document-focus requests, left-to-right layout and column side policy.
    fn new(open: &[Pane]) -> Self {
        Self {
            open: open.to_vec(),
            states: HashMap::new(),
            log: Vec::new(),
            focused: None,
            to_document: 0,
            rtl: false,
            policy: SidePolicy::Columns,
            title_bar: true,
            drag_document: false,
            document_drags: 0,
        }
    }

    /// Render `PaneHost` with the app's open panes, policy and RTL flag, then a central document
    /// panel. Applies pane requests to stored state and records requests, focused pane and
    /// document-focus transitions.
    fn frame(&mut self, ui: &mut Ui) {
        let states = self.states.clone();
        let out = PaneHost::new(Id::unique("panes")).rtl(self.rtl).policy(self.policy).show(
            ui,
            &self.open,
            |p| states.get(p).copied(),
            &mut Titled(self.title_bar),
        );
        let drag_document = self.drag_document;
        // A canvas fills the panel, without the default frame's margin.
        let panel = if drag_document { egui::CentralPanel::default().frame(Frame::NONE) } else { egui::CentralPanel::default() };
        let dragged = panel
            .show(ui, |ui| {
                if drag_document {
                    ui.interact(ui.max_rect(), Id::unique("document-drag"), Sense::click_and_drag()).dragged()
                } else {
                    ui.label("Document").dragged()
                }
            })
            .inner;
        self.document_drags += usize::from(dragged);
        for request in &out.requests {
            match request {
                PaneRequest::Close(p) => self.open.retain(|o| o != p),
                PaneRequest::Focus(_) => {}
                PaneRequest::Before(p, before) => {
                    self.open.retain(|o| o != p);
                    let at = before.and_then(|b| self.open.iter().position(|o| *o == b)).unwrap_or(self.open.len());
                    self.open.insert(at, *p);
                }
                other => {
                    let pane = *other.pane();
                    let state = self.states.entry(pane).or_insert_with(|| Content.default_state(&pane));
                    state.apply(other);
                }
            }
        }
        self.log.extend(out.requests);
        self.focused = out.focused;
        self.to_document += usize::from(out.to_document);
    }

    fn place(&self, pane: Pane) -> Place {
        self.states.get(&pane).map_or_else(|| Content.default_state(&pane).place, |s| s.place)
    }
}

/// Build an 800×600 harness around `App::frame`, mark input focused, and run
/// two frames before returning it.
fn harness(app: App) -> Harness<'static, App> {
    let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(|ui, app: &mut App| app.frame(ui), app);
    h.input_mut().focused = true;
    h.run_steps(2);
    h
}

/// The stable response id for `pane` under the `panes` host scope.
fn pane_id(pane: Pane) -> Id {
    Id::unique("panes").with(("pane", &pane))
}

fn rect(h: &Harness<'_, App>, id: Id) -> Option<Rect> {
    h.ctx.read_response(id).map(|r| r.rect)
}

fn title(h: &Harness<'_, App>, pane: Pane) -> Rect {
    rect(h, pane_id(pane).with("title")).unwrap()
}

/// The x of a side pane's inner edge.
fn edge(h: &Harness<'_, App>, pane: Pane) -> f32 {
    rect(h, pane_id(pane).with("resize")).unwrap().center().x
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn send(h: &mut Harness<'_, App>, events: Vec<Event>) {
    h.input_mut().events.extend(events);
    h.run_steps(1);
}

fn click(h: &mut Harness<'_, App>, at: Pos2) {
    send(h, vec![Event::PointerMoved(at), button(at, true)]);
    send(h, vec![button(at, false)]);
    h.run_steps(1);
}

fn hold(h: &mut Harness<'_, App>, from: Pos2, to: Pos2) {
    send(h, vec![Event::PointerMoved(from), button(from, true)]);
    send(h, vec![Event::PointerMoved(from + vec2(8.0, 6.0))]);
    send(h, vec![Event::PointerMoved(to)]);
}

fn drag(h: &mut Harness<'_, App>, from: Pos2, to: Pos2) {
    hold(h, from, to);
    send(h, vec![button(to, false)]);
    h.run_steps(2);
}

/// The rect a docked pane's panel takes. egui 0.37 keys a panel's persisted `PanelState` by the
/// scope of the `Ui` it is shown in and the panel's id salt, not by the id alone.
fn panel(h: &Harness<'_, App>, pane: Pane) -> Rect {
    egui::containers::panel::PanelState::load(&h.ctx, h.ui_id().with(pane_id(pane).with("panel"))).unwrap().outer_rect
}

/// The drop previews painted in the last frame: 2 pt accent outlines with a 15% accent fill.
fn previews(h: &Harness<'_, App>) -> Vec<Rect> {
    h.output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(shape) if shape.stroke.width == 2.0 && shape.fill == shape.stroke.color.gamma_multiply(0.15) => {
                Some(shape.rect.expand(1.0))
            }
            _ => None,
        })
        .collect()
}

fn key(h: &mut Harness<'_, App>, key: Key, modifiers: Modifiers) {
    send(h, vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
    h.run_steps(1);
}

/// The requests that change a pane's state or open list.
fn changes(app: &App) -> Vec<PaneRequest<Pane>> {
    app.log.iter().filter(|r| !matches!(r, PaneRequest::Focus(_))).cloned().collect()
}

fn last_resize(app: &App, pane: Pane) -> Option<f32> {
    app.log.iter().rev().find_map(|r| match r {
        PaneRequest::Resize(p, size) if *p == pane => Some(*size),
        _ => None,
    })
}

#[test]
fn a_docked_pane_without_a_title_bar_fills_the_pane_and_still_resizes() {
    for title_bar in [true, false] {
        let mut app = App::new(&[Pane::Nav]);
        app.title_bar = title_bar;
        let mut h = harness(app);
        let chrome = [
            rect(&h, pane_id(Pane::Nav).with("title")).is_some(),
            h.query_by_label("Move Nav pane").is_some(),
            h.query_by_label("Pane options").is_some(),
            h.query_by_label("Close").is_some(),
        ];
        assert_eq!(chrome, [title_bar; 4], "title bar {title_bar}: title, its node, ▾ and ×");
        // The body's first line sits under the body margin, below the title bar when there is one.
        let body = h.get_by_label("Nav body").rect();
        let top = if title_bar { L + TITLE_BAR_HEIGHT + 8.0 } else { L + 8.0 };
        assert!((body.top() - top).abs() < 2.0, "title bar {title_bar}: body {body:?}");

        // Without the bar, the body is the pane's accessible region, named by its title.
        let regions = h.query_all_by_label("Nav").filter(|n| n.accesskit_node().role() == egui::accesskit::Role::Pane).count();
        assert_eq!(regions, usize::from(!title_bar), "title bar {title_bar}");
        assert_eq!(h.get_by_label("Resize pane").accesskit_node().role(), egui::accesskit::Role::Splitter);

        drag(&mut h, pos2(L + 250.0, 300.0), pos2(L + 300.0, 300.0));
        assert_eq!(last_resize(h.state(), Pane::Nav), Some(300.0), "title bar {title_bar}");
        assert_eq!(edge(&h, Pane::Nav), L + 300.0);
    }
}

#[test]
fn f6_reaches_a_pane_without_a_title_bar() {
    let mut app = App::new(&[Pane::Nav, Pane::Comments]);
    app.title_bar = false;
    let mut h = harness(app);
    key(&mut h, Key::F6, Modifiers::NONE);
    assert_eq!(h.state().focused, Some(Pane::Nav));
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(pane_id(Pane::Nav).with("body")), "F6 focuses the body");
    key(&mut h, Key::F6, Modifiers::NONE);
    assert_eq!(h.state().focused, Some(Pane::Comments));
    key(&mut h, Key::F6, Modifiers::NONE);
    assert_eq!((h.state().focused, h.state().to_document), (None, 1));
}

#[test]
fn the_resize_edge_works_on_both_sides_of_the_line_over_a_dragging_document() {
    for offset in [-3.5, 3.5] {
        let mut app = App::new(&[Pane::Nav]);
        app.drag_document = true;
        let mut h = harness(app);
        // The line is at L + 250; positive offsets lie over the document.
        drag(&mut h, pos2(L + 250.0 + offset, 300.0), pos2(L + 300.0 + offset, 300.0));
        // The edge follows the pointer.
        let size = last_resize(h.state(), Pane::Nav);
        assert!(size.is_some_and(|size| (size - (300.0 + offset)).abs() <= 1.0), "offset {offset}: {size:?}");
        assert_eq!(h.state().document_drags, 0, "offset {offset}: the document took the drag");
    }
    // A floating pane over the edge keeps its presses: the edge stays under floating windows.
    let mut app = App::new(&[Pane::Nav, Pane::Styles]);
    app.states.insert(Pane::Styles, PaneState { place: Place::Floating, size: None, float_rect: Some([L + 150.0, 100.0, 220.0, 300.0]) });
    let mut h = harness(app);
    h.run_steps(2);
    drag(&mut h, pos2(L + 250.0, 300.0), pos2(L + 300.0, 300.0));
    assert_eq!(last_resize(h.state(), Pane::Nav), None, "the edge under a floating pane took the drag");

    // Away from the edge, the document still drags.
    let mut app = App::new(&[Pane::Nav]);
    app.drag_document = true;
    let mut h = harness(app);
    drag(&mut h, pos2(L + 400.0, 300.0), pos2(L + 450.0, 300.0));
    assert!(h.state().document_drags > 0);
    assert_eq!(last_resize(h.state(), Pane::Nav), None);
}

#[test]
fn columns_sit_side_by_side_resize_and_move_to_end() {
    let mut h = harness(App::new(&[Pane::Nav, Pane::Styles]));
    assert_eq!(edge(&h, Pane::Nav), L + 250.0);
    assert_eq!(edge(&h, Pane::Styles), L + 500.0);
    assert!(title(&h, Pane::Nav).left() < L + 2.0);

    drag(&mut h, pos2(L + 250.0, 300.0), pos2(L + 300.0, 300.0));
    assert_eq!(last_resize(h.state(), Pane::Nav), Some(300.0));
    assert_eq!(edge(&h, Pane::Nav), L + 300.0);
    assert_eq!(edge(&h, Pane::Styles), L + 550.0, "the second column follows");

    let at = rect(&h, pane_id(Pane::Nav).with("title").with("menu")).unwrap().center();
    click(&mut h, at);
    for label in ["Move to Start", "Move to End", "Float", "Close"] {
        assert!(h.query_all_by_label(label).count() >= 1, "{label}");
    }
    h.get_by_label("Move to End").click();
    h.run_steps(3);
    assert!(h.state().log.contains(&PaneRequest::Place(Pane::Nav, Place::Docked(Edge::End))));
    assert_eq!(edge(&h, Pane::Nav), R - 300.0, "a 300 pt pane at the right edge");
    assert_eq!(edge(&h, Pane::Styles), L + 250.0);
}

#[test]
fn a_bottom_pane_dragged_to_a_side_previews_the_width_it_docks_at() {
    let mut app = App::new(&[Pane::Comments]);
    app.states.insert(Pane::Comments, PaneState::docked(Edge::Bottom, 300.0));
    let mut h = harness(app);
    let from = title(&h, Pane::Comments).center();
    let to = pos2(L + 10.0, 200.0);
    hold(&mut h, from, to);
    let previewed = h.output().shapes.iter().any(|clipped| match &clipped.shape {
        egui::Shape::Rect(shape) => shape.rect.left() == L + 1.0 && shape.rect.width() == 298.0,
        _ => false,
    });
    assert!(previewed, "a 300 pt side pane");
    send(&mut h, vec![button(to, false)]);
    h.run_steps(2);
    assert_eq!(h.state().place(Pane::Comments), Place::Docked(Edge::Start));
    assert_eq!(edge(&h, Pane::Comments), L + 300.0);
}

#[test]
fn title_dragged_out_floats_and_a_drop_near_an_edge_docks() {
    // Drag-out-to-float and the 32 pt edge snap follow wordcraft#2584.
    let mut h = harness(App::new(&[Pane::Styles]));
    let from = title(&h, Pane::Styles).center();
    // Within DOCK_SNAP of the pane: nothing happens.
    drag(&mut h, from, pos2(L + 270.0, 300.0));
    assert!(changes(h.state()).is_empty(), "{:?}", h.state().log);

    drag(&mut h, from, pos2(400.0, 300.0));
    assert_eq!(
        changes(h.state()),
        vec![PaneRequest::Place(Pane::Styles, Place::Floating), PaneRequest::FloatRect(Pane::Styles, [360.0, 112.0, 270.0, 480.0])],
        "floats under the pointer, clamped to the window"
    );
    assert_eq!(title(&h, Pane::Styles).min, pos2(360.0, 116.0));

    // Dropped within 32 pt of the right edge: docked at End.
    h.state_mut().log.clear();
    let from = title(&h, Pane::Styles).center();
    drag(&mut h, from, pos2(780.0, 300.0));
    assert_eq!(h.state().log.last(), Some(&PaneRequest::Place(Pane::Styles, Place::Docked(Edge::End))));
    assert_eq!(edge(&h, Pane::Styles), R - 250.0);
    // The floating rect's size is kept for the next float.
    assert_eq!(h.state().states[&Pane::Styles].float_rect.map(|r| [r[2], r[3]]), Some([270.0, 480.0]));
}

#[test]
fn a_floating_pane_near_a_side_edge_previews_the_column_it_docks_in() {
    let mut app = App::new(&[Pane::Nav, Pane::Styles]);
    app.states.insert(Pane::Styles, PaneState { place: Place::Floating, size: Some(300.0), float_rect: Some([300.0, 100.0, 270.0, 400.0]) });
    let mut h = harness(app);
    let from = title(&h, Pane::Styles).center();
    hold(&mut h, from, pos2(400.0, 300.0));
    assert!(previews(&h).is_empty(), "away from the edges");
    let to = pos2(L + 10.0, 300.0);
    send(&mut h, vec![Event::PointerMoved(to)]);
    let landing = Rect::from_x_y_ranges(L..=L + 300.0, 8.0..=592.0);
    assert_eq!(previews(&h), vec![landing], "outside Nav, 300 pt wide");
    send(&mut h, vec![button(to, false)]);
    h.run_steps(2);
    assert_eq!(h.state().place(Pane::Styles), Place::Docked(Edge::Start));
    assert_eq!(panel(&h, Pane::Styles), landing);
    assert_eq!(edge(&h, Pane::Nav), L + 550.0);
}

#[test]
fn a_docked_pane_dragged_across_previews_the_column_it_lands_in() {
    let mut h = harness(App::new(&[Pane::Nav, Pane::Comments]));
    let from = title(&h, Pane::Comments).center();
    let to = pos2(L + 10.0, 300.0);
    hold(&mut h, from, to);
    let landing = Rect::from_x_y_ranges(L..=L + 250.0, 8.0..=592.0);
    assert_eq!(previews(&h), vec![landing], "the window edge docks outermost");
    send(&mut h, vec![button(to, false)]);
    h.run_steps(2);
    assert_eq!(
        changes(h.state()),
        vec![PaneRequest::Place(Pane::Comments, Place::Docked(Edge::Start)), PaneRequest::Before(Pane::Comments, Some(Pane::Nav))]
    );
    assert_eq!(panel(&h, Pane::Comments), landing);
    assert_eq!(panel(&h, Pane::Nav), Rect::from_x_y_ranges(L + 250.0..=L + 500.0, 8.0..=592.0));

    // Right to left: Start is the right edge.
    let mut app = App::new(&[Pane::Nav, Pane::Comments]);
    app.rtl = true;
    let mut h = harness(app);
    let from = title(&h, Pane::Comments).center();
    let to = pos2(R - 10.0, 300.0);
    hold(&mut h, from, to);
    let landing = Rect::from_x_y_ranges(R - 250.0..=R, 8.0..=592.0);
    assert_eq!(previews(&h), vec![landing]);
    send(&mut h, vec![button(to, false)]);
    h.run_steps(2);
    assert_eq!(panel(&h, Pane::Comments), landing);
    assert_eq!(panel(&h, Pane::Nav), Rect::from_x_y_ranges(R - 500.0..=R - 250.0, 8.0..=592.0));
}

#[test]
fn a_pane_dropped_over_a_docked_panes_outer_or_inner_third_docks_beside_it_there() {
    let full = |x: std::ops::RangeInclusive<f32>| Rect::from_x_y_ranges(x, 8.0..=592.0);
    // Drags Comments' title to `to`, checks the preview, drops it and returns the harness.
    let drop = |app: App, to: Pos2, preview: Option<Rect>| {
        let mut h = harness(app);
        let from = title(&h, Pane::Comments).center();
        hold(&mut h, from, to);
        let previewed = previews(&h);
        match preview {
            Some(rect) => assert_eq!(previewed, vec![rect], "at {to:?}"),
            None => assert!(previewed.len() == 1 && previewed[0].height() == FLOAT_DEFAULT.y, "a float preview at {to:?}: {previewed:?}"),
        }
        send(&mut h, vec![button(to, false)]);
        h.run_steps(2);
        h
    };

    // Over Nav's outer third, beyond the window-edge strip: outside Nav.
    let h = drop(App::new(&[Pane::Nav, Pane::Comments]), pos2(L + 40.0, 300.0), Some(full(L..=L + 250.0)));
    assert_eq!(
        changes(h.state()),
        vec![PaneRequest::Place(Pane::Comments, Place::Docked(Edge::Start)), PaneRequest::Before(Pane::Comments, Some(Pane::Nav))]
    );
    assert_eq!((panel(&h, Pane::Comments), panel(&h, Pane::Nav)), (full(L..=L + 250.0), full(L + 250.0..=L + 500.0)));

    // Over its inner third: inside Nav, innermost.
    let h = drop(App::new(&[Pane::Comments, Pane::Nav]), pos2(L + 220.0, 300.0), Some(full(L + 250.0..=L + 500.0)));
    assert_eq!(changes(h.state()), vec![PaneRequest::Place(Pane::Comments, Place::Docked(Edge::Start)), PaneRequest::Before(Pane::Comments, None)]);
    assert_eq!(panel(&h, Pane::Comments), full(L + 250.0..=L + 500.0));

    // Over its middle third: no dock target, so it floats.
    let h = drop(App::new(&[Pane::Nav, Pane::Comments]), pos2(L + 125.0, 300.0), None);
    assert_eq!(h.state().place(Pane::Comments), Place::Floating);

    // Right to left: Nav is at the right edge, and its outer third is on its right.
    let mut app = App::new(&[Pane::Nav, Pane::Comments]);
    app.rtl = true;
    let h = drop(app, pos2(R - 40.0, 300.0), Some(full(R - 250.0..=R)));
    assert_eq!((panel(&h, Pane::Comments), panel(&h, Pane::Nav)), (full(R - 250.0..=R), full(R - 500.0..=R - 250.0)));

    // One slot per edge: a pane dropped over the slot's pane takes the slot.
    let mut app = App::new(&[Pane::Comments, Pane::Nav]);
    app.policy = SidePolicy::OneSlot;
    let h = drop(app, pos2(L + 40.0, 300.0), Some(full(L..=L + 250.0)));
    assert_eq!(h.state().open, vec![Pane::Nav, Pane::Comments]);
    assert_eq!(panel(&h, Pane::Comments), full(L..=L + 250.0));

    // On the same edge: over the outer third of the pane outside it moves it outermost; over the
    // inner third of that pane, where it already is, nothing changes.
    let mut h = harness(App::new(&[Pane::Nav, Pane::Styles]));
    let from = title(&h, Pane::Styles).center();
    drag(&mut h, from, pos2(L + 200.0, 300.0));
    assert!(changes(h.state()).is_empty(), "{:?}", h.state().log);
    drag(&mut h, from, pos2(L + 40.0, 300.0));
    assert_eq!(changes(h.state()), vec![PaneRequest::Before(Pane::Styles, Some(Pane::Nav))]);
    assert_eq!((edge(&h, Pane::Styles), edge(&h, Pane::Nav)), (L + 250.0, L + 500.0));

    // A floating pane: no target over Nav's middle third, beside it over its inner third.
    let mut app = App::new(&[Pane::Nav, Pane::Styles]);
    app.states.insert(Pane::Styles, PaneState { place: Place::Floating, size: Some(300.0), float_rect: Some([300.0, 100.0, 270.0, 400.0]) });
    let mut h = harness(app);
    let from = title(&h, Pane::Styles).center();
    hold(&mut h, from, pos2(L + 125.0, 300.0));
    assert!(previews(&h).is_empty());
    let to = pos2(L + 220.0, 300.0);
    send(&mut h, vec![Event::PointerMoved(to)]);
    assert_eq!(previews(&h), vec![full(L + 250.0..=L + 550.0)]);
    send(&mut h, vec![button(to, false)]);
    h.run_steps(2);
    assert_eq!(h.state().place(Pane::Styles), Place::Docked(Edge::Start));
    assert_eq!(panel(&h, Pane::Styles), full(L + 250.0..=L + 550.0));
}

#[test]
fn one_slot_previews_a_pane_beside_the_context_pane() {
    let mut app = App::new(&[Pane::Pivot, Pane::Styles]);
    app.policy = SidePolicy::OneSlot;
    let mut h = harness(app);
    let from = title(&h, Pane::Styles).center();
    let to = pos2(R - 10.0, 300.0);
    hold(&mut h, from, to);
    let landing = Rect::from_x_y_ranges(R - 500.0..=R - 250.0, 8.0..=592.0);
    assert_eq!(previews(&h), vec![landing], "inside the Pivot pane, which comes first in open");
    send(&mut h, vec![button(to, false)]);
    h.run_steps(2);
    assert_eq!(panel(&h, Pane::Styles), landing);
}

#[test]
fn one_slot_previews_the_room_a_pane_it_reveals_leaves() {
    // Nav and Styles share the start slot, so only Styles shows. Moved to the end, it reveals
    // Nav, which takes 500 of the host's 784 pt.
    let mut app = App::new(&[Pane::Nav, Pane::Styles]);
    app.policy = SidePolicy::OneSlot;
    for pane in [Pane::Nav, Pane::Styles] {
        app.states.insert(pane, PaneState::docked(Edge::Start, 500.0));
    }
    let mut h = harness(app);
    let from = title(&h, Pane::Styles).center();
    let to = pos2(R - 10.0, 300.0);
    hold(&mut h, from, to);
    let landing = Rect::from_x_y_ranges(L + 500.0..=R, 8.0..=592.0);
    assert_eq!(previews(&h), vec![landing]);
    send(&mut h, vec![button(to, false)]);
    h.run_steps(2);
    assert_eq!(panel(&h, Pane::Styles), landing);
}

#[test]
fn escape_and_focus_loss_restore_a_floating_pane_during_a_move() {
    let mut app = App::new(&[Pane::Styles]);
    let start = [300.0, 100.0, 270.0, 400.0];
    app.states.insert(Pane::Styles, PaneState { place: Place::Floating, size: None, float_rect: Some(start) });
    let mut h = harness(app);
    let from = title(&h, Pane::Styles).center();
    let float_rect = |h: &Harness<'_, App>| h.state().states[&Pane::Styles].float_rect;

    hold(&mut h, from, from + vec2(40.0, 30.0));
    assert_eq!(float_rect(&h), Some([340.0, 130.0, 270.0, 400.0]), "follows the pointer");
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert_eq!(float_rect(&h), Some(start));
    send(&mut h, vec![Event::PointerMoved(pos2(790.0, 300.0))]);
    send(&mut h, vec![button(pos2(790.0, 300.0), false)]);
    h.run_steps(1);
    assert_eq!(h.state().place(Pane::Styles), Place::Floating, "the rest of the drag is ignored");
    assert_eq!(float_rect(&h), Some(start));

    hold(&mut h, from, from + vec2(-60.0, 20.0));
    assert_eq!(float_rect(&h), Some([240.0, 120.0, 270.0, 400.0]));
    h.input_mut().focused = false;
    h.run_steps(1);
    h.input_mut().focused = true;
    send(&mut h, vec![button(from + vec2(-60.0, 20.0), false)]);
    assert_eq!(float_rect(&h), Some(start));
}

#[test]
fn f6_cycles_through_the_panes_and_back_to_the_document() {
    let mut h = harness(App::new(&[Pane::Nav, Pane::Comments]));
    let focused_widget = |h: &Harness<'_, App>| h.ctx.memory(|m| m.focused());
    key(&mut h, Key::F6, Modifiers::NONE);
    assert_eq!(h.state().focused, Some(Pane::Nav));
    assert_eq!(focused_widget(&h), Some(pane_id(Pane::Nav).with("title")));
    key(&mut h, Key::F6, Modifiers::NONE);
    assert_eq!(h.state().focused, Some(Pane::Comments));
    key(&mut h, Key::F6, Modifiers::NONE);
    assert_eq!((h.state().focused, h.state().to_document), (None, 1));
    assert_eq!(focused_widget(&h), None);
    key(&mut h, Key::F6, Modifiers::SHIFT);
    assert_eq!(h.state().focused, Some(Pane::Comments), "Shift+F6 from the document goes to the last pane");
    let focus: Vec<_> = h.state().log.iter().filter(|r| matches!(r, PaneRequest::Focus(_))).cloned().collect();
    assert_eq!(focus, vec![PaneRequest::Focus(Pane::Nav), PaneRequest::Focus(Pane::Comments), PaneRequest::Focus(Pane::Comments)]);

    // A click in the document clears it; a click in a pane sets it.
    click(&mut h, pos2(400.0, 300.0));
    assert_eq!(h.state().focused, None);
    click(&mut h, pos2(100.0, 300.0));
    assert_eq!(h.state().focused, Some(Pane::Nav));
}

#[test]
fn one_slot_shows_the_last_pane_per_edge_beside_context_panes() {
    let mut app = App::new(&[Pane::Pivot, Pane::Comments, Pane::Styles]);
    app.policy = SidePolicy::OneSlot;
    app.states.insert(Pane::Styles, PaneState::docked(Edge::End, 300.0));
    let mut h = harness(app);
    assert!(rect(&h, pane_id(Pane::Comments).with("title")).is_none());
    assert_eq!(edge(&h, Pane::Pivot), R - 250.0, "the context pane stays, outermost");
    assert_eq!(edge(&h, Pane::Styles), R - 550.0);
    assert!(rect(&h, pane_id(Pane::Pivot).with("title").with("close")).is_none());

    let at = rect(&h, pane_id(Pane::Styles).with("title").with("close")).unwrap().center();
    click(&mut h, at);
    h.run_steps(1);
    assert_eq!(h.state().open, vec![Pane::Pivot, Pane::Comments]);
    assert_eq!(edge(&h, Pane::Comments), R - 500.0, "the slot shows the remaining pane");
}

#[test]
fn rtl_mirrors_sides_and_edge_snaps() {
    let mut app = App::new(&[Pane::Nav, Pane::Comments]);
    app.rtl = true;
    let mut h = harness(app);
    assert_eq!(edge(&h, Pane::Nav), R - 250.0, "Start is the right edge");
    assert_eq!(edge(&h, Pane::Comments), L + 250.0);

    let from = title(&h, Pane::Nav).center();
    drag(&mut h, from, pos2(10.0, 300.0));
    assert_eq!(changes(h.state()), vec![PaneRequest::Place(Pane::Nav, Place::Docked(Edge::End))], "the left edge is End");
    // Both on End, outermost first in open order.
    assert_eq!(edge(&h, Pane::Nav), L + 250.0);
    assert_eq!(edge(&h, Pane::Comments), L + 500.0);
}

#[test]
fn float_rect_is_finite_and_inside_the_bounds() {
    let bounds = Rect::from_min_size(pos2(0.0, 100.0), vec2(800.0, 500.0));
    let min = vec2(200.0, 160.0);
    assert_eq!(float_rect(Some([300.0, 150.0, 280.0, 300.0]), bounds, min), Rect::from_min_size(pos2(300.0, 150.0), vec2(280.0, 300.0)));
    for junk in [[f32::NAN, 0.0, 1.0, -3.0], [5000.0, -5000.0, 1.0e9, 1.0e9], [f32::NEG_INFINITY, f32::INFINITY, f32::NAN, 0.0]] {
        let r = float_rect(Some(junk), bounds, min);
        assert!(r.is_finite() && bounds.contains_rect(r) && r.width() >= 200.0, "{junk:?} -> {r:?}");
    }
    assert_eq!(float_rect(None, bounds, min).size(), FLOAT_DEFAULT.min(bounds.size()));
}

#[cfg(feature = "serde")]
#[test]
/// A `PaneState` with a docked edge, size and float rect serializes to JSON and
/// deserializes back unchanged.
fn pane_state_round_trips_through_json() {
    let state = PaneState { place: Place::Docked(Edge::End), size: Some(280.0), float_rect: Some([1.0, 2.0, 270.0, 480.0]) };
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(serde_json::from_str::<PaneState>(&json).unwrap(), state);
}

#[test]
/// A pane body may request 400 pt inside its 250 pt side pane; the document region still begins at
/// the pane's inner edge on both sides.
fn a_body_wider_than_its_pane_is_clipped_and_the_document_starts_at_the_pane_edge() {
    struct Wide;
    impl PaneContent<Pane> for Wide {
        fn title(&self, pane: &Pane) -> String {
            format!("{pane:?}")
        }
        /// A pane body that requests 400×40 pt inside its pane to exercise clipping.
        fn body(&mut self, ui: &mut Ui, _: &Pane) {
            ui.allocate_exact_size(vec2(400.0, 40.0), Sense::hover());
        }
        fn default_state(&self, pane: &Pane) -> PaneState {
            PaneState::docked(if *pane == Pane::Comments { Edge::End } else { Edge::Start }, 250.0)
        }
    }
    let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        |ui, document: &mut Rect| {
            PaneHost::new(Id::unique("panes")).show(ui, &[Pane::Nav, Pane::Comments], |_| None, &mut Wide);
            *document = ui.available_rect_before_wrap();
        },
        Rect::NOTHING,
    );
    h.run_steps(2);
    let document = *h.state();
    assert_eq!(document.left(), L + 250.0, "{document:?}");
    assert_eq!(document.right(), R - 250.0, "{document:?}");
}

#[test]
fn each_title_bar_is_named_after_its_pane() {
    let h = harness(App::new(&[Pane::Nav, Pane::Comments]));
    assert_eq!(h.query_all_by_label("Move Nav pane").count(), 1);
    assert_eq!(h.query_all_by_label("Move Comments pane").count(), 1);
}
