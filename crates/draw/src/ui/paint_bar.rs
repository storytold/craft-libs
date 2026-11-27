//! [`PaintBar`]: the options bar members of the Brush, Pencil, Eraser and Mixer Brush tools after the preset picker.

use std::cell::RefCell;
use std::sync::Arc;

use craft_behavior::{EditEvent, EditEvents, IconId, Mixed, Value};
use craft_theme::Srgba;
use craft_ui::buttons::{IconButton, ToggleButton};
use craft_ui::choice::{Checkbox, Swatch, SwatchFill, SwatchImage};
use craft_ui::contract::{
    Bind, BodyCx, Entry, Gap, LabelKind, PopupKind, Props as _, Remember as _, Show as _, ShowContainer as _, SideLabel, Trigger, clicked,
};
use craft_ui::display::Divider;
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::pickers::Select;
use craft_ui::popover::Popover;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::text::{NumberField, RangeAlert};
use egui::{Response, Ui};

#[cfg(feature = "suite")]
pub(crate) mod stories;
mod words;

pub use words::PaintBarWords;

/// The icon of the toggles that let pen pressure control the opacity or the size.
const PRESSURE: &str = "target";

/// The painting tools the bar serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui::contract::MemoProps)]
#[non_exhaustive]
pub enum PaintTool {
    /// The Brush tool.
    Brush,
    /// The Pencil tool.
    Pencil,
    /// The Eraser tool.
    Eraser,
    /// The Mixer Brush tool.
    MixerBrush,
}

/// The bar's values. The size and the hardness are the preset picker's, which comes before the bar.
#[derive(Clone, Debug, PartialEq, craft_ui::contract::MemoProps)]
#[non_exhaustive]
pub struct PaintValues<M> {
    /// The opacity.
    pub opacity: f64,
    /// The flow.
    pub flow: f64,
    /// The stroke smoothing.
    pub smoothing: Smoothing,
    /// Pressure controls the opacity.
    pub pressure_opacity: bool,
    /// Pressure controls the size.
    pub pressure_size: bool,
    /// Build-up (airbrush) is on.
    pub build_up: bool,
    /// The blend mode, as the app's blend key.
    pub mode: M,
    /// The eraser erases to history.
    pub erase_to_history: bool,
    /// The pencil erases where it starts on the foreground colour.
    pub auto_erase: bool,
    /// The Mixer Brush's values.
    pub mixer: MixerValues,
}

impl<M> PaintValues<M> {
    /// Starting values for a host with no stored tool options, with `mode` as the blend mode; a host with stored
    /// options overwrites every field.
    pub fn new(mode: M) -> Self {
        PaintValues {
            opacity: 1.0,
            flow: 1.0,
            smoothing: Smoothing::default(),
            pressure_opacity: false,
            pressure_size: true,
            build_up: false,
            mode,
            erase_to_history: false,
            auto_erase: false,
            mixer: MixerValues::default(),
        }
    }
}

/// The stroke smoothing options.
#[derive(Clone, Copy, Debug, PartialEq, craft_ui::contract::MemoProps)]
#[non_exhaustive]
pub struct Smoothing {
    /// The amount, 0 to 1.
    pub amount: f64,
    /// Pulled string mode.
    pub pulled_string: bool,
    /// The stroke catches up when the pointer pauses.
    pub catch_up: bool,
    /// The stroke catches up at the end.
    pub catch_up_on_end: bool,
    /// The amount adjusts for zoom.
    pub adjust_for_zoom: bool,
}

impl Default for Smoothing {
    /// No smoothing, every option off.
    fn default() -> Self {
        Smoothing { amount: 0.0, pulled_string: false, catch_up: false, catch_up_on_end: false, adjust_for_zoom: false }
    }
}

/// What the Mixer Brush holds: the paint its next stroke starts with.
#[derive(Clone, Debug, Default, PartialEq, craft_ui::contract::MemoProps)]
#[non_exhaustive]
pub enum MixerLoad {
    /// Nothing: the brush is clean.
    #[default]
    Clean,
    /// One colour.
    Color(Srgba),
    /// The tip image an Alt-click sampled.
    Image(SwatchImage),
}

/// The Mixer Brush's values.
#[derive(Clone, Debug, PartialEq, craft_ui::contract::MemoProps)]
#[non_exhaustive]
pub struct MixerValues {
    /// Load the brush after each stroke.
    pub load_after_stroke: bool,
    /// Clean the brush after each stroke.
    pub clean_after_stroke: bool,
    /// The wetness.
    pub wet: f64,
    /// The load.
    pub load: f64,
    /// The mix.
    pub mix: f64,
    /// The flow.
    pub flow: f64,
    /// Sample all layers.
    pub sample_all_layers: bool,
    /// The index into [`MIXER_COMBINATIONS`].
    pub combination: usize,
    /// What the brush holds, which the load swatch shows.
    pub current_load: MixerLoad,
    /// Only solid colours load.
    pub solid_colors_only: bool,
}

impl Default for MixerValues {
    /// The Custom combination, loading after each stroke.
    fn default() -> Self {
        MixerValues {
            load_after_stroke: true,
            clean_after_stroke: false,
            wet: 0.2,
            load: 0.5,
            mix: 0.5,
            flow: 1.0,
            sample_all_layers: false,
            combination: 0,
            current_load: MixerLoad::Clean,
            solid_colors_only: false,
        }
    }
}

/// What the Eraser tool erases with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, craft_ui::contract::MemoProps)]
#[non_exhaustive]
pub enum EraserMode {
    /// A brush.
    #[default]
    Brush,
    /// A pencil.
    Pencil,
    /// A block.
    Block,
}

/// The mixer combinations as (wet, load, mix), fractions 0..1; `None` is Custom.
pub const MIXER_COMBINATIONS: [Option<(f64, f64, f64)>; 13] = [
    None,
    Some((0.0, 0.5, 0.0)),
    Some((0.0, 0.05, 0.0)),
    Some((0.0, 1.0, 0.0)),
    Some((0.1, 0.05, 0.5)),
    Some((0.1, 0.05, 0.0)),
    Some((0.1, 0.05, 1.0)),
    Some((0.5, 0.5, 0.5)),
    Some((0.5, 0.5, 0.0)),
    Some((0.5, 0.5, 1.0)),
    Some((1.0, 0.5, 0.5)),
    Some((1.0, 0.5, 0.0)),
    Some((1.0, 0.5, 1.0)),
];

/// The index of the first combination of each group (Dry, Moist, Wet, Very Wet); a separator goes above each.
pub const MIXER_GROUP_STARTS: [usize; 4] = [1, 4, 7, 10];

/// Whether the smoothing options popover is open.
#[derive(Clone, Default)]
struct SmoothingOpen(bool);

/// The bar's members for one painting tool, after the preset picker that carries the size and the hardness: a rule,
/// Mode, Opacity and the pressure-for-opacity toggle, Flow and the build-up toggle (not the Pencil), Smoothing and its
/// options (the Pencil's Auto Erase in their place), Erase to History for the Eraser, a rule and the
/// pressure-for-size toggle. The Mixer Brush shows its load, combinations and mix values instead.
#[must_use = "show the bar with `show`"]
#[derive(craft_ui::contract::Pattern)]
#[pattern(probe(PaintBar<'static, u32>))]
pub struct PaintBar<'a, M: Value> {
    tool: PaintTool,
    values: &'a mut PaintValues<M>,
    eraser_mode: Option<&'a mut EraserMode>,
    modes: &'a [Entry<M>],
}

/// What [`PaintBar::show`] reports.
#[non_exhaustive]
pub struct PaintBarOutput<M> {
    /// The bar's response.
    pub response: Response,
    /// The edits of the values, one gesture each.
    pub edits: EditEvents<PaintValues<M>>,
    /// The Eraser's mode changed; the bar has already set the mode it was given.
    pub eraser_mode: Option<EditEvent<EraserMode>>,
    /// Load Brush was chosen from the load swatch's menu.
    pub mixer_load: bool,
    /// Clean Brush was chosen from the load swatch's menu.
    pub mixer_clean: bool,
    /// A value was typed out of range, the last one this pass.
    pub out_of_range: Option<RangeAlert>,
}

impl<'a, M: Value> PaintBar<'a, M> {
    /// The bar of `tool` over `values`.
    pub fn new(tool: PaintTool, values: &'a mut PaintValues<M>) -> Self {
        PaintBar { tool, values, eraser_mode: None, modes: &[] }
    }

    /// The Eraser's mode, shown for the Eraser tool.
    pub fn eraser_mode(mut self, mode: &'a mut EraserMode) -> Self {
        self.eraser_mode = Some(mode);
        self
    }

    /// The blend modes the Mode list offers.
    pub fn modes(mut self, modes: &'a [Entry<M>]) -> Self {
        self.modes = modes;
        self
    }

    /// Shows the members.
    pub fn show(self, ui: &mut Ui) -> PaintBarOutput<M> {
        let PaintBar { tool, values, mut eraser_mode, modes } = self;
        let words = Env::of(ui).words::<PaintBarWords>();
        let eraser = if tool == PaintTool::Eraser { eraser_mode.as_deref().copied().unwrap_or_default() } else { EraserMode::Brush };
        // The Block eraser has no brush engine behind it; the Pencil eraser has no soft edge and no flow.
        let sized = eraser != EraserMode::Block;
        let soft = eraser == EraserMode::Brush;
        let mut pass = Pass { values, edits: EditEvents::new(), eraser_edit: None, out_of_range: None, load: false, clean: false };
        let shown = Row::new().gap(Gap::Item).show(ui, |ui, cx| {
            if tool == PaintTool::MixerBrush {
                pass.mixer_load(ui, &words);
                pass.check(ui, &words.load_after_stroke, true, None, pass.values.mixer.load_after_stroke, |v, x| v.mixer.load_after_stroke = x);
                pass.check(ui, &words.clean_after_stroke, true, None, pass.values.mixer.clean_after_stroke, |v, x| v.mixer.clean_after_stroke = x);
                pass.combination(ui, &words);
                pass.percent(ui, "mixer-wet", &words.wet, true, pass.values.mixer.wet, |v, x| v.mixer.wet = x);
                pass.percent(ui, "mixer-load", &words.load, true, pass.values.mixer.load, |v, x| v.mixer.load = x);
                pass.percent(ui, "mixer-mix", &words.mix, true, pass.values.mixer.mix, |v, x| v.mixer.mix = x);
                pass.percent(ui, "mixer-flow", &words.flow, true, pass.values.mixer.flow, |v, x| v.mixer.flow = x);
                pass.check(ui, &words.sample_all_layers, true, None, pass.values.mixer.sample_all_layers, |v, x| v.mixer.sample_all_layers = x);
                return;
            }
            // The preset picker before the bar carries the size and the hardness.
            Divider::vertical().show(ui);
            if tool == PaintTool::Eraser {
                pass.eraser(ui, &words, eraser_mode.as_deref_mut());
            } else {
                pass.mode(ui, &words.mode, modes);
            }
            pass.percent(ui, "opacity", &words.opacity, sized, pass.values.opacity, |v, x| v.opacity = x);
            pass.toggle(ui, PRESSURE, &words.pressure_opacity, sized, pass.values.pressure_opacity, |v, x| v.pressure_opacity = x);
            if tool != PaintTool::Pencil {
                pass.percent(ui, "flow", &words.flow, sized && soft, pass.values.flow, |v, x| v.flow = x);
                pass.toggle(ui, "sparkles", &words.build_up, sized && soft, pass.values.build_up, |v, x| v.build_up = x);
            }
            pass.percent(ui, "smoothing", &words.smoothing, true, pass.values.smoothing.amount, |v, x| v.smoothing.amount = x);
            if tool == PaintTool::Pencil {
                Divider::vertical().show(ui);
                pass.check(ui, &words.auto_erase, true, Some(&words.auto_erase_tip), pass.values.auto_erase, |v, x| v.auto_erase = x);
            } else {
                pass.smoothing_options(ui, cx, &words);
            }
            if tool == PaintTool::Eraser {
                pass.check(ui, &words.erase_to_history, true, None, pass.values.erase_to_history, |v, x| v.erase_to_history = x);
            }
            Divider::vertical().show(ui);
            pass.toggle(ui, PRESSURE, &words.pressure_size, sized, pass.values.pressure_size, |v, x| v.pressure_size = x);
        });
        PaintBarOutput {
            response: shown.output,
            edits: pass.edits,
            eraser_mode: pass.eraser_edit,
            mixer_load: pass.load,
            mixer_clean: pass.clean,
            out_of_range: pass.out_of_range,
        }
    }
}

/// One pass over the bar: the values the members edit and what they report.
struct Pass<'p, M: Value> {
    values: &'p mut PaintValues<M>,
    edits: EditEvents<PaintValues<M>>,
    eraser_edit: Option<EditEvent<EraserMode>>,
    out_of_range: Option<RangeAlert>,
    load: bool,
    clean: bool,
}

impl<M: Value> Pass<'_, M> {
    /// Turns the events of one member into events of the whole values and writes each proposed value into them, so
    /// the members after it show the edit while its gesture runs.
    fn emit<T>(&mut self, events: EditEvents<T>, apply: impl Fn(&mut PaintValues<M>, T)) {
        for event in events {
            let snapshot = self.values.clone();
            let mapped = event.map(|member| {
                let mut next = snapshot.clone();
                apply(&mut next, member);
                next
            });
            match &mapped {
                EditEvent::Update { value, .. } => *self.values = value.clone(),
                EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => *self.values = after.clone(),
                EditEvent::Cancel { before: Some(Mixed::Same(before)), .. } => *self.values = before.clone(),
                _ => {}
            }
            self.edits.push(mapped);
        }
    }

    /// One `Set` of `change`, named `label`; nothing when it changes nothing.
    fn set_with(&mut self, label: &Label, change: impl FnOnce(&mut PaintValues<M>)) {
        let before = self.values.clone();
        change(&mut *self.values);
        if *self.values != before {
            self.edits.push(EditEvent::set(Mixed::Same(before), self.values.clone(), Arc::from(label.as_str()), None));
        }
    }

    /// A pen toggle over a flag: an icon button that stays pressed while on, named `caption`; a click is one `Set`.
    fn toggle(&mut self, ui: &mut Ui, icon: &'static str, caption: &Label, enabled: bool, current: bool, set: fn(&mut PaintValues<M>, bool)) {
        let shown = ToggleButton::new(caption.clone(), Bind::given(current)).icon(IconId::new(icon)).icon_only().enabled(enabled).show(ui);
        if let Some(next) = shown.value().copied() {
            self.set_with(caption, |v| set(v, next));
        }
    }

    /// A percent field over a fraction 0..1.
    fn percent(&mut self, ui: &mut Ui, salt: &'static str, caption: &Label, enabled: bool, current: f64, set: fn(&mut PaintValues<M>, f64)) {
        // The body may run more than once in a pass and only the run that reads the input reports its events, so every
        // run's events and alert are collected.
        let collected: RefCell<(EditEvents<Option<f64>>, Option<RangeAlert>)> = RefCell::new((EditEvents::new(), None));
        let _ = Field::new(caption.clone()).side(SideLabel::new().scrub(true)).id_salt(salt).enabled(enabled).show(ui, |ui, _| {
            let field = NumberField::new(caption.clone(), Bind::given(Some(current * 100.0)))
                .unit_kind(UnitKind::Percent)
                .range(0.0..=100.0)
                .commit_decimals(0)
                .scrub_by_label()
                .show(ui);
            let mut sink = collected.borrow_mut();
            for event in field.edit.events.clone() {
                sink.0.push(event);
            }
            if field.out_of_range.is_some() {
                sink.1 = field.out_of_range;
            }
        });
        let (events, alert) = collected.into_inner();
        if alert.is_some() {
            self.out_of_range = alert;
        }
        self.emit(events, move |v, shown: Option<f64>| {
            if let Some(percent) = shown {
                set(v, (percent / 100.0).clamp(0.0, 1.0));
            }
        });
    }

    /// A checkbox over a flag; a pick is one `Set`.
    fn check(&mut self, ui: &mut Ui, caption: &Label, enabled: bool, tip: Option<&Label>, current: bool, set: fn(&mut PaintValues<M>, bool)) {
        let mut checkbox = Checkbox::new(Bind::given(current)).label(caption.clone()).enabled(enabled);
        if let Some(tip) = tip {
            checkbox = checkbox.tooltip(tip.clone());
        }
        if let Some(next) = checkbox.show(ui).value().copied() {
            self.set_with(caption, |v| set(v, next));
        }
    }

    /// The Mode list over the host's entries.
    fn mode(&mut self, ui: &mut Ui, caption: &Label, entries: &[Entry<M>]) {
        let current = self.values.mode.clone();
        let pick = Field::new(caption.clone())
            .id_salt("mode")
            .show(ui, |ui, _| Select::new(caption.clone(), Bind::given(current.clone()), entries).show(ui).value().cloned())
            .inner
            .flatten();
        if let Some(mode) = pick {
            self.set_with(caption, |v| v.mode = mode);
        }
    }

    /// The Eraser's mode list; the mode is not part of the values.
    fn eraser(&mut self, ui: &mut Ui, words: &PaintBarWords, mode: Option<&mut EraserMode>) {
        let Some(mode) = mode else { return };
        let entries = [
            Entry::new(EraserMode::Brush, words.eraser_brush.clone()),
            Entry::new(EraserMode::Pencil, words.eraser_pencil.clone()),
            Entry::new(EraserMode::Block, words.eraser_block.clone()),
        ];
        let current = *mode;
        let pick = Field::new(words.mode.clone())
            .id_salt("mode")
            .show(ui, |ui, _| Select::new(words.mode.clone(), Bind::given(current), &entries).show(ui).value().copied())
            .inner
            .flatten();
        if let Some(next) = pick.filter(|next| *next != current) {
            *mode = next;
            self.eraser_edit = Some(EditEvent::set(Mixed::Same(current), next, Arc::from(words.mode.as_str()), None));
        }
    }

    /// The gear and its popover of the four smoothing options, enabled while the amount shows above 0 %.
    fn smoothing_options(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, words: &PaintBarWords) {
        let mut open = cx.memory::<SmoothingOpen>().0;
        let gear =
            IconButton::new(IconId::new("settings")).label(words.smoothing_options.clone()).popup(PopupKind::Dialog).expanded(open).show(ui).response;
        if clicked(&gear) {
            open = !open;
        }
        let active = (self.values.smoothing.amount * 100.0).round() > 0.0;
        let _ = Popover::new(words.smoothing_options.clone(), Bind::bound(&mut open)).opened_by(&gear).show(ui, |ui, _| {
            let current = self.values.smoothing;
            self.check(ui, &words.pulled_string, active, None, current.pulled_string, |v, x| v.smoothing.pulled_string = x);
            self.check(ui, &words.catch_up, active, None, current.catch_up, |v, x| v.smoothing.catch_up = x);
            self.check(ui, &words.catch_up_on_end, active, None, current.catch_up_on_end, |v, x| v.smoothing.catch_up_on_end = x);
            self.check(ui, &words.adjust_for_zoom, active, None, current.adjust_for_zoom, |v, x| v.smoothing.adjust_for_zoom = x);
        });
        cx.memory::<SmoothingOpen>().0 = open;
    }

    /// The Mixer Brush's load swatch, showing what the brush holds (a checkerboard when it is clean), and its menu.
    fn mixer_load(&mut self, ui: &mut Ui, words: &PaintBarWords) {
        let fill = match &self.values.mixer.current_load {
            MixerLoad::Clean => SwatchFill::Empty,
            MixerLoad::Color(color) => SwatchFill::Color(Mixed::Same(*color)),
            MixerLoad::Image(image) => SwatchFill::Image(image.clone()),
        };
        let solid = self.values.mixer.solid_colors_only;
        let swatch = Swatch::with_fill(words.load_swatch.clone(), fill).show(ui).response;
        let (mut load, mut clean, mut next_solid) = (false, false, None);
        let _ = Menu::new(words.load_swatch.clone()).opened_by(&swatch, Trigger::Click).show(ui, |ui, _| {
            load = MenuItem::new(words.load_brush.clone()).show(ui).ran;
            clean = MenuItem::new(words.clean_brush.clone()).show(ui).ran;
            Divider::horizontal().show(ui);
            next_solid = Checkbox::new(Bind::given(solid)).label(words.solid_colors_only.clone()).show(ui).value().copied();
        });
        self.load |= load;
        self.clean |= clean;
        if let Some(next) = next_solid {
            self.set_with(&words.solid_colors_only, |v| v.mixer.solid_colors_only = next);
        }
    }

    /// The combinations list: a pick writes Wet, Load and Mix in one `Set` and keeps Flow.
    fn combination(&mut self, ui: &mut Ui, words: &PaintBarWords) {
        let mut entries = Vec::with_capacity(MIXER_COMBINATIONS.len() + MIXER_GROUP_STARTS.len());
        for (index, name) in words.combinations.iter().enumerate() {
            if MIXER_GROUP_STARTS.contains(&index) {
                entries.push(Entry::separator());
            }
            entries.push(Entry::new(index, name.clone()));
        }
        let shown = self.values.mixer.combination.min(MIXER_COMBINATIONS.len() - 1);
        let pick = Field::new(words.combination.clone())
            .side(SideLabel::new().kind(LabelKind::Hidden))
            .id_salt("mixer-combination")
            .show(ui, |ui, _| Select::new(words.combination.clone(), Bind::given(shown), &entries).show(ui).value().copied())
            .inner
            .flatten();
        if let Some(index) = pick {
            self.set_with(&words.combination, |v| {
                v.mixer.combination = index;
                if let Some(Some((wet, load, mix))) = MIXER_COMBINATIONS.get(index) {
                    (v.mixer.wet, v.mixer.load, v.mixer.mix) = (*wet, *load, *mix);
                }
            });
        }
    }
}
