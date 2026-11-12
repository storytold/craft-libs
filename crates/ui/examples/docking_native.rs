//! Docking with floating panels in their own OS windows. Run with
//! `cargo run -p craft-ui --example docking_native --features native-windows`.
//!
//! Prints every action it applies and every native window's place, for checking moves by
//! hand or under a headless X server.

use craft_ui::docking::{DockArea, DockContent, DockStyle, Floating, Layout, Node};
use craft_ui::layout::{SplitAxis, SplitSize};

struct Panels;

impl DockContent<u32> for Panels {
    fn body(&mut self, ui: &mut egui::Ui, panel: &u32) {
        ui.label(format!("Body of panel {panel}"));
    }

    /// Return the example content's logical and accessibility label, `Panel {panel}`.
    fn logical_label(&self, panel: &u32) -> String {
        format!("Panel {panel}")
    }
}

struct App {
    layout: Layout<u32>,
}

impl eframe::App for App {
    /// Render the dock, apply every returned action with default permissions, print floating
    /// window places, and forward residual native-window events to the host.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let style = DockStyle::from_ui(ui);
        let out = DockArea::new(egui::Id::unique("dock")).show_content(ui, &self.layout, &style, &mut Panels);
        if !out.actions.is_empty() {
            println!("actions {:?}", out.actions);
            if let Err(error) = self.layout.apply_all(out.actions, |_| Default::default()) {
                println!("rejected {error:?}");
            }
            for window in &self.layout.floating {
                println!("window {} panels {:?} native {:?}", window.id, window.panels().collect::<Vec<_>>(), window.native);
            }
        }
        if !out.forwarded_events.is_empty() {
            println!("forwarded {:?} {:?}", out.forwarded_events, out.forwarded_modifiers);
        }
    }
}

fn main() -> eframe::Result {
    let tabs = |panels: &[u32]| Node::Tabs { panels: panels.to_vec(), active: 0 };
    let layout = Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::Ratio(0.5),
            first: Box::new(tabs(&[1, 2])),
            second: Box::new(tabs(&[3, 4])),
        }),
        floating: vec![Floating::single(vec![5, 6], 0, [300.0, 200.0, 320.0, 260.0]), Floating::single(vec![7], 0, [700.0, 300.0, 260.0, 200.0])],
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1000.0, 700.0]).with_position([0.0, 0.0]),
        ..Default::default()
    };
    eframe::run_native("Docking", options, Box::new(|_| Ok(Box::new(App { layout }))))
}
