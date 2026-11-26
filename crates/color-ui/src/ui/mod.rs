//! The colour widgets.
//!
//! - Fields and strips of a colour model: [`ColorArea`], [`ColorStrip`], [`ColorRamp`], [`ColorDisc`], [`ColorWheel`].
//! - The picker: [`ColorPicker`], its popover and dialog, the hex and readout fields, the choice list and the droppers.
//! - Wells and paints: [`SwatchPair`], [`PaintChip`], [`PaintList`], [`SwatchPalette`], `craft_ui::choice::SwatchTable`.
//! - Panels: [`ColorPanel`], [`HarmonyPanel`], [`SwatchesPanel`], [`InfoPanel`]; over a canvas: [`ColorHud`].
//! - Gradients: the [`gradient`] module (`GradientEditor`, `GradientStops`, the stop rules, `GradientRamp`).
//! - Sampling: the eyedropper, colour samplers and the screen picker.
//! - Imaging prompts: [`LutPicker`] and [`ProfileMismatchDialog`].
//!
//! Colour edits are values of a colour model ([`ModelValues`](craft_theme::ModelValues)) or of [`PickedColor`], and
//! report through the edit lifecycle of the contract.

pub(crate) mod area;
mod channels;
pub(crate) mod choice;
pub(crate) mod compare;
pub(crate) mod disc;
pub(crate) mod droppers;
pub(crate) mod eyedropper;
pub mod gradient;
pub(crate) mod harmony;
mod hex;
pub(crate) mod hud;
pub(crate) mod info;
pub(crate) mod lut_picker;
pub(crate) mod paint_chip;
pub(crate) mod pair;
pub(crate) mod palette;
pub(crate) mod panel;
pub(crate) mod picker;
pub(crate) mod picker_dialog;
pub(crate) mod popover;
pub(crate) mod profile_prompts;
pub(crate) mod ramp;
pub(crate) mod readout;
pub(crate) mod sample_size;
pub(crate) mod samplers;
pub(crate) mod screen_picker;
pub(crate) mod strip;
pub(crate) mod swatches;
/// The stories and trials this crate registers for the integration binary.
#[cfg(feature = "screen-testing")]
pub mod testing;
pub(crate) mod warning;
pub(crate) mod wheel;

pub use crate::AreaMode;
pub use area::{AreaShape, ColorArea, ColorAreaStyle, smooth_mesh};
pub use channels::finite_values;
pub use choice::{ChosenColor, ColorChoice, ColorChoiceOutput, ColorChoiceWords, NamedColor};
pub use compare::{CompareSwatch, CompareSwatchOutput, CompareSwatchStyle};
pub use disc::{ColorDisc, ColorDiscOutput, ColorDiscStyle, ColorDiscWords, DiscHit, DiscLook, DiscMove, DiscRadius, patch_at};
pub use droppers::{DropperButton, DropperButtonOutput, DropperKind, DropperTool};
pub use eyedropper::{
    EyedropperBar, EyedropperBarOutput, EyedropperOptions, EyedropperPick, EyedropperTool, EyedropperWords, SampleSize, SampleSource,
};
pub use harmony::{
    GuideCell, GuideCellAt, HarmonyCommand, HarmonyMenu, HarmonyMenuAction, HarmonyPanel, HarmonyPanelOutput, HarmonyPanelState, HarmonyPicked,
    HarmonyWheel, HarmonyWheelOutput, HarmonyWords,
};
pub use hex::{HexPair, HexPairField, PairCodec};
pub use hud::{ColorHud, ColorHudOutput};
pub use info::{InfoColor, InfoMeasure, InfoPanel, InfoReadings, InfoWords};
pub use lut_picker::{LutChoice, LutPicker, LutPickerOutput, LutWords};
pub use paint_chip::{
    FillKind, GradientPreview, ImagePreview, Paint, PaintChip, PaintChipOutput, PaintChipSize, PaintEditor, PaintLink, PaintLinkKind, PaintList,
    PaintListEvent, PaintListOptions, PaintListOutput, PaintRow, PaintWords,
};
pub use pair::{DropAccepts, PairWell, SwatchPair, SwatchPairOutput, Well};
pub use palette::{SwatchPalette, SwatchPaletteOutput};
pub use panel::{
    ColorPanel, ColorPanelAction, ColorPanelCommand, ColorPanelLayout, ColorPanelMenu, ColorPanelMode, ColorPanelOutput, ColorPanelRamp,
    ColorPanelShown, ColorPanelState, ColorPanelWords, PanelTarget, TintRow,
};
pub use picker::{
    ChannelRow, ChannelRule, ChannelUnit, ColorConstraint, ColorPicker, ColorPickerLayout, ColorPickerOutput, ColorPickerWords, PickedColor,
    ScreenPickGesture, TrackRamp,
};
pub use picker_dialog::{
    ColorPickerDialog, ColorPickerDialogLayout, ColorPickerDialogWords, ColorPickerMemory, ColorPickerResult, ColorPickerTab, PickerLibrary,
    PickerSwatches,
};
pub use popover::{ColorPopover, ColorPopoverOutput};
pub use profile_prompts::{ProfileAction, ProfileAnswer, ProfileMismatchDialog, ProfileMode, ProfileQuestion, ProfileStatus, ProfileWords};
pub use ramp::{ColorRamp, ColorRampOutput, ColorRampStyle, ColorRampWords, Gamut, RampKind, RampPick, RampRecolor};
pub use readout::{ChannelEdit, ColorReadout, ColorReadoutOutput, ColorReadoutWords, ReadoutChannel};
pub use sample_size::SampleSizeSelect;
pub use samplers::{
    ColorSamplerCommand, ColorSamplerMarks, ColorSamplerPass, ColorSamplerTool, ColorSamplers, MAX_SAMPLERS, SamplerCache, SamplerCarry,
    SamplerError, SamplerEvent, SamplerOptions, SamplerOptionsOutput, SamplerReadout, SamplerReadouts, SamplerWords,
};
pub use screen_picker::{CaptureResult, CaptureSource, LOUPE_PIXELS, LOUPE_ZOOM, ScreenColorPicker, ScreenPick, ScreenPickerWords, StillCapture};
pub use strip::{ColorStrip, ColorStripStyle};
pub use swatches::{
    MAX_GROUPS, MAX_SWATCHES, SwatchAt, SwatchEdit, SwatchKey, SwatchLibrary, SwatchesCommand, SwatchesFileOutput, SwatchesMenu, SwatchesMenuOutput,
    SwatchesPanel, SwatchesPanelOutput, SwatchesState, SwatchesWords, swatch_rows, swatch_source,
};
pub use warning::{ColorWarning, ColorWarningArrangement, ColorWarningChip, ColorWarningChipOutput, ColorWarningChipStyle};
pub use wheel::{
    ColorWheel, ColorWheelOutput, ColorWheelStyle, ColorWheelWords, WheelGeometry, WheelInner, WheelPart, hue_at, hue_direction, triangle_corners,
    triangle_point, triangle_sv,
};
