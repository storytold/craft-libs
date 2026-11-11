//! The dock: what the dock asks of the viewer for each panel of the shown workspace.

use super::panels::Panels;
use crate::viewer::{GalleryCommands, PanelId, Preview};
use craft_behavior::dock::Permissions;
use craft_theme::variant::Tone;
use craft_ui::contract::Show as _;
use craft_ui::contract::{BodyCx, SizeLimits};
use craft_ui::docking::{DockHeights, DockPanels, PanelSpec};
use craft_ui::menu::PanelRow;
use craft_ui::notices::Notice;
use egui::Ui;

/// The viewer's panels as the dock's contents.
#[non_exhaustive]
pub struct GalleryDock<'a> {
    /// What the panel bodies read and edit.
    pub panels: Panels<'a>,
    /// What the theme workspace's preview shows.
    pub preview: Preview,
    /// The mock application the App preview shows.
    pub app: &'a mut crate::mock::MockState,
}

impl DockPanels<PanelId> for GalleryDock<'_> {
    fn spec(&self, panel: &PanelId) -> PanelSpec {
        let spec = PanelSpec::new(panel.title()).limits(SizeLimits::default());
        match panel {
            PanelId::Stories => spec.heights(DockHeights::new(80.0).compact(60.0)),
            PanelId::Properties | PanelId::Inspect | PanelId::Tokens => spec.heights(DockHeights::new(240.0).compact(120.0).fill(300.0)),
            PanelId::Problems | PanelId::Detail | PanelId::AppSizes => spec.heights(DockHeights::new(210.0)),
            _ => spec,
        }
    }

    fn permissions(&self, panel: &PanelId) -> Permissions {
        panel.permissions()
    }

    fn body(&mut self, panel: &PanelId, ui: &mut Ui, _: &mut BodyCx<'_>) {
        match (panel, self.preview) {
            (PanelId::Preview, Preview::App) => {
                crate::mock::EditorMock::new("Preview", self.app).show(ui);
            }
            (PanelId::Preview, Preview::Components) => {
                Notice::new(Tone::Info, "The Controls level comes with the overview.").show(ui);
            }
            _ => self.panels.show(ui, *panel),
        }
    }

    /// Reset args and Reset layout in the menu of the group that holds the target story's panels.
    fn panel_rows(&mut self, panel: &PanelId) -> Vec<PanelRow> {
        match panel {
            PanelId::Properties | PanelId::Inspect | PanelId::Tokens => {
                vec![PanelRow::Command(GalleryCommands::ResetStories.into()), PanelRow::Command(GalleryCommands::ResetLayout.into())]
            }
            _ => Vec::new(),
        }
    }
}
