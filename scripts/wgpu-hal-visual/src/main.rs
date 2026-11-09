//! Windows-only regression harness; never linked into the Craft Apps.
mod capture;
mod compare;

use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

const STATES: [&str; 4] = ["restored", "resized", "maximized", "restored-again"];
type Outcome = Arc<Mutex<Option<Result<Value, String>>>>;

struct Test {
    ticks: u32,
    stage: usize,
    pending: Option<mpsc::Receiver<Result<capture::Info, String>>>,
    directory: PathBuf,
    internal_size: Option<[usize; 2]>,
    records: Vec<Value>,
    started: Instant,
    outcome: Outcome,
    adapter: Value,
    done: bool,
}

impl Test {
    fn step(&mut self, ctx: &egui::Context, frame: &eframe::Frame) -> Result<bool, String> {
        if self.started.elapsed() > Duration::from_secs(25) {
            return Err(format!("Timed out capturing {}", STATES[self.stage]));
        }
        for event in ctx.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| [c.r(), c.g(), c.b()]).collect();
                image::save_buffer(
                    self.directory.join(format!("{}-internal.png", STATES[self.stage])),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgb8,
                )
                .map_err(|error| error.to_string())?;
                self.internal_size = Some(image.size);
            }
        }
        self.ticks += 1;
        if let Some(receiver) = &self.pending {
            let info = match receiver.try_recv() {
                Ok(result) => result?,
                Err(mpsc::TryRecvError::Empty) => return Ok(false),
                Err(mpsc::TryRecvError::Disconnected) => return Err("Capture worker disconnected".into()),
            };
            self.records.push(json!({
                "state": STATES[self.stage], "window_size": info.window_size,
                "client_size": info.client_size, "client_offset": info.client_offset,
                "framebuffer_size": self.internal_size, "pixels_per_point": ctx.pixels_per_point(),
            }));
            self.stage += 1;
            self.pending = None;
            self.ticks = 0;
            self.internal_size = None;
            self.started = Instant::now();
            match self.stage {
                1 => ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1000.0, 750.0))),
                2 => ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true)),
                3 => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1440.0, 900.0)));
                }
                _ => return Ok(true),
            }
        } else if self.ticks == 30 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        } else if self.ticks >= 45 && self.internal_size.is_some() {
            let handle = frame.window_handle().map_err(|error| error.to_string())?;
            let RawWindowHandle::Win32(raw) = handle.as_raw() else {
                return Err("A Windows HWND is required".into());
            };
            println!("Capturing {} at {} pixels per point", STATES[self.stage], ctx.pixels_per_point());
            self.pending = Some(capture::spawn(raw.hwnd.get(), self.directory.join(format!("{}-presented.png", STATES[self.stage]))));
        }
        Ok(false)
    }

    fn finish(&mut self, ctx: &egui::Context, result: Result<(), String>) {
        self.done = true;
        let report = result.map(|()| {
            json!({
                "status": "CAPTURED", "adapter": self.adapter, "states": self.records,
                "renderer": "eframe/egui 0.36.2, winit 0.30.13, wgpu 30.0.1",
            })
        });
        if let Ok(mut outcome) = self.outcome.lock() {
            *outcome = Some(report);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for Test {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.done {
            return;
        }
        let ctx = ui.ctx().clone();
        ui.add_space(24.0);
        ui.label(egui::RichText::new("DX12 presentation — real egui text").size(28.0));
        ui.add_space(12.0);
        ui.label("Same renderer versions as the original FigCraft 0.6.0 test.");
        for size in [12.0, 14.0, 16.0, 18.0, 22.0, 28.0] {
            ui.add_space(12.0);
            ui.label(egui::RichText::new(format!("{size}px  The quick brown fox jumps over the lazy dog. 0123456789")).size(size));
        }
        ui.add_space(18.0);
        ui.label("One-pixel lines (physical pixels):");
        let ppp = ctx.pixels_per_point();
        let start = ui.cursor().min;
        for x in 0..350 {
            let a = start + egui::vec2(x as f32 * 2.0 / ppp, 0.0);
            ui.painter().line_segment([a, a + egui::vec2(0.0, 80.0)], egui::Stroke::new(1.0 / ppp, egui::Color32::WHITE));
        }
        match self.step(&ctx, frame) {
            Ok(true) => self.finish(&ctx, Ok(())),
            Err(error) => self.finish(&ctx, Err(error)),
            Ok(false) => ctx.request_repaint_after(Duration::from_millis(16)),
        }
    }
}

fn run_capture(directory: &Path) -> Result<(), String> {
    if directory.exists() && std::fs::read_dir(directory).map_err(|e| e.to_string())?.next().is_some() {
        return Err("Capture directory must be empty: stale images must not pass a new run".into());
    }
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let outcome: Outcome = Arc::new(Mutex::new(None));
    let output = outcome.clone();
    let destination = directory.to_owned();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_position([80.0, 80.0])
            .with_decorations(false)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false),
        ..Default::default()
    };
    eframe::run_native(
        "DX12 egui presentation test",
        options,
        Box::new(move |cc| {
            let state = cc.wgpu_render_state.as_ref().ok_or_else(|| std::io::Error::other("Missing wgpu renderer"))?;
            let info = state.adapter.get_info();
            if info.backend != wgpu::Backend::Dx12 {
                return Err(std::io::Error::other("Expected DX12 backend").into());
            }
            let adapter = json!({"name": info.name, "driver": info.driver, "backend": "Dx12"});
            println!("Adapter: {adapter}");
            Ok(Box::new(Test {
                ticks: 0,
                stage: 0,
                pending: None,
                directory: destination,
                internal_size: None,
                records: Vec::new(),
                started: Instant::now(),
                outcome: output,
                adapter,
                done: false,
            }))
        }),
    )
    .map_err(|error| error.to_string())?;
    let report = outcome.lock().map_err(|_| "Capture outcome unavailable")?.take().ok_or("Window closed before all four captures completed")??;
    let data = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(directory.join("capture.json"), data).map_err(|error| error.to_string())?;
    println!("CAPTURED: all four window states");
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    let result = match (args.get(1).and_then(|s| s.to_str()), args.get(2)) {
        (Some("capture"), Some(path)) => run_capture(Path::new(path)).map(|()| 0),
        (Some("compare"), Some(path)) => compare::run(Path::new(path)),
        _ => Err("usage: dx12-egui-smoke capture DIRECTORY | compare RUN_DIRECTORY".into()),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
    }
}
