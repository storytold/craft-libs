//! Docking (20.6): panels and groups dragged through the drag session land on the group and side the pointer is over.

use craft_theme::{Contrast, ControlSize, Mode, Variant};
use craft_ui::docking::DockSizes;
use craft_ui::testing::style::{tokens, with_cx};

pub(crate) mod render;

pub(crate) mod focus;
pub(crate) mod panel_menu;
pub(crate) mod runs;

pub(crate) mod split;

pub(crate) mod drop;
pub(crate) mod drop_preview;
pub(crate) mod drop_session;

pub(crate) mod float_order;
pub(crate) mod floating;
pub(crate) mod floating_area;
pub(crate) mod floating_rail;
pub(crate) mod hidden;
pub(crate) mod sources;

pub(crate) mod app_modal;
pub(crate) mod columns;
pub(crate) mod dock_back;
pub(crate) mod documents;
pub(crate) mod group_view;
pub(crate) mod host_body;
pub(crate) mod host_keys;
pub(crate) mod host_runs;
pub(crate) mod tab_focus;
pub(crate) mod workspace_dialogs;

pub(crate) mod native;
pub(crate) mod native_drag;
pub(crate) mod native_float;
pub(crate) mod panes;

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
