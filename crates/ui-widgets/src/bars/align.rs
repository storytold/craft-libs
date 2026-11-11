//! `AlignBar`: the Align and Distribute controls of the Move tool's options bar. A pattern over `BarItems`: the six
//! align commands, then the eight distribute commands after a rule, which overflow into the "More" menu or live in
//! it.

use super::items::{BarItem, BarItems};
use super::words::BarWords;
use crate::contract::{NoValue, Outcome, Props as _, Show as _};
use crate::runtime::env::Env;
use craft_behavior::fit::FitKeep;
use egui::{Response, Ui};

#[cfg(feature = "testing")]
mod stories;

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

/// Where the distribute commands sit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DistributeButtons {
    /// In the menu only.
    #[default]
    Menu,
    /// Also as a second group of buttons after a rule; they move into the menu when the room runs out.
    Row,
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

/// The align buttons, the optional distribute buttons and the menu button, shown as children of a `Bar`.
#[derive(crate::contract::Pattern)]
pub struct AlignBar {
    distribute: DistributeButtons,
}

impl AlignBar {
    /// An align bar with the distribute commands in the menu.
    pub fn new() -> Self {
        AlignBar { distribute: DistributeButtons::Menu }
    }

    /// Where the distribute commands sit.
    #[must_use]
    pub fn distribute(mut self, distribute: DistributeButtons) -> Self {
        self.distribute = distribute;
        self
    }

    /// Shows the bar. Each command's enabled state is the app's command state.
    pub fn show(self, ui: &mut Ui) -> AlignBarOutput {
        let name = Env::of(ui).words::<BarWords>().align_bar.clone();
        let distribute_keep = if self.distribute == DistributeButtons::Row { FitKeep::Free } else { FitKeep::Menu };
        let align = AlignAction::ALIGN.into_iter().map(BarItem::new);
        let distribute = AlignAction::DISTRIBUTE.into_iter().map(|action| BarItem::new(action).section(1).keep(distribute_keep));
        let response = BarItems::new(align.chain(distribute).collect()).label(name).show(ui);
        AlignBarOutput { response }
    }
}

impl Default for AlignBar {
    fn default() -> Self {
        AlignBar::new()
    }
}
