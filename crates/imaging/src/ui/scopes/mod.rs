//! The scope panel: Photoshop's Histogram panel and Camera Raw's scope with tone zones,
//! clipping warnings and the vectorscope, plus the clipping overlay for a preview.
//!
//! The app counts (colour management, the histogram, cache levels and jobs stay its own); the
//! library draws, samples the pointer and reports:
//!
//! ```ignore
//! let frame = ScopeFrame::histogram(HistogramSet::new(&channels).original(adjusted_over.as_deref()).stale(stale))
//!     .vectorscope(VectorscopeFrame { counts: &scope.counts, hues: HUES, saturations: SATURATIONS, hover: pointer_sample })
//!     .clipping(ClippingFrame { shadows, highlights });
//! let out = ScopePanel::new(&mut state.scope).frame(&frame).sources(&sources).markers(&marks).show(ui);
//! if out.refresh { uncached_refresh(app); }
//! if out.settings_changed { persist(&state.scope.settings); }
//! if let Some(edit) = out.zone { app.edit_tone_zone(edit); }
//! ClippingOverlay::new("Clipping", &mask, revision, state.scope.settings.clipping_mode(diagnostic)).show_at(ui, canvas_rect);
//! ```
//!
//! | Part | Names |
//! |---|---|
//! | The panel | [`ScopePanel`], [`ScopePanelOutput`], [`ScopePanelState`], [`ScopeSettings`], [`ScopeView`], [`ScopeWords`] |
//! | The data | [`ScopeFrame`], [`HistogramSet`], [`ScopeChannel`], [`ChannelOverlay`], [`ScopeSource`], [`ScopeReadout`], [`VectorscopeFrame`], [`ClippingFrame`] |
//! | Tone zones | [`ToneZones`], [`ToneZone`], [`ScopeZoneEdit`], [`ZoneChange`] |
//! | Commands | [`ScopeCommand`], [`ScopePanel::panel_rows`], [`ScopePanel::run_command`], [`ScopeMenuAction`] |
//! | Clipping | [`ClipWarning`], [`ClippingOverlay`], [`ClippingMode`], [`ClippingWarnings`], [`ClippingDiagnostic`] |
//! | Vectorscope alone | [`VectorscopePlot`], [`scope_point`] |

mod clipping;
mod menu;
mod panel;
mod plot;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;
mod vectorscope;
mod words;
mod zones;

pub use clipping::{ClipWarning, ClippingDiagnostic, ClippingFrame, ClippingMode, ClippingOverlay, ClippingWarnings};
pub use menu::{ScopeCommand, ScopeMenuAction};
pub use panel::{
    COLORS_CHANNEL, ChannelOverlay, HistogramSet, ScopeChannel, ScopeFrame, ScopePanel, ScopePanelOutput, ScopePanelState, ScopeReadout,
    ScopeSettings, ScopeSource, ScopeView,
};
pub use vectorscope::{VectorscopeFrame, VectorscopePlot, scope_point};
pub use words::ScopeWords;
pub use zones::{ScopeZoneEdit, ToneZone, ToneZones, ZoneChange};
