//! `ColorPicker`: the colour picker's body, the area and strip with the channel rows, the hex field and the hooks a
//! host adds. A container: its body lays the parts out with the form layouts, the "Field shows" choice is a
//! `RadioGroup` and the channel rows share one label column.

mod channel;
mod compact;
mod full;
mod screen_pick;
#[cfg(feature = "screen-testing")]
mod stories;
mod value;

use std::sync::Arc;

use craft_behavior::{EditEvent, EditEvents, GestureId, Mixed, Value};
use craft_theme::{ColorModel, ModelValues, Srgba, builtin_models};
use craft_ui::contract::{Bind, BindMut, BodyCx, EditResponse, Outcome, Props as _, Remember as _, ShowContainer};
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::Label;
use egui::{Pos2, Response, Ui};

use super::AreaMode;
pub(crate) use channel::{ChannelRow, ChannelRule, channel_name};
pub use full::{ColorWarning, ColorWarningChip, ColorWarningChipOutput};
pub use value::ColorConstraint;
use value::Rules;

/// What a [`ColorPicker`] reports: the colour, and the values of the model the user edited it in while they still
/// give it, so a host can keep a CMYK colour CMYK.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PickedColor {
    /// Encoded sRGB with straight alpha, after the host's [`constrain`](ColorPicker::constrain).
    pub color: Srgba,
    /// The values of the model edited, as the user set them: a typed Lab or CMYK colour outside sRGB stays as typed.
    /// `None` for a colour set as such (hex, a sample) and for one the constraint moved away from its values.
    pub model: Option<ModelValues>,
}

impl PickedColor {
    /// A colour set as such (hex, a sample): `color` is encoded sRGB with straight alpha and no model values stay
    /// with it.
    pub fn rgb(color: Srgba) -> PickedColor {
        PickedColor { color, model: None }
    }

    /// A colour picked in the model of `values`, which stay with it as typed. `color` is what `values` give in
    /// encoded sRGB; the caller converts it, as the model decides.
    pub fn in_model(color: Srgba, values: ModelValues) -> PickedColor {
        PickedColor { color, model: Some(values) }
    }

    /// A colour picked as CMYK `inks` in percent (cyan, magenta, yellow, black, each 0 to 100, the units of the
    /// picker's fields). `color` is the sRGB the inks convert to under the host's CMYK converter.
    pub fn cmyk(color: Srgba, inks: [f32; 4]) -> PickedColor {
        PickedColor::in_model(color, ModelValues::new("cmyk", &inks))
    }
}

/// How a [`ColorPicker`] lays out its parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColorPickerLayout {
    /// The Color panel and compact popup: the area over the channel slider rows of one model, a model switch, the hex
    /// row and the optional alpha row.
    #[default]
    Compact,
    /// The Color Picker dialog: the square area and the strip, the compare swatch and the host's warnings beside them
    /// over the channel grid, every model's channels at once with radios choosing what the area shows, and the hex
    /// field.
    Full,
}

/// The screen-pick button's press, for the host that samples the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ScreenPickGesture {
    /// The button was clicked: the host starts sampling and answers with [`ColorPicker::sampled`].
    Clicked,
    /// The button is held and the pointer moved to `at`: Photoshop's press and drag.
    #[non_exhaustive]
    Held {
        /// The pointer, in the picker's points.
        at: Pos2,
    },
    /// The held button was released at `at`.
    #[non_exhaustive]
    Released {
        /// The pointer, in the picker's points.
        at: Pos2,
    },
}

/// What [`ColorPicker`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the edit event"]
pub struct ColorPickerOutput {
    /// The picker's response.
    pub response: Response,
    /// The picker's edits of its colour in this pass, in order, each with the whole colour as its value; empty when
    /// nothing was edited. A colour bound by reference has taken them; a given one needs the host to apply them.
    pub events: EditEvents<PickedColor>,
    /// The screen-pick button's press this pass.
    pub screen_pick: Option<ScreenPickGesture>,
}

impl Outcome for ColorPickerOutput {
    type Value = PickedColor;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        let mut facts = craft_ui::testing::StoryOutput::default();
        facts.id = Some(self.response.id);
        facts.clicked = craft_ui::contract::clicked(&self.response);
        facts.edits = self.events.iter().filter_map(craft_ui::testing::EditKind::of).collect();
        facts
    }
}

craft_ui::words! {
    /// The picker's words, English by default. The words of the colour models and their channels are the models'.
    #[non_exhaustive]
    pub struct ColorPickerWords {
        /// The opacity row's name.
        opacity: "colorPicker.opacity" = "Opacity";
        /// The hex field's accessible name.
        hex: "colorPicker.hex" = "Hex color";
        /// The model switch's accessible name; `{model}` is the model's name.
        color_model: template "colorPicker.colorModel" = "Color model: {model}";
        /// The Full layout's radios.
        field_shows: "colorPicker.fieldShows" = "Field shows";
        /// The caption over the compare swatch's new half.
        new_caption: "colorPicker.newCaption" = "new";
        /// The caption under the compare swatch's current half.
        current_caption: "colorPicker.currentCaption" = "current";
        /// The new half's accessible name.
        new_color: "colorPicker.newColor" = "New color";
        /// The current half's accessible name and tooltip.
        restore: "colorPicker.restore" = "Restore current color";
        /// A channel's label in the Full grid; `{label}` is the channel's short label.
        grid_label: template "colorPicker.gridLabel" = "{label}:";
        /// The check box under the Full strip.
        only_web_colors: "colorPicker.onlyWebColors" = "Only Web Colors";
        /// The screen-pick button's name and tooltip.
        sample_screen: "colorPicker.sampleScreen" = "Sample color from the screen";
        /// The channel rows' group name.
        channels: "colorPicker.channels" = "Channels";
    }
}

/// What the picker keeps between passes.
#[derive(Clone, Default)]
struct Memo {
    /// The model of the Compact rows.
    model: Option<&'static str>,
    /// What the area and strip show while the host holds no mode.
    mode: AreaMode,
    /// The Full layout has taken the focus for the hex field.
    opened: bool,
    /// The running gesture of a child and the colour when it began, for the `before` of its end.
    gesture: Option<(GestureId, PickedColor)>,
}

/// What the children of one pass reported.
#[derive(Default)]
struct Collected {
    /// The children's edits as edits of the colour shown, in the order they were made.
    edits: EditEvents<PickedColor>,
    /// Colours the compare swatch and the warning chips set.
    clicks: Vec<Srgba>,
    screen_pick: Option<ScreenPickGesture>,
    model_pick: Option<&'static str>,
    /// The mode the "Field shows" group picked.
    mode_pick: Option<AreaMode>,
    /// The Only Web Colors check box was set to this.
    web_pick: Option<bool>,
    /// The picker held no colour when the pass began: its edits begin from Mixed.
    empty: bool,
}

impl Collected {
    /// Adopts `edit` of a child as an edit of the picker's colour: `map` applies the child's value to a copy of `shown`,
    /// the colour the pass showed.
    fn adopt<V: Value>(&mut self, edit: EditResponse<V>, shown: PickedColor, map: impl Fn(&V, &mut PickedColor)) {
        let before = if self.empty { Mixed::Mixed } else { Mixed::Same(shown) };
        let made = |value: &V| {
            let mut colour = shown;
            map(value, &mut colour);
            colour
        };
        for event in &edit.events {
            let adopted = match event {
                EditEvent::Begin { id, label, doc_key, .. } => EditEvent::begin(*id, before, label.clone(), *doc_key),
                EditEvent::Update { id, value, .. } => EditEvent::update(*id, made(value)),
                EditEvent::Commit { id, after, .. } => EditEvent::commit(*id, before, made(after)),
                EditEvent::Cancel { id, reason, .. } => EditEvent::cancel(*id, *reason, Some(before)),
                EditEvent::Set { after, label, doc_key, .. } => EditEvent::set(before, made(after), label.clone(), *doc_key),
                _ => continue,
            };
            self.edits.push(adopted);
        }
    }
}

/// What the body settled before it showed its children: the colour shown and the rules its edits go through.
#[derive(Clone)]
struct Pass<'a> {
    shown: PickedColor,
    /// The bound value is Mixed: no colour yet. The fields and the new half of the compare swatch show Mixed until the
    /// first edit; `shown` is the base the edit starts from.
    empty: bool,
    rules: Rules<'a>,
    mode: AreaMode,
    /// The model of the Compact rows.
    model: Option<&'static str>,
    /// Only Web Colors, as the host holds it.
    web: bool,
    /// Whether the hex field takes the focus in this pass: the Full layout's first show.
    focus_hex: bool,
}

impl Pass<'_> {
    /// `value`, or Mixed while the picker holds no colour yet.
    fn shows<T>(&self, value: T) -> Mixed<T> {
        if self.empty { Mixed::Mixed } else { Mixed::Same(value) }
    }
}

/// A colour picker's body: a [`ColorArea`](super::ColorArea) and a [`ColorStrip`](super::ColorStrip), the channel
/// fields of its [`ColorModel`]s, the hex field and the host's hooks, in the Color panel and compact popup layout
/// ([`ColorPickerLayout::Compact`]) or the Color Picker dialog's ([`ColorPickerLayout::Full`]).
///
/// - Compact: the area over the channel rows of one model (a label that scrubs, a slider whose track shows the
///   channel's colours, a number field), a model switch, the hex row with the screen-pick button, and with
///   [`alpha`](ColorPicker::alpha) an "Opacity" row over the checkerboard.
/// - Full: the area square, the strip at its reading end, the opacity row and the screen-pick button under them.
///   Beside them the [`compare`](ColorPicker::compare) swatch (new over current) and the host's
///   [`warnings`](ColorPicker::warnings), over the channel grid: H S B, R G B, then "#" and the hex field; L a b and
///   the host's models; OKLCH in a column of its own when the host turns it on. A radio before each built-in
///   channel's label chooses the [`AreaMode`]; it edits no colour. The hex field takes the focus, its text selected,
///   when the layout first shows ([`focus_on_open`](ColorPicker::focus_on_open)).
/// - Edits: every child's gesture is the picker's, with the whole colour as its value: an area or strip drag, a slider
///   drag, a scrub or a typing run is one Begin to Commit (or Cancel); keys, the wheel, Enter, a sample, a compare or
///   warning click are `Set`s.
/// - Kept components: the picker keeps the values the user last edited, in that model, while they still give the
///   colour's 8-bit sRGB: greys keep their hue, black its hue and saturation, a typed Lab colour outside sRGB its
///   values. [`PickedColor::model`] carries them, and the host's binding holds them between passes.
/// - Only Web Colors ([`web_colors`](ColorPicker::web_colors)): every colour reported has each of red, green and blue
///   on a multiple of 0x33, whichever child made it. The Full layout shows the check box.
///
/// AccessKit: a `Group` named by the label; channel fields and sliders are named by the channel's name ("Hue"), or
/// `<model> <channel>` ("OKLCH hue") while another shown model has a channel of that name.
#[must_use = "show the picker with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct ColorPicker<'a> {
    label: Label,
    value: Bind<'a, PickedColor>,
    /// The layout; Compact by default.
    #[prop]
    layout: ColorPickerLayout,
    models: Option<Vec<&'a dyn ColorModel>>,
    /// What the area and the strip show, held by the host; kept per picker without it.
    #[prop]
    mode: Option<Bind<'a, AreaMode>>,
    /// Only Web Colors, held by the host (kept across opens). The Full layout shows its check box; the Compact layout
    /// has none, the host sets it.
    #[prop]
    web_colors: Option<Bind<'a, bool>>,
    /// Whether the picker edits the alpha: an "Opacity" row, and eight hex digits.
    #[prop]
    alpha: bool,
    /// Whether the channel sliders' tracks show the channel's colours with the others fixed (Dynamic Color Sliders).
    #[prop]
    dynamic_tracks: bool,
    /// Shows the screen-pick button.
    #[prop]
    eyedropper: bool,
    /// Disables the screen-pick button while the host samples.
    #[prop]
    eyedropper_busy: bool,
    /// The Full layout's compare swatch: the colour being edited over this one, the colour the host started from; a
    /// click on it sets it again.
    #[prop(into)]
    compare: Option<Mixed<Srgba>>,
    warnings: Vec<ColorWarning>,
    /// Applied to every colour before it is reported, after Only Web Colors' snap.
    #[prop(func = "Fn(Srgba) -> Srgba")]
    constrain: Option<ColorConstraint<'a>>,
    /// The host's answer to a screen or canvas pick, for this pass; a `Set` that keeps the picker's alpha.
    #[prop]
    sampled: Option<Srgba>,
    /// Whether the Full layout focuses its hex field when it first shows; on by default.
    #[prop]
    focus_on_open: bool,
}

impl<'a> ColorPicker<'a> {
    /// A picker named `label` editing `value`.
    pub fn new(label: impl Into<craft_ui::runtime::env::Label>, value: Bind<'a, PickedColor>) -> Self {
        ColorPicker {
            label: label.into(),
            value,
            layout: ColorPickerLayout::default(),
            models: None,
            mode: None,
            web_colors: None,
            alpha: false,
            dynamic_tracks: true,
            eyedropper: false,
            eyedropper_busy: false,
            compare: None,
            warnings: Vec::new(),
            constrain: None,
            sampled: None,
            focus_on_open: true,
        }
    }

    /// Every key a picker offering `models` may ask the host's catalogue for, each once: the area's names
    /// ([`AreaMode::name`]), and for the built-in models and `models` the model's name and each channel's name,
    /// qualified name ("OKLCH hue") and short label.
    pub fn keys<'m>(models: impl IntoIterator<Item = &'m dyn ColorModel>) -> Vec<&'static str> {
        let builtin: [&dyn ColorModel; 4] = builtin_models();
        let models: Vec<&dyn ColorModel> = builtin.into_iter().chain(models).collect();
        let words = models.iter().flat_map(|m| std::iter::once(m.name()).chain(m.channels().iter().flat_map(|ch| [ch.name, ch.qualified, ch.label])));
        let mut keys: Vec<&'static str> = Vec::new();
        for key in AreaMode::ALL.map(AreaMode::name).into_iter().chain(words) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        keys
    }

    /// The models offered, in order, the host's own among them; the built-in models on by default in the layout
    /// without it.
    pub fn models(mut self, models: impl IntoIterator<Item = &'a dyn ColorModel>) -> Self {
        self.models = Some(models.into_iter().collect());
        self
    }

    /// The warnings beside the Full layout's compare swatch (out-of-gamut and web-colour warnings); a click on one
    /// sets its nearest passing colour. The host gives this pass's warnings for the colour it shows.
    pub fn warnings(mut self, warnings: impl IntoIterator<Item = ColorWarning>) -> Self {
        self.warnings = warnings.into_iter().collect();
        self
    }

    /// The modes the Full layout's radios offer.
    pub(super) fn offered(models: &[&dyn ColorModel]) -> Vec<AreaMode> {
        AreaMode::PHOTOSHOP.into_iter().chain(AreaMode::OKLCH).filter(|mode| models.iter().any(|m| m.id() == mode.model().id())).collect()
    }
}

impl<'a> ColorPicker<'a> {
    /// Shows the picker.
    pub fn show(self, ui: &mut Ui) -> ColorPickerOutput {
        let ColorPicker {
            label,
            mut value,
            layout,
            models,
            mut mode,
            mut web_colors,
            alpha,
            dynamic_tracks,
            eyedropper,
            eyedropper_busy,
            compare,
            warnings,
            constrain,
            sampled,
            focus_on_open,
        } = self;
        let full = layout == ColorPickerLayout::Full;
        let models: Vec<&'a dyn ColorModel> = models
            .unwrap_or_else(|| builtin_models().into_iter().filter(|m| if full { m.default_on().full } else { m.default_on().compact }).collect());
        let parts = compact::Body {
            layout,
            models: models.clone(),
            warnings,
            compare,
            alpha,
            dynamic_tracks,
            eyedropper,
            eyedropper_busy,
            has_web_check: web_colors.is_some(),
        };
        let mut run = Run {
            name: Arc::from(label.as_str()),
            full,
            models: &models,
            value: value.reborrow(),
            mode: mode.as_mut().map(|held| held.reborrow()),
            web: web_colors.as_mut().map(|held| held.reborrow()),
            constrain,
            sampled,
            focus_on_open,
            parts: &parts,
            latest: PickedColor::rgb(Srgba::BLACK),
            empty: false,
            events: EditEvents::new(),
        };
        let shown = Stack::new().label(label).show(ui, |ui, body| run.show(ui, body));
        let (events, screen_pick) = shown.inner.unwrap_or_default();
        ColorPickerOutput { response: shown.output, events, screen_pick }
    }
}

/// One pass of the picker: its body settles the rules, shows the children and applies each child's output to the
/// bound colour in the same pass.
struct Run<'s, 'a> {
    /// The picker's name, the label of the edits it makes itself.
    name: Arc<str>,
    full: bool,
    models: &'s [&'a dyn ColorModel],
    value: BindMut<'s, PickedColor>,
    mode: Option<BindMut<'s, AreaMode>>,
    web: Option<BindMut<'s, bool>>,
    constrain: Option<ColorConstraint<'a>>,
    sampled: Option<Srgba>,
    focus_on_open: bool,
    parts: &'s compact::Body<'a>,
    /// The colour after the edits landed so far.
    latest: PickedColor,
    /// No edit has landed on a colour-less picker yet.
    empty: bool,
    /// The edits landed, in order.
    events: EditEvents<PickedColor>,
}

impl<'a> Run<'_, 'a> {
    fn show(&mut self, ui: &mut Ui, body: &mut BodyCx<'_>) -> (EditEvents<PickedColor>, Option<ScreenPickGesture>) {
        let memo = body.memory::<Memo>().clone();
        let held_mode = self.mode.as_ref().and_then(|held| held.read(&mut *body)).and_then(|held| held.same().copied());
        let mut mode = held_mode.unwrap_or(memo.mode);
        let held_web = self.web.as_ref().and_then(|held| held.read(&mut *body)).and_then(|held| held.same().copied()).unwrap_or(false);
        let rules = Rules { web: held_web, constrain: self.constrain.clone() };

        // The Full layout shows a mode its models offer.
        if self.full {
            let offered = ColorPicker::offered(self.models);
            let wanted = offered.first().copied().filter(|first| !offered.contains(&mode) && *first != mode);
            if let Some(next) = wanted {
                self.write_mode(body, mode, next);
                mode = next;
            }
        }

        let first_show = !memo.opened;
        let model = memo.model.filter(|id| self.models.iter().any(|m| m.id() == *id)).or_else(|| self.models.first().map(|m| m.id()));
        {
            let memo = body.memory::<Memo>();
            memo.mode = mode;
            memo.model = model;
            memo.opened = true;
        }
        let focus_hex = self.full && self.focus_on_open && first_show && body.resolved.enabled;
        let held = self.value.read(&mut *body).and_then(|held| held.same().copied());
        let empty = held.is_none();
        self.empty = empty;
        self.latest = held.unwrap_or(PickedColor::rgb(Srgba::BLACK));
        if let Some(color) = self.sampled {
            let next = self.latest.with_color(color.with_alpha(self.latest.color.a), &rules);
            self.set(body, next);
        }

        let pass = Pass { shown: self.latest, empty, rules: rules.clone(), mode, model, web: held_web, focus_hex };
        let mut got = Collected { empty, ..Collected::default() };
        self.parts.run(ui, &pass, &mut got);

        for edit in got.edits {
            self.land(body, edit);
        }
        for color in got.clicks {
            let next = self.latest.with_color(color, &rules);
            self.set(body, next);
        }
        if let Some(next) = got.mode_pick.filter(|next| *next != mode) {
            self.write_mode(body, mode, next);
            body.memory::<Memo>().mode = next;
        }
        if let Some(on) = got.web_pick.filter(|on| *on != held_web) {
            if let Some(held) = self.web.as_mut() {
                let event = EditEvent::set(Mixed::Same(held_web), on, Arc::from("Only Web Colors"), None);
                let _ = held.write(&mut *body, &event);
            }
            if on {
                let snapped_rules = Rules { web: true, constrain: self.constrain.clone() };
                let snapped = snapped_rules.apply(self.latest.color);
                if !snapped.same_rgb8(self.latest.color) {
                    let next = self.latest.with_color(snapped, &snapped_rules);
                    self.set(body, next);
                }
            }
        }
        if let Some(model) = got.model_pick {
            body.memory::<Memo>().model = Some(model);
        }
        (std::mem::take(&mut self.events), got.screen_pick)
    }

    /// Lands `event`, an edit of the colour: a gesture's end takes the colour it began from, the binding takes the
    /// event, and the pass reports it.
    fn land(&mut self, body: &mut BodyCx<'_>, mut event: EditEvent<PickedColor>) {
        match &mut event {
            EditEvent::Begin { id, before: Mixed::Same(start), .. } => body.memory::<Memo>().gesture = Some((*id, *start)),
            EditEvent::Commit { id, before, .. } => {
                if let Some((_, start)) = body.memory::<Memo>().gesture.take().filter(|(running, _)| *running == *id) {
                    *before = Mixed::Same(start);
                }
            }
            EditEvent::Cancel { id, before, .. } => {
                if let Some((_, start)) = body.memory::<Memo>().gesture.take().filter(|(running, _)| *running == *id) {
                    *before = Some(Mixed::Same(start));
                }
            }
            _ => {}
        }
        match &event {
            EditEvent::Update { value, .. } | EditEvent::Commit { after: value, .. } | EditEvent::Set { after: value, .. } => self.latest = *value,
            EditEvent::Cancel { before: Some(Mixed::Same(start)), .. } => self.latest = *start,
            _ => {}
        }
        let _ = self.value.write(&mut *body, &event);
        self.empty = false;
        self.events.push(event);
    }

    /// A one-shot change of the colour to `next`.
    fn set(&mut self, body: &mut BodyCx<'_>, next: PickedColor) {
        if next == self.latest {
            return;
        }
        let before = if self.empty { Mixed::Mixed } else { Mixed::Same(self.latest) };
        let event = EditEvent::set(before, next, self.name.clone(), None);
        self.land(body, event);
    }

    /// Sets the mode the host holds, if it holds one.
    fn write_mode(&mut self, body: &mut BodyCx<'_>, from: AreaMode, to: AreaMode) {
        if let Some(held) = self.mode.as_mut() {
            let event = EditEvent::set(Mixed::Same(from), to, Arc::from("Field shows"), None);
            let _ = held.write(&mut *body, &event);
        }
    }
}
