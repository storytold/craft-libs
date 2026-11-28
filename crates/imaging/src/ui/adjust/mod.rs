//! Photoshop's adjustments as editors: each takes a plain value in a binding and reports an
//! [`EditResponse`](craft_ui::contract::EditResponse) of it, and the same editor serves the Image › Adjustments dialog
//! and the Properties panel ([`AdjustContext`]). The app converts values to its engine's parameters; the values
//! serialize with their own field names (serde).
//!
//! - Tone: [`LevelsEditor`] (one channel's levels over its histogram), [`LevelsPanel`] and [`CurvesPanel`] around it
//!   and the curve editor, with the channels ([`ToneChannels`]) and the point pickers ([`TonePickerRow`],
//!   [`TonePickTool`]).
//! - Slider sets: [`AdjustSliders`] (Exposure, Vibrance, Posterize, any table), [`BrightnessContrastEditor`],
//!   [`ThresholdEditor`], [`ColorBalanceEditor`], [`PhotoFilterEditor`], [`HueSaturationEditor`],
//!   [`BlackWhiteEditor`], [`ChannelMixerEditor`], [`SelectiveColorEditor`].
//! - The Image › Adjustments dialog around any of them on the dialog host ([`HostedAdjustDialog`]).
//! - The Properties panel's session ([`AdjustSession`]).
//! - The Adjustments panel: a grid of buttons that add an adjustment layer ([`AdjustmentsPanel`], [`AdjustmentKind`]).
//!
//! Gradient Map is `craft_draw::ui::gradient::map`, beside the gradient editor it builds on.

pub(crate) mod basic;
mod context;
pub(crate) mod curves_panel;
mod hosted;
pub(crate) mod hue_saturation;
mod kinds;
mod labels;
pub(crate) mod levels_editor;
pub(crate) mod levels_panel;
pub(crate) mod mixers;
pub(crate) mod panel;
pub(crate) mod session;
pub(crate) mod sliders;
mod style;
pub(crate) mod tone;

pub use basic::{
    BrightnessContrast, BrightnessContrastEditor, ColorBalance, ColorBalanceEditor, ColorBalanceOutput, ColorBalanceTone, PhotoFilter,
    PhotoFilterEditor, PhotoFilterOutput, PhotoFilterPreset, ThresholdEditor,
};
pub use context::{AdjustContext, ToneChannel, ToneChannelCommands, ToneChannels, ToneSpace};
pub use curves_panel::{CurvesMode, CurvesPanel, CurvesPanelOutput, ToneCurves};
pub use hosted::{AdjustDialogCx, EditPhase, HostedAdjustDialog};
pub use hue_saturation::{AdjustedFn, HueRangeValue, HueSaturation, HueSaturationEditor, HueSaturationOutput};
pub use kinds::AdjustmentKind;
pub use labels::AdjustWords;
pub use levels_editor::{Levels, LevelsClip, LevelsEditor, LevelsEditorOutput, LevelsWords};
pub use levels_panel::{LevelsPanel, LevelsPanelOutput};
pub use mixers::{
    BlackWhite, BlackWhiteEditor, BlackWhiteOutput, ChannelMixer, ChannelMixerEditor, SelectiveColor, SelectiveColorEditor, SelectiveColorOutput,
    SelectiveMethod,
};
pub use panel::{AdjustmentsPanel, AdjustmentsPanelOutput};
pub use session::AdjustSession;
pub use sliders::{AdjustField, AdjustSliders, AdjustTrack};
pub use tone::{TonePick, TonePickTool, TonePicker, TonePickerRow, TonePickerRowOutput};
