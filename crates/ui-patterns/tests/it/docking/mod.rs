//! Docking (20.6): panels and groups dragged through the drag session land on the group and side the pointer is over.

use craft_theme::{Contrast, ControlSize, Mode, Variant};
use craft_ui::docking::DockSizes;
use craft_ui::testing::style::{tokens, with_cx};

mod render;

mod panel_menu;
mod runs;

mod split;

mod drop;
mod drop_preview;
mod drop_session;

mod floating;
mod floating_area;
mod hidden;

mod columns;
mod dock_back;
mod group_view;
mod workspace_dialogs;

mod native;
mod panes;

/// The docking sizes of the default theme with the given divider gap, least pane and strip height.
fn sizes(gap: f32, least_pane: f32, strip: f32) -> DockSizes {
    with_cx(&egui::Context::default(), &tokens(Mode::Light, Contrast::Standard), Variant::SECONDARY, ControlSize::Md, |cx| {
        let mut sizes = DockSizes::style(cx);
        sizes.gap = gap;
        sizes.least_pane = least_pane;
        sizes.strip = strip;
        sizes
    })
}

/// Panel specs that name each panel by its number ("Panel 3"), with every permission.
#[derive(craft_ui::contract::PlainData)]
struct Numbered;

impl<P: std::fmt::Display> craft_ui::docking::PanelSpecs<P> for Numbered {
    fn spec(&self, panel: &P) -> craft_ui::docking::PanelSpec {
        craft_ui::docking::PanelSpec::new(format!("Panel {panel}"))
    }
}
