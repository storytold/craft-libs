//! [`ColorHud`]: the colour area and hue strip in a popover at a point, driven by the host's press.

use craft_behavior::EditEvents;
use craft_theme::Srgba;
use craft_ui::contract::{Bind, Pattern};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::overlay::CloseReason;
use egui::{Pos2, Response, Ui};

/// The area and hue strip in a popover at `at`, with the start colour under it, driven by the host's press: the pick
/// maps `pointer` through `ColorArea` and the strip's `value_at`; no pick while `pointer` is still `at`; one gesture
/// per open; Escape or `held(false)` commits and closes. Every input is a prop of the pass; the HUD keeps no state.
#[must_use = "show the HUD with `show`"]
#[derive(Pattern)]
#[allow(dead_code)] // P3(panels-w7): pn.C31
pub struct ColorHud<'a> {
    label: Label,
    at: Pos2,
    color: Bind<'a, Srgba>,
    start: Option<Srgba>,
    pointer: Option<Pos2>,
    held: bool,
}

impl<'a> ColorHud<'a> {
    /// A HUD named `label` at `at` over `color`.
    pub fn new(label: impl Into<Label>, at: Pos2, color: Bind<'a, Srgba>) -> Self {
        let _ = (label.into(), at, color);
        todo!("pn.C31")
    }

    /// The colour under `at` when the host's press began; the popover is placed so this colour's point sits at `at`.
    #[must_use]
    pub fn start(mut self, color: Srgba) -> Self {
        let _ = (&mut self, color);
        todo!("pn.C31")
    }

    /// The host's pointer, or `None` while it has none.
    #[must_use]
    pub fn pointer(mut self, at: Option<Pos2>) -> Self {
        let _ = (&mut self, at);
        todo!("pn.C31")
    }

    /// Whether the host's press is still held.
    #[must_use]
    pub fn held(mut self, held: bool) -> Self {
        let _ = (&mut self, held);
        todo!("pn.C31")
    }

    /// Draws the HUD.
    pub fn show(self, ui: &mut Ui) -> ColorHudOutput {
        let _ = (self, ui);
        todo!("pn.C31")
    }
}

/// What the HUD reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ColorHudOutput {
    /// The popover's response.
    pub response: Response,
    /// The edit events of the colour.
    pub events: EditEvents<Srgba>,
    /// Why it closed this pass.
    pub closed: Option<CloseReason>,
}
