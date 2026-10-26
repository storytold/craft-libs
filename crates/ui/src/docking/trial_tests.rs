//! Fixes from the DesignCraft trial: group float size, tab label elision, the drag ghost and
//! grip, double-clicks on minimized groups, pointer focus, label keys and borrowed lookups; then
//! split permissions, body focus, right-to-left strips, close controls, menu and button names,
//! zone mapping, closed panels from `apply`, group heights and flyout events.

use super::*;

/// Every shape, with nested `Shape::Vec`s flattened.
fn flat(shapes: &[egui::epaint::ClippedShape]) -> Vec<(Rect, egui::Shape)> {
    fn walk(clip: Rect, shape: &egui::Shape, out: &mut Vec<(Rect, egui::Shape)>) {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(clip, s, out)),
            shape => out.push((clip, shape.clone())),
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(clipped.clip_rect, &clipped.shape, &mut out);
    }
    out
}

fn texts(d: &Driver) -> Vec<(Rect, egui::epaint::TextShape)> {
    flat(&d.shapes)
        .into_iter()
        .filter_map(|(clip, shape)| match shape {
            egui::Shape::Text(text) => Some((clip, text)),
            _ => None,
        })
        .collect()
}

fn rects(d: &Driver) -> Vec<egui::epaint::RectShape> {
    flat(&d.shapes)
        .into_iter()
        .filter_map(|(_, shape)| match shape {
            egui::Shape::Rect(rect) => Some(rect),
            _ => None,
        })
        .collect()
}

fn last_char(galley: &egui::Galley) -> Option<char> {
    galley.rows.last().and_then(|row| row.glyphs.last()).map(|glyph| glyph.chr)
}

#[test]
fn a_full_height_docked_group_floats_at_the_host_or_capped_height_inside_the_window() {
    for size in [None, Some(vec2(200.0, 150.0))] {
        let mut layout = docked();
        let Some(Node::Split { first, .. }) = &mut layout.root else { panic!() };
        **first = Node::Dock { side: Side::Start, columns: vec![Column::new(vec![Group::new(vec![2, 3], 1)], 240.0)] };
        let content = TestContent { protected: vec![0], floating_size: size, body_height: Some(600.0), ..Default::default() };
        let mut d = Driver::with(layout, content, Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0)));
        let bar = d.rect("drag-bar", 2);
        let out = d.drag(bar.center(), pos2(420.0, 300.0));
        let Some(Target::Float { rect: Some([x, y, width, height]) }) = moved_group(&out.actions) else { panic!("{:?}", out.actions) };
        match size {
            Some(size) => assert_eq!(*height, size.y),
            None => assert_eq!(*height, 320.0),
        }
        assert!(*width > 100.0 && *width <= 240.0, "{width}");
        let window = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        assert!(window.contains_rect(Rect::from_min_size(pos2(*x, *y), vec2(*width, *height))), "{x} {y} {width} {height}");
    }
}

/// Labels as a host's pre-laid-out galleys, which egui does not wrap again.
struct GalleyLabels;

impl DockContent<u32> for GalleyLabels {
    fn body(&mut self, _: &mut egui::Ui, _: &u32) {}
    fn logical_label(&self, panel: &u32) -> String {
        format!("Character Styles {panel}")
    }
    /// Host-provided galley label that lays the logical text out without wrapping,
    /// letting the dock handle pre-shaped widget text.
    fn label(&mut self, ui: &egui::Ui, _: &u32, logical: &str, font: &egui::FontId, color: Color32) -> egui::WidgetText {
        egui::WidgetText::Galley(ui.painter().layout_no_wrap(logical.to_owned(), font.clone(), color))
    }
}

#[test]
/// Galley-backed labels in shrunk tabs end in an ellipsis and stay wholly inside their tab, both
/// for the active tab and for inactive tabs, at wide and narrow dock widths.
fn galley_labels_in_shrunk_tabs_end_with_an_ellipsis_inside_the_tab_active_or_not() {
    let panels = [1, 2, 3, 4, 5, 6];
    let layout = Layout { root: Some(tabs(&panels, 3)), floating: Vec::new() };
    for width in [320.0, 90.0] {
        let ctx = Context::default();
        let mut content = GalleyLabels;
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let input = RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), ..Default::default() };
            let mut full = ctx.run_ui(input, |ui| {
                let style = DockStyle::from_ui(ui);
                ui.scope_builder(UiBuilder::new().max_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 400.0))), |ui| {
                    DockArea::new(Id::unique(AREA)).show_content(ui, &layout, &style, &mut content);
                });
            });
            full.textures_delta.clear();
            shapes = full.shapes;
        }
        let texts: Vec<_> = flat(&shapes)
            .into_iter()
            .filter_map(|(clip, shape)| match shape {
                egui::Shape::Text(text) => Some((clip, text)),
                _ => None,
            })
            .collect();
        let shown: Vec<u32> = panels.into_iter().filter(|p| ctx.read_response(Id::unique(AREA).with(("tab", p))).is_some()).collect();
        assert!(shown.contains(&4) && shown.len() < panels.len(), "{width}: {shown:?}");
        for panel in shown {
            let tab = ctx.read_response(Id::unique(AREA).with(("tab", &panel))).unwrap().rect;
            let Some((clip, text)) = texts.iter().find(|(_, t)| t.galley.text().ends_with(&format!(" {panel}")) && tab.contains(t.pos)) else {
                panic!("no label in tab {panel}")
            };
            assert_eq!(last_char(&text.galley), Some('…'), "{width}: tab {panel}");
            let drawn = Rect::from_min_size(text.pos, text.galley.size());
            assert!(drawn.right() <= tab.right() + 0.5 && drawn.left() >= tab.left(), "tab {panel}: {drawn:?} in {tab:?}");
            assert!(tab.contains_rect(clip.intersect(drawn)), "tab {panel}");
        }
    }
}

#[test]
fn drag_bar_draws_no_mark_at_rest_and_a_group_drag_paints_a_filled_labeled_ghost() {
    let mut d = Driver::new(frames());
    let visuals = d.ctx.global_style().visuals.clone();
    let bar = d.rect("drag-bar", 2);
    let mark = flat(&d.shapes).into_iter().any(|(_, shape)| match shape {
        egui::Shape::LineSegment { points, .. } => bar.contains(points[0]) && bar.contains(points[1]),
        _ => false,
    });
    assert!(!mark, "a mark in the drag bar at rest");

    d.hold(bar.center(), pos2(150.0, 300.0));
    let fill = visuals.selection.stroke.color.gamma_multiply(0.15);
    let Some(ghost) = rects(&d).into_iter().find(|r| r.fill == fill && r.stroke.width == 2.0 && r.rect.contains(pos2(150.0, 300.0))) else {
        panic!("no filled ghost under the pointer")
    };
    for label in ["Panel 2", "Panel 3"] {
        assert!(texts(&d).iter().any(|(_, t)| t.galley.text() == label && ghost.rect.contains(t.pos)), "{label} missing from the ghost");
    }
    d.step(vec![key(egui::Key::Escape)]);
}

#[test]
fn double_click_on_a_tab_of_a_minimized_group_ends_expanded() {
    let layout = Layout { root: Some(tabs(&[1], 0)), floating: vec![Floating::single(vec![5, 6], 1, [80.0, 70.0, 300.0, 220.0])] };
    let mut d = Driver::new(layout);
    let minimized = |d: &Driver| d.layout.floating[0].minimized;
    let active = d.tab(6).center();
    d.click(active);
    d.click(active);
    assert!(minimized(&d));
    d.wait();

    let other = d.tab(5).center();
    d.click(other);
    d.click(other);
    assert!(!minimized(&d));
    assert_eq!(d.layout.floating[0].groups()[0].active_panel(), Some(&5));
}

#[test]
/// Double-clicking the active tab of a minimized floating window or docked
/// group expands it; a second double-click minimizes the docked group again.
fn double_click_on_the_active_tab_of_a_minimized_group_or_window_ends_expanded() {
    let window = Floating { minimized: true, ..Floating::single(vec![5, 6], 1, [80.0, 70.0, 300.0, 220.0]) };
    let mut d = Driver::new(Layout { root: Some(tabs(&[1], 0)), floating: vec![window] });
    let active = d.tab(6).center();
    d.click(active);
    d.click(active);
    assert!(!d.layout.floating[0].minimized);

    let mut layout = docked();
    let Some(Node::Split { first, .. }) = &mut layout.root else { panic!() };
    let group = Group { minimized: true, ..Group::new(vec![2, 3], 1) };
    **first = Node::Dock { side: Side::Start, columns: vec![Column::new(vec![group], 240.0)] };
    let mut d = Driver::new(layout);
    let minimized = |d: &Driver| {
        let Some(Node::Split { first, .. }) = &d.layout.root else { panic!() };
        let Node::Dock { columns, .. } = &**first else { panic!() };
        columns[0].groups[0].minimized
    };
    let active = d.tab(3).center();
    d.click(active);
    d.click(active);
    assert!(!minimized(&d));
    d.wait();
    d.click(active);
    d.click(active);
    assert!(minimized(&d));
}

#[test]
/// A pointer click on a tab shows no focus outline even if the platform then focuses it, while
/// keyboard (Tab) focus paints the accent outline.
fn a_clicked_tab_shows_no_focus_outline_but_tab_key_focus_does() {
    let mut d = Driver::new(frames());
    let accent = d.ctx.global_style().visuals.selection.stroke.color;
    let id = Id::unique(AREA).with(("tab", &3u32));
    let outline = d.tab(3).shrink(2.0);
    let outlined = |d: &Driver| rects(d).iter().any(|r| r.rect == outline && r.stroke.width > 0.0 && r.stroke.color == accent);

    d.click(d.tab(3).center());
    // A platform may focus the clicked widget, as an AccessKit focus request does.
    d.ctx.memory_mut(|m| m.request_focus(id));
    d.step(vec![]);
    assert_eq!(d.ctx.memory(|m| m.focused()), Some(id));
    assert!(!outlined(&d));

    d.ctx.memory_mut(|m| m.surrender_focus(id));
    for _ in 0..20 {
        if d.ctx.memory(|m| m.focused()) == Some(id) {
            break;
        }
        d.step(vec![key(egui::Key::Tab)]);
    }
    assert_eq!(d.ctx.memory(|m| m.focused()), Some(id));
    d.step(vec![]);
    assert!(outlined(&d));

    // Space and Enter activate the tab without counting as a pointer click.
    for activate in [egui::Key::Space, egui::Key::Enter] {
        let out = d.step(vec![key(activate)]);
        assert!(out.actions.contains(&Action::Activate { panel: 3 }), "{:?}", out.actions);
        d.step(vec![]);
        assert!(outlined(&d), "{activate:?} hid the focus outline");
    }
}

#[test]
fn localized_covers_every_label_and_contains_takes_a_borrowed_panel() {
    let mut debug = String::new();
    let mut full = Context::default().run_ui(RawInput::default(), |ui| {
        debug = format!("{:?}", DockStyle::from_ui(ui).localized(|key| Some(key.to_owned())));
    });
    full.textures_delta.clear();
    let values: Vec<&str> = debug.split("_label: \"").skip(1).filter_map(|rest| rest.split('"').next()).collect();
    assert_eq!(values.len(), 24, "{debug}");
    assert!(values.iter().all(|v| v.starts_with("dock.")), "a label without a key: {values:?}");
    let unique: std::collections::HashSet<_> = values.iter().collect();
    assert_eq!(unique.len(), values.len());

    let window = Floating::single(vec!["swatches".to_owned()], 0, [0.0, 0.0, 200.0, 200.0]);
    assert!(window.contains("swatches"));
    assert!(!window.contains("layers"));
}

const AREA_600: Rect = Rect { min: pos2(0.0, 0.0), max: pos2(600.0, 600.0) };

#[test]
fn a_protected_canvas_takes_no_side_split_and_a_group_dropped_on_its_edge_floats() {
    let protected = |p: &u32| if *p == 0 { Permissions::PROTECTED } else { Permissions::default() };
    let split = Action::Move { panel: 4, anchor: 0, placement: Placement::Split(Zone::Left) };
    let mut layout = docked();
    assert_eq!(layout.apply_with_permissions(split.clone(), protected), Err(DockError::NotAllowed));
    let opt_in = |p: &u32| if *p == 0 { Permissions { accepts_splits: true, ..Permissions::PROTECTED } } else { Permissions::default() };
    assert!(layout.apply_with_permissions(split, opt_in).is_ok());

    let mut d = Driver::with(docked(), TestContent { protected: vec![0], ..Default::default() }, AREA_600);
    let bar = d.rect("drag-bar", 2);
    let out = d.drag(bar.center(), pos2(300.0, 300.0));
    assert!(matches!(moved_group(&out.actions), Some(Target::Float { .. })), "{:?}", out.actions);
}

#[test]
fn a_press_in_a_stripless_panel_reports_it_focused() {
    let mut d = Driver::with(docked(), TestContent { protected: vec![0], ..Default::default() }, AREA_600);
    for pressed in [PointerButton::Primary, PointerButton::Secondary, PointerButton::Middle] {
        let at = pos2(420.0, 300.0);
        let press = Event::PointerButton { pos: at, button: pressed, pressed: true, modifiers: Modifiers::NONE };
        let out = d.step(vec![Event::PointerMoved(at), press]);
        assert_eq!(out.focused, Some(0), "{pressed:?}");
        d.step(vec![Event::PointerButton { pos: at, button: pressed, pressed: false, modifiers: Modifiers::NONE }]);
        d.step(vec![key(egui::Key::Escape)]);
    }
}

#[test]
/// In a right-to-left strip the first tab sits at the right edge and the bar
/// lies to its left; drops just right of the first tab insert before panel 2,
/// and insertion slots are mirrored at the strip edges.
fn right_to_left_mirrors_the_tab_strip_and_its_insertion_slots() {
    let mut d = Driver::with(frames(), TestContent { header: 30.0, ..Default::default() }, AREA_600);
    d.tweak = |style| style.right_to_left = true;
    d.step(vec![]);
    d.step(vec![]);
    let (first, second, bar) = (d.tab(2), d.tab(3), d.rect("drag-bar", 2));
    assert!((first.right() - 600.0).abs() < 1.0, "the first tab starts at the right edge: {first:?}");
    assert!(second.right() <= first.left() + 0.5, "{first:?} {second:?}");
    assert!(bar.right() <= second.left() + 0.5 && bar.left() >= 304.0 + 30.0 - 0.5, "the bar sits between the tabs and the header: {bar:?}");

    // Just right of the first tab's center is before it in a right-to-left strip.
    let out = d.drag(d.tab(1).center(), pos2(first.center().x + 6.0, first.center().y));
    assert!(out.actions.contains(&Action::Move { panel: 1, anchor: 3, placement: Placement::Tab { before: Some(2) } }), "{:?}", out.actions);

    let strip = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 20.0));
    let target = DropTarget {
        anchor: 1,
        rect: Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 80.0)),
        tab_strip: Some(strip),
        tabs: vec![(1, Rect::from_min_max(pos2(60.0, 0.0), pos2(100.0, 20.0))), (2, Rect::from_min_max(pos2(20.0, 0.0), pos2(60.0, 20.0)))],
        allow_splits: true,
        accepts_tabs: true,
        right_to_left: true,
    };
    let drop = |x: f32| resolve_drop(&9, pos2(x, 10.0), std::slice::from_ref(&target), 40.0).unwrap();
    assert_eq!((drop(90.0).placement, drop(90.0).insertion.unwrap().center().x), (Placement::Tab { before: Some(1) }, 99.0));
    assert_eq!((drop(70.0).placement, drop(70.0).insertion.unwrap().center().x), (Placement::Tab { before: Some(2) }, 60.0));
    assert_eq!((drop(10.0).placement, drop(10.0).insertion.unwrap().center().x), (Placement::Tab { before: None }, 20.0));
}

#[test]
/// In a right-to-left title bar the close and menu buttons sit on the left, the drag surface ends at
/// the bar's right edge, and a galley title is truncated with an ellipsis inside the bar.
fn a_right_to_left_title_bar_puts_its_buttons_left_and_cuts_a_galley_title() {
    let ctx = Context::default();
    let mut shapes = Vec::new();
    let mut rects = (Rect::NOTHING, Rect::NOTHING, Rect::NOTHING);
    let bar = Rect::from_min_size(pos2(10.0, 10.0), vec2(120.0, 18.0));
    for _ in 0..2 {
        let mut full = ctx.run_ui(RawInput::default(), |ui| {
            let mut style = DockStyle::from_ui(ui);
            style.right_to_left = true;
            let chrome = chrome::ChromeStyle::from_dock(&style);
            let title = egui::WidgetText::Galley(ui.painter().layout_no_wrap("Paragraph Styles and More".into(), style.font.clone(), style.text));
            let shown = chrome::TitleBar::new(Id::unique("title"), "Move").title(title).menu("Menu").close("Close").show(ui, bar, &chrome);
            rects = (shown.bar.rect, shown.menu.unwrap().rect, shown.close.unwrap().rect);
        });
        full.textures_delta.clear();
        shapes = full.shapes;
    }
    let (grip, menu, close) = rects;
    assert!(close.center().x < menu.center().x && menu.right() <= grip.left() + 0.5, "{close:?} {menu:?} {grip:?}");
    assert!((grip.right() - bar.right()).abs() < 0.5);
    let title = flat(&shapes)
        .into_iter()
        .find_map(|(_, shape)| match shape {
            egui::Shape::Text(text) if text.galley.text().starts_with("Para") => Some(text),
            _ => None,
        })
        .unwrap();
    assert_eq!(last_char(&title.galley), Some('…'));
    assert!(title.pos.x + title.galley.size().x <= grip.right() + 0.5 && title.pos.x >= grip.left());
}

#[test]
/// With close buttons and close menu items disabled, no close control renders and tab menus contain
/// neither Close Panel nor Close Tab Group; the host can still apply its own CloseWindow action.
fn hidden_close_controls_leave_closing_to_the_host() {
    let layout = Layout { root: Some(tabs(&[1, 2], 0)), floating: vec![Floating::single(vec![5, 6], 0, [80.0, 70.0, 300.0, 220.0])] };
    let close = Id::unique(AREA).with(("move-group", Some(&5u32))).with("close");
    let mut d = Driver::new(layout);
    assert!(d.ctx.read_response(close).is_some());
    d.tweak = |style| {
        style.close_button = false;
        style.close_menu_items = false;
    };
    d.step(vec![]);
    d.step(vec![]);
    assert!(d.ctx.read_response(close).is_none());
    let at = d.tab(1).center();
    let secondary = |pressed| Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    d.step(vec![Event::PointerMoved(at), secondary(true)]);
    d.step(vec![secondary(false)]);
    d.step(vec![]);
    let painted = |d: &Driver, label: &str| texts(d).iter().any(|(_, t)| t.galley.text() == label);
    assert!(painted(&d, "Float Panel"), "the tab menu is open");
    assert!(!painted(&d, "Close Panel") && !painted(&d, "Close Tab Group"));
    // The panels stay closable: the host's own Close applies and reports what it closed.
    assert_eq!(d.layout.apply_with_permissions(Action::CloseWindow { panel: 5 }, |p| d.content.permissions(p)), Ok(vec![5, 6]));
}

#[test]
/// `apply_all` returns every panel closed by close-others and close-window in
/// order, leaving the remaining panel open until a later `apply` closes it.
fn apply_all_returns_every_panel_its_actions_closed() {
    let mut layout = Layout { root: Some(tabs(&[1, 2, 3], 0)), floating: vec![Floating::single(vec![5, 6], 0, [80.0, 70.0, 300.0, 220.0])] };
    let closed = layout
        .apply_all([Action::Activate { panel: 2 }, Action::CloseOthers { panel: 2 }, Action::CloseWindow { panel: 6 }], |_| Permissions::default());
    assert_eq!(closed, Ok(vec![1, 3, 5, 6]));
    assert_eq!(layout.apply(Action::Close { panel: 2 }), Ok(vec![2]));
}

#[test]
/// The column bar button and the column menu item expose distinct accessible names
/// (`Collapse column to icons` versus `Collapse to Icons`).
fn the_column_button_and_the_column_menu_item_have_distinct_names() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        |ui, content: &mut TestContent| {
            DockArea::new(Id::unique(AREA)).show_content(ui, &docked(), &DockStyle::from_ui(ui), content);
        },
        TestContent { protected: vec![0], ..Default::default() },
    );
    h.run_steps(2);
    assert_eq!(h.query_all_by_label("Collapse column to icons").count(), 1);
    assert_eq!(h.query_all_by_label("Collapse to Icons").count(), 0);
    let at = h.ctx.read_response(Id::unique(AREA).with(("column-bar", Some(&2u32)))).unwrap().rect.center();
    h.input_mut().events.extend([
        Event::PointerMoved(at),
        Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(1);
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(h.query_all_by_label("Collapse to Icons").count(), 1);
    assert_eq!(h.query_all_by_label("Collapse column to icons").count(), 1);
}

#[test]
fn for_zone_maps_side_zones_onto_columns_groups_and_splits() {
    let layout = Layout { root: docked().root, floating: vec![Floating::single(vec![5], 0, [300.0, 80.0, 200.0, 200.0])] };
    let zone = |anchor, zone, rtl| Placement::for_zone(&layout, &anchor, zone, rtl);
    assert_eq!(zone(2, Zone::Right, false), Ok(Placement::Column { after: true }));
    assert_eq!(zone(2, Zone::Right, true), Ok(Placement::Column { after: false }));
    assert_eq!(zone(4, Zone::Bottom, false), Ok(Placement::Group { below: true, height: None }));
    assert_eq!(zone(2, Zone::Center, false), Ok(Placement::Split(Zone::Center)));
    assert_eq!(zone(5, Zone::Top, false), Ok(Placement::Split(Zone::Top)));
    assert_eq!(zone(5, Zone::Left, false), Ok(Placement::Split(Zone::Left)));
    assert_eq!(zone(0, Zone::Left, true), Ok(Placement::Split(Zone::Left)));
    assert_eq!(zone(9, Zone::Left, false), Err(DockError::MissingTarget));

    // An end dock mirrors a start dock.
    let mut end = docked();
    let Some(Node::Split { first, second, size, .. }) = &mut end.root else { panic!() };
    std::mem::swap(first, second);
    *size = SplitSize::FixedSecond(240.0);
    let Node::Dock { side, .. } = &mut **second else { panic!() };
    *side = Side::End;
    assert_eq!(Placement::for_zone(&end, &2, Zone::Left, false), Ok(Placement::Column { after: true }));
    assert_eq!(Placement::for_zone(&end, &2, Zone::Left, true), Ok(Placement::Column { after: false }));
}

#[test]
fn a_group_placement_gives_the_inserted_group_its_height() {
    let mut layout = docked();
    let below = |height| Placement::Group { below: true, height };
    layout.apply(Action::OpenAt { panel: 7, anchor: 4, placement: below(Some(180.0)) }).unwrap();
    layout.apply(Action::MoveGroup { panel: 2, to: Target::At { anchor: 7, placement: below(Some(90.0)) } }).unwrap();
    layout.apply(Action::OpenAt { panel: 8, anchor: 4, placement: below(None) }).unwrap();
    let Some(Node::Split { first, .. }) = &layout.root else { panic!() };
    let Node::Dock { columns, .. } = &**first else { panic!() };
    let heights: Vec<_> = columns[0].groups.iter().map(|g| (g.panels.clone(), g.height)).collect();
    assert_eq!(heights, vec![(vec![4], None), (vec![8], None), (vec![7], Some(180.0)), (vec![2, 3], Some(90.0))]);
    assert_eq!(layout.apply(Action::OpenAt { panel: 9, anchor: 4, placement: below(Some(f32::NAN)) }), Err(DockError::InvalidGeometry));
}

struct Icons;

impl DockContent<u32> for Icons {
    fn body(&mut self, _: &mut egui::Ui, _: &u32) {}
}

#[test]
fn every_flyout_change_reports_one_event_whatever_caused_it() {
    use crate::docking::testing::DockDriver;
    let mut column = Column::new(vec![Group::new(vec![1, 2], 0), Group::new(vec![3], 0)], 240.0);
    column.iconic = true;
    let dock = Node::Dock { side: Side::Start, columns: vec![column] };
    let canvas = Node::Tabs { panels: vec![0], active: 0 };
    let layout = Layout {
        root: Some(Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::FixedFirst(240.0), first: Box::new(dock), second: Box::new(canvas) }),
        floating: Vec::new(),
    };
    let mut d = DockDriver::new(layout, Icons);
    let flyouts = |outputs: Vec<DockOutput<u32>>| -> Vec<Option<u32>> {
        outputs
            .into_iter()
            .flat_map(|out| out.events)
            .filter_map(|event| match event {
                DockEvent::FlyoutChanged { panel } => Some(panel),
                _ => None,
            })
            .collect()
    };
    let icon = |d: &DockDriver<u32, Icons>, panel| d.hit(&Hit::Icon(panel)).unwrap().center();
    let quiet = |d: &mut DockDriver<u32, Icons>| (0..3).map(|_| d.step(Vec::new())).collect::<Vec<_>>();

    let (one, two) = (icon(&d, 1), icon(&d, 2));
    assert_eq!(flyouts(vec![d.press(one), d.release(one)]), [Some(1)], "an icon opens it");
    assert_eq!(flyouts(quiet(&mut d)), [], "an open flyout reports nothing more");
    assert_eq!(flyouts(vec![d.press(two), d.release(two)]), [Some(2)], "another icon switches it");
    assert_eq!(flyouts(vec![d.key(egui::Key::Escape)]), [None], "Escape closes it");
    open_flyout(&d.ctx, d.id, 3u32);
    assert_eq!(flyouts(vec![d.step(Vec::new())]), [Some(3)], "open_flyout");
    close_flyout::<u32>(&d.ctx, d.id);
    assert_eq!(flyouts(vec![d.step(Vec::new())]), [None], "close_flyout");
    d.raise = Some(1);
    assert_eq!(flyouts(vec![d.step(Vec::new())]), [Some(1)], "raising an iconic panel opens it");
    let outside = pos2(600.0, 400.0);
    assert_eq!(flyouts(vec![d.press(outside), d.release(outside)]), [None], "a press outside closes it");
    assert_eq!(flyouts(quiet(&mut d)), []);
}
