//! `ScreenModeButton`: the status bar's screen mode button, a flyout of the three modes.

use super::screen_mode::{ScreenMode, ScreenModeCommands};
use super::words::ShellWords;
use crate::contract::{NoValue, Outcome, Props as _, Show as _};
use crate::rails::{FlyoutButton, RailTool};
use crate::runtime::env::Env;
use egui::{Response, Ui};

#[cfg(feature = "testing")]
mod stories;

/// A flyout button over the three screen modes, with `mode` marked: a click, Enter or Space cycles; a hold, a drag away
/// or a right click lists the three modes without cycling. The face is the Cycle command's icon and tooltip; disabled
/// while Cycle is disabled.
#[must_use = "show the button with `show`"]
#[derive(crate::contract::Pattern)]
pub struct ScreenModeButton {
    mode: ScreenMode,
}

/// What a [`ScreenModeButton`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ScreenModeButtonOutput {
    /// The button's response.
    pub response: Response,
}

impl Outcome for ScreenModeButtonOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl ScreenModeButton {
    /// A button showing `mode` as the current one.
    pub fn new(mode: ScreenMode) -> Self {
        ScreenModeButton { mode }
    }

    /// Shows the button in `ui`. The rows are the radio commands of the three modes, so their marks and states come
    /// from the shell; the shell applies whichever command runs.
    pub fn show(self, ui: &mut Ui) -> ScreenModeButtonOutput {
        let modes = [ScreenMode::Standard, ScreenMode::FullScreenWithMenuBar, ScreenMode::FullScreen];
        let tools: Vec<RailTool<ScreenMode>> = modes.iter().map(|mode| RailTool::new(*mode, ScreenModeCommands::of(*mode))).collect();
        // The shell's own word names the button when the Cycle command does not resolve.
        let name = Env::of(ui).words::<ShellWords>().screen_mode.clone();
        let out = FlyoutButton::new(&tools, &self.mode).label(name).hold_selects(false).click(ScreenModeCommands::Cycle).show(ui);
        ScreenModeButtonOutput { response: out.response }
    }
}
