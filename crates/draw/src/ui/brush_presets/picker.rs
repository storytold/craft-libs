//! The Brush Preset picker ([`BrushPresetPicker`]): a closed field that opens a popover with the brush's tip angle and
//! roundness, Size and Hardness, the buttons and gear menu, and the preset browser.

use std::cell::{Cell, RefCell};
use std::hash::Hash;

use craft_behavior::{EditEvent, EditEvents, IconId, Mixed, Value};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{
    Bind, BodyCx, EditResponse, Outcome, Props as _, Remember as _, SaveKey, Show as _, ShowContainer as _, TooltipSpec, Trigger, clicked,
};
use craft_ui::display::Divider;
use craft_ui::form::SliderField;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::patterns::presets::{
    CellShape, PickerField, PresetBrowser, PresetEvent, PresetFooterOptions, PresetPicker, PresetRow, PresetState, PresetView,
};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::tile_grid::TileZoom;
use egui::{Pos2, Response, Ui};

use super::settings::{BrushCardPart, BrushCardParts, BrushPickerSettings};
use super::words::BrushPresetWords;
use crate::ui::brush_settings::{BrushSizeField, TipAngle, TipEllipse};

/// The largest diameter the picker's Size row allows until the app sets its engine's limit.
const DEFAULT_MAX_DIAMETER: f64 = 5000.0;
/// How far one key press changes the thumbnail scale.
const SCALE_STEP: f64 = 0.05;

/// The brush values the picker's head edits, given by the app each pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct BrushPickerValues {
    /// The diameter in pixels: the Size field and its log slider.
    pub diameter: f64,
    /// The hardness, 0 to 1, shown in percent; `None` for a tip without one (a sampled tip): then no Hardness
    /// slider shows.
    pub hardness: Option<f64>,
    /// The tip's angle and roundness; `None` for a tool without them: then no angle widget shows.
    pub tip: Option<TipAngle>,
}

impl BrushPickerValues {
    /// A round tip: its `diameter` and `hardness`, without an angle widget.
    pub const fn new(diameter: f64, hardness: Option<f64>) -> Self {
        Self { diameter, hardness, tip: None }
    }

    /// These values with the tip's angle and roundness, which the head's angle widget edits.
    #[must_use]
    pub const fn with_tip(mut self, tip: TipAngle) -> Self {
        self.tip = Some(tip);
        self
    }
}

/// What the picker asks the app to do beyond the head's edits.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum BrushPick<K> {
    /// A click on a preset: the binding holds it and the picker stays open.
    Select(K),
    /// A double click or Enter on a preset: the binding holds it and the picker closes.
    Activate(K),
    /// A browser event: `Rename` (also from the gear's Rename Brush…, which opens the name field on the current
    /// preset), `Delete` (also from the gear), `Reorder`, `MoveTo` and the footer's buttons.
    List(PresetEvent<K>),
    /// Brush Settings…: open the Brush Settings panel.
    OpenSettings,
    /// New Brush Preset (the button or the gear): open the app's New Brush Preset dialog.
    NewPreset,
    /// Import Brushes… (the gear, shown only with [`BrushPresetPicker::import`]).
    Import,
}

impl<K> BrushPick<K> {
    /// Whether this pick closed the picker: `Activate`, and every pick that opens another panel or dialog (Brush
    /// Settings, New Brush Preset, Import).
    pub fn closes(&self) -> bool {
        matches!(self, BrushPick::Activate(_) | BrushPick::OpenSettings | BrushPick::NewPreset | BrushPick::Import)
    }
}

/// A section of the gear menu, in the menu's order. The app's rows ([`BrushPresetPicker::gear_rows`]) follow the
/// picker's own rows of each section.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GearSection {
    /// New Brush Preset….
    Create,
    /// Rename Brush… and Delete Brush….
    Preset,
    /// The Brush Name, Brush Stroke and Brush Tip boxes.
    Parts,
    /// Show Search Bar.
    View,
    /// Import Brushes… (with [`BrushPresetPicker::import`]).
    Library,
    /// After the last section, set off by a divider.
    End,
}

type GearRowsFn<'a> = Box<dyn FnMut(&mut Ui, GearSection) + 'a>;

/// The app's own rows of the gear menu: called once per [`GearSection`], after the picker's rows of that section.
pub struct GearRows<'a>(GearRowsFn<'a>);

impl<'a> GearRows<'a> {
    /// The rows that `show` adds to each section.
    pub fn new(show: impl FnMut(&mut Ui, GearSection) + 'a) -> Self {
        GearRows(Box::new(show))
    }
}

/// The picker's state, kept by the host while the app runs: the browser's view (search, open groups, scale), the
/// parts the presets show and whether the search field shows. [`settings`](Self::settings) is what persists.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct BrushPickerState<K> {
    /// The browser's view settings; its zoom is the thumbnail scale.
    pub browser: PresetState<K>,
    /// Which parts the presets show.
    pub parts: BrushCardParts,
    /// The gear's Show Search Bar.
    pub search: bool,
}

impl<K> Default for BrushPickerState<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K> BrushPickerState<K> {
    /// Every part on, the search field shown, scale 1, an empty search.
    pub fn new() -> Self {
        Self { browser: PresetState::new(), parts: BrushCardParts::ALL, search: true }
    }
}

impl<K: Eq + Hash> PartialEq for BrushPickerState<K> {
    fn eq(&self, other: &Self) -> bool {
        self.browser == other.browser && self.parts == other.parts && self.search == other.search
    }
}

/// What [`BrushPresetPicker::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the picker's edits and picks"]
pub struct BrushPresetPickerOutput<K> {
    /// The closed field.
    pub response: Response,
    /// The field's value and its edit; `None` when the body did not run.
    pub edit: Option<EditResponse<Option<K>>>,
    /// The Size row: the new diameter, whole pixels from 1 to the maximum. A drag is one gesture; a typed value is a
    /// `Set`. The app sends it to its tool options host.
    pub diameter: EditEvents<f64>,
    /// The Hardness slider: the new hardness, 0 to 1 in whole percent.
    pub hardness: EditEvents<f64>,
    /// The angle widget: the new angle and roundness.
    pub tip: EditEvents<TipAngle>,
    /// Escape closed the picker: the values to set back, each the one it had when the picker opened (or a preset
    /// picked since brought) while it still holds the value the picker's head last set; a value changed from
    /// elsewhere since keeps that change. `None` when nothing goes back.
    pub reverted: Option<BrushPickerValues>,
    /// Whether the popover is open after this pass.
    pub open: bool,
    /// The picks, in the order they happened.
    pub picks: Vec<BrushPick<K>>,
    /// The persisted view when the user changed it this pass (a part box, Show Search Bar, the scale): the app saves
    /// it.
    pub settings: Option<BrushPickerSettings>,
}

impl<K: Value> Outcome for BrushPresetPickerOutput<K> {
    type Value = Option<K>;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        match &self.edit {
            Some(edit) => edit.story_facts(),
            None => craft_ui::testing::StoryOutput::default(),
        }
    }
}

/// What Escape goes back to while the picker is open: the values it opened with (or a picked preset brought) and the
/// values its head last set.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Opened {
    opened: BrushPickerValues,
    typed: BrushPickerValues,
}

impl Opened {
    fn new(values: BrushPickerValues) -> Self {
        Opened { opened: values, typed: values }
    }

    /// `now` with each value the head last set back at its opening value; a value changed from elsewhere stays.
    fn revert(&self, now: BrushPickerValues) -> BrushPickerValues {
        fn back<T: PartialEq>(now: T, typed: T, opened: T) -> T {
            if now == typed { opened } else { now }
        }
        let tip = match (now.tip, self.typed.tip, self.opened.tip) {
            (Some(now), Some(typed), Some(opened)) => {
                Some(TipAngle::new(back(now.degrees, typed.degrees, opened.degrees), back(now.roundness, typed.roundness, opened.roundness)))
            }
            (now, _, _) => now,
        };
        BrushPickerValues {
            diameter: back(now.diameter, self.typed.diameter, self.opened.diameter),
            hardness: back(now.hardness, self.typed.hardness, self.opened.hardness),
            tip,
        }
    }
}

/// Photoshop's Brush Preset picker: a closed field showing the current preset's tip with the brush size under it, which opens
/// a popover over [`PresetPicker`].
///
/// - **Head.** While [`BrushPickerValues::tip`] is set, [`TipEllipse`] (angle and roundness) at the start; beside it
///   [`BrushSizeField`] (Size, 1 to [`max_diameter`](Self::max_diameter), log slider) and, while
///   [`BrushPickerValues::hardness`] is set, a Hardness slider field (0 to 100 %); then a divider. They edit the
///   brush live and report [`BrushPresetPickerOutput::tip`], [`diameter`](BrushPresetPickerOutput::diameter) and
///   [`hardness`](BrushPresetPickerOutput::hardness).
/// - **Search row.** The browser's search field (any letter case, groups open while filtering) unless Show Search
///   Bar is off, then at its reading end the Brush Settings and New Brush Preset buttons and the gear.
/// - **Gear menu.** New Brush Preset…; Rename Brush… (opens the name field on the current preset) and Delete Brush…
///   (enabled with a current preset); the Brush Name, Brush Stroke and Brush Tip boxes (at least one stays on); Show
///   Search Bar; Import Brushes… with [`import`](Self::import). The app adds its own rows to each section with
///   [`gear_rows`](Self::gear_rows).
/// - **Presets.** The browser, with the scale slider (0.15 to 2.0) in its footer. The tip alone is a grid with the
///   size as each caption; the name or the stroke is a list, with the stroke band while the stroke shows. A click
///   picks and keeps the picker open; a double click or Enter picks and closes it.
/// - **Opening.** The closed field, or the app at a point with [`open_at`](Self::open_at) (a right-click on the
///   canvas), without the field. With [`resizable`](Self::resizable) the popover's grips resize it and the size is
///   saved.
/// - **Closing.** The popover's own: a press outside, Escape, and the picks that open something else
///   ([`BrushPick::closes`]). Escape also reports [`BrushPresetPickerOutput::reverted`].
#[must_use = "show the picker with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
#[pattern(probe(BrushPresetPicker<'static, u32>))]
pub struct BrushPresetPicker<'a, K: Value + Eq + Hash> {
    label: Label,
    current: Bind<'a, Option<K>>,
    rows: &'a [PresetRow<K>],
    state: Bind<'a, BrushPickerState<K>>,
    values: BrushPickerValues,
    /// The largest diameter the app's engine paints; 5000 until set.
    #[prop(quantity = "pixels")]
    max_diameter: f64,
    /// Shows Import Brushes… in the gear menu.
    #[prop(flag)]
    import: bool,
    /// The app's rows of the gear menu, added to each section.
    #[prop(func = "FnMut(&mut Ui, GearSection)")]
    gear_rows: Option<GearRows<'a>>,
    /// Edge grips resize the popover, and the size the user leaves it at is saved under this key.
    #[prop]
    resizable: Option<SaveKey>,
    opened_at: Option<(Pos2, Bind<'a, bool>)>,
}

impl<'a, K: Value + Eq + Hash> BrushPresetPicker<'a, K> {
    /// A picker named `label` over the presets in `rows` (build each with [`brush_row`](super::brush_row)), holding the
    /// preset the brush was picked from in `current`, its view in `state` and the brush's `values`.
    pub fn new(
        label: impl Into<Label>,
        current: Bind<'a, Option<K>>,
        rows: &'a [PresetRow<K>],
        state: Bind<'a, BrushPickerState<K>>,
        values: BrushPickerValues,
    ) -> Self {
        Self {
            label: label.into(),
            current,
            rows,
            state,
            values,
            max_diameter: DEFAULT_MAX_DIAMETER,
            import: false,
            gear_rows: None,
            resizable: None,
            opened_at: None,
        }
    }

    /// Opens the popover at `at`, a point in the parent's layer space, while `open` is true, and shows no field: the
    /// app opens it (a right-click on the canvas) and every close writes `false`.
    pub fn open_at(mut self, at: Pos2, open: Bind<'a, bool>) -> Self {
        self.opened_at = Some((at, open));
        self
    }

    /// Shows the picker and returns this pass's edits and picks, in order.
    pub fn show(self, ui: &mut Ui) -> BrushPresetPickerOutput<K> {
        let BrushPresetPicker { label, mut current, rows, mut state, values, max_diameter, import, mut gear_rows, resizable, mut opened_at } = self;
        let env = Env::of(ui);
        let words = env.words::<BrushPresetWords>();
        let name: std::sync::Arc<str> = std::sync::Arc::from(label.as_str());
        let shown = Stack::new().label(label.clone()).show(ui, |ui, cx| {
            let before = match state.reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => BrushPickerState::new(),
            };
            let parts = before.parts.normalised();
            let held = match current.reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => None,
            };
            let has_current = held.as_ref().is_some_and(|key| rows.iter().any(|row| !row.is_group() && row.key == *key));
            let mut browser = before.browser.clone();
            let next_parts = Cell::new(parts);
            let next_search = Cell::new(before.search);
            let tool_picks: RefCell<Vec<BrushPick<K>>> = RefCell::new(Vec::new());
            let (mut diameter, mut hardness, mut tip) = (EditEvents::new(), EditEvents::new(), EditEvents::new());
            let browser_id = ui.scope_id().with("browser");
            let has_app_rows = gear_rows.is_some();
            let head = |ui: &mut Ui, _: &mut BodyCx<'_>| {
                Row::new().show(ui, |ui, _| {
                    tip = match values.tip {
                        Some(angle) => TipEllipse::new(words.tip.clone(), Bind::given(angle)).show(ui).events,
                        None => EditEvents::new(),
                    };
                    Stack::new().show(ui, |ui, _| {
                        let size = BrushSizeField::new(words.size.clone(), Bind::given(values.diameter), max_diameter).show(ui);
                        diameter = size.edit.events;
                        hardness = EditEvents::new();
                        if let Some(value) = values.hardness {
                            let field = SliderField::new(words.hardness.clone(), Bind::given(value * 100.0), 0.0..=100.0)
                                .step(1.0)
                                .unit_kind(UnitKind::Percent)
                                .show(ui);
                            hardness = field.events.map(|percent| (percent / 100.0).clamp(0.0, 1.0));
                        }
                    });
                });
                Divider::horizontal().show(ui);
            };
            // The buttons and the gear at the reading end of the browser's search row.
            let tools = |ui: &mut Ui| {
                let mut picks = tool_picks.borrow_mut();
                picks.clear();
                next_parts.set(parts);
                next_search.set(before.search);
                Row::new().show(ui, |ui, _| {
                    let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
                    let settings = IconButton::new(icon("adjustments")).label(words.settings.clone()).tooltip(words.settings.clone()).show(ui);
                    if clicked(&settings) {
                        picks.push(BrushPick::OpenSettings);
                    }
                    let new_preset = IconButton::new(icon("plus")).label(words.new_preset.clone()).tooltip(words.new_preset.clone()).show(ui);
                    if clicked(&new_preset) {
                        picks.push(BrushPick::NewPreset);
                    }
                    let gear = IconButton::new(icon("settings")).label(words.options.clone()).tooltip(words.options.clone()).show(ui);
                    let _ = Menu::new(words.options.clone()).opened_by(&gear, Trigger::Click).show(ui, |ui, _| {
                        let mut app_rows = |ui: &mut Ui, section: GearSection| {
                            if let Some(rows) = gear_rows.as_mut() {
                                (rows.0)(ui, section);
                            }
                        };
                        if MenuItem::new(words.menu_new.clone()).show(ui).ran {
                            picks.push(BrushPick::NewPreset);
                        }
                        app_rows(ui, GearSection::Create);
                        Divider::horizontal().show(ui);
                        let on_current = |row: MenuItem<'static>| {
                            let row = row.enabled(has_current);
                            if has_current {
                                row
                            } else {
                                row.tooltip(TooltipSpec::new(words.no_current.clone()).disabled_text(words.no_current.clone()))
                            }
                        };
                        if on_current(MenuItem::new(words.menu_rename.clone())).show(ui).ran
                            && let Some(key) = held.as_ref()
                        {
                            PresetBrowser::request_rename(ui.ctx(), browser_id, key);
                        }
                        if on_current(MenuItem::new(words.menu_delete.clone())).show(ui).ran
                            && let Some(key) = held.clone()
                        {
                            picks.push(BrushPick::List(PresetEvent::Delete(key)));
                        }
                        app_rows(ui, GearSection::Preset);
                        Divider::horizontal().show(ui);
                        for (part, label) in [
                            (BrushCardPart::Name, &words.part_name),
                            (BrushCardPart::Stroke, &words.part_stroke),
                            (BrushCardPart::Tip, &words.part_tip),
                        ] {
                            let on = next_parts.get().get(part);
                            if MenuItem::new(label.clone()).check(on).keep_open(true).show(ui).ran {
                                next_parts.set(next_parts.get().set(part, !on));
                            }
                        }
                        app_rows(ui, GearSection::Parts);
                        Divider::horizontal().show(ui);
                        let search = next_search.get();
                        if MenuItem::new(words.search_bar.clone()).check(search).keep_open(true).show(ui).ran {
                            next_search.set(!search);
                        }
                        app_rows(ui, GearSection::View);
                        Divider::horizontal().show(ui);
                        if import && MenuItem::new(words.menu_import.clone()).show(ui).ran {
                            picks.push(BrushPick::Import);
                        }
                        app_rows(ui, GearSection::Library);
                        if has_app_rows {
                            Divider::horizontal().show(ui);
                            app_rows(ui, GearSection::End);
                        }
                    });
                });
            };
            let mut picker = PresetPicker::new(words.picker.clone(), current.reborrowed(), rows, Bind::bound(&mut browser))
                .field(PickerField::Thumbnail)
                .caption(env.units().number(values.diameter.round(), 0..=0))
                .cell_shape(CellShape::Square)
                .view(if parts.tip_only() { PresetView::Grid } else { PresetView::List })
                .toolbar(true)
                .search(before.search)
                .keep_open()
                .thumb_range(TileZoom::bounded(
                    f64::from(*BrushPickerSettings::SCALE_RANGE.start()),
                    f64::from(*BrushPickerSettings::SCALE_RANGE.end()),
                    SCALE_STEP,
                    1.0,
                ))
                .footer(PresetFooterOptions::new().new_group(false))
                .browser_id(browser_id)
                .head(head)
                .tools(tools);
            if let Some(key) = resizable.clone() {
                picker = picker.resizable(key);
            }
            if let Some((at, open)) = opened_at.as_mut() {
                picker = picker.open_at(*at, open.reborrowed());
            }
            let mut out = picker.show(ui);
            let mut picks = tool_picks.take();
            if let Some(key) = out.picked.clone() {
                picks.insert(0, if out.closed { BrushPick::Activate(key) } else { BrushPick::Select(key) });
            }
            picks.extend(std::mem::take(&mut out.events).into_iter().map(BrushPick::List));
            let after = BrushPickerState { browser, parts: next_parts.get(), search: next_search.get() };
            let mut settings = None;
            if after != before {
                let event = EditEvent::set(Mixed::Same(before.clone()), after.clone(), name.clone(), None);
                state.reborrow().write(cx, &event);
                settings = (after.settings() != before.settings()).then(|| after.settings());
            }
            let reverted = escape_revert(cx.memory::<Option<Opened>>(), values, &out, [&diameter, &hardness], &tip);
            BrushPresetPickerOutput {
                response: out.edit.response.clone(),
                edit: Some(out.edit),
                diameter,
                hardness,
                tip,
                reverted,
                open: out.open,
                picks,
                settings,
            }
        });
        match shown.inner {
            Some(output) => output,
            None => BrushPresetPickerOutput {
                response: shown.output,
                edit: None,
                diameter: EditEvents::new(),
                hardness: EditEvents::new(),
                tip: EditEvents::new(),
                reverted: None,
                open: false,
                picks: Vec::new(),
                settings: None,
            },
        }
    }
}

/// Keeps what Escape goes back to while the popover is open and returns the values Escape sets back, if any.
///
/// The pass the popover shows open first takes `values` as the opening values; the head's edits become the values
/// it last set; a pick starts over from the values the preset brings, on the next pass.
fn escape_revert<K>(
    opened: &mut Option<Opened>,
    values: BrushPickerValues,
    out: &craft_ui::patterns::presets::PresetPickerOutput<K>,
    [diameter, hardness]: [&EditEvents<f64>; 2],
    tip: &EditEvents<TipAngle>,
) -> Option<BrushPickerValues> {
    let shown = out.open || out.closed_as.is_some();
    let now = BrushPickerValues {
        diameter: diameter.landed().copied().unwrap_or(values.diameter),
        hardness: hardness.landed().copied().or(values.hardness),
        tip: tip.landed().copied().or(values.tip),
    };
    let mut reverted = None;
    if shown {
        let held = opened.get_or_insert_with(|| Opened::new(values));
        if now != values {
            held.typed = now;
        }
        if out.closed_as == Some(CloseReason::Escape) {
            reverted = Some(held.revert(now)).filter(|back| *back != now);
        }
    }
    if !out.open || out.picked.is_some() {
        *opened = None;
    }
    reverted
}
