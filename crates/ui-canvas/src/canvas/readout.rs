//! The readout: values beside the pointer during a canvas gesture ("W 120 px", "Angle 15 deg").

use crate::contract::{CxCore, ghost};
use crate::runtime::env::{Env, EnvValue, Label};
use egui::{Context, Pos2};

/// The one Show Transformation Values gate: while it holds `false` in the root env, [`readout`] and the tool host's
/// paint of a tool's readout rows paint nothing. [`hud`] is never gated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShowReadouts(pub bool);

impl EnvValue for ShowReadouts {
    fn root() -> Self {
        ShowReadouts(true)
    }
}

/// Paints rows of a label and a value beside the pointer at `at` ([`ghost::readout`]), in the env of `cx`'s region, for
/// this pass only: a gesture calls it each pass it runs. The readout takes no input. No rows paint nothing.
pub fn readout(cx: &CxCore<'_>, at: Pos2, rows: &[(Label, String)]) {
    if !shown(&cx.ctx) {
        return;
    }
    ghost::readout(&cx.ctx, &cx.env, at, rows);
}

/// Whether the root env's [`ShowReadouts`] lets value readouts paint: the one reading of the gate, shared by [`readout`]
/// and the tool host.
pub(super) fn shown(ctx: &Context) -> bool {
    Env::of_root(ctx).view().get::<ShowReadouts>().0
}

/// [`readout`] that the Show Transformation Values gate ([`ShowReadouts`]) never silences: brush size readouts.
pub fn hud(cx: &CxCore<'_>, at: Pos2, rows: &[(Label, String)]) {
    ghost::readout(&cx.ctx, &cx.env, at, rows);
}
