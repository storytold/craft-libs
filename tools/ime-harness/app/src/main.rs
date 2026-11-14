//! One multiline `TextEdit` for the IME harness.
//!
//! The text is written to `$IME_HARNESS_OUT` after every change and on exit, so the harness can
//! read the committed text without closing the window. Key, text and IME events are logged to
//! stderr; `IME_HARNESS_LOG_ALL=1` logs every event.

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result {
    // winit logs through `tracing`; RUST_LOG=winit=trace shows its XIM and text-input traffic.
    if std::env::var_os("RUST_LOG").is_some() {
        tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_writer(std::io::stderr).init();
    }
    let out = std::env::var_os("IME_HARNESS_OUT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("ime-harness-out.txt"));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("ime-harness").with_app_id("ime-harness").with_inner_size([480.0, 200.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native("ime-harness", options, Box::new(move |_cc| Ok(Box::new(App::new(out)))))
}

struct App {
    text: String,
    out: PathBuf,
    window_focused: bool,
    focus_requested: bool,
    ready_logged: bool,
    log_all_events: bool,
}

impl App {
    fn new(out: PathBuf) -> Self {
        let app = Self {
            text: String::new(),
            out,
            window_focused: false,
            focus_requested: false,
            ready_logged: false,
            log_all_events: std::env::var_os("IME_HARNESS_LOG_ALL").is_some(),
        };
        app.write_out();
        app
    }

    fn write_out(&self) {
        // Write then rename, so the harness never reads a partial file.
        let tmp = self.out.with_extension("tmp");
        if let Err(err) = std::fs::write(&tmp, &self.text).and_then(|()| std::fs::rename(&tmp, &self.out)) {
            eprintln!("ime-harness: cannot write {}: {err}", self.out.display());
        }
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        for event in &raw_input.events {
            match event {
                egui::Event::WindowFocused(focused) => {
                    self.window_focused = *focused;
                    eprintln!("ime-harness: event {event:?}");
                }
                egui::Event::Key { pressed: true, .. } | egui::Event::Text(_) | egui::Event::Ime(_) => eprintln!("ime-harness: event {event:?}"),
                _ if self.log_all_events => eprintln!("ime-harness: event {event:?}"),
                _ => {}
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            // Fills the window, so the harness can click anywhere to hit it.
            let response = ui.add_sized(ui.available_size(), egui::TextEdit::multiline(&mut self.text).id_salt("ime-harness-text"));
            // Only once: every `request_focus` also interrupts the IME composition.
            if !self.focus_requested {
                self.focus_requested = true;
                response.request_focus();
            }
            if response.changed() {
                eprintln!("ime-harness: text {:?}", self.text);
                self.write_out();
            }
            if response.has_focus() && self.window_focused && !self.ready_logged {
                self.ready_logged = true;
                eprintln!("ime-harness: ready");
            }
        });
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.write_out();
    }
}
