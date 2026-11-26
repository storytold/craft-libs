//! `AlignBar`: the Align and Distribute controls of the Move tool's options bar. A pattern over `BarItems`: two
//! sections of four buttons ([`AlignAction::BAR`]), then the six edge and centre distribute commands, which sit in
//! the "More" menu only. The menu lists every item in item order with a rule between sections.

use super::items::{BarItem, BarItems, MoreMenu};
use super::words::BarWords;
use crate::contract::{NoValue, Outcome, Props as _, Show as _};
use crate::runtime::env::Env;
use crate::style::GlyphRole;
use craft_behavior::fit::FitKeep;
use egui::{Response, Ui};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// One align or distribute command. The commands are app commands: the app gates them with command state and takes
/// them when they run, from a button, the menu or a chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "align")]
#[non_exhaustive]
pub enum AlignAction {
    /// Align the layers' left edges.
    #[command(label = "Align Left Edges", icon = "align_edge_left")]
    AlignLeftEdges,
    /// Align the layers' horizontal centres.
    #[command(label = "Align Horizontal Centers", icon = "align_edge_hcenter")]
    AlignHorizontalCenters,
    /// Align the layers' right edges.
    #[command(label = "Align Right Edges", icon = "align_edge_right")]
    AlignRightEdges,
    /// Align the layers' top edges.
    #[command(label = "Align Top Edges", icon = "align_edge_top")]
    AlignTopEdges,
    /// Align the layers' vertical centres.
    #[command(label = "Align Vertical Centers", icon = "align_edge_vcenter")]
    AlignVerticalCenters,
    /// Align the layers' bottom edges.
    #[command(label = "Align Bottom Edges", icon = "align_edge_bottom")]
    AlignBottomEdges,
    /// Space the layers' top edges evenly.
    #[command(label = "Distribute Top Edges", icon = "distribute_top")]
    DistributeTopEdges,
    /// Space the layers' vertical centres evenly.
    #[command(label = "Distribute Vertical Centers", icon = "distribute_vcenter")]
    DistributeVerticalCenters,
    /// Space the layers' bottom edges evenly.
    #[command(label = "Distribute Bottom Edges", icon = "distribute_bottom")]
    DistributeBottomEdges,
    /// Space the layers' left edges evenly.
    #[command(label = "Distribute Left Edges", icon = "distribute_left")]
    DistributeLeftEdges,
    /// Space the layers' horizontal centres evenly.
    #[command(label = "Distribute Horizontal Centers", icon = "distribute_hcenter")]
    DistributeHorizontalCenters,
    /// Space the layers' right edges evenly.
    #[command(label = "Distribute Right Edges", icon = "distribute_right")]
    DistributeRightEdges,
    /// Equal gaps between the layers, left to right.
    #[command(label = "Distribute Horizontally", icon = "distribute_horizontal")]
    DistributeHorizontally,
    /// Equal gaps between the layers, top to bottom.
    #[command(label = "Distribute Vertically", icon = "distribute_vertical")]
    DistributeVertically,
}

impl AlignAction {
    /// The six align commands, in the order of their buttons.
    pub const ALIGN: [AlignAction; 6] = [
        AlignAction::AlignLeftEdges,
        AlignAction::AlignHorizontalCenters,
        AlignAction::AlignRightEdges,
        AlignAction::AlignTopEdges,
        AlignAction::AlignVerticalCenters,
        AlignAction::AlignBottomEdges,
    ];

    /// The bar's two sections of four buttons: align left, centre, right edges and distribute vertically; align top,
    /// centre, bottom edges and distribute horizontally.
    pub const BAR: [[AlignAction; 4]; 2] = [
        [AlignAction::AlignLeftEdges, AlignAction::AlignHorizontalCenters, AlignAction::AlignRightEdges, AlignAction::DistributeVertically],
        [AlignAction::AlignTopEdges, AlignAction::AlignVerticalCenters, AlignAction::AlignBottomEdges, AlignAction::DistributeHorizontally],
    ];

    /// The eight distribute commands, in the order of their buttons and menu rows.
    pub const DISTRIBUTE: [AlignAction; 8] = [
        AlignAction::DistributeTopEdges,
        AlignAction::DistributeVerticalCenters,
        AlignAction::DistributeBottomEdges,
        AlignAction::DistributeLeftEdges,
        AlignAction::DistributeHorizontalCenters,
        AlignAction::DistributeRightEdges,
        AlignAction::DistributeHorizontally,
        AlignAction::DistributeVertically,
    ];
}

/// What the bar reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct AlignBarOutput {
    /// The strip's response.
    pub response: Response,
}

impl Outcome for AlignBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The two sections of four align and distribute buttons, then the menu button, shown as children of a `Bar`. The
/// remaining six distribute commands are in the menu only. The buttons show their icons at the tool size
/// (`GlyphRole::Tool`).
#[derive(crate::contract::Pattern)]
pub struct AlignBar {}

impl AlignBar {
    /// The align bar.
    pub fn new() -> Self {
        AlignBar {}
    }

    /// Shows the bar. Each command's enabled state is the app's command state.
    pub fn show(self, ui: &mut Ui) -> AlignBarOutput {
        let name = Env::of(ui).words::<BarWords>().align_bar.clone();
        // `BAR` already holds two of the eight distribute commands, so the menu-only tail is the first six.
        let buttons = AlignAction::BAR
            .into_iter()
            .enumerate()
            .flat_map(|(section, row)| row.into_iter().map(move |action| BarItem::new(action).section(section as _)));
        let menu_only = AlignAction::DISTRIBUTE[..6].iter().copied().map(|action| BarItem::new(action).section(2).keep(FitKeep::Menu));
        let response = BarItems::new(buttons.chain(menu_only).collect()).label(name).more(MoreMenu::Every).glyphs(GlyphRole::Tool).show(ui);
        AlignBarOutput { response }
    }
}

impl Default for AlignBar {
    fn default() -> Self {
        AlignBar::new()
    }
}
