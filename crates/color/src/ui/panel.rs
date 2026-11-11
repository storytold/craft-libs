//! `ColorPanel`: Photoshop's Color panel. The foreground and background wells, one of nine views of the edited colour
//! (two cubes, a wheel, five slider sets), the out-of-gamut and web-colour warnings, and the colour ramp under the
//! sliders; the panel menu chooses the view and the ramp and copies the colour.

use std::hash::{DefaultHasher, Hash, Hasher};

use craft_behavior::{EditEvent, GesturePhase, IconId, Mixed};
use craft_cms::convert::CmykConverter;
use craft_theme::{ColorModel, HsbModel, LabModel, ModelValues, RgbModel, Srgba};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{Bind, CommandSet, Held, NoValue, Outcome, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _, clicked};
use craft_ui::layout::RowAlign;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::menu::{Menu, MenuItem, PanelRow};
use craft_ui::runtime::commands::{CommandRef, CommandState};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::requests::ColorRequest;
use egui::{Response, Ui};

use super::channels::finite_values;
use super::picker::{ChannelRow, ChannelRule, channel_name};
use super::{
    AreaMode, ColorArea, ColorPickerDialogWords, ColorRamp, ColorReadout, ColorStrip, ColorWarning, ColorWarningChip, ColorWheel, PairWell,
    PickedColor, RampKind, SwatchPair, Well,
};
use crate::{CmykModel, GrayModel, not_web_safe};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The Color panel's words.
    #[non_exhaustive]
    pub struct ColorPanelWords {
        title: "colorPanel.title" = "Color";
        hue_cube: "colorPanel.hueCube" = "Hue Cube";
        brightness_cube: "colorPanel.brightnessCube" = "Brightness Cube";
        color_wheel: "colorPanel.colorWheel" = "Color Wheel";
        grayscale_slider: "colorPanel.grayscaleSlider" = "Grayscale Slider";
        rgb_sliders: "colorPanel.rgbSliders" = "RGB Sliders";
        hsb_sliders: "colorPanel.hsbSliders" = "HSB Sliders";
        cmyk_sliders: "colorPanel.cmykSliders" = "CMYK Sliders";
        lab_sliders: "colorPanel.labSliders" = "Lab Sliders";
        web_color_sliders: "colorPanel.webColorSliders" = "Web Color Sliders";
        rgb_spectrum: "colorPanel.rgbSpectrum" = "RGB Spectrum";
        cmyk_spectrum: "colorPanel.cmykSpectrum" = "CMYK Spectrum";
        grayscale_ramp: "colorPanel.grayscaleRamp" = "Grayscale Ramp";
        current_colors: "colorPanel.currentColors" = "Current Colors";
        color_ramp: "colorPanel.colorRamp" = "Color Ramp";
        foreground: "colorPanel.foreground" = "Foreground color";
        background: "colorPanel.background" = "Background color";
        wells: "colorPanel.wells" = "Colors";
        add_to_swatches: "colorPanel.addToSwatches" = "Add to Swatches";
        create_swatch: "colorPanel.createSwatch" = "Create new swatch";
        hex_color: "colorPanel.hexColor" = "Hex color";
    }
}

/// What the panel shows: Photoshop's panel menu choices, in its order. Hue Cube is the default;
/// the theme never changes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ColorPanelMode {
    #[default]
    #[allow(missing_docs)]
    HueCube,
    #[allow(missing_docs)]
    BrightnessCube,
    #[allow(missing_docs)]
    ColorWheel,
    #[allow(missing_docs)]
    GrayscaleSlider,
    #[allow(missing_docs)]
    RgbSliders,
    #[allow(missing_docs)]
    HsbSliders,
    #[allow(missing_docs)]
    CmykSliders,
    #[allow(missing_docs)]
    LabSliders,
    #[allow(missing_docs)]
    WebColorSliders,
}

impl ColorPanelMode {
    /// Every mode, in the panel menu's order (one section, no separator).
    pub const ALL: [ColorPanelMode; 9] = [
        ColorPanelMode::HueCube,
        ColorPanelMode::BrightnessCube,
        ColorPanelMode::ColorWheel,
        ColorPanelMode::GrayscaleSlider,
        ColorPanelMode::RgbSliders,
        ColorPanelMode::HsbSliders,
        ColorPanelMode::CmykSliders,
        ColorPanelMode::LabSliders,
        ColorPanelMode::WebColorSliders,
    ];

    /// Hue Cube and Brightness Cube.
    pub fn is_cube(self) -> bool {
        matches!(self, ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube)
    }

    /// The slider modes, which show the ramp under their sliders.
    pub fn is_sliders(self) -> bool {
        !matches!(self, ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube | ColorPanelMode::ColorWheel)
    }

    /// The menu item.
    pub fn label(self, labels: &ColorPanelWords) -> Label {
        match self {
            ColorPanelMode::HueCube => &labels.hue_cube,
            ColorPanelMode::BrightnessCube => &labels.brightness_cube,
            ColorPanelMode::ColorWheel => &labels.color_wheel,
            ColorPanelMode::GrayscaleSlider => &labels.grayscale_slider,
            ColorPanelMode::RgbSliders => &labels.rgb_sliders,
            ColorPanelMode::HsbSliders => &labels.hsb_sliders,
            ColorPanelMode::CmykSliders => &labels.cmyk_sliders,
            ColorPanelMode::LabSliders => &labels.lab_sliders,
            ColorPanelMode::WebColorSliders => &labels.web_color_sliders,
        }
        .clone()
    }
}

/// The ramp under the sliders. Shift-click on the ramp cycles them in this order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ColorPanelRamp {
    #[default]
    #[allow(missing_docs)]
    RgbSpectrum,
    #[allow(missing_docs)]
    CmykSpectrum,
    #[allow(missing_docs)]
    GrayscaleRamp,
    #[allow(missing_docs)]
    CurrentColors,
}

impl ColorPanelRamp {
    /// All ramps, in menu order.
    pub const ALL: [ColorPanelRamp; 4] =
        [ColorPanelRamp::RgbSpectrum, ColorPanelRamp::CmykSpectrum, ColorPanelRamp::GrayscaleRamp, ColorPanelRamp::CurrentColors];

    /// The next ramp of the Shift-click cycle (Current Colors goes back to RGB Spectrum).
    pub fn next(self) -> ColorPanelRamp {
        let i = ColorPanelRamp::ALL.iter().position(|r| *r == self).unwrap_or(0);
        ColorPanelRamp::ALL.get(i + 1).copied().unwrap_or(ColorPanelRamp::RgbSpectrum)
    }

    /// The [`ColorRamp`](super::ColorRamp) kind that draws it: Spectrum, CmykSpectrum,
    /// Grayscale, or Between the foreground and background.
    pub fn kind(self) -> RampKind {
        match self {
            ColorPanelRamp::RgbSpectrum => RampKind::Spectrum,
            ColorPanelRamp::CmykSpectrum => RampKind::CmykSpectrum,
            ColorPanelRamp::GrayscaleRamp => RampKind::Grayscale,
            ColorPanelRamp::CurrentColors => RampKind::Between,
        }
    }

    /// The menu item.
    pub fn label(self, labels: &ColorPanelWords) -> Label {
        match self {
            ColorPanelRamp::RgbSpectrum => &labels.rgb_spectrum,
            ColorPanelRamp::CmykSpectrum => &labels.cmyk_spectrum,
            ColorPanelRamp::GrayscaleRamp => &labels.grayscale_ramp,
            ColorPanelRamp::CurrentColors => &labels.current_colors,
        }
        .clone()
    }
}

/// The Color panel's settings, kept across runs (settings section `color_panel`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct ColorPanelState {
    /// The panel edits the background colour (its well was clicked), not the foreground.
    pub background: bool,
    /// The picker layout.
    pub mode: ColorPanelMode,
    /// The colour ramp under the picker.
    pub ramp: ColorPanelRamp,
    /// Make Ramp Web Safe: the ramp shows and picks web colours only.
    pub web_safe_ramp: bool,
}

impl craft_host::Section for ColorPanelState {
    const KEY: &'static str = "color_panel";
}

/// Which colour a [`ColorPanel`] edit is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PanelTarget {
    /// The foreground colour.
    Foreground,
    /// The background colour.
    Background,
}

impl PanelTarget {
    /// The key of the [`ColorRequest`] for this colour's picker.
    pub fn key(self) -> &'static str {
        match self {
            PanelTarget::Foreground => "colorPanel.foreground",
            PanelTarget::Background => "colorPanel.background",
        }
    }

    /// The target whose [`key`](PanelTarget::key) is `key`.
    pub fn from_key(key: &str) -> Option<PanelTarget> {
        [PanelTarget::Foreground, PanelTarget::Background].into_iter().find(|t| t.key() == key)
    }

    fn index(self) -> usize {
        match self {
            PanelTarget::Foreground => 0,
            PanelTarget::Background => 1,
        }
    }

    fn other(self) -> PanelTarget {
        match self {
            PanelTarget::Foreground => PanelTarget::Background,
            PanelTarget::Background => PanelTarget::Foreground,
        }
    }
}

/// What [`ColorPanel::show`] returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the edit"]
pub struct ColorPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// The edits of the foreground or background colour this pass (from the view, the sliders, the ramp or a warning
    /// chip), in order, each with the colour it is for. Usually one; a field that commits as a slider begins makes
    /// two.
    pub edits: Vec<(PanelTarget, EditEvent<PickedColor>)>,
    /// A well was clicked while it was already the edited one, for this pass only: the app opens the colour picker
    /// dialog from the request (Photoshop). Its key is the well's [`PanelTarget::key`]; the colour is the well's.
    pub pick_color: Option<ColorRequest>,
    /// The Add to Swatches button under the Color Wheel was pressed: the app adds the colour of this well to the
    /// swatches.
    pub add_to_swatches: Option<PanelTarget>,
}

impl Outcome for ColorPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What the panel keeps between passes: the values last edited, one list per colour, and the ramp gesture's target.
#[derive(Clone, Debug, Default)]
struct Kept {
    values: [Vec<ModelValues>; 2],
    /// The values the slider tracks were drawn from when a drag began, per model and colour.
    tracks: Vec<(&'static str, usize, ModelValues)>,
    /// A channel gesture is running.
    dragging: bool,
    /// The ramp gesture in progress picks for the colour not edited.
    other: bool,
}

/// The Color panel.
///
/// - Wells: the foreground over the background (`SwatchPair`); a click on the other well makes it the edited one
///   ([`ColorPanelState::background`]); a click on the edited one asks for the picker. The swap and default buttons
///   are the pair's.
/// - Views: Hue Cube and Brightness Cube are `ColorArea` and `ColorStrip` (hue or brightness in the strip); Color
///   Wheel is `ColorWheel`; the slider modes are the channel rows of that model (Grayscale is [`GrayModel`], CMYK is
///   [`CmykModel`] through the converter, Web Color is RGB with sliders that snap to web colours). Slider values show
///   rounded to whole numbers, as Photoshop's fields.
/// - Warnings: in the slider modes, the out-of-gamut and web-colour chips beside the wells, the cubes the gamut chip
///   only; a click takes the nearest colour.
/// - Ramp: in the slider modes, a `ColorRamp` of the chosen kind; a press picks for the edited well, Alt picks for the
///   other; Shift-click cycles the kind. A drag keeps the Current Colors ends it began with. A secondary click opens
///   the ramp rows as a context menu.
/// - Focus: the panel never takes the focus when shown.
#[must_use = "show the panel with `show`"]
#[derive(Pattern)]
pub struct ColorPanel<'a> {
    state: Bind<'a, ColorPanelState>,
    colors: [Srgba; 2],
    cmyk: Option<&'a CmykConverter>,
    dynamic: bool,
}

impl<'a> ColorPanel<'a> {
    /// The panel on the app's `[foreground, background]`.
    pub fn new(state: Bind<'a, ColorPanelState>, colors: [Srgba; 2]) -> Self {
        ColorPanel { state, colors, cmyk: None, dynamic: true }
    }

    /// The document's CMYK converter, for the CMYK sliders, the CMYK Spectrum ramp and the gamut warning; without it
    /// they use the built-in coated CMYK.
    pub fn cmyk(mut self, converter: &'a CmykConverter) -> Self {
        self.cmyk = Some(converter);
        self
    }

    /// Dynamic Color Sliders (PhotoCraft's interface preference, on by default): the slider tracks show the channel's
    /// colours with the other channels as they are. Off, the tracks keep the colours they had when a drag began.
    pub fn dynamic_tracks(mut self, on: bool) -> Self {
        self.dynamic = on;
        self
    }

    /// Shows the panel and returns this pass's edits and requests.
    pub fn show(self, ui: &mut Ui) -> ColorPanelOutput {
        let ColorPanel { state, colors, cmyk, dynamic } = self;
        let env = Env::of(ui);
        let (words, dialog) = (env.words::<ColorPanelWords>(), env.words::<ColorPickerDialogWords>());
        let mut held = Held::new(state);
        let (mut edits, mut pick_color, mut add_to_swatches) = (Vec::new(), None, None);
        let mut shown = Stack::new().label(words.title.clone()).show(ui, |ui, cx| {
            let before = held.read(cx)?;
            let target = if before.background { PanelTarget::Background } else { PanelTarget::Foreground };
            let mut body = Body {
                target,
                colors,
                converter: cmyk.unwrap_or_else(|| CmykConverter::coated()),
                dynamic,
                words: &words,
                dialog: &dialog,
                now: before,
                kept: cx.memory::<Kept>().clone(),
                events: Vec::new(),
                pick_color: None,
                add_to_swatches: None,
            };
            body.show(ui);
            *cx.memory::<Kept>() = std::mem::take(&mut body.kept);
            edits = body.events;
            pick_color = body.pick_color;
            add_to_swatches = body.add_to_swatches;
            Some((before, body.now))
        });
        // Once per pass, after the real run: the edited state is stored.
        shown.settle(|cx, state| {
            if let Some((before, now)) = state.take()
                && now != before
            {
                held.write(cx, &before, now, words.title.as_str());
            }
        });
        ColorPanelOutput { response: shown.output, edits, pick_color, add_to_swatches }
    }
}

/// The commands of the Color panel menu and of the ramp's context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "colorPanel")]
#[non_exhaustive]
pub enum ColorPanelCommand {
    /// Hue Cube view.
    #[command(label = "Hue Cube", kind = "radio", group = "colorPanel.mode")]
    HueCube,
    /// Brightness Cube view.
    #[command(label = "Brightness Cube", kind = "radio", group = "colorPanel.mode")]
    BrightnessCube,
    /// Color Wheel view.
    #[command(label = "Color Wheel", kind = "radio", group = "colorPanel.mode")]
    ColorWheel,
    /// Grayscale Slider view.
    #[command(label = "Grayscale Slider", kind = "radio", group = "colorPanel.mode")]
    GrayscaleSlider,
    /// RGB Sliders view.
    #[command(label = "RGB Sliders", kind = "radio", group = "colorPanel.mode")]
    RgbSliders,
    /// HSB Sliders view.
    #[command(label = "HSB Sliders", kind = "radio", group = "colorPanel.mode")]
    HsbSliders,
    /// CMYK Sliders view.
    #[command(label = "CMYK Sliders", kind = "radio", group = "colorPanel.mode")]
    CmykSliders,
    /// Lab Sliders view.
    #[command(label = "Lab Sliders", kind = "radio", group = "colorPanel.mode")]
    LabSliders,
    /// Web Color Sliders view.
    #[command(label = "Web Color Sliders", kind = "radio", group = "colorPanel.mode")]
    WebColorSliders,
    /// Copy the edited colour as `color="#rrggbb"`.
    #[command(label = "Copy Color as HTML")]
    CopyHtml,
    /// Copy the edited colour's hex code.
    #[command(label = "Copy Color's Hex Code")]
    CopyHex,
    /// The RGB Spectrum ramp.
    #[command(label = "RGB Spectrum", kind = "radio", group = "colorPanel.ramp")]
    RgbSpectrum,
    /// The CMYK Spectrum ramp.
    #[command(label = "CMYK Spectrum", kind = "radio", group = "colorPanel.ramp")]
    CmykSpectrum,
    /// The Grayscale Ramp.
    #[command(label = "Grayscale Ramp", kind = "radio", group = "colorPanel.ramp")]
    GrayscaleRamp,
    /// The Current Colors ramp.
    #[command(label = "Current Colors", kind = "radio", group = "colorPanel.ramp")]
    CurrentColors,
    /// The ramp shows and picks web colours only.
    #[command(label = "Make Ramp Web Safe", kind = "toggle")]
    WebSafeRamp,
}

impl ColorPanelMode {
    /// The command that chooses this view.
    pub fn command(self) -> ColorPanelCommand {
        match self {
            ColorPanelMode::HueCube => ColorPanelCommand::HueCube,
            ColorPanelMode::BrightnessCube => ColorPanelCommand::BrightnessCube,
            ColorPanelMode::ColorWheel => ColorPanelCommand::ColorWheel,
            ColorPanelMode::GrayscaleSlider => ColorPanelCommand::GrayscaleSlider,
            ColorPanelMode::RgbSliders => ColorPanelCommand::RgbSliders,
            ColorPanelMode::HsbSliders => ColorPanelCommand::HsbSliders,
            ColorPanelMode::CmykSliders => ColorPanelCommand::CmykSliders,
            ColorPanelMode::LabSliders => ColorPanelCommand::LabSliders,
            ColorPanelMode::WebColorSliders => ColorPanelCommand::WebColorSliders,
        }
    }
}

impl ColorPanelRamp {
    /// The command that chooses this ramp.
    pub fn command(self) -> ColorPanelCommand {
        match self {
            ColorPanelRamp::RgbSpectrum => ColorPanelCommand::RgbSpectrum,
            ColorPanelRamp::CmykSpectrum => ColorPanelCommand::CmykSpectrum,
            ColorPanelRamp::GrayscaleRamp => ColorPanelCommand::GrayscaleRamp,
            ColorPanelRamp::CurrentColors => ColorPanelCommand::CurrentColors,
        }
    }
}

/// The Color panel menu: the rows for the dock's panel menu, the states the host publishes, and the carrying out of a
/// fired command (also the ramp's context menu's).
pub struct ColorPanelMenu;

impl ColorPanelMenu {
    /// The rows: the nine views, a separator, Copy Color as HTML and Copy Color's Hex Code, a separator, the four ramps,
    /// a separator, Make Ramp Web Safe.
    pub fn rows() -> Vec<PanelRow> {
        let row = |command: ColorPanelCommand| PanelRow::Command(CommandRef::from(command));
        let mut rows: Vec<PanelRow> = ColorPanelMode::ALL.iter().map(|mode| row(mode.command())).collect();
        rows.extend([PanelRow::Separator, row(ColorPanelCommand::CopyHtml), row(ColorPanelCommand::CopyHex), PanelRow::Separator]);
        rows.extend(Self::ramp_rows());
        rows
    }

    /// The ramps, a separator and Make Ramp Web Safe: the last rows of the menu and the ramp's context menu.
    pub fn ramp_rows() -> Vec<PanelRow> {
        let row = |command: ColorPanelCommand| PanelRow::Command(CommandRef::from(command));
        let mut rows: Vec<PanelRow> = ColorPanelRamp::ALL.iter().map(|ramp| row(ramp.command())).collect();
        rows.extend([PanelRow::Separator, row(ColorPanelCommand::WebSafeRamp)]);
        rows
    }

    /// The state of each command: the current view and ramp checked, Make Ramp Web Safe checked when on.
    pub fn states(state: &ColorPanelState) -> Vec<(ColorPanelCommand, CommandState)> {
        let mut states: Vec<(ColorPanelCommand, CommandState)> =
            ColorPanelMode::ALL.iter().map(|mode| (mode.command(), CommandState::new().checked(state.mode == *mode))).collect();
        states.extend([(ColorPanelCommand::CopyHtml, CommandState::new()), (ColorPanelCommand::CopyHex, CommandState::new())]);
        states.extend(ColorPanelRamp::ALL.iter().map(|ramp| (ramp.command(), CommandState::new().checked(state.ramp == *ramp))));
        states.push((ColorPanelCommand::WebSafeRamp, CommandState::new().checked(state.web_safe_ramp)));
        states
    }

    /// Carries out `command`: sets the view or the ramp, toggles Make Ramp Web Safe, or returns the text of `edited`
    /// for the clipboard, as HTML (`color="#rrggbb"`) or as a hex code (`rrggbb`).
    pub fn run_command(state: &mut ColorPanelState, command: ColorPanelCommand, edited: Srgba) -> Option<String> {
        if let Some(mode) = ColorPanelMode::ALL.into_iter().find(|mode| mode.command() == command) {
            state.mode = mode;
        } else if let Some(ramp) = ColorPanelRamp::ALL.into_iter().find(|ramp| ramp.command() == command) {
            state.ramp = ramp;
        } else {
            match command {
                ColorPanelCommand::WebSafeRamp => state.web_safe_ramp = !state.web_safe_ramp,
                ColorPanelCommand::CopyHtml => return Some(format!("color=\"{}\"", Srgba::from_rgb_array([edited.r, edited.g, edited.b]).to_hex())),
                ColorPanelCommand::CopyHex => {
                    return Some(Srgba::from_rgb_array([edited.r, edited.g, edited.b]).to_hex().trim_start_matches('#').to_owned());
                }
                _ => {}
            }
        }
        None
    }
}

/// What the panel draws and collects in one pass.
struct Body<'a> {
    /// The colour the panel edits.
    target: PanelTarget,
    colors: [Srgba; 2],
    converter: &'a CmykConverter,
    dynamic: bool,
    words: &'a ColorPanelWords,
    dialog: &'a ColorPickerDialogWords,
    /// The state as this pass leaves it.
    now: ColorPanelState,
    kept: Kept,
    events: Vec<(PanelTarget, EditEvent<PickedColor>)>,
    pick_color: Option<ColorRequest>,
    add_to_swatches: Option<PanelTarget>,
}

impl Body<'_> {
    fn color(&self) -> Srgba {
        self.colors[self.target.index()]
    }

    fn push(&mut self, target: PanelTarget, event: EditEvent<PickedColor>) {
        self.events.push((target, event));
    }

    fn show(&mut self, ui: &mut Ui) {
        let mode = self.now.mode;
        Stack::new().show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    self.wells(ui);
                    // The cubes show the out-of-gamut chip only: the web-colour chip is for Web Color.
                    if mode.is_sliders() || mode.is_cube() {
                        self.warnings(ui, mode == ColorPanelMode::WebColorSliders);
                    }
                });
                Stack::new().fill_width().show(ui, |ui, _| match mode {
                    ColorPanelMode::HueCube => self.cube(ui, AreaMode::Hue),
                    ColorPanelMode::BrightnessCube => self.cube(ui, AreaMode::Brightness),
                    ColorPanelMode::ColorWheel => self.rows(ui, &HsbModel, false),
                    ColorPanelMode::GrayscaleSlider => self.rows(ui, &GrayModel, false),
                    ColorPanelMode::RgbSliders => self.rows(ui, &RgbModel, false),
                    ColorPanelMode::HsbSliders => self.rows(ui, &HsbModel, false),
                    ColorPanelMode::CmykSliders => self.rows(ui, &CmykModel::new(self.converter.clone()), false),
                    ColorPanelMode::LabSliders => self.rows(ui, &LabModel, false),
                    ColorPanelMode::WebColorSliders => self.rows(ui, &RgbModel, true),
                });
            });
            match mode {
                ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube => self.readout(ui),
                ColorPanelMode::ColorWheel => self.wheel(ui),
                _ => self.ramp(ui),
            }
        });
    }

    /// The foreground over the background: a click on the other well edits it, a click on the edited one asks for the
    /// picker.
    fn wells(&mut self, ui: &mut Ui) {
        let was = if self.now.background { Well::Secondary } else { Well::Primary };
        let mut active = was;
        let wells = [PairWell::new(self.words.foreground.clone(), self.colors[0]), PairWell::new(self.words.background.clone(), self.colors[1])];
        let pair = SwatchPair::new(self.words.wells.clone(), wells, Bind::bound(&mut active)).popover(false).show(ui);
        self.now.background = active == Well::Secondary;
        if pair.clicked.as_ref() == Some(&was) {
            let title = match self.target {
                PanelTarget::Foreground => self.dialog.foreground_title.clone(),
                PanelTarget::Background => self.dialog.background_title.clone(),
            };
            self.pick_color = Some(ColorRequest::new(self.target.key(), self.color()).title(title));
        }
    }

    /// The out-of-gamut chip and, for Web Color, the web-colour chip under the wells.
    fn warnings(&mut self, ui: &mut Ui, web: bool) {
        let color = self.color();
        let rgb = [color.r, color.g, color.b];
        let nearest = |near: [f32; 3]| Srgba::rgb(near[0], near[1], near[2]);
        let mut warnings = Vec::new();
        if let Some(near) = self.converter.nearest_in_gamut(rgb, CmykConverter::PICKER_GAMUT_DE) {
            warnings.push(ColorWarning::new(self.dialog.out_of_gamut.clone(), nearest(near)));
        }
        if web && let Some(near) = not_web_safe(rgb) {
            warnings.push(ColorWarning::web_colors(self.dialog.not_web_color.clone(), nearest(near)));
        }
        Row::new().show(ui, |ui, _| {
            for warning in warnings.clone() {
                if ColorWarningChip::new(&warning).show(ui).clicked {
                    let picked = PickedColor { color: warning.nearest().with_alpha(color.a), model: None };
                    let name = self.words.title.as_str();
                    self.push(self.target, EditEvent::set(Mixed::Same(PickedColor { color, model: None }), picked, name.into(), None));
                }
            }
        });
    }

    /// The model's values for the edited colour, whole numbers as Photoshop's fields.
    fn values(&self, model: &dyn ColorModel) -> ModelValues {
        let color = self.color();
        let kept = self.kept.values[self.target.index()].iter().find(|v| v.model == model.id() && model.to_srgb(&v.values).same_rgb8(color));
        let mut values = kept.copied().unwrap_or_else(|| self.read(model, color));
        for (slot, ch) in values.values.iter_mut().zip(model.channels()) {
            let scale = 10f32.powi(i32::from(ch.decimals));
            *slot = if slot.is_finite() { (*slot * scale).round() / scale } else { 0.0 };
        }
        values
    }

    /// `color` in `model`. HSB takes the hue of a grey, and the hue and saturation of black, from the last HSB values
    /// (PhotoCraft's remembered hue), which no longer give the colour.
    fn read(&self, model: &dyn ColorModel, color: Srgba) -> ModelValues {
        let mut values = model.from_srgb(color);
        if model.id() == HsbModel.id()
            && let Some(old) = self.kept.values[self.target.index()].iter().find(|v| v.model == model.id())
        {
            if values.values[2] < 1.0 {
                values.values[0] = old.values[0];
                values.values[1] = old.values[1];
            } else if values.values[1] < 1.0 {
                values.values[0] = old.values[0];
            }
        }
        values
    }

    /// Keeps `values`, edited in `model`, and the hue and saturation of the colour they give.
    fn remember(&mut self, model: &dyn ColorModel, values: ModelValues) {
        let kept = &mut self.kept.values[self.target.index()];
        kept.retain(|v| v.model != values.model);
        kept.push(values);
        if model.id() != HsbModel.id() {
            let hsb = HsbModel.from_srgb(model.to_srgb(&values.values));
            if hsb.values[1] >= 1.0 && hsb.values[2] >= 1.0 {
                kept.retain(|v| v.model != hsb.model);
                kept.push(hsb);
            }
        }
    }

    /// An edit of `model`'s values for the edited colour.
    fn model_event(&mut self, model: &dyn ColorModel, event: EditEvent<ModelValues>) {
        let event = event.map(finite_values);
        self.kept.dragging = matches!(event.phase(), Some(GesturePhase::Begin | GesturePhase::Update));
        if let Some(values) = event.value() {
            self.remember(model, *values);
        }
        let alpha = self.color().a;
        let event = event.map(|values| PickedColor { color: model.to_srgb(&values.values).with_alpha(alpha), model: Some(values) });
        self.push(self.target, event);
    }

    /// An edit of channel `channel` of `current`.
    fn channel_event(&mut self, model: &dyn ColorModel, current: ModelValues, channel: usize, event: EditEvent<f64>) {
        self.model_event(
            model,
            event.map(|x| {
                let mut values = current;
                values.values[channel] = x as f32;
                values
            }),
        );
    }

    /// The channel rows of `model`: a label that scrubs the number field at the row's end, and the slider over the
    /// channel's colours. Web Color has sliders that step by the web levels.
    fn rows(&mut self, ui: &mut Ui, model: &dyn ColorModel, web: bool) {
        let current = self.values(model);
        // Dynamic Color Sliders off: the tracks keep the colours they had when a drag began.
        let slot = (model.id(), self.target.index());
        let held = self.kept.tracks.iter().find(|(id, target, _)| (*id, *target) == slot).map(|(_, _, values)| *values);
        let basis = if self.dynamic || !self.kept.dragging { current } else { held.unwrap_or(current) };
        if !self.kept.dragging {
            self.kept.tracks.retain(|(id, target, _)| (*id, *target) != slot);
            self.kept.tracks.push((slot.0, slot.1, current));
        }
        for (i, ch) in model.channels().iter().enumerate() {
            let mut rule = ChannelRule::of(ch);
            if web {
                rule.step = WEB_STEP;
            }
            let mut track_values = basis;
            // The hue track is the whole spectrum, whatever the saturation and brightness.
            if model.id() == HsbModel.id() && i == 0 {
                track_values.values[1] = 100.0;
                track_values.values[2] = 100.0;
            }
            let track = model.track(i, &track_values);
            let row = ChannelRow::new(channel_name(model, i, &[model]), Bind::given(f64::from(current.values[i])), rule).track(track).show(ui);
            for event in row.into_iter().flat_map(|row| row.events) {
                self.channel_event(model, current, i, event);
            }
        }
    }

    /// Hue Cube or Brightness Cube: the area and the strip of `mode`.
    fn cube(&mut self, ui: &mut Ui, mode: AreaMode) {
        let model = mode.model();
        let values = self.values(model);
        let [across, down] = mode.area();
        let names = [channel_name(model, across, &[model]), channel_name(model, down, &[model])];
        let (area_name, strip_name) = (Label::from(mode.name()), channel_name(model, mode.strip(), &[model]));
        let (area, strip) = Row::new()
            .show(ui, |ui, _| {
                let area = ColorArea::new(area_name.clone(), mode, Bind::given(values)).names(names.clone()).show(ui);
                let strip = ColorStrip::new(strip_name.clone(), mode, Bind::given(values)).show(ui);
                (area, strip)
            })
            .inner
            .map_or_else(Default::default, |(area, strip)| (area.events, strip.events));
        for event in area.into_iter().chain(strip) {
            self.model_event(model, event);
        }
    }

    /// The readout under the cubes: the hex field and the R, G and B number fields. Any of them sets the colour.
    fn readout(&mut self, ui: &mut Ui) {
        let color = self.color();
        let edit = ColorReadout::new(self.words.hex_color.clone(), Bind::given(color.to_rgba())).show(ui);
        for event in edit.events {
            self.push(self.target, event.map(|rgba| PickedColor { color: Srgba::from(rgba).with_alpha(color.a), model: None }));
        }
    }

    /// The Color Wheel with its Add to Swatches button.
    fn wheel(&mut self, ui: &mut Ui) {
        let values = self.values(&HsbModel);
        let wheel = ColorWheel::new(self.words.color_wheel.clone(), Bind::given(values)).show(ui);
        for event in wheel.events {
            self.model_event(&HsbModel, event);
        }
        let add = Row::new().align(RowAlign::End).show(ui, |ui, _| {
            IconButton::new(IconId::new("plus")).label(self.words.add_to_swatches.clone()).tooltip(self.words.create_swatch.clone()).show(ui).response
        });
        if add.inner.as_ref().is_some_and(clicked) {
            self.add_to_swatches = Some(self.target);
        }
    }

    /// The ramp; Alt picks for the colour not edited, Shift-click cycles the kind, a secondary click opens the ramp
    /// menu.
    fn ramp(&mut self, ui: &mut Ui) {
        let converter = self.converter;
        let gamut = |c: Srgba| {
            let back = converter.to_srgb(converter.from_srgb([c.r, c.g, c.b]));
            Srgba::new(back[0], back[1], back[2], c.a)
        };
        let mut hasher = DefaultHasher::new();
        std::sync::Arc::as_ptr(converter.profile()).hash(&mut hasher);
        let ramp = ColorRamp::new(self.words.color_ramp.clone(), self.now.ramp.kind(), Bind::kept())
            .kind_name(self.now.ramp.label(self.words))
            .ends([self.colors[0], self.colors[1]])
            .gamut(gamut)
            .gamut_revision(hasher.finish())
            .web_safe(self.now.web_safe_ramp)
            .show(ui);
        // Alt is on every event of a gesture that has a value; the others go where it went.
        for event in ramp.edit.events {
            if let Some(pick) = event.value() {
                self.kept.other = pick.other;
            }
            let target = if self.kept.other { self.target.other() } else { self.target };
            let alpha = self.colors[target.index()].a;
            self.push(target, event.map(|pick| PickedColor { color: pick.color.with_alpha(alpha), model: None }));
        }
        if ramp.cycle {
            self.now.ramp = self.now.ramp.next();
        }
        Menu::context(self.words.color_ramp.clone(), &ramp.edit.response).show(ui, |ui, _| {
            for row in ColorPanelMenu::ramp_rows() {
                if let PanelRow::Command(command) = row {
                    MenuItem::command(command).show(ui);
                }
            }
        });
    }
}

/// The step of a Web Color slider: the distance between two web-safe levels in 0 to 255.
const WEB_STEP: f64 = 51.0;
