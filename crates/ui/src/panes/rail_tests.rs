//! Behavior tests for the icon rail, driven by egui pointer and key events. The harness's root ui
//! has an 8 pt margin, so in an 800 pt wide window the rail and panes span x 8..792.

use egui::accesskit::{Role, Toggled};
use egui::load::Bytes;
use egui::{Event, ImageSource, Key, Modifiers, PointerButton, Pos2, Rect, Ui, vec2};
use egui_kittest::{Harness, kittest::NodeT, kittest::Queryable};

use super::*;

const L: f32 = 8.0;
const R: f32 = 792.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Tool {
    Comments,
    Bookmarks,
    Pages,
    Layers,
    Files,
    Tags,
}

const TOOLS: [Tool; 6] = [Tool::Comments, Tool::Bookmarks, Tool::Pages, Tool::Layers, Tool::Files, Tool::Tags];

/// A tiny valid icon for the rail's images. The tests install the SVG loader below; without a
/// loadable image, egui paints its failed-image placeholder ("⚠️"), which the harness's missing
/// glyph policy rejects.
const ICON_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"
    viewBox="0 0 24 24"><rect width="24" height="24" fill="#000"/></svg>"##;

fn items() -> Vec<RailItem<Tool>> {
    TOOLS
        .iter()
        .map(|t| {
            let icon = ImageSource::Bytes { uri: format!("bytes://rail-test/{t:?}.svg").into(), bytes: Bytes::Static(ICON_SVG) };
            RailItem::new(*t, icon, format!("{t:?}"))
        })
        .collect()
}

/// The one-slot task pane the rail opens, at the rail's edge.
struct Content {
    edge: Edge,
}

impl PaneContent<Tool> for Content {
    fn title(&self, pane: &Tool) -> String {
        format!("{pane:?} pane")
    }
    fn body(&mut self, ui: &mut Ui, pane: &Tool) {
        ui.label(format!("{pane:?} body"));
    }
    fn default_state(&self, _: &Tool) -> PaneState {
        PaneState::docked(self.edge, 250.0)
    }
    fn movable(&self, _: &Tool) -> bool {
        false
    }
}

/// The app side: which pane is open, and every request the rail sent.
struct App {
    items: Vec<RailItem<Tool>>,
    open: Option<Tool>,
    log: Vec<RailRequest<Tool>>,
    edge: Edge,
    rtl: bool,
    /// Show the open pane with a `PaneHost` after the rail.
    pane: bool,
    foot: bool,
    out: Option<RailOutput<Tool>>,
    /// `PaneOutput::focused` of the pane host.
    pane_focused: Option<Tool>,
    document: Rect,
}

impl App {
    fn new() -> Self {
        Self {
            items: items(),
            open: None,
            log: Vec::new(),
            edge: Edge::End,
            rtl: false,
            pane: false,
            foot: false,
            out: None,
            pane_focused: None,
            document: Rect::NOTHING,
        }
    }

    fn frame(&mut self, ui: &mut Ui) {
        let foot = self.foot;
        let out = Rail::new(Id::unique("rail")).edge(self.edge).rtl(self.rtl).show(ui, &self.items, self.open.as_ref(), |ui| {
            if foot {
                let _ = ui.button("Foot");
            }
        });
        if self.pane {
            let open: Vec<Tool> = self.open.into_iter().collect();
            let panes = PaneHost::new(Id::unique("panes")).rtl(self.rtl).policy(SidePolicy::OneSlot).show(
                ui,
                &open,
                |_| None,
                &mut Content { edge: self.edge },
            );
            self.pane_focused = panes.focused;
        }
        self.document = egui::CentralPanel::default().show(ui, |ui| ui.label("Document")).response.rect;
        match &out.request {
            Some(RailRequest::Open(p)) => self.open = Some(*p),
            Some(RailRequest::Close(_)) => self.open = None,
            None => {}
        }
        self.log.extend(out.request.clone());
        self.out = Some(out);
    }

    fn out(&self) -> &RailOutput<Tool> {
        self.out.as_ref().unwrap()
    }
}

fn harness(app: App, height: f32) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_context_setup(egui_extras::install_image_loaders)
        .with_size(vec2(800.0, height))
        .build_ui_state(|ui, app: &mut App| app.frame(ui), app);
    h.input_mut().focused = true;
    h.run_steps(2);
    h
}

fn item(h: &Harness<'_, App>, tool: Tool) -> Rect {
    h.ctx.read_response(Rail::new(Id::unique("rail")).item_id(&tool)).unwrap().rect
}

fn click(h: &mut Harness<'_, App>, at: Pos2) {
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.input_mut().events.extend([Event::PointerMoved(at), button(true)]);
    h.run_steps(1);
    h.input_mut().events.push(button(false));
    h.run_steps(2);
}

fn click_item(h: &mut Harness<'_, App>, tool: Tool) {
    let at = item(h, tool).center();
    click(h, at);
}

fn key(h: &mut Harness<'_, App>, key: Key) {
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

fn focused(h: &Harness<'_, App>) -> Option<Tool> {
    h.state().out().focused
}

#[test]
fn a_click_opens_the_open_item_closes_and_another_swaps() {
    let mut h = harness(App::new(), 600.0);
    click_item(&mut h, Tool::Comments);
    assert_eq!(h.state().log, [RailRequest::Open(Tool::Comments)]);
    assert_eq!(h.state().open, Some(Tool::Comments));
    click_item(&mut h, Tool::Comments);
    assert_eq!(h.state().log[1..], [RailRequest::Close(Tool::Comments)]);
    assert_eq!(h.state().open, None);
    click_item(&mut h, Tool::Comments);
    click_item(&mut h, Tool::Pages);
    assert_eq!(h.state().log[2..], [RailRequest::Open(Tool::Comments), RailRequest::Open(Tool::Pages)]);
    assert_eq!(h.state().open, Some(Tool::Pages));
}

#[test]
fn a_disabled_item_sends_nothing_and_the_marker_shows_only_while_closed() {
    let mut app = App::new();
    app.items[1].enabled = false;
    app.items[0].marker = true;
    app.items[0].marker_label = Some("Has comments".into());
    app.items[3] = app.items[3].clone().marker(true).marker_label("Has layers");
    app.open = Some(Tool::Comments);
    let mut h = harness(app, 600.0);
    click_item(&mut h, Tool::Bookmarks);
    assert!(h.state().log.is_empty());
    assert_eq!(h.state().open, Some(Tool::Comments));

    let markers = |h: &Harness<'_, App>| -> Vec<Pos2> {
        h.output()
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Circle(circle) if circle.radius == 3.0 => Some(circle.center),
                _ => None,
            })
            .collect()
    };
    let shown = markers(&h);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(item(&h, Tool::Layers).contains(shown[0]));
    assert!(shown[0].x > item(&h, Tool::Layers).center().x, "the marker sits at the top-right corner");
    let description = |h: &Harness<'_, App>, label: &str| h.get_by_label(label).accesskit_node().description();
    assert_eq!(description(&h, "Layers").as_deref(), Some("Has layers"));
    // egui 0.37 uses tooltip text as the default accessible description.
    assert_eq!(description(&h, "Comments").as_deref(), Some("Comments"), "no marker description while the pane is open");

    // Closed, Comments shows its marker too.
    click_item(&mut h, Tool::Comments);
    assert_eq!(h.state().open, None);
    let shown = markers(&h);
    assert_eq!(shown.len(), 2, "{shown:?}");
    assert!(shown.iter().any(|p| item(&h, Tool::Comments).contains(*p)));
    h.run_steps(1);
    // Hovering Comments shows a tooltip with its label; move away before reading by label.
    h.input_mut().events.push(Event::PointerMoved(Pos2::new(400.0, 300.0)));
    h.run_steps(2);
    assert_eq!(description(&h, "Comments").as_deref(), Some("Has comments"));

    let mut app = App::new();
    app.rtl = true;
    app.items[3].marker = true;
    let h = harness(app, 600.0);
    let shown = markers(&h);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(shown[0].x < item(&h, Tool::Layers).center().x, "right-to-left: the marker sits at the top-left corner");
}

#[test]
fn the_rail_is_outermost_and_the_open_pane_sits_between_it_and_the_document() {
    for (edge, rtl, left) in [(Edge::End, false, false), (Edge::End, true, true), (Edge::Start, false, true), (Edge::Start, true, false)] {
        let mut app = App::new();
        app.edge = edge;
        app.rtl = rtl;
        app.pane = true;
        app.open = Some(Tool::Comments);
        let h = harness(app, 600.0);
        let rail = h.state().out().rect;
        let pane_id = Id::unique("panes").with(("pane", &Tool::Comments));
        // The resize handle lies on the pane's inner edge; the pane is 250 wide.
        let inner = h.ctx.read_response(pane_id.with("resize")).unwrap().rect.center().x;
        let pane = if left {
            Rect::from_x_y_ranges(inner - 250.0..=inner, rail.y_range())
        } else {
            Rect::from_x_y_ranges(inner..=inner + 250.0, rail.y_range())
        };
        let title = h.ctx.read_response(pane_id.with("title")).unwrap().rect;
        let document = h.state().document;
        let case = format!("{edge:?} rtl {rtl}: rail {rail:?}, pane {pane:?}, title {title:?}, document {document:?}");
        assert_eq!(rail.width(), RAIL_WIDTH, "{case}");
        assert!(pane.x_range().contains(title.left()) && pane.x_range().contains(title.right()), "{case}");
        if left {
            assert_eq!(rail.left(), L, "{case}");
            assert_eq!(pane.left(), rail.right(), "{case}");
            assert!(document.left() >= pane.right(), "{case}");
        } else {
            assert_eq!(rail.right(), R, "{case}");
            assert_eq!(pane.right(), rail.left(), "{case}");
            assert!(document.right() <= pane.left(), "{case}");
        }
        assert!(rail.x_range().contains(item(&h, Tool::Comments).center().x), "{case}");
    }
}

#[test]
fn in_a_short_window_the_items_scroll_and_the_foot_stays_visible() {
    let mut app = App::new();
    app.foot = true;
    let mut h = harness(app, 200.0);
    let foot = h.get_by_label("Foot").rect();
    assert!(foot.bottom() <= 192.0, "{foot:?}");
    assert!(item(&h, Tool::Comments).top() < 30.0);
    assert!(item(&h, Tool::Tags).bottom() > foot.top(), "the last item starts out of view");

    let over = item(&h, Tool::Comments).center();
    h.input_mut().events.extend([
        Event::PointerMoved(over),
        Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -500.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        },
    ]);
    h.run_steps(3);
    let tags = item(&h, Tool::Tags);
    assert!(tags.bottom() <= h.get_by_label("Foot").rect().top(), "{tags:?} above the foot after scrolling");
    assert_eq!(h.get_by_label("Foot").rect(), foot);
}

#[test]
fn items_are_named_toggle_buttons_and_the_keyboard_moves_between_and_activates_them() {
    let mut app = App::new();
    app.items[1].enabled = false;
    app.open = Some(Tool::Comments);
    let mut h = harness(app, 600.0);
    let comments = h.get_by_label("Comments").accesskit_node();
    assert_eq!(comments.role(), Role::Button);
    assert_eq!(comments.toggled(), Some(Toggled::True));
    assert!(!comments.is_disabled());
    let pages = h.get_by_label("Pages").accesskit_node();
    assert_eq!(pages.role(), Role::Button);
    assert_eq!(pages.toggled(), Some(Toggled::False));
    assert!(h.get_by_label("Bookmarks").accesskit_node().is_disabled());

    h.ctx.memory_mut(|m| m.request_focus(Rail::new(Id::unique("rail")).item_id(&Tool::Comments)));
    h.run_steps(2);
    assert_eq!(focused(&h), Some(Tool::Comments));
    key(&mut h, Key::ArrowUp);
    assert_eq!(focused(&h), Some(Tool::Comments), "Up on the first item stays");
    key(&mut h, Key::ArrowDown);
    assert_eq!(focused(&h), Some(Tool::Pages), "Down skips the disabled item");
    key(&mut h, Key::Enter);
    assert_eq!(h.state().log, [RailRequest::Open(Tool::Pages)]);
    key(&mut h, Key::Space);
    assert_eq!(h.state().log[1..], [RailRequest::Close(Tool::Pages)]);
    key(&mut h, Key::ArrowUp);
    assert_eq!(focused(&h), Some(Tool::Comments));
    for _ in 0..TOOLS.len() {
        key(&mut h, Key::ArrowDown);
    }
    assert_eq!(focused(&h), Some(Tool::Tags), "Down on the last item stays");
}

#[test]
fn an_empty_rail_draws_and_bad_sizes_fall_back() {
    let mut app = App::new();
    app.items.clear();
    let h = harness(app, 600.0);
    let rail = h.state().out().rect;
    assert_eq!((rail.right(), rail.width()), (R, RAIL_WIDTH));
    assert_eq!(h.state().out().request, None);

    let mut h = Harness::builder().with_context_setup(egui_extras::install_image_loaders).with_size(vec2(400.0, 300.0)).build_ui(|ui| {
        let out = Rail::new(Id::unique("bad")).width(f32::NAN).button_size(f32::INFINITY).show(ui, &items(), None, |_| {});
        assert_eq!(out.rect.width(), RAIL_WIDTH);
    });
    h.run_steps(2);
}

#[test]
fn a_disabled_item_takes_no_tab_focus_and_a_focused_item_disabled_hands_focus_on() {
    let mut app = App::new();
    app.items[1].enabled = false;
    let mut h = harness(app, 600.0);
    let focused_id = |h: &Harness<'_, App>| h.ctx.memory(|m| m.focused());
    let id = |tool: Tool| Rail::new(Id::unique("rail")).item_id(&tool);
    h.ctx.memory_mut(|m| m.request_focus(id(Tool::Comments)));
    h.run_steps(2);
    key(&mut h, Key::Tab);
    assert_eq!(focused_id(&h), Some(id(Tool::Pages)), "Tab skips the disabled item");

    // Pages is disabled while focused: focus goes to the next enabled item.
    h.state_mut().items[2].enabled = false;
    h.run_steps(2);
    assert_eq!(focused_id(&h), Some(id(Tool::Layers)));
    assert_eq!(focused(&h), Some(Tool::Layers));

    // The last item disabled while focused: focus goes to the enabled one above.
    h.ctx.memory_mut(|m| m.request_focus(id(Tool::Tags)));
    h.run_steps(2);
    h.state_mut().items[5].enabled = false;
    h.run_steps(2);
    assert_eq!(focused_id(&h), Some(id(Tool::Files)));

    // Nothing left enabled: focus leaves the rail.
    for item in &mut h.state_mut().items {
        item.enabled = false;
    }
    h.run_steps(2);
    assert_eq!((focused_id(&h), focused(&h)), (None, None));
}

#[test]
fn a_press_on_the_rail_clears_the_pane_hosts_focused_pane() {
    let mut app = App::new();
    app.pane = true;
    app.open = Some(Tool::Comments);
    let mut h = harness(app, 600.0);
    let body = h.get_by_label("Comments body").rect().center();
    click(&mut h, body);
    assert_eq!(h.state().pane_focused, Some(Tool::Comments));
    // Below the items: the rail, not a pane.
    let rail = h.state().out().rect;
    click(&mut h, Pos2::new(rail.center().x, rail.bottom() - 40.0));
    assert_eq!(h.state().pane_focused, None);
    assert!(h.state().log.is_empty());
    assert_eq!(h.state().open, Some(Tool::Comments));
}

#[test]
fn a_bottom_edge_draws_the_rail_at_the_end() {
    let mut app = App::new();
    app.edge = Edge::Bottom;
    let h = harness(app, 600.0);
    let rail = h.state().out().rect;
    assert_eq!((rail.right(), rail.height()), (R, 584.0), "{rail:?}");
}

#[test]
fn chrome_style_from_visuals_matches_the_dock_style_it_skips() {
    use crate::docking::{DockStyle, chrome::ChromeStyle};
    for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
        let direct = ChromeStyle::from_visuals(&visuals);
        let through_dock = ChromeStyle::from_dock(&DockStyle::from_visuals(&visuals));
        assert_eq!(format!("{direct:?}"), format!("{through_dock:?}"));
    }
}
