//! Manual comparison gallery: cargo run -p craft-ui --example shadows -- /tmp/craft-shadows
//! Generates PNGs for human review; there are no approved pixel goldens.

use craft_ui::theme::shadow::{self, Elevation};
use egui::{Color32, CornerRadius, FontId, Pos2, RawInput, Rect, Shadow, Stroke, StrokeKind, pos2, vec2};
use egui_kittest::{TestRenderer, wgpu::WgpuTestRenderer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os().nth(1).map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("craft-shadows"));
    std::fs::create_dir_all(&directory)?;
    for dark in [false, true] {
        for scale in [1.0, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
            let mut renderer = WgpuTestRenderer::new();
            for frame in 0..3 {
                let mut input = RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(960.0, 520.0))),
                    time: Some(frame as f64),
                    ..Default::default()
                };
                input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
                let mut output = ctx.run_ui(input, |ui| {
                    let canvas = if dark { Color32::from_gray(30) } else { Color32::from_gray(245) };
                    let panel = if dark { Color32::from_gray(44) } else { Color32::WHITE };
                    let border = if dark { Color32::from_gray(68) } else { Color32::from_gray(230) };
                    let ink = if dark { Color32::WHITE } else { Color32::BLACK };
                    let color = Color32::from_black_alpha(if dark { 90 } else { 40 });
                    let painter = ui.painter();
                    painter.rect_filled(ui.max_rect(), 0, canvas);
                    for row in 0..10 {
                        for column in 0..24 {
                            if (row + column) % 2 == 0 {
                                let rect = Rect::from_min_size(pos2(column as f32 * 40.0, 280.0 + row as f32 * 24.0), vec2(40.0, 24.0));
                                painter.rect_filled(rect, 0, if dark { Color32::from_gray(42) } else { Color32::from_gray(230) });
                            }
                        }
                    }
                    let source = Shadow { offset: [0, 4], blur: 16, spread: 0, color };
                    let recipes: [(&str, Vec<Shadow>); 3] = [
                        ("Single layer", vec![source]),
                        ("PhotoCraft / Medium", Elevation::Medium.layers(color).into()),
                        ("Floating toolbar", shadow::floating_toolbar(color).into()),
                    ];
                    for (column, (label, layers)) in recipes.iter().enumerate() {
                        let x = 36.0 + column as f32 * 310.0;
                        painter.text(pos2(x, 24.0), egui::Align2::LEFT_TOP, label, FontId::proportional(16.0), ink);
                        for (y, height, radius) in [(90.0, 100.0, 8), (355.0, 48.0, 13)] {
                            let rect = Rect::from_min_size(pos2(x, y), vec2(260.0, height));
                            shadow::paint(painter, rect, CornerRadius::same(radius), layers);
                            painter.rect(rect, radius, panel, Stroke::new(1.0, border), StrokeKind::Inside);
                            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Surface", FontId::proportional(14.0), ink);
                        }
                    }
                });
                renderer.handle_delta(&mut output.textures_delta);
                let image = renderer.render(&ctx, &output)?;
                if frame == 2 {
                    let path = directory.join(format!("{}-{}x.png", if dark { "dark" } else { "light" }, scale as u8));
                    image.save(&path)?;
                    println!("{}", path.display());
                }
            }
        }
    }
    Ok(())
}
