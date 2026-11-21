//! The dock: what the dock asks of the viewer for each panel of the shown workspace.

use super::inspect::{self, Inspect};
use super::overview::{self, Centre};
use super::panels::{self, ThemePanels};
use super::variants::{VariantsEvent, VariantsIn};
use crate::viewer::{Browser, GalleryCommands, PanelId, Preview};
use craft_behavior::dock::Permissions;
use craft_theme::variant::Tone;
use craft_ui::contract::Show as _;
use craft_ui::contract::{BodyCx, Outbox, SizeLimits, Tracked};
use craft_ui::display::InspectLayers;
use craft_ui::docking::{DockHeights, DockPanels, PanelSpec, PanelSpecs};
use craft_ui::menu::PanelRow;
use craft_ui::notices::Notice;
use craft_window::policy::WindowKind;
use egui::Ui;

/// The viewer's panels as the dock's contents. The panels' requests that change the viewer beyond the browser's state
/// collect in `opened` and `variants_out` for the shell to apply after the dock.
#[non_exhaustive]
pub struct GalleryDock<'a> {
    /// What the panels browse.
    pub browser: &'a mut Browser,
    /// Where the inspector is on.
    pub inspect: Inspect,
    /// The drawings the inspector shows; the Inspect panel edits them.
    pub layers: &'a mut Tracked<InspectLayers>,
    /// The centre tab an open Variants view is in, and the view.
    pub variants: Option<(PanelId, VariantsIn)>,
    /// The theme workspace's panels.
    pub theme: ThemePanels<'a>,
    /// What the theme workspace's preview shows.
    pub preview: Preview,
    /// The mock application the App preview shows.
    pub app: &'a mut crate::mock::MockState,
    /// The components whose tab a panel asked to open in this pass.
    pub opened: Vec<&'static str>,
    /// What the Variants view asked for in this pass.
    pub variants_out: Vec<VariantsEvent>,
}

impl GalleryDock<'_> {
    /// The centre tab `tab`'s view of the browser, with the rest of the browser's values the Overview reads.
    fn centre_body(&mut self, ui: &mut Ui, tab: &PanelId) {
        let Browser { catalog, filter, selection, story_of, cells, generation, level, followed, .. } = &mut *self.browser;
        let centre = Centre {
            catalog,
            selection,
            story_of,
            cells,
            generation: *generation,
            inspected: self.inspect.of(tab, **self.layers),
            variants: self.variants.as_ref().filter(|(owner, _)| owner == tab).map(|(_, view)| *view),
            opened: Outbox::new(&mut self.opened),
            variants_out: Outbox::new(&mut self.variants_out),
        };
        match tab {
            PanelId::Overview => overview::overview(ui, centre, filter, level, followed),
            _ => overview::component_tab(ui, centre, tab),
        }
    }
}

/// The gallery's panel specs: each panel's follow from its id.
#[derive(Clone, Copy, Debug, Default, PartialEq, craft_ui::contract::PlainData)]
pub struct GallerySpecs;

impl PanelSpecs<PanelId> for GallerySpecs {
    fn spec(&self, panel: &PanelId) -> PanelSpec {
        let spec = PanelSpec::new(panel.title()).limits(SizeLimits::default());
        match panel {
            PanelId::Stories => spec.heights(DockHeights::new(80.0).compact(60.0)),
            PanelId::Properties | PanelId::Inspect | PanelId::Tokens => spec.heights(DockHeights::new(240.0).compact(120.0).fill(300.0)),
            PanelId::Problems | PanelId::Detail | PanelId::AppSizes => spec.heights(DockHeights::new(210.0)),
            // The centre's pages are documents: the dock shows them edge to edge.
            PanelId::Overview | PanelId::Component(_) | PanelId::Preview => spec.window_kind(WindowKind::Document),
            _ => spec,
        }
    }

    fn permissions(&self, panel: &PanelId) -> Permissions {
        panel.permissions()
    }
}

impl DockPanels<PanelId> for GalleryDock<'_> {
    fn body(&mut self, panel: &PanelId, ui: &mut Ui, _: &mut BodyCx<'_>) {
        match (panel, self.preview) {
            (PanelId::Preview, Preview::App) => {
                crate::mock::EditorMock::new("Preview", self.app).show(ui);
            }
            (PanelId::Preview, Preview::Components) => {
                Notice::new(Tone::Info, "The Controls level comes with the overview.").show(ui);
            }
            (PanelId::Components, _) => panels::components(ui, self.browser, &mut self.opened),
            (PanelId::Stories, _) => panels::stories(ui, self.browser),
            (PanelId::Properties, _) => panels::properties(ui, self.browser),
            (PanelId::Inspect, _) => inspect::panel(ui, self.browser, &self.inspect, self.layers),
            (PanelId::Overview | PanelId::Component(_), _) => self.centre_body(ui, panel),
            _ => self.theme.show(ui, panel),
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
