//! [`MaskViewMode`], [`MaskViewState`] and the [`MaskViewRecipe`] an engine builds the view from
//!.

use craft_theme::Srgba;
use serde::{Deserialize, Serialize};

use super::{MaskViewKeys, MaskViewWords};
use crate::selection::ColorIndicates;
use craft_ui::runtime::commands::FirePhase;

/// One of Select and Mask's seven views, in menu order (the order F cycles).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum MaskViewMode {
    /// Only the masked layer, its unselected area see-through by the opacity. Default.
    #[default]
    OnionSkin,
    /// The image with the refined selection's ants.
    MarchingAnts,
    /// The image with the Overlay colour over the masked (or selected) area.
    Overlay,
    /// The selection over black.
    OnBlack,
    /// The selection over white.
    OnWhite,
    /// The coverage as a black and white mask.
    BlackAndWhite,
    /// The masked layer alone, its unselected area transparent.
    OnLayers,
}

impl MaskViewMode {
    /// Every view, in menu order.
    // pc: crates/ui-egui/src/canvas/select_mask_view.rs:42 `Mode::ALL`.
    pub const ALL: [MaskViewMode; 7] = [
        MaskViewMode::OnionSkin,
        MaskViewMode::MarchingAnts,
        MaskViewMode::Overlay,
        MaskViewMode::OnBlack,
        MaskViewMode::OnWhite,
        MaskViewMode::BlackAndWhite,
        MaskViewMode::OnLayers,
    ];

    /// PhotoCraft's id (dialog fields, automation): `onionSkin`, `marchingAnts`, `overlay`,
    /// `onBlack`, `onWhite`, `blackWhite`, `onLayers`.
    pub fn id(self) -> &'static str {
        // pc: crates/ui-egui/src/canvas/select_mask_view.rs:44-54 `Mode::id`.
        match self {
            MaskViewMode::OnionSkin => "onionSkin",
            MaskViewMode::MarchingAnts => "marchingAnts",
            MaskViewMode::Overlay => "overlay",
            MaskViewMode::OnBlack => "onBlack",
            MaskViewMode::OnWhite => "onWhite",
            MaskViewMode::BlackAndWhite => "blackWhite",
            MaskViewMode::OnLayers => "onLayers",
        }
    }

    /// The view with `id`, if any.
    pub fn from_id(id: &str) -> Option<MaskViewMode> {
        MaskViewMode::ALL.into_iter().find(|m| m.id() == id)
    }

    /// The view's opacity in percent until the user changes it: 50 for Onion Skin and Overlay, 100
    /// for On Black and On White, `None` for the three without one.
    pub fn default_opacity(self) -> Option<f32> {
        // pc: crates/ui-egui/src/canvas/select_mask_view.rs:86-92 `Mode::default_opacity`.
        match self {
            MaskViewMode::OnionSkin | MaskViewMode::Overlay => Some(50.0),
            MaskViewMode::OnBlack | MaskViewMode::OnWhite => Some(100.0),
            MaskViewMode::MarchingAnts | MaskViewMode::BlackAndWhite | MaskViewMode::OnLayers => None,
        }
    }

    fn index(self) -> usize {
        MaskViewMode::ALL.iter().position(|m| *m == self).unwrap_or(0)
    }

    /// The next view in menu order, Onion Skin after On Layers (F).
    pub fn next(self) -> MaskViewMode {
        // pc: crates/ui-egui/src/canvas/select_mask_view.rs:94-97 `Mode::next`.
        let i = MaskViewMode::ALL.iter().position(|m| *m == self).unwrap_or(0);
        MaskViewMode::ALL[(i + 1) % MaskViewMode::ALL.len()]
    }

    /// The view's name in `words`.
    pub fn label(self, words: &MaskViewWords) -> &str {
        match self {
            MaskViewMode::OnionSkin => words.onion_skin.as_str(),
            MaskViewMode::MarchingAnts => words.marching_ants.as_str(),
            MaskViewMode::Overlay => words.overlay.as_str(),
            MaskViewMode::OnBlack => words.on_black.as_str(),
            MaskViewMode::OnWhite => words.on_white.as_str(),
            MaskViewMode::BlackAndWhite => words.black_and_white.as_str(),
            MaskViewMode::OnLayers => words.on_layers.as_str(),
        }
    }

    /// The command that picks the view.
    pub const fn command(self) -> MaskViewKeys {
        match self {
            MaskViewMode::OnionSkin => MaskViewKeys::OnionSkin,
            MaskViewMode::MarchingAnts => MaskViewKeys::MarchingAnts,
            MaskViewMode::Overlay => MaskViewKeys::Overlay,
            MaskViewMode::OnBlack => MaskViewKeys::OnBlack,
            MaskViewMode::OnWhite => MaskViewKeys::OnWhite,
            MaskViewMode::BlackAndWhite => MaskViewKeys::BlackAndWhite,
            MaskViewMode::OnLayers => MaskViewKeys::OnLayers,
        }
    }
}

/// The view state of the Select and Mask dialog (PhotoCraft's `__view`, `__opacity`,
/// `__showEdge`, `__showOriginal`, `__hideView` fields). View state, not undoable.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct MaskViewState {
    /// The view shown.
    pub mode: MaskViewMode,
    /// Each view's opacity in percent (0..=100), by [`MaskViewMode::ALL`] index; the three
    /// without one keep 100 and ignore it.
    pub opacities: [f32; 7],
    /// J: show the band the edge detection refines instead of the refined selection.
    pub show_edge: bool,
    /// P: show the selection before refinement.
    pub show_original: bool,
    /// X held: the image without the view.
    pub hide_view: bool,
    /// Overlay's colour, encoded sRGB (Photoshop's rubylith red by default).
    pub overlay_color: Srgba,
    /// Whether Overlay's colour marks the masked areas (default) or the selected ones.
    pub overlay_indicates: ColorIndicates,
}

impl Default for MaskViewState {
    /// Onion Skin, the default opacities (50, 50, 100, 100), nothing shown extra, red masked areas.
    fn default() -> Self {
        // pc: crates/ui-egui/src/canvas/select_mask_view.rs:100-113 (`mode` default, `opacity`),
        // :456-465 (`View::of`: red, masked unless `indicates` is "selected").
        let opacities = MaskViewMode::ALL.map(|m| m.default_opacity().unwrap_or(100.0));
        Self {
            mode: MaskViewMode::OnionSkin,
            opacities,
            show_edge: false,
            show_original: false,
            hide_view: false,
            overlay_color: Srgba { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
            overlay_indicates: ColorIndicates::MaskedAreas,
        }
    }
}

impl MaskViewState {
    /// The opacity of `mode` in percent, `None` for a view without one.
    pub fn opacity(&self, mode: MaskViewMode) -> Option<f32> {
        // pc: crates/ui-egui/src/canvas/select_mask_view.rs:109-113 `opacity` (clamped 0..=100).
        mode.default_opacity()?;
        Some(self.opacities[mode.index()].clamp(0.0, 100.0))
    }

    /// Set `mode`'s opacity (percent, rounded, clamped to 0..=100); a view without one ignores it.
    pub fn set_opacity(&mut self, mode: MaskViewMode, percent: f32) {
        // pc: crates/ui-egui/src/canvas/select_mask_view.rs:115-119 `set_opacity`; :192 (rounded).
        if mode.default_opacity().is_none() || !percent.is_finite() {
            return;
        }
        self.opacities[mode.index()] = percent.round().clamp(0.0, 100.0);
    }

    /// Apply one change (from the keys or the controls).
    pub fn apply(&mut self, change: MaskViewChange) {
        // new: one field per variant.
        match change {
            MaskViewChange::Mode(mode) => self.mode = mode,
            MaskViewChange::Opacity(mode, percent) => self.set_opacity(mode, percent),
            MaskViewChange::ShowEdge(on) => self.show_edge = on,
            MaskViewChange::ShowOriginal(on) => self.show_original = on,
            MaskViewChange::HideView(on) => self.hide_view = on,
        }
    }

    /// Applies a fired view command and returns the change it made. A view's letter picks it, F the next view, J and P
    /// toggle Show Edge and Show Original, and X held hides the view until it is released. A repeat of a key does
    /// nothing, and neither does the release of a key that is not held.
    pub fn run(&mut self, command: MaskViewKeys, phase: FirePhase) -> Option<MaskViewChange> {
        let change = match (command, phase) {
            (MaskViewKeys::HideView, FirePhase::Press) => MaskViewChange::HideView(true),
            (MaskViewKeys::HideView, FirePhase::Release) => MaskViewChange::HideView(false),
            (_, FirePhase::Press) => match command {
                MaskViewKeys::NextView => MaskViewChange::Mode(self.mode.next()),
                MaskViewKeys::ShowEdge => MaskViewChange::ShowEdge(!self.show_edge),
                MaskViewKeys::ShowOriginal => MaskViewChange::ShowOriginal(!self.show_original),
                other => MaskViewChange::Mode(MaskViewMode::ALL.into_iter().find(|mode| mode.command() == other)?),
            },
            _ => return None,
        };
        self.apply(change);
        Some(change)
    }

    /// What the engine builds for the current view.
    ///
    /// With `o` the view's opacity (0..=1) and `c` the coverage:
    /// - coverage: [`MaskCoverage::EdgeBand`] with Show Edge, else [`MaskCoverage::Original`] with
    ///   Show Original, else [`MaskCoverage::Refined`];
    /// - Marching Ants: no veil, `ants` on;
    /// - Overlay: one veil of `overlay_color` at `o`, masked by `1 − c` (masked areas) or `c`
    ///   (selected areas);
    /// - On Black, On White: one black (white) veil at `o` masked by `1 − c`;
    /// - Black & White: a black veil at 1 over everything, then a white veil at 1 masked by `c`;
    /// - Onion Skin: the other layers hidden and the active layer masked by `c + (1 − c) · (1 − o)`
    ///   (`layer_floor` `1 − o`); On Layers: masked by `c` (`layer_floor` 0), the others kept;
    /// - `hidden` while X is held (the image without a view; the engine shows the document as is).
    pub fn recipe(&self) -> MaskViewRecipe {
        // pc: crates/ui-egui/src/canvas/select_mask_view.rs:497-507 (which coverage), :586-620
        // `show` (the veils and the layer mask).
        let mode = self.mode;
        let o = self.opacity(mode).unwrap_or(100.0) / 100.0;
        let coverage = if self.show_edge {
            MaskCoverage::EdgeBand
        } else if self.show_original {
            MaskCoverage::Original
        } else {
            MaskCoverage::Refined
        };
        let black = Srgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
        let white = Srgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
        let mut veils = Vec::new();
        let mut layer_floor = None;
        let mut hide_other_layers = false;
        match mode {
            MaskViewMode::MarchingAnts => {}
            MaskViewMode::Overlay => {
                let mask = match self.overlay_indicates {
                    ColorIndicates::MaskedAreas => VeilMask::Inverse,
                    ColorIndicates::SelectedAreas => VeilMask::Coverage,
                };
                veils.push(Veil { color: self.overlay_color, opacity: o, mask });
            }
            MaskViewMode::OnBlack => veils.push(Veil { color: black, opacity: o, mask: VeilMask::Inverse }),
            MaskViewMode::OnWhite => veils.push(Veil { color: white, opacity: o, mask: VeilMask::Inverse }),
            MaskViewMode::BlackAndWhite => {
                veils.push(Veil { color: black, opacity: 1.0, mask: VeilMask::Everywhere });
                veils.push(Veil { color: white, opacity: 1.0, mask: VeilMask::Coverage });
            }
            MaskViewMode::OnionSkin => {
                layer_floor = Some(1.0 - o);
                hide_other_layers = true;
            }
            MaskViewMode::OnLayers => layer_floor = Some(0.0),
        }
        MaskViewRecipe { mode, coverage, veils, layer_floor, hide_other_layers, ants: mode == MaskViewMode::MarchingAnts, hidden: self.hide_view }
    }
}

/// Which coverage the view shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MaskCoverage {
    /// The refined selection (the engine runs the refinement).
    #[default]
    Refined,
    /// The selection before refinement (Show Original).
    Original,
    /// The band `radius` pixels either side of the edge, where grown and shrunk coverage differ,
    /// with the Refine Edge Brush strokes (Show Edge).
    EdgeBand,
}

/// Which part of the canvas a veil covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum VeilMask {
    /// Everywhere.
    Everywhere,
    /// Masked by the coverage `c`: the selected area.
    Coverage,
    /// Masked by `1 − c`: the unselected area.
    Inverse,
}

/// A solid fill over the image, as the engine adds it (PhotoCraft's veil fill layers).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Veil {
    /// Encoded sRGB.
    pub color: Srgba,
    /// 0..=1.
    pub opacity: f32,
    /// What the veil's mask covers.
    pub mask: VeilMask,
}

/// What the engine builds for a view ([`MaskViewState::recipe`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct MaskViewRecipe {
    /// The selected view.
    pub mode: MaskViewMode,
    /// What the mask covers in this view.
    pub coverage: MaskCoverage,
    /// Fills over the image, bottom first.
    pub veils: Vec<Veil>,
    /// The active layer's mask becomes `c + (1 − c) · floor` (Onion Skin `1 − o`, On Layers 0);
    /// `None` leaves the layers' masks alone.
    pub layer_floor: Option<f32>,
    /// Hide every layer but the active one (with its groups and contents): Onion Skin.
    pub hide_other_layers: bool,
    /// The refined selection's ants show (Marching Ants only): `SelectionOverlay::visible`.
    pub ants: bool,
    /// X is held: show the image without the view.
    pub hidden: bool,
}

/// One change to a [`MaskViewState`], as the keys and the controls report it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum MaskViewChange {
    /// A view picked (a letter, F, the drop-down).
    Mode(MaskViewMode),
    /// A view's opacity changed, in percent.
    Opacity(MaskViewMode, f32),
    /// Show Edge turned on or off.
    ShowEdge(bool),
    /// Show Original turned on or off.
    ShowOriginal(bool),
    /// X pressed (`true`) or released.
    HideView(bool),
}
