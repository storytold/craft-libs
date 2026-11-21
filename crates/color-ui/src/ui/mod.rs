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

mod area;
mod channels;
mod choice;
mod compare;
mod disc;
mod droppers;
mod eyedropper;
pub mod gradient;
mod harmony;
mod hex;
mod hud;
mod info;
mod lut_picker;
mod paint_chip;
mod pair;
mod palette;
mod panel;
mod picker;
mod picker_dialog;
mod popover;
mod profile_prompts;
mod ramp;
mod readout;
mod sample_size;
mod samplers;
mod screen_picker;
mod strip;
mod swatches;
/// The stories and trials this crate registers for the integration binary.
#[cfg(feature = "screen-testing")]
pub mod testing;
mod warning;
mod wheel;

pub use crate::AreaMode;
pub use area::{AreaShape, ColorArea, ColorAreaStyle};
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
    FillKind, GradientPreview, ImagePreview, Paint, PaintChip, PaintChipOutput, PaintEditor, PaintLink, PaintLinkKind, PaintList, PaintListEvent,
    PaintListOptions, PaintListOutput, PaintRow, PaintWords,
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
pub use picker_dialog::{ColorPickerDialog, ColorPickerDialogWords, ColorPickerMemory, ColorPickerResult, PickerLibrary, PickerSwatches};
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
    SwatchesPanel, SwatchesPanelOutput, SwatchesState, SwatchesWords, swatch_rows,
};
pub use warning::{ColorWarning, ColorWarningArrangement, ColorWarningChip, ColorWarningChipOutput, ColorWarningChipStyle};
pub use wheel::{
    ColorWheel, ColorWheelOutput, ColorWheelStyle, ColorWheelWords, WheelGeometry, WheelInner, WheelPart, hue_at, hue_direction, triangle_corners,
    triangle_point, triangle_sv,
};
