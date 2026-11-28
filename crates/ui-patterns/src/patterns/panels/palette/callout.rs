//! The callout of the palette's active row: the mark on its shown control and the box beside it.

use super::super::tour;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::display::{TargetMark, Text};
use crate::popover::Popover;
use crate::runtime::commands::{self, CommandRef};
use crate::runtime::env::Env;
use crate::runtime::focus::Initial;
use egui::{Rect, Ui};

/// What the callout did for the active row's command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pointed {
    /// The control is marked; its box shows beside it when a side keeps clear of the palette.
    Marked,
    /// No control to mark: it is shown nowhere in this window, is under another window or is under the palette. The
    /// footer names the command's home instead.
    Nowhere,
}

/// Marks where `command` is shown, if anywhere the palette does not cover, and opens a box with its label beside the
/// mark. The box is the palette's own popover (opened while its body runs), so a press on it never counts as outside
/// the palette, and it takes no focus from the search field. `palette` is the palette's rect in this pass.
pub(super) fn show(ui: &mut Ui, command: &CommandRef, palette: Rect) -> Pointed {
    let ctx = ui.ctx().clone();
    let Some(shown) = commands::shown(&ctx, command).filter(|shown| shown.viewport == ctx.viewport_id() && !shown.rect.intersects(palette)) else {
        return Pointed::Nowhere;
    };
    TargetMark::new(shown.rect).id_salt("mark").show_mark(ui);
    // The box hangs from the control as the tour's card does; the placement keeps it off the palette, and a placement
    // with no free side shows no box, which leaves the mark alone.
    let mut layer = tour::layer(Some(shown.rect.expand(Env::of(ui).tokens.space.pad.regular)));
    layer.place = layer.place.clear_of(palette);
    let label = shown.label;
    Popover::new(label.clone(), Bind::given(true)).overlay(|_| layer).initial_focus(Initial::Keep).show(ui, |ui, _| {
        Text::new(label.clone()).show(ui);
    });
    Pointed::Marked
}
