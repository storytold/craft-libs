//! Photoshop's adjustments as editors: each takes a plain value in a binding and reports an
//! [`EditResponse`](craft_ui::contract::EditResponse) of it, and the same editor serves the Image › Adjustments dialog
//! and the Properties panel ([`AdjustContext`]). The app converts values to its engine's parameters; the library has no
//! JSON.
//!
//! - Tone: [`LevelsEditor`] (one channel's levels over its histogram), [`LevelsPanel`] and [`CurvesPanel`] around it
//!   and the curve editor, with the channels ([`ToneChannels`]) and the point pickers ([`TonePickerRow`],
//!   [`TonePickTool`]).
//! - Slider sets: [`AdjustSliders`] (Exposure, Vibrance, Posterize, any table), [`BrightnessContrastEditor`],
//!   [`ThresholdEditor`], [`ColorBalanceEditor`], [`PhotoFilterEditor`], [`HueSaturationEditor`],
//!   [`BlackWhiteEditor`], [`ChannelMixerEditor`], [`SelectiveColorEditor`].
//! - The Properties panel's session ([`AdjustSession`]).
//!
//! Gradient Map is `craft_draw::ui::gradient::map`, beside the gradient editor it builds on.

mod basic;
mod context;
mod curves_panel;
mod hue_saturation;
mod labels;
mod levels_editor;
mod levels_panel;
mod mixers;
mod session;
mod sliders;
mod style;
mod tone;

pub use basic::{
    BrightnessContrast, BrightnessContrastEditor, ColorBalance, ColorBalanceEditor, ColorBalanceOutput, ColorBalanceTone, PhotoFilter,
    PhotoFilterEditor, PhotoFilterOutput, PhotoFilterPreset, ThresholdEditor,
};
pub use context::{AdjustContext, ToneChannel, ToneChannels, ToneSpace};
pub use curves_panel::{CurvesMode, CurvesPanel, CurvesPanelOutput, ToneCurves};
pub use hue_saturation::{HueRangeValue, HueSaturation, HueSaturationEditor, HueSaturationOutput};
pub use labels::AdjustWords;
pub use levels_editor::{Levels, LevelsClip, LevelsEditor, LevelsEditorOutput, LevelsWords};
pub use levels_panel::{LevelsPanel, LevelsPanelOutput};
pub use mixers::{
    BlackWhite, BlackWhiteEditor, BlackWhiteOutput, ChannelMixer, ChannelMixerEditor, SelectiveColor, SelectiveColorEditor, SelectiveColorOutput,
    SelectiveMethod,
};
pub use session::AdjustSession;
pub use sliders::{AdjustField, AdjustSliders, AdjustTrack};
pub use tone::{TonePick, TonePickTool, TonePicker, TonePickerRow, TonePickerRowOutput};
