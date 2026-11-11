//! The scope panel's view, persisted settings and state.

use crate::ui::histogram::LevelRange;
use crate::ui::scopes::clipping::{ClippingDiagnostic, ClippingMode, ClippingWarnings};

/// The panel's view (its menu's Compact, Expanded and All Channels View).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ScopeView {
    /// The plot alone (Photoshop's default).
    #[default]
    Compact,
    /// The Channel row, the plot, the Source row and the statistics.
    Expanded,
    /// Expanded plus one plot per colour channel.
    AllChannels,
}

/// The scope panel's persisted section: the app keeps it with its preferences (PhotoCraft's
/// `histogramPanel` and `filter.cameraRaw.scope`). Unknown fields are ignored, missing ones take
/// their defaults.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct ScopeSettings {
    /// The view.
    pub view: ScopeView,
    /// The Channel key ([`ScopeChannel::key`] or [`COLORS_CHANNEL`]); empty or a key the mode
    /// lacks shows the composite. Kept across views.
    pub channel: String,
    /// Show Channels in Color: single channels in Expanded and All Channels View in their colour
    /// (Compact always colours them; Colors is always in colour).
    pub show_channels_in_color: bool,
    /// Show Statistics (on by default).
    pub show_statistics: bool,
    /// The Source key ([`ScopeSource::key`]); empty is the first source.
    pub source: String,
    /// Show Lab Color Readouts: the host passes [`ScopeReadout::Lab`].
    pub lab_readouts: bool,
    /// Show Vectorscope.
    pub vectorscope: bool,
    /// Show Selected Region: the host counts the vectorscope from the selection.
    pub selected_region: bool,
    /// Show Red at 3 o'clock (else red sits at −100°).
    pub red_right: bool,
    /// Show Skin Tone Indicator (on by default).
    pub skin_tone: bool,
    /// The shadow clipping warning (U).
    pub shadow_warning: bool,
    /// The highlight clipping warning (O).
    pub highlight_warning: bool,
}

impl Default for ScopeSettings {
    // pc: crates/ui-egui/src/panels/histogram_panel.rs:89-99 (panel defaults); state/panel_state.rs:98 `CameraRawScopeState`.
    fn default() -> Self {
        Self {
            view: ScopeView::Compact,
            channel: String::new(),
            show_channels_in_color: false,
            show_statistics: true,
            source: String::new(),
            lab_readouts: false,
            vectorscope: false,
            selected_region: false,
            red_right: false,
            skin_tone: true,
            shadow_warning: false,
            highlight_warning: false,
        }
    }
}

impl ScopeSettings {
    /// What the clipping overlay draws: `diagnostic` (an Alt-drag on a tone control, which the
    /// host recognises) wins; else the warnings that are on; else [`ClippingMode::None`].
    pub fn clipping_mode(&self, diagnostic: Option<ClippingDiagnostic>) -> ClippingMode {
        // pc: crates/ui-egui/src/dialogs/camera_raw_scope_ui.rs:608-614 (`mode`).
        match diagnostic {
            Some(ClippingDiagnostic::Shadows) => ClippingMode::ShadowChannels,
            Some(ClippingDiagnostic::Highlights) => ClippingMode::HighlightChannels,
            None if self.shadow_warning || self.highlight_warning => ClippingMode::Warnings(self.clipping_warnings()),
            None => ClippingMode::None,
        }
    }

    /// The warnings that are on.
    pub fn clipping_warnings(&self) -> ClippingWarnings {
        ClippingWarnings { shadows: self.shadow_warning, highlights: self.highlight_warning }
    }
}

/// What the panel keeps for the app between passes: the persisted settings and the selected
/// level range (not persisted).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ScopePanelState {
    /// The persisted section.
    pub settings: ScopeSettings,
    /// The level range dragged on the plot, in levels; `None` when nothing is selected.
    pub range: Option<LevelRange>,
}
