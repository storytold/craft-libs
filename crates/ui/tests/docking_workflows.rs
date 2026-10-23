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
            floating: vec![Floating { panels: vec![BRUSHES], active: 0, rect: [38.0, 76.0, 210.0, 250.0] }],
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

    fn settle(&mut self) {
        for _ in 0..4 {
            self.step(Vec::new());
        }
    }

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
                assert!(h.layout.floating.iter().any(|g| g.panels.contains(&BRUSHES)), "{name}: {:?}", h.applied);
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
