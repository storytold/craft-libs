//! The colour widgets.
//!
//! - Fields and strips of a colour model: [`ColorArea`], [`ColorStrip`], [`ColorRamp`], [`ColorDisc`], [`ColorWheel`].
//! - The picker: [`ColorPicker`], its popover and dialog, the hex and readout fields, the choice list and the droppers.
//! - Wells and paints: [`SwatchPair`], [`PaintChip`], [`PaintList`], [`SwatchPalette`], [`SwatchTable`].
//! - Panels: [`ColorPanel`], [`HarmonyPanel`], [`SwatchesPanel`].
//! - Sampling: the eyedropper, colour samplers and the screen picker.
//! - Imaging prompts: [`LutPicker`] and [`ProfileMismatchDialog`].
//!
//! Colour edits are values of a colour model ([`ModelValues`](craft_theme::ModelValues)) or of [`PickedColor`], and
//! report through the edit lifecycle of the contract.

mod area;
mod channels;
mod choice;
mod disc;
mod droppers;
mod eyedropper;
mod harmony;
mod hex;
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
mod samplers;
mod screen_picker;
mod strip;
mod swatches;
/// The stories and trials this crate registers for the integration binary.
#[cfg(feature = "screen-testing")]
pub mod testing;
mod wheel;

pub use crate::AreaMode;
pub use area::{ColorArea, ColorAreaStyle};
pub use channels::finite_values;
pub use choice::{ChosenColor, ColorChoice, ColorChoiceOutput, ColorChoiceWords, NamedColor};
pub use disc::{ColorDisc, ColorDiscOutput, ColorDiscStyle, ColorDiscWords, DiscMove};
pub use droppers::{DropperButton, DropperButtonOutput, DropperKind, DropperTool};
pub use eyedropper::{
    EyedropperBar, EyedropperBarOutput, EyedropperOptions, EyedropperPick, EyedropperTool, EyedropperWords, SampleSize, SampleSource,
};
pub use harmony::{
    HarmonyCommand, HarmonyMenu, HarmonyMenuAction, HarmonyPanel, HarmonyPanelOutput, HarmonyPanelState, HarmonyPicked, HarmonyWheel,
    HarmonyWheelOutput, HarmonyWords,
};
pub use hex::{HexPair, HexPairField, PairCodec};
pub use lut_picker::{LutChoice, LutPicker, LutPickerOutput, LutWords};
pub use paint_chip::{
    FillKind, GradientPreview, ImagePreview, Paint, PaintChip, PaintChipOutput, PaintEditor, PaintLink, PaintLinkKind, PaintList, PaintListEvent,
    PaintListOptions, PaintListOutput, PaintRow, PaintWords,
};
pub use pair::{PairWell, SwatchPair, SwatchPairOutput, Well};
pub use palette::{SwatchCell, SwatchCellStyle, SwatchPalette, SwatchPaletteOutput, SwatchRows, SwatchTable, SwatchTableOutput, SwatchTableStyle};
pub use panel::{
    ColorPanel, ColorPanelCommand, ColorPanelMenu, ColorPanelMode, ColorPanelOutput, ColorPanelRamp, ColorPanelState, ColorPanelWords, PanelTarget,
};
pub use picker::{
    ColorConstraint, ColorPicker, ColorPickerLayout, ColorPickerOutput, ColorPickerWords, ColorWarning, ColorWarningChip, ColorWarningChipOutput,
    PickedColor, ScreenPickGesture,
};
pub use picker_dialog::{ColorPickerDialog, ColorPickerDialogWords, ColorPickerMemory, ColorPickerResult};
pub use popover::{ColorPopover, ColorPopoverOutput};
pub use profile_prompts::{ProfileAction, ProfileAnswer, ProfileMismatchDialog, ProfileMode, ProfileQuestion, ProfileStatus, ProfileWords};
pub use ramp::{ColorRamp, ColorRampOutput, ColorRampStyle, ColorRampWords, Gamut, RampKind, RampPick};
pub use readout::{ChannelEdit, ColorReadout, ColorReadoutOutput, ColorReadoutWords, ReadoutChannel};
pub use samplers::{
    ColorSamplerCommand, ColorSamplerMarks, ColorSamplerTool, ColorSamplers, MAX_SAMPLERS, SamplerCache, SamplerError, SamplerOptions,
    SamplerOptionsOutput, SamplerReadout, SamplerReadouts, SamplerWords,
};
pub use screen_picker::{CaptureResult, CaptureSource, LOUPE_PIXELS, LOUPE_ZOOM, ScreenColorPicker, ScreenPick, ScreenPickerWords, StillCapture};
pub use strip::{ColorStrip, ColorStripStyle};
pub use swatches::{
    MAX_GROUPS, MAX_SWATCHES, SwatchAt, SwatchKey, SwatchLibrary, SwatchesCommand, SwatchesFileOutput, SwatchesMenu, SwatchesMenuOutput,
    SwatchesPanel, SwatchesPanelOutput, SwatchesState, SwatchesWords,
};
pub use wheel::{
    ColorWheel, ColorWheelStyle, ColorWheelWords, WheelGeometry, WheelPart, hue_at, hue_direction, triangle_corners, triangle_point, triangle_sv,
};
