//! The docked and floating scope of the workspace.

use craft_ui::contract::{Bind, Pattern, Props};
use egui::Ui;

use craft_ui::contract::EditResponse;

use crate::raw::RawDevelopSettings;
use crate::ui::scopes::{ScopeFrame, ScopePanelOutput, ScopePanelState};

/// What the docked and the floating scope report: the scope panel's events and the tone-zone edits of the bound
/// settings.
// P3(dialogs-w18): dl.K67 `RawScopeStripOutput`
#[derive(Debug)]
#[non_exhaustive]
pub struct RawScopeStripOutput {
    /// The scope panel's events.
    pub panel: ScopePanelOutput,
    /// The edits of the settings' tone zones.
    pub edit: EditResponse<RawDevelopSettings>,
}

/// The docked scope above the controls: a `ScopePanel` with Camera Raw's tone zones and typical histogram scale, its
/// context menu and a Float action in the section header.
#[must_use = "show the strip with `show`"]
#[derive(Pattern, Props)]
#[allow(dead_code)] // P3(dialogs-w18): dl.K67
pub struct RawScopeStrip<'a> {
    state: &'a mut ScopePanelState,
    frame: Option<&'a ScopeFrame<'a>>,
    /// Whether the scope floats; its Float action sets it.
    #[prop]
    floating: Option<Bind<'a, bool>>,
}

impl<'a> RawScopeStrip<'a> {
    /// The strip over `state` and the host's `frame`, showing the zone values of `settings`.
    // P3(dialogs-w18): dl.K67 `settings`
    pub fn new(state: &'a mut ScopePanelState, frame: Option<&'a ScopeFrame<'a>>, settings: Bind<'a, RawDevelopSettings>) -> Self {
        let _ = (state, frame, settings);
        todo!("dl.K67")
    }

    /// Draws the scope.
    // P3(dialogs-w18): dl.K67 `RawScopeStripOutput`
    pub fn show(self, ui: &mut Ui) -> RawScopeStripOutput {
        let _ = (self, ui);
        todo!("dl.K67")
    }
}

/// The floating scope: a modeless child dialog of the workspace, resizable and remembered; its Dock action clears
/// `floating`.
#[must_use = "show the scope with `show`"]
#[derive(Pattern)]
#[allow(dead_code)] // P3(dialogs-w18): dl.K67
pub struct RawFloatingScope<'a> {
    state: &'a mut ScopePanelState,
    frame: Option<&'a ScopeFrame<'a>>,
    floating: Bind<'a, bool>,
}

impl<'a> RawFloatingScope<'a> {
    /// The floating scope over `state` and the host's `frame`, showing the zone values of `settings`.
    // P3(dialogs-w18): dl.K67 `settings`
    pub fn new(
        state: &'a mut ScopePanelState,
        frame: Option<&'a ScopeFrame<'a>>,
        floating: Bind<'a, bool>,
        settings: Bind<'a, RawDevelopSettings>,
    ) -> Self {
        let _ = (state, frame, floating, settings);
        todo!("dl.K67")
    }

    /// Draws the scope.
    // P3(dialogs-w18): dl.K67 `RawScopeStripOutput`
    pub fn show(self, ui: &mut Ui) -> RawScopeStripOutput {
        let _ = (self, ui);
        todo!("dl.K67")
    }
}
