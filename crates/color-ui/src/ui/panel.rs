//! `ColorPanel`: Photoshop's Color panel. The foreground and background wells, one of nine views of the edited colour
//! (two cubes, a wheel, five slider sets), the out-of-gamut and web-colour warnings, and the colour ramp under the
//! sliders; the panel menu chooses the view and the ramp and copies the colour.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use craft_behavior::{EditEvent, GesturePhase, IconId, Mixed};
use craft_cms::convert::CmykConverter;
use craft_theme::{ColorModel, HsbModel, LabModel, ModelValues, RgbModel, Srgba};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{
    Bind, CommandSet, Gap, Held, LabelScheme, NoValue, Outcome, Pattern, Props as _, Remember as _, Scroll, Show as _, ShowContainer as _, Sizing,
    clicked,
};
use craft_ui::form::FieldGroup;
use craft_ui::layout::RowAlign;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::menu::{Menu, MenuItem, PanelRow};
use craft_ui::runtime::commands::{CommandRef, CommandState};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::requests::ColorRequest;
use egui::{Response, Ui};

use super::channels::finite_values;
use super::picker::{ChannelRow, ChannelRule, ChannelUnit, channel_name};
use super::{
    AreaMode, ColorArea, ColorPickerDialogWords, ColorRamp, ColorReadout, ColorStrip, ColorWarning, ColorWarningArrangement, ColorWarningChip,
    ColorWheel, Gamut, PairWell, PickedColor, RampKind, RampPick, RampRecolor, SwatchPair, SwatchPairOutput, SwatchPalette, Well,
};
use crate::{CmykModel, GrayModel, not_web_safe};

mod fill_stroke;
#[cfg(feature = "suite")]
pub(crate) mod stories;

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
        swap_colors: "colorPanel.swapColors" = "Swap fill and stroke";
        default_colors: "colorPanel.defaultColors" = "Default fill and stroke";
        none: "colorPanel.none" = "None";
        black: "colorPanel.black" = "Black";
        white: "colorPanel.white" = "White";
        recent_colors: "colorPanel.recentColors" = "Recent Colors";
        tint: "colorPanel.tint" = "Tint";
        global: "colorPanel.global" = "Global";
        spot: "colorPanel.spot" = "Spot";
    }
}

/// What the panel shows: Photoshop's panel menu choices, in its order. Hue Cube is the default;
/// the theme never changes it.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    craft_ui::contract::MemoProps,
    craft_ui::contract::PlainData,
)]
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
    /// The modes of the FillStroke layout's menu, in its order; the Shift-click on the spectrum cycles through them.
    pub const FILL_STROKE: [ColorPanelMode; 5] = [
        ColorPanelMode::GrayscaleSlider,
        ColorPanelMode::RgbSliders,
        ColorPanelMode::HsbSliders,
        ColorPanelMode::CmykSliders,
        ColorPanelMode::WebColorSliders,
    ];

    /// The mode after this one in [`FILL_STROKE`](Self::FILL_STROKE); a mode outside the list goes to the first.
    pub fn fill_stroke_next(self) -> ColorPanelMode {
        let next = Self::FILL_STROKE.iter().position(|mode| *mode == self).and_then(|at| Self::FILL_STROKE.get(at + 1));
        next.copied().unwrap_or(ColorPanelMode::GrayscaleSlider)
    }

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
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    craft_ui::contract::MemoProps,
    craft_ui::contract::PlainData,
)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, craft_ui::contract::MemoProps, craft_ui::contract::PlainData)]
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
    /// Hide Options: the FillStroke layout keeps the proxy and the spectrum only.
    pub hide_options: bool,
    /// The FillStroke layout shows the Recent Colors slot.
    pub recent_colors: bool,
    /// The R, G and B rows count percents, 0 to 100, in place of 0 to 255.
    pub rgb_percent: bool,
}

impl Default for ColorPanelState {
    fn default() -> Self {
        ColorPanelState {
            background: false,
            mode: ColorPanelMode::default(),
            ramp: ColorPanelRamp::default(),
            web_safe_ramp: false,
            hide_options: false,
            recent_colors: true,
            rgb_percent: false,
        }
    }
}

/// How a [`ColorPanel`] arranges its body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColorPanelLayout {
    /// The foreground over the background wells with one view of the edited colour (Photoshop's panel).
    #[default]
    ForegroundBackground,
    /// The fill and stroke proxy, the rows of the shown mode, the Spectrum ramp and the Recent Colors slot.
    ///
    /// The rows come from [`ColorPanel::shown`]. [`ColorPanelState::mode`] counts only when it is one of
    /// [`ColorPanelMode::FILL_STROKE`]: any other value means no mode was picked, and the host's `shown` decides.
    FillStroke,
}

/// What rows a [`ColorPanel`] shows: the host's model rule.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ColorPanelShown {
    /// The channel rows of this mode.
    Mode(ColorPanelMode),
    /// The Tint row of a swatch-linked colour.
    Tint(TintRow),
    /// No rows.
    Empty,
}

/// The Tint row of a swatch-linked colour: a chip with the global corner and the spot dot, the name and the T row.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TintRow {
    /// The swatch's name.
    pub name: Label,
    /// The swatch's colour at full strength.
    pub base: Srgba,
    /// The swatch is global: the chip has the global corner.
    pub global: bool,
    /// The swatch is a spot colour: the chip has the spot dot.
    pub spot: bool,
    /// The tint, 0 to 100.
    pub tint: f32,
}

impl TintRow {
    /// A tint of `base` named `name`, neither global nor spot.
    pub fn new(name: impl Into<Label>, base: Srgba, tint: f32) -> Self {
        TintRow { name: name.into(), base, global: false, spot: false, tint }
    }
}

/// What a [`ColorPanelMenu`] command asks the host to do.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ColorPanelAction {
    /// Show this mode.
    Mode(ColorPanelMode),
    /// Put this text on the clipboard.
    Copy(String),
    /// Invert the edited colour.
    Invert,
    /// Replace the edited colour by its complement.
    Complement,
    /// Create a swatch of the edited colour.
    NewSwatch,
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
    /// The output of the proxy wells (the swap and default buttons, the clicks), for the host to read.
    pub pair: Option<SwatchPairOutput>,
    /// The edit of the Tint row.
    pub tint: Option<EditEvent<f32>>,
    /// The None well was pressed: the target to set to no paint.
    pub none: Option<PanelTarget>,
    /// A recolour click on the Spectrum ramp, with nothing picked.
    pub recolor: Option<RampRecolor>,
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
/// - Wells: the foreground over the background (`SwatchPair`), the edited one framed; a click on the other well makes
///   it the edited one ([`ColorPanelState::background`]); a click on the edited one asks for the picker. The swap and default buttons
///   are the pair's.
/// - Views: Hue Cube and Brightness Cube are `ColorArea` and `ColorStrip` (hue or brightness in the strip); Color
///   Wheel is `ColorWheel`; the slider modes are the channel rows of that model (Grayscale is [`GrayModel`], CMYK is
///   [`CmykModel`] through the converter, Web Color is RGB with sliders that snap to web colours). Slider values show
///   rounded to whole numbers, as Photoshop's fields.
/// - Warnings: in the slider modes, the out-of-gamut and web-colour chips under the wells, the cubes the gamut chip
///   only; a click takes the nearest colour.
/// - Ramp: in the slider modes, a `ColorRamp` of the chosen kind; a press picks for the edited well, Alt picks for the
///   other; Shift-click cycles the kind. A drag keeps the Current Colors ends it began with. A secondary click opens
///   the ramp rows as a context menu.
/// - Layouts: [`ColorPanelLayout::FillStroke`] shows the host's wells, the rows the host's model rule picks, the
///   Spectrum ramp and the Recent Colors instead. In both layouts [`PanelTarget::Foreground`] is the edited well
///   (`Well::Primary`) and [`PanelTarget::Background`] the other.
/// - Focus: the panel never takes the focus when shown.
#[must_use = "show the panel with `show`"]
#[derive(Pattern)]
pub struct ColorPanel<'a> {
    state: Bind<'a, ColorPanelState>,
    colors: [Srgba; 2],
    cmyk: Option<&'a CmykConverter>,
    rgb: Option<&'a dyn ColorModel>,
    dynamic: bool,
    layout: ColorPanelLayout,
    wells: Option<([PairWell; 2], Bind<'a, Well>)>,
    edited: Option<Option<PickedColor>>,
    shown: Option<ColorPanelShown>,
    recent: &'a [Srgba],
    recent_setup: Option<RecentSetup>,
    gamut: Option<Gamut<'a>>,
}

/// Sets up the Recent Colors palette: the payload its chips drag.
type RecentSetup = Box<dyn for<'p> Fn(SwatchPalette<'p>) -> SwatchPalette<'p>>;

/// The palette setup that drags `map`'s payload for the chip under the pointer.
fn recent_setup<P: Send + Sync + 'static>(map: impl Fn(usize) -> Option<P> + Send + Sync + 'static) -> RecentSetup {
    let map = Arc::new(map);
    Box::new(move |palette| {
        let map = Arc::clone(&map);
        palette.drag_payload(move |drag| map(drag.grabbed))
    })
}

/// What the host gave the pass, borrowed from the panel.
#[derive(Clone, Copy)]
struct Inputs<'a> {
    wells: Option<&'a [PairWell; 2]>,
    edited: Option<&'a Option<PickedColor>>,
    shown: &'a ColorPanelShown,
    recent: &'a [Srgba],
    recent_setup: Option<&'a RecentSetup>,
}

impl<'a> ColorPanel<'a> {
    /// The panel on the app's `[foreground, background]`.
    pub fn new(state: Bind<'a, ColorPanelState>, colors: [Srgba; 2]) -> Self {
        ColorPanel {
            state,
            colors,
            cmyk: None,
            rgb: None,
            dynamic: true,
            layout: ColorPanelLayout::default(),
            wells: None,
            edited: None,
            shown: None,
            recent: &[],
            recent_setup: None,
            gamut: None,
        }
    }

    /// The document's CMYK converter, for the CMYK sliders, the CMYK Spectrum ramp and the gamut warning; without it
    /// they use the built-in coated CMYK.
    pub fn cmyk(mut self, converter: &'a CmykConverter) -> Self {
        self.cmyk = Some(converter);
        self
    }

    /// The host's working-RGB colour model, whose channels replace the built-in `rgb` rows (wide working spaces show
    /// their own numbers).
    pub fn working_rgb(mut self, model: &'a dyn ColorModel) -> Self {
        self.rgb = Some(model);
        self
    }

    /// Dynamic Color Sliders (PhotoCraft's interface preference, on by default): the slider tracks show the channel's
    /// colours with the other channels as they are. Off, the tracks keep the colours they had when a drag began.
    pub fn dynamic_tracks(mut self, on: bool) -> Self {
        self.dynamic = on;
        self
    }

    /// How the panel arranges its body; the Photoshop foreground and background arrangement by default.
    pub fn layout(mut self, layout: ColorPanelLayout) -> Self {
        self.layout = layout;
        self
    }

    /// The host's wells (paints) and the active well; `active` replaces `ColorPanelState::background` as the edited
    /// well.
    pub fn wells(mut self, wells: [PairWell; 2], active: Bind<'a, Well>) -> Self {
        self.wells = Some((wells, active));
        self
    }

    /// The colour the rows edit, with its model values; `None` disables the rows (mixed, None, a pattern). The edited
    /// well's colour by default.
    pub fn edited(mut self, color: Option<PickedColor>) -> Self {
        self.edited = Some(color);
        self
    }

    /// Which rows show, by the host's model rule; `Mode(state.mode)` by default.
    pub fn shown(mut self, shown: ColorPanelShown) -> Self {
        self.shown = Some(shown);
        self
    }

    /// The Recent Colors, a spaced `SwatchPalette` that wraps and drags.
    pub fn recent(mut self, colors: &'a [Srgba]) -> Self {
        self.recent = colors;
        self
    }

    /// The payload a Recent Colors chip drags.
    pub fn recent_payload<P: Send + Sync + 'static>(mut self, map: impl Fn(usize) -> Option<P> + Send + Sync + 'static) -> Self {
        self.recent_setup = Some(recent_setup(map));
        self
    }

    /// The host's gamut for the out-of-gamut chip and the spectrum.
    pub fn gamut(mut self, gamut: Gamut<'a>) -> Self {
        self.gamut = Some(gamut);
        self
    }

    /// Shows the panel and returns this pass's edits and requests.
    pub fn show(self, ui: &mut Ui) -> ColorPanelOutput {
        let ColorPanel { state, colors, cmyk, rgb, dynamic, layout, wells, edited, shown: rows, recent, recent_setup, gamut } = self;
        let env = Env::of(ui);
        let (words, dialog) = (env.words::<ColorPanelWords>(), env.words::<ColorPickerDialogWords>());
        let mut held = Held::new(state);
        let (wells, active) = match wells {
            Some((wells, active)) => (Some(wells), Some(Held::new(active))),
            None => (None, None),
        };
        let mut active = active;
        let converter = cmyk.unwrap_or_else(|| CmykConverter::coated());
        let gamut = gamut.unwrap_or_else(|| Gamut::cmyk(converter.clone()));
        let rgb: &dyn ColorModel = rgb.unwrap_or(&RgbModel);
        let (mut edits, mut pick_color, mut add_to_swatches) = (Vec::new(), None, None);
        let (mut pair, mut tint, mut none, mut recolor) = (None, None, None, None);
        let mut shown = Stack::new().label(words.title.clone()).fill().show(ui, |ui, cx| {
            let before = held.read(cx)?;
            let was = match active.as_mut() {
                Some(active) => active.read(cx)?,
                None if before.background => Well::Secondary,
                None => Well::Primary,
            };
            let target = if was == Well::Secondary { PanelTarget::Background } else { PanelTarget::Foreground };
            let default_rows = ColorPanelShown::Mode(before.mode);
            let body = Body {
                target,
                active: was,
                layout,
                colors,
                converter,
                rgb,
                gamut: &gamut,
                dynamic,
                words: &words,
                dialog: &dialog,
                inputs: Inputs {
                    wells: wells.as_ref(),
                    edited: edited.as_ref(),
                    shown: rows.as_ref().unwrap_or(&default_rows),
                    recent,
                    recent_setup: recent_setup.as_ref(),
                },
                now: before,
                kept: cx.memory::<Kept>().clone(),
                events: Vec::new(),
                pick_color: None,
                add_to_swatches: None,
                pair: None,
                tint: None,
                none: None,
                recolor: None,
            };
            let mut body = match layout {
                ColorPanelLayout::ForegroundBackground => body.show(ui),
                ColorPanelLayout::FillStroke => body.fill_stroke(ui),
            };
            *cx.memory::<Kept>() = std::mem::take(&mut body.kept);
            edits = body.events;
            pick_color = body.pick_color;
            add_to_swatches = body.add_to_swatches;
            pair = body.pair;
            tint = body.tint;
            none = body.none;
            recolor = body.recolor;
            Some((before, body.now, was, body.active))
        });
        // Once per pass, after the real run: the edited state and the host's active well are stored.
        shown.settle(|cx, state| {
            if let Some((before, now, was, now_active)) = state.take() {
                if now != before {
                    held.write(cx, &before, now, words.title.as_str());
                }
                if let Some(active) = active.as_mut()
                    && now_active != was
                {
                    active.write(cx, &was, now_active, words.title.as_str());
                }
            }
        });
        ColorPanelOutput { response: shown.output, edits, pick_color, add_to_swatches, pair, tint, none, recolor }
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
    /// Hide or show the options of the FillStroke layout.
    #[command(label = "Hide Options")]
    ToggleOptions,
    /// Show the Recent Colors slot.
    #[command(label = "Show Recent Colors", kind = "toggle")]
    ShowRecentColors,
    /// Invert the edited colour.
    #[command(label = "Invert")]
    Invert,
    /// Replace the edited colour by its complement.
    #[command(label = "Complement")]
    Complement,
    /// Create a swatch of the edited colour.
    #[command(label = "Create New Swatch…")]
    NewSwatch,
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
    /// The rows. `ForegroundBackground`: the nine views, a separator, Copy Color as HTML and Copy Color's Hex Code, a
    /// separator, the four ramps, a separator, Make Ramp Web Safe. `FillStroke`: Hide Options and Show Recent Colors, a
    /// separator, the five modes, a separator, Invert, Complement and Create New Swatch…, a separator, Copy Color Value
    /// (Hex).
    pub fn rows(layout: ColorPanelLayout) -> Vec<PanelRow> {
        let row = |command: ColorPanelCommand| PanelRow::Command(CommandRef::from(command));
        match layout {
            ColorPanelLayout::ForegroundBackground => {
                let mut rows: Vec<PanelRow> = ColorPanelMode::ALL.iter().map(|mode| row(mode.command())).collect();
                rows.extend([PanelRow::Separator, row(ColorPanelCommand::CopyHtml), row(ColorPanelCommand::CopyHex), PanelRow::Separator]);
                rows.extend(Self::ramp_rows());
                rows
            }
            ColorPanelLayout::FillStroke => {
                let mut rows = vec![row(ColorPanelCommand::ToggleOptions), row(ColorPanelCommand::ShowRecentColors), PanelRow::Separator];
                rows.extend(ColorPanelMode::FILL_STROKE.iter().map(|mode| row(mode.command())));
                rows.extend([
                    PanelRow::Separator,
                    row(ColorPanelCommand::Invert),
                    row(ColorPanelCommand::Complement),
                    row(ColorPanelCommand::NewSwatch),
                    PanelRow::Separator,
                    row(ColorPanelCommand::CopyHex),
                ]);
                rows
            }
        }
    }

    /// The commands of `layout`'s rows, in their order.
    fn commands(layout: ColorPanelLayout) -> Vec<ColorPanelCommand> {
        match layout {
            ColorPanelLayout::ForegroundBackground => {
                let commands = ColorPanelMode::ALL.iter().map(|mode| mode.command());
                let commands = commands.chain([ColorPanelCommand::CopyHtml, ColorPanelCommand::CopyHex]);
                commands.chain(ColorPanelRamp::ALL.iter().map(|ramp| ramp.command())).chain([ColorPanelCommand::WebSafeRamp]).collect()
            }
            ColorPanelLayout::FillStroke => [ColorPanelCommand::ToggleOptions, ColorPanelCommand::ShowRecentColors]
                .into_iter()
                .chain(ColorPanelMode::FILL_STROKE.iter().map(|mode| mode.command()))
                .chain([ColorPanelCommand::Invert, ColorPanelCommand::Complement, ColorPanelCommand::NewSwatch, ColorPanelCommand::CopyHex])
                .collect(),
        }
    }

    /// The ramps, a separator and Make Ramp Web Safe: the last rows of the menu and the ramp's context menu.
    pub fn ramp_rows() -> Vec<PanelRow> {
        let row = |command: ColorPanelCommand| PanelRow::Command(CommandRef::from(command));
        let mut rows: Vec<PanelRow> = ColorPanelRamp::ALL.iter().map(|ramp| row(ramp.command())).collect();
        rows.extend([PanelRow::Separator, row(ColorPanelCommand::WebSafeRamp)]);
        rows
    }

    /// The state of each command of `layout`: the current view (under `FillStroke`, the one `shown` shows; a tint checks
    /// none) and ramp checked, the toggles checked when on, and the commands that need a colour disabled without one.
    /// Show Options is titled so while the options are hidden.
    pub fn states(
        layout: ColorPanelLayout,
        state: &ColorPanelState,
        shown: &ColorPanelShown,
        has_color: bool,
    ) -> Vec<(ColorPanelCommand, CommandState)> {
        let needs_color = [
            ColorPanelCommand::CopyHtml,
            ColorPanelCommand::CopyHex,
            ColorPanelCommand::Invert,
            ColorPanelCommand::Complement,
            ColorPanelCommand::NewSwatch,
        ];
        Self::commands(layout)
            .into_iter()
            .map(|command| {
                let mut flag = Self::checked(layout, state, shown, command).map_or_else(CommandState::new, |on| CommandState::new().checked(on));
                if !has_color && needs_color.contains(&command) {
                    flag = flag.enabled(false);
                }
                if command == ColorPanelCommand::ToggleOptions && state.hide_options {
                    flag = flag.title("Show Options");
                }
                (command, flag)
            })
            .collect()
    }

    /// Whether `command`'s row is checked: `Some` for the views (by `state` under `ForegroundBackground`, by `shown`
    /// under `FillStroke`), the ramps and the toggles, `None` for the other commands, which have no check.
    pub fn checked(layout: ColorPanelLayout, state: &ColorPanelState, shown: &ColorPanelShown, command: ColorPanelCommand) -> Option<bool> {
        if let Some(mode) = ColorPanelMode::ALL.into_iter().find(|mode| mode.command() == command) {
            return Some(match layout {
                ColorPanelLayout::ForegroundBackground => state.mode == mode,
                ColorPanelLayout::FillStroke => matches!(shown, ColorPanelShown::Mode(shown) if *shown == mode),
            });
        }
        if let Some(ramp) = ColorPanelRamp::ALL.into_iter().find(|ramp| ramp.command() == command) {
            return Some(state.ramp == ramp);
        }
        match command {
            ColorPanelCommand::WebSafeRamp => Some(state.web_safe_ramp),
            ColorPanelCommand::ShowRecentColors => Some(state.recent_colors),
            _ => None,
        }
    }

    /// Carries out `command`: sets the view or the ramp, toggles the options, Recent Colors or Make Ramp Web Safe, or
    /// answers what the host does: show a mode, copy the text of `edited` (as HTML, `color="#rrggbb"`, or as a hex code,
    /// `rrggbb`), invert, complement, create a swatch.
    pub fn run_command(state: &mut ColorPanelState, command: ColorPanelCommand, edited: Srgba) -> Option<ColorPanelAction> {
        if let Some(mode) = ColorPanelMode::ALL.into_iter().find(|mode| mode.command() == command) {
            state.mode = mode;
            return Some(ColorPanelAction::Mode(mode));
        }
        if let Some(ramp) = ColorPanelRamp::ALL.into_iter().find(|ramp| ramp.command() == command) {
            state.ramp = ramp;
            return None;
        }
        let hex = || Srgba::from_rgb_array([edited.r, edited.g, edited.b]).to_hex();
        match command {
            ColorPanelCommand::WebSafeRamp => state.web_safe_ramp = !state.web_safe_ramp,
            ColorPanelCommand::ToggleOptions => state.hide_options = !state.hide_options,
            ColorPanelCommand::ShowRecentColors => state.recent_colors = !state.recent_colors,
            ColorPanelCommand::CopyHtml => return Some(ColorPanelAction::Copy(format!("color=\"{}\"", hex()))),
            ColorPanelCommand::CopyHex => return Some(ColorPanelAction::Copy(hex().trim_start_matches('#').to_owned())),
            ColorPanelCommand::Invert => return Some(ColorPanelAction::Invert),
            ColorPanelCommand::Complement => return Some(ColorPanelAction::Complement),
            ColorPanelCommand::NewSwatch => return Some(ColorPanelAction::NewSwatch),
            _ => {}
        }
        None
    }
}

/// What the panel draws and collects in one pass.
struct Body<'a> {
    /// The colour the panel edits.
    target: PanelTarget,
    /// The well in front, which `target` follows.
    active: Well,
    layout: ColorPanelLayout,
    colors: [Srgba; 2],
    converter: &'a CmykConverter,
    /// The model of the R, G and B rows: the host's working RGB, else sRGB.
    rgb: &'a dyn ColorModel,
    gamut: &'a Gamut<'a>,
    dynamic: bool,
    words: &'a ColorPanelWords,
    dialog: &'a ColorPickerDialogWords,
    /// The state as this pass leaves it.
    now: ColorPanelState,
    kept: Kept,
    inputs: Inputs<'a>,
    events: Vec<(PanelTarget, EditEvent<PickedColor>)>,
    pick_color: Option<ColorRequest>,
    add_to_swatches: Option<PanelTarget>,
    pair: Option<SwatchPairOutput>,
    tint: Option<EditEvent<f32>>,
    none: Option<PanelTarget>,
    recolor: Option<RampRecolor>,
}

impl Body<'_> {
    /// The colour the rows edit: the host's `edited` colour, else the edited well's.
    fn color(&self) -> Srgba {
        self.picked().map_or(self.colors[self.target.index()], |picked| picked.color)
    }

    /// The host's colour with its model values, when it gave one.
    fn picked(&self) -> Option<&PickedColor> {
        self.inputs.edited.and_then(Option::as_ref)
    }

    /// Whether the rows edit anything: the host gave no `edited(None)`.
    fn enabled(&self) -> bool {
        self.inputs.edited.is_none_or(Option::is_some)
    }

    fn push(&mut self, target: PanelTarget, event: EditEvent<PickedColor>) {
        self.events.push((target, event));
    }

    /// The cubes take the height the readout leaves them, as Photoshop's field fills its dock group. In the slider modes
    /// the channel rows come first and the ramp under them takes the room they leave, at least a colour band thick, as
    /// Photoshop's does. In a group shorter than the content at its least the body scrolls.
    fn show(mut self, ui: &mut Ui) -> Self {
        let mode = self.now.mode;
        Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| {
            self.content(ui);
            if mode.is_sliders() {
                self.ramp(ui);
            }
        });
        self
    }

    /// The wells with their warnings beside the view of the mode; under the cubes their readout, under the wheel its
    /// button.
    fn content(&mut self, ui: &mut Ui) {
        let mode = self.now.mode;
        let top = if mode.is_cube() { Row::new().fill() } else { Row::new() };
        top.show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                self.wells(ui);
                // The cubes show the out-of-gamut chip only: the web-colour chip is for Web Color.
                if mode.is_sliders() || mode.is_cube() {
                    self.warnings(ui, mode == ColorPanelMode::WebColorSliders);
                }
            });
            match mode {
                ColorPanelMode::HueCube => self.cube(ui, AreaMode::Hue),
                ColorPanelMode::BrightnessCube => self.cube(ui, AreaMode::Brightness),
                _ => self.mode_rows(ui, mode),
            }
        });
        match mode {
            ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube => self.readout(ui),
            ColorPanelMode::ColorWheel => self.wheel(ui),
            _ => {}
        }
    }

    /// The channel rows of the slider `mode`. They stand as close as Photoshop's, their letters, fields and units in
    /// shared columns.
    fn mode_rows(&mut self, ui: &mut Ui, mode: ColorPanelMode) {
        let rgb = self.rgb;
        FieldGroup::new(mode.label(self.words)).labels(LabelScheme::Inspector).gap(Gap::Tight).show(ui, |ui, _| match mode {
            ColorPanelMode::GrayscaleSlider => self.rows(ui, &GrayModel, false),
            ColorPanelMode::RgbSliders => self.rows(ui, rgb, false),
            ColorPanelMode::CmykSliders => self.rows(ui, &CmykModel::new(self.converter.clone()), false),
            ColorPanelMode::LabSliders => self.rows(ui, &LabModel, false),
            ColorPanelMode::WebColorSliders => self.rows(ui, rgb, true),
            _ => self.rows(ui, &HsbModel, false),
        });
    }

    /// The foreground over the background (the host's wells when it gave them): a click on the other well edits it, a
    /// click on the edited one asks for the picker. `FillStroke` adds the pair's swap and default buttons.
    fn wells(&mut self, ui: &mut Ui) {
        let was = self.active;
        let mut active = was;
        let wells = self.inputs.wells.cloned().unwrap_or_else(|| {
            [PairWell::new(self.words.foreground.clone(), self.colors[0]), PairWell::new(self.words.background.clone(), self.colors[1])]
        });
        let pair = SwatchPair::<()>::new(self.words.wells.clone(), wells, Bind::bound(&mut active)).popover(false).frame_active(true);
        let pair = match self.layout {
            ColorPanelLayout::ForegroundBackground => pair,
            ColorPanelLayout::FillStroke => pair.swap_button(self.words.swap_colors.clone()).default_button(self.words.default_colors.clone()),
        };
        let pair = pair.show(ui);
        self.active = active;
        if self.inputs.wells.is_none() {
            self.now.background = active == Well::Secondary;
        }
        if pair.clicked == Some(was) {
            let title = match self.target {
                PanelTarget::Foreground => self.dialog.foreground_title.clone(),
                PanelTarget::Background => self.dialog.background_title.clone(),
            };
            self.pick_color = Some(ColorRequest::new(self.target.key(), self.color()).title(title));
        }
        self.pair = Some(pair);
    }

    /// The out-of-gamut chip and, for Web Color, the web-colour chip under the wells, each its icon beside a small swatch.
    fn warnings(&mut self, ui: &mut Ui, web: bool) {
        let color = self.color();
        let rgb = [color.r, color.g, color.b];
        let nearest = |near: [f32; 3]| Srgba::rgb(near[0], near[1], near[2]);
        let mut warnings = Vec::new();
        if let Some(near) = self.gamut.nearest(color) {
            warnings.push(ColorWarning::new(self.dialog.out_of_gamut.clone(), near));
        }
        if web && let Some(near) = not_web_safe(rgb) {
            warnings.push(ColorWarning::web_colors(self.dialog.not_web_color.clone(), nearest(near)));
        }
        Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
            for warning in warnings.clone() {
                if ColorWarningChip::new(&warning).arrangement(ColorWarningArrangement::Beside).show(ui).clicked {
                    let picked = PickedColor { color: warning.nearest().with_alpha(color.a), model: None };
                    let name = self.words.title.as_str();
                    self.push(self.target, EditEvent::set(Mixed::Same(PickedColor { color, model: None }), picked, name.into(), None));
                }
            }
        });
    }

    /// The model's values for the edited colour, unrounded: an edit of one channel leaves the others as they are.
    fn values(&self, model: &dyn ColorModel) -> ModelValues {
        let color = self.color();
        // The host's own values of the colour (typed CMYK, a working space's numbers) stand when they give the colour.
        let given = self.picked().and_then(|picked| picked.model);
        let kept = self.kept.values[self.target.index()]
            .iter()
            .chain(given.as_ref())
            .find(|v| v.model == model.id() && model.to_srgb(&v.values).same_rgb8(color));
        finite_values(kept.copied().unwrap_or_else(|| self.read(model, color)))
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

    /// The channel rows of `model`, one line each: the channel's letter, which scrubs the value, the slider over the
    /// channel's colours, and the number field with its unit. Web Color has sliders that step by the web levels.
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
            // The R, G and B rows count percents of the channel's range; an edit goes back through `scale`.
            // PLAN: a double click on the units cell flips `rgb_percent` while the rows are enabled; `ChannelRow`
            // (color-w2) reports no signal for its units cell, so the panel cannot read it.
            let percent = self.now.rgb_percent && model.id() == self.rgb.id();
            let scale = if percent { 100.0 / *rule.range.end() } else { 1.0 };
            let unit = if percent { ChannelUnit::Percent } else { ChannelUnit::of(ch.unit) };
            if percent {
                rule = ChannelRule { range: 0.0..=100.0, step: (rule.step * scale).max(1.0), decimals: 0 };
            }
            let mut track_values = basis;
            // The hue track is the whole spectrum, whatever the saturation and brightness.
            if model.id() == HsbModel.id() && i == 0 {
                track_values.values[1] = 100.0;
                track_values.values[2] = 100.0;
            }
            let track = model.track(i, &track_values);
            // The field shows the channel's places, as Photoshop's; the edits start from the unrounded values.
            let places = 10f64.powi(i32::from(rule.decimals));
            let shown = (f64::from(current.values[i]) * scale * places).round() / places;
            let row = ChannelRow::new(channel_name(model, i, &[model]), Bind::given(shown), rule).short(ch.label).unit(unit).track(track).show(ui);
            for event in row.into_iter().flat_map(|row| row.events) {
                self.channel_event(model, current, i, event.map(|x| x / scale));
            }
        }
    }

    /// Hue Cube or Brightness Cube: the area and the strip of `mode`, in the room the wells and the readout leave.
    fn cube(&mut self, ui: &mut Ui, mode: AreaMode) {
        let model = mode.model();
        let values = self.values(model);
        let [across, down] = mode.area();
        let names = [channel_name(model, across, &[model]), channel_name(model, down, &[model])];
        let (area_name, strip_name) = (Label::from(mode.name()), channel_name(model, mode.strip(), &[model]));
        let (area, strip) = Row::new()
            .fill()
            .show(ui, |ui, _| {
                let area = ColorArea::new(area_name.clone(), mode, Bind::given(values)).names(names.clone()).sizing(Sizing::Fill).show(ui);
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
        for event in wheel.edit.events {
            self.model_event(&HsbModel, event);
        }
        let add = Row::new().align(RowAlign::End).show(ui, |ui, _| {
            IconButton::new(IconId::new("plus")).label(self.words.add_to_swatches.clone()).tooltip(self.words.create_swatch.clone()).show(ui).response
        });
        if add.inner.as_ref().is_some_and(clicked) {
            self.add_to_swatches = Some(self.target);
        }
    }

    /// The ramp's map into the host's gamut, and the counter that recomputes the ramp's colours when the CMYK working
    /// space changes. The map is the gamut's own `nearest`, so the warning chip and the ramp agree.
    // PLAN: `Gamut` has no identity of its own; a host gamut that changes while the converter stays the same needs a
    // revision on `Gamut` (color-w1).
    fn ramp_gamut(&self) -> (Gamut<'_>, u64) {
        let gamut = self.gamut;
        let mut hasher = DefaultHasher::new();
        Arc::as_ptr(self.converter.profile()).hash(&mut hasher);
        (Gamut::new(move |color| gamut.nearest(color).unwrap_or(color)), hasher.finish())
    }

    /// The picks of a ramp as edits. Alt is on every event of a gesture that has a value; the others go where it went.
    fn ramp_events(&mut self, events: impl IntoIterator<Item = EditEvent<RampPick>>) {
        for event in events {
            if let Some(pick) = event.value() {
                self.kept.other = pick.other;
            }
            let target = if self.kept.other { self.target.other() } else { self.target };
            let alpha = self.colors[target.index()].a;
            self.push(target, event.map(|pick| PickedColor { color: pick.color.with_alpha(alpha), model: None }));
        }
    }

    /// The ramp; Alt picks for the colour not edited, Shift-click cycles the kind, a secondary click opens the ramp
    /// menu.
    fn ramp(&mut self, ui: &mut Ui) {
        let (gamut, revision) = self.ramp_gamut();
        let ramp = ColorRamp::new(self.words.color_ramp.clone(), self.now.ramp.kind(), Bind::kept())
            .kind_name(self.now.ramp.label(self.words))
            .ends([self.colors[0], self.colors[1]])
            .gamut(gamut)
            .gamut_revision(revision)
            .web_safe(self.now.web_safe_ramp)
            .sizing(Sizing::Fill)
            .show(ui);
        self.ramp_events(ramp.edit.events);
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

#[cfg(test)]
pub(crate) mod tests {
    use craft_ui::testing::trial;

    use super::*;

    /// The checks a host draws (`checked`) and the ones it publishes (`states`) agree in every layout: under
    /// `ForegroundBackground` the state says which view is checked, under `FillStroke` the shown rows do (a tint checks
    /// none), and the copy commands carry no check.
    #[trial]
    fn color_panel_menu_checks_follow_the_state() {
        let state =
            ColorPanelState { mode: ColorPanelMode::CmykSliders, ramp: ColorPanelRamp::GrayscaleRamp, web_safe_ramp: true, ..Default::default() };
        let shown = ColorPanelShown::Mode(ColorPanelMode::HsbSliders);
        for layout in [ColorPanelLayout::ForegroundBackground, ColorPanelLayout::FillStroke] {
            for (command, published) in ColorPanelMenu::states(layout, &state, &shown, true) {
                let expected =
                    ColorPanelMenu::checked(layout, &state, &shown, command).map_or_else(CommandState::new, |on| CommandState::new().checked(on));
                assert_eq!(published, expected, "{layout:?} {command:?}");
            }
            assert_eq!(ColorPanelMenu::checked(layout, &state, &shown, ColorPanelCommand::CopyHex), None, "{layout:?}");
        }
        let checked = |layout: ColorPanelLayout, shown: &ColorPanelShown| -> Vec<ColorPanelCommand> {
            ColorPanelMenu::states(layout, &state, shown, true)
                .into_iter()
                .map(|(command, _)| command)
                .filter(|c| ColorPanelMenu::checked(layout, &state, shown, *c) == Some(true))
                .collect()
        };
        assert_eq!(
            checked(ColorPanelLayout::ForegroundBackground, &shown),
            [ColorPanelCommand::CmykSliders, ColorPanelCommand::GrayscaleRamp, ColorPanelCommand::WebSafeRamp]
        );
        assert_eq!(checked(ColorPanelLayout::FillStroke, &shown), [ColorPanelCommand::ShowRecentColors, ColorPanelCommand::HsbSliders]);
        let tint = ColorPanelShown::Tint(TintRow::new("Brand", Srgba::rgb(1.0, 0.0, 0.0), 50.0));
        assert_eq!(checked(ColorPanelLayout::FillStroke, &tint), [ColorPanelCommand::ShowRecentColors]);
        let without = ColorPanelMenu::states(ColorPanelLayout::FillStroke, &state, &shown, false);
        assert!(without.contains(&(ColorPanelCommand::Invert, CommandState::new().enabled(false))), "no colour, nothing to invert");
    }
}
