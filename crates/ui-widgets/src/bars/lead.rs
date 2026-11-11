//! `BarHome` and `BarTool`: the parts an options bar starts with, before the tool's options.

use crate::buttons::{ButtonOutput, IconButton, ToggleButton};
use crate::contract::{Bind, EditResponse, NoValue, Outcome, Props as _, Show as _};
use crate::controls::disclosure::DisclosureGlyph;
use crate::display::{Divider, Icon};
use crate::primitives::IconSpec;
use crate::runtime::env::Label;
use craft_behavior::IconId;
use egui::{Response, Ui};

/// The Home button at the start of an options bar: a toggle with the home icon, then a rule. The host decides what
/// it shows and when it can toggle.
#[must_use = "show the button with `show`"]
#[derive(crate::contract::Pattern)]
pub struct BarHome<'a> {
    label: Label,
    on: Bind<'a, bool>,
}

impl<'a> BarHome<'a> {
    /// A Home button named `label` over `on`.
    pub fn new(label: impl Into<Label>, on: Bind<'a, bool>) -> Self {
        BarHome { label: label.into(), on }
    }

    /// Shows the button and the rule after it.
    pub fn show(self, ui: &mut Ui) -> EditResponse<bool> {
        let shown = ToggleButton::new(self.label, self.on).icon(IconSpec::svg(IconId::new("home"))).icon_only().show(ui);
        Divider::vertical().show(ui);
        shown
    }
}

/// What a [`BarTool`] reports: the tool icon's response, which the host gives its context menu (Reset Tool, Reset All
/// Tools) with `Menu::context`.
#[derive(Debug)]
#[non_exhaustive]
pub struct BarToolOutput {
    /// The tool icon's response.
    pub response: Response,
}

impl Outcome for BarToolOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The active tool's icon at the start of an options bar, after Home: the icon, the dropdown arrow that tells it has
/// a menu, then a rule. A click does nothing; the host attaches the tool's context menu to the icon's response.
#[must_use = "show the icon with `show`"]
#[derive(crate::contract::Pattern)]
pub struct BarTool {
    icon: IconId,
    label: Label,
}

impl BarTool {
    /// The icon `icon` of the tool named `label`.
    pub fn new(icon: IconId, label: impl Into<Label>) -> Self {
        BarTool { icon, label: label.into() }
    }

    /// Shows the icon, its arrow and the rule after them.
    pub fn show(self, ui: &mut Ui) -> BarToolOutput {
        let shown: ButtonOutput = IconButton::new(self.icon).label(self.label).show(ui);
        Icon::new(DisclosureGlyph::dropdown()).show(ui);
        Divider::vertical().show(ui);
        BarToolOutput { response: shown.response }
    }
}
