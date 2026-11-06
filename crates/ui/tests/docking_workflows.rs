//! Content-bearing offscreen acceptance of the shared docking surface. No OS windows are opened.
//! Set CRAFT_UI_DOCK_VISUAL_DIR to retain every stage of the pointer-driven workflow.

use std::{collections::BTreeMap, path::PathBuf};

use craft_ui::{
    docking::{Action, DockArea, DockStyle, Floating, Layout, Node, Permissions},
    layout::{SplitAxis, SplitSize},
};
use egui::{Color32, Context, Event, FullOutput, Id, Modifiers, PointerButton, Pos2, RawInput, Rect, Stroke, pos2, vec2};
use egui_kittest::{TestRenderer, wgpu::WgpuTestRenderer};

const CANVAS: u32 = 0;
const LAYERS: u32 = 1;
const PROPERTIES: u32 = 2;
const BRUSHES: u32 = 3;

fn label(panel: &u32) -> String {
    match *panel {
        CANVAS => "Document",
        LAYERS => "Layers",
        PROPERTIES => "Properties",
        BRUSHES => "Brushes",
        _ => "Unknown",
    }
    .into()
}

struct Harness {
    ctx: Context,
    renderer: WgpuTestRenderer,
    layout: Layout<u32>,
    output: FullOutput,
    width: f32,
    frame: usize,
    values: BTreeMap<u32, String>,
    body_ids: BTreeMap<u32, Id>,
    edit_ids: BTreeMap<u32, Id>,
    applied: Vec<Action<u32>>,
}

impl Harness {
    /// Create a workflow harness with the requested theme, width and pixel scale,
    /// zero animation and no cursor blink, and a canvas plus side tabs and floating
    /// brushes layout, then settle it.
    fn new(dark: bool, width: f32, scale: f32) -> Self {
        let ctx = Context::default();
        ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
        ctx.set_pixels_per_point(scale);
        ctx.all_styles_mut(|style| {
            style.animation_time = 0.0;
            style.visuals.text_cursor.blink = false;
        });
        let layout = Layout {
            root: Some(Node::Split {
                axis: SplitAxis::Horizontal,
                size: SplitSize::FixedSecond(240.0),
                first: Box::new(Node::Tabs { panels: vec![CANVAS], active: 0 }),
                second: Box::new(Node::Tabs { panels: vec![LAYERS, PROPERTIES], active: 0 }),
            }),
            floating: vec![Floating::single(vec![BRUSHES], 0, [38.0, 76.0, 210.0, 250.0])],
        };
        let mut harness = Self {
            ctx,
            renderer: WgpuTestRenderer::new(),
            layout,
            output: FullOutput::default(),
            width,
            frame: 0,
            values: BTreeMap::from([
                (BRUSHES, "Soft round 24 px".into()),
                (PROPERTIES, "72% opacity".into()),
                (LAYERS, "Foreground selected".into()),
            ]),
            body_ids: BTreeMap::new(),
            edit_ids: BTreeMap::new(),
            applied: Vec::new(),
        };
        harness.settle();
        harness
    }

    /// One 520 pt-high frame through the real wgpu test renderer. Applies returned actions with
    /// canvas protection (round-tripping them through JSON when `serde` is enabled), validates the
    /// layout, and records body ids and applied actions.
    fn step(&mut self, events: Vec<Event>) {
        self.frame += 1;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(self.width, 520.0))),
            time: Some(self.frame as f64 / 60.0),
            events,
            focused: true,
            max_texture_side: Some(4096),
            ..Default::default()
        };
        let mut actions = Vec::new();
        self.output = self.ctx.run_ui(input, |ui| {
            let style = DockStyle::from_ui(ui);
            let out = DockArea::new(Id::unique("workflow")).show(
                ui,
                &self.layout,
                &style,
                label,
                |panel| if *panel == CANVAS { Permissions::PROTECTED } else { Permissions::default() },
                |ui, panel| {
                    self.body_ids.insert(*panel, ui.scope_id());
                    if *panel == CANVAS {
                        let rect = ui.available_rect_before_wrap();
                        ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(44, 53, 66));
                        let page = rect.shrink(24.0);
                        if page.is_positive() {
                            ui.painter().rect_filled(page, 1.0, Color32::from_rgb(243, 239, 225));
                            ui.painter().text(
                                page.min + vec2(12.0, 28.0),
                                egui::Align2::LEFT_CENTER,
                                "SYNTHETIC LAYOUT",
                                egui::FontId::proportional(18.0),
                                Color32::from_rgb(26, 37, 57),
                            );
                            for row in 0..5 {
                                let y = page.top() + 68.0 + row as f32 * 24.0;
                                ui.painter().line_segment(
                                    [pos2(page.left() + 12.0, y), pos2(page.right() - 12.0, y)],
                                    Stroke::new(4.0, Color32::from_rgb(122, 138, 151)),
                                );
                            }
                            let art = Rect::from_min_max(page.min + vec2(12.0, 220.0), page.max - vec2(12.0, 20.0));
                            if art.is_positive() {
                                ui.painter().rect_filled(art, 6.0, Color32::from_rgb(94, 143, 139));
                            }
                        }
                    } else {
                        egui::ScrollArea::vertical().id_salt(("content-scroll", panel)).show(ui, |ui| {
                            ui.add_space(8.0);
                            ui.heading(label(panel));
                            if let Some(value) = self.values.get_mut(panel) {
                                let response = ui.text_edit_singleline(value);
                                self.edit_ids.insert(*panel, response.id);
                            }
                            ui.separator();
                            if *panel == BRUSHES {
                                for row in 1..=6 {
                                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), egui::Sense::hover());
                                    ui.painter().circle_filled(rect.left_center() + vec2(14.0, 0.0), row as f32 + 2.0, ui.visuals().text_color());
                                    ui.painter().text(
                                        rect.left_center() + vec2(32.0, 0.0),
                                        egui::Align2::LEFT_CENTER,
                                        format!("Brush preset {row}"),
                                        egui::FontId::proportional(12.0),
                                        ui.visuals().text_color(),
                                    );
                                }
                            } else if *panel == LAYERS {
                                for text in ["Foreground", "Title", "Shapes", "Background"] {
                                    ui.label(text);
                                }
                            } else {
                                ui.label("Width: 320 px");
                                ui.label("Height: 180 px");
                                ui.label("Blend: Normal");
                                ui.add(egui::ProgressBar::new(0.72).text("Opacity 72%"));
                            }
                        });
                    }
                },
            );
            assert!(out.error.is_none(), "{:?}", out.error);
            actions = out.actions;
        });
        self.renderer.handle_delta(&mut self.output.textures_delta);
        for action in actions {
            // Apps send renderer actions through their JSON command boundary before applying.
            #[cfg(feature = "serde")]
            let action: Action<u32> = serde_json::from_value(serde_json::to_value(&action).unwrap()).unwrap();
            self.layout
                .apply_with_permissions(action.clone(), |panel| if *panel == CANVAS { Permissions::PROTECTED } else { Permissions::default() })
                .unwrap();
            self.applied.push(action);
        }
        self.layout.validate().unwrap();
    }

    /// Run four empty workflow frames to settle sizing.
    fn settle(&mut self) {
        for _ in 0..4 {
            self.step(Vec::new());
        }
    }

    /// The response rect of `panel`'s tab in the workflow dock.
    fn tab(&self, panel: u32) -> Rect {
        self.ctx.read_response(Id::unique("workflow").with(("tab", &panel))).unwrap().rect
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        let pointer = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.step(vec![Event::PointerMoved(from), pointer(from, true)]);
        self.step(vec![Event::PointerMoved(from + vec2(8.0, 0.0))]);
        self.step(vec![Event::PointerMoved(to)]);
        self.step(vec![pointer(to, false)]);
        self.settle();
    }

    fn edit_brush_value(&mut self) {
        let point = self.ctx.read_response(self.edit_ids[&BRUSHES]).unwrap().rect.center();
        let pointer = |pressed| Event::PointerButton { pos: point, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.step(vec![Event::PointerMoved(point), pointer(true)]);
        self.step(vec![pointer(false)]);
        self.step(vec![
            Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND },
            Event::Text("Edited soft round 36 px".into()),
        ]);
        self.step(vec![Event::Key { key: egui::Key::A, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::COMMAND }]);
        assert_eq!(self.values[&BRUSHES], "Edited soft round 36 px");
    }

    fn capture(&mut self, directory: &Option<PathBuf>, name: &str) {
        if let Some(directory) = directory {
            std::fs::create_dir_all(directory).unwrap();
            let image = self.renderer.render(&self.ctx, &self.output).unwrap();
            image.save(directory.join(format!("{name}.png"))).unwrap();
        }
    }
}

#[test]
fn floating_brushes_merge_detach_and_redock_through_real_pointer_input() {
    let output = std::env::var_os("CRAFT_UI_DOCK_VISUAL_DIR").map(PathBuf::from);
    for dark in [true, false] {
        for width in [640.0, 1000.0] {
            for scale in [1.0, 2.0] {
                let name = format!("{}-{}-{}x", if dark { "dark" } else { "light" }, width as u32, scale as u32);
                let directory = output.as_ref().map(|p| p.join(&name));
                let mut h = Harness::new(dark, width, scale);
                h.edit_brush_value();
                h.settle();
                h.capture(&directory, "01-floating-brushes");
                let original_values = h.values.clone();
                let brush_id = h.body_ids[&BRUSHES];
                h.drag(h.tab(BRUSHES).center(), h.tab(PROPERTIES).left_center() + vec2(4.0, 0.0));
                assert!(h.applied.iter().any(|a| matches!(a, Action::Move { panel: BRUSHES, .. })), "{name}: {:?}", h.applied);
                assert!(h.layout.floating.is_empty(), "{name}");
                assert_eq!(h.body_ids[&BRUSHES], brush_id);
                h.capture(&directory, "02-merged-main-tabs");
                h.drag(h.tab(BRUSHES).center(), pos2((width - 240.0) * 0.5, 270.0));
                assert!(h.layout.floating.iter().any(|g| g.contains(&BRUSHES)), "{name}: {:?}", h.applied);
                h.capture(&directory, "03-detached-again");
                h.drag(h.tab(BRUSHES).center(), h.tab(LAYERS).left_center() + vec2(4.0, 0.0));
                assert!(h.layout.floating.is_empty(), "{name}");
                assert_eq!(h.body_ids[&BRUSHES], brush_id);
                assert_eq!(h.values, original_values);
                assert_eq!(h.layout.panels().len(), 4);
                h.capture(&directory, "04-redocked");
                if let Some(directory) = &directory {
                    std::fs::write(directory.join("actions.txt"), format!("{:#?}\n", h.applied)).unwrap();
                }
            }
        }
    }
}

/// Each app's saved v1 layout goes through a docking session driven by real pointer input and a
/// command, then writes and reads back unchanged.
#[cfg(all(feature = "serde", feature = "testing"))]
mod v1_fixtures {
    use craft_ui::docking::{DockCommand, DockContent, Hit, Layout, Node, Permissions, ResizeEdge, Side, testing::DockDriver};
    use egui::{Pos2, Rect, pos2};
    use serde_json::{Value, json};

    struct Content;

    impl DockContent<String> for Content {
        fn body(&mut self, ui: &mut egui::Ui, panel: &String) {
            ui.label(panel.as_str());
        }
        fn logical_label(&self, panel: &String) -> String {
            panel.clone()
        }
        fn permissions(&self, panel: &String) -> Permissions {
            if panel == "canvas" { Permissions::PROTECTED } else { Permissions::default() }
        }
    }

    type Driver = DockDriver<String, Content>;
    type Columns = Vec<Vec<Vec<String>>>;

    /// App, file contents and the JSON pointer to a saved layout.
    const FIXTURES: [(&str, &str, &str); 10] = [
        ("designcraft", include_str!("fixtures/v1/designcraft.json"), "/docking"),
        ("designcraft", include_str!("fixtures/v1/designcraft.json"), "/customWorkspaces/0/docking"),
        ("vectorcraft", include_str!("fixtures/v1/vectorcraft.json"), "/docking"),
        ("lightcraft", include_str!("fixtures/v1/lightcraft.json"), "/docking/layout"),
        ("pdfcraft", include_str!("fixtures/v1/pdfcraft.json"), "/docking/layout"),
        ("cadcraft", include_str!("fixtures/v1/cadcraft.json"), "/workspace/docking/layout"),
        ("soundcraft", include_str!("fixtures/v1/soundcraft.json"), "/docking/layout"),
        ("soundcraft", include_str!("fixtures/v1/soundcraft.json"), "/mix_docking/layout"),
        ("photocraft", include_str!("fixtures/v1/photocraft.json"), "/docking"),
        ("photocraft", include_str!("fixtures/v1/photocraft.json"), "/panel_layout/docking"),
    ];

    /// The start dock's columns, each a list of groups.
    fn start_columns(layout: &Layout<String>) -> Columns {
        fn find(node: &Node<String>) -> Option<Columns> {
            match node {
                Node::Dock { side: Side::Start, columns } => {
                    Some(columns.iter().map(|column| column.groups.iter().map(|group| group.panels.clone()).collect()).collect())
                }
                Node::Split { first, second, .. } => find(first).or_else(|| find(second)),
                _ => None,
            }
        }
        layout.root.as_ref().and_then(find).unwrap_or_default()
    }

    fn iconic(layout: &Layout<String>) -> bool {
        fn find(node: &Node<String>) -> Option<bool> {
            match node {
                Node::Dock { side: Side::Start, columns } => columns.first().map(|column| column.iconic),
                Node::Split { first, second, .. } => find(first).or_else(|| find(second)),
                _ => None,
            }
        }
        layout.root.as_ref().and_then(find).unwrap_or(false)
    }

    fn hit(d: &Driver, hit: Hit<String>) -> Rect {
        d.hit(&hit).unwrap_or_else(|| panic!("{hit:?} in {:?}", d.hits()))
    }

    /// A point of `from` that no floating window above it covers, as drawn last frame.
    fn grab(d: &Driver, from: &Hit<String>, panel: &String) -> Option<Pos2> {
        let rect = d.hit(from)?;
        let own = d.layout.floating.iter().position(|w| w.contains(panel));
        let windows: Vec<Rect> = d
            .layout
            .floating
            .iter()
            .filter_map(|w| {
                let first = w.panels().next()?.clone();
                Some(Rect::from_min_max(d.hit(&Hit::Title(first.clone()))?.min, d.hit(&Hit::Resize(first, ResizeEdge::BottomRight))?.max))
            })
            .collect();
        let covered = |p: Pos2| windows.iter().enumerate().any(|(i, w)| own.is_none_or(|own| i > own) && w.contains(p));
        let mut x = rect.left() + 4.0;
        while x < rect.right() {
            let p = pos2(x, rect.center().y);
            if !covered(p) {
                return Some(p);
            }
            x += 4.0;
        }
        None
    }

    /// Drag from an uncovered point of `from` to `to`; something must change.
    fn drag(d: &mut Driver, from: Hit<String>, to: Pos2, what: &str) {
        let panel = match &from {
            Hit::DragBar(p) | Hit::Title(p) => p.clone(),
            other => panic!("{other:?}"),
        };
        let at = grab(d, &from, &panel).unwrap_or_else(|| panic!("{what}: {from:?} is covered"));
        let out = d.drag(at, to);
        assert!(out.error.is_none() && !out.actions.is_empty(), "{what}: {out:?}\n{:?}\n{:?}", d.layout, d.hits());
        d.settle();
    }

    fn sorted(layout: &Layout<String>) -> Vec<String> {
        let mut panels: Vec<String> = layout.panels().into_iter().cloned().collect();
        panels.sort();
        panels
    }

    #[test]
    fn v1_layouts_dock_collapse_float_redock_and_reload() {
        let mut seams = Vec::new();
        for (app, text, pointer) in FIXTURES {
            let at = format!("{app}{pointer}");
            let file: Value = serde_json::from_str(text).unwrap();
            let layout: Layout<String> = serde_json::from_value(file.pointer(pointer).unwrap().clone()).unwrap_or_else(|e| panic!("{at}: {e}"));
            let panels = sorted(&layout);
            let mut d = Driver::new(layout, Content);
            let mut bars: Vec<String> = Vec::new();
            for (h, _) in d.hits() {
                if let Hit::DragBar(panel) = h
                    && !bars.contains(&panel)
                {
                    bars.push(panel);
                }
            }
            assert!(bars.len() >= 2, "{at}: two movable groups in {bars:?}");

            // The first group's bar dropped on the window's start edge makes a dock.
            drag(&mut d, Hit::DragBar(bars[0].clone()), pos2(2.0, 300.0), &format!("{at}: new dock"));
            let columns = start_columns(&d.layout);
            assert!(columns.len() == 1 && columns[0].len() == 1 && columns[0][0][0] == bars[0], "{at}: {columns:?}");

            // The second group stacks below it, and a third goes into the seam between them.
            let first = hit(&d, Hit::Tab(bars[0].clone()));
            let bottom = d.area.bottom();
            drag(&mut d, Hit::DragBar(bars[1].clone()), pos2(first.center().x, bottom - 10.0), &format!("{at}: stack below"));
            let third = bars[2..].iter().find(|p| grab(&d, &Hit::DragBar((*p).clone()), p).is_some()).cloned();
            if let Some(third) = &third {
                seams.push(at.clone());
                let second = hit(&d, Hit::Tab(bars[1].clone()));
                drag(&mut d, Hit::DragBar(third.clone()), pos2(second.center().x, second.top() + 2.0), &format!("{at}: seam"));
            }
            let columns = start_columns(&d.layout);
            let heads: Vec<&String> = columns[0].iter().map(|group| &group[0]).collect();
            let expected: Vec<&String> = match &third {
                Some(third) => vec![&bars[0], third, &bars[1]],
                None => vec![&bars[0], &bars[1]],
            };
            assert_eq!((columns.len(), heads), (1, expected), "{at}");

            // Collapse the column to icons and expand it again.
            d.click(hit(&d, Hit::Collapse(bars[0].clone())).center());
            d.settle();
            assert!(iconic(&d.layout), "{at}: collapsed");
            assert!(d.hit(&Hit::Icon(bars[0].clone())).is_some(), "{at}: icons");
            d.click(hit(&d, Hit::Collapse(bars[0].clone())).center());
            d.settle();
            assert!(!iconic(&d.layout), "{at}: expanded");

            // Float the column by command, then drag its window back to the start edge.
            let float: DockCommand<String> =
                serde_json::from_value(json!({"command": "window.panel.float", "panel": bars[0], "column": true})).unwrap();
            let space = craft_ui::docking::float_space(&d.ctx, d.id).unwrap();
            let actions =
                float.to_actions_with(&craft_ui::docking::Workspace::new(d.layout.clone()), false, &space, |p| d.content.permissions(p)).unwrap();
            d.layout.apply_all(actions, |p| d.content.permissions(p)).unwrap_or_else(|e| panic!("{at}: {e}"));
            d.settle();
            assert!(start_columns(&d.layout).is_empty(), "{at}: floated");
            drag(&mut d, Hit::Title(bars[0].clone()), pos2(2.0, 300.0), &format!("{at}: dock back"));
            assert_eq!(start_columns(&d.layout), columns, "{at}: docked back");

            // Nothing was lost, and the layout reads back as written.
            assert_eq!(sorted(&d.layout), panels, "{at}");
            let written = serde_json::to_string(&d.layout).unwrap();
            let read: Layout<String> = serde_json::from_str(&written).unwrap();
            read.validate().unwrap_or_else(|e| panic!("{at}: {e}"));
            assert_eq!(read, d.layout, "{at}");
        }
        // The fixtures with a third movable group, uncovered, try the seam.
        assert_eq!(seams.len(), 6, "{seams:?}");
    }
}
