//! `BarCell`: a control in a bar with its caption before it.

use crate::contract::{Gap, Show as _, ShowContainer as _};
use crate::display::{Shade, Text};
use crate::layout_components::Row;
use crate::runtime::env::Label;
use egui::Ui;

/// One control with its caption before it, as wide as the two together. The caption is the control's name: the cell
/// hands the control its label, so the visible text and the accessible name are one string. The caption and the
/// control are the items of a row, so they share the row's cross-axis alignment.
pub struct BarCell {
    caption: Label,
}

impl BarCell {
    /// A cell captioned `caption`.
    pub fn new(caption: impl Into<Label>) -> Self {
        BarCell { caption: caption.into() }
    }

    /// Shows the caption, then the control `control` builds and shows with the caption as its label. Returns what
    /// `control` returns.
    pub fn control<R>(self, ui: &mut Ui, control: impl FnOnce(&mut Ui, Label) -> R) -> R {
        let mut control = Some(control);
        let mut result = None;
        Row::new().gap(Gap::Item).show(ui, |ui, _| {
            Text::new(self.caption.clone()).shade(Shade::Muted).show(ui);
            if let Some(control) = control.take() {
                result = Some(control(ui, self.caption.clone()));
            }
        });
        result.expect("the row body runs")
    }
}
