//! [`CropBar`]: the Crop tool's options as children of the app's options [`Bar`](craft_ui::bars::Bar).

use super::bar_words::CropBarWords;
use super::menus::{CropOptionsMenu, CropOverlayMenu};
use super::options::{CropFill, CropRatio, CropSettings, CropUserPreset, RATIO_DECIMALS, RATIO_MAX, RATIO_STEP};
use super::preset_list::CropPresetList;
use super::session::CropDocument;
use super::size_cells::{self, SizeLimits};
use craft_behavior::{EditEvent, EditEvents, IconId, NumberCodec};
use craft_ui::buttons::{Button, IconButton, ToggleButton};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{
    Bind, BodyCx, Entry, Gap, LabelKind, NoValue, Outcome, Props as _, Remember as _, Show as _, ShowContainer as _, SideLabel, clicked,
};
use craft_ui::display::Divider;
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, Units};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::text::NumberField;
use egui::{Response, Ui};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The ratio, a ratio field, a preset or W × H × Resolution changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CropRatioChange {
    /// True while a ratio field is scrubbed, so the frame refits from the frame the edit began with. A typed edit
    /// reports once, when the typing run ends, so typing never refits live.
    pub editing: bool,
}

/// What the bar reported this pass. Edits of the options go straight into the settings the bar was given
/// ([`settings_changed`](Self::settings_changed) says they changed); these are the rest. ✓ and ⊘ are the app's
/// [`Bar::commit_cancel`](craft_ui::bars::Bar::commit_cancel).
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the app refits the frame, ends the session or opens a dialog for each field that is set"]
pub struct CropBarOutput {
    /// The strip's response.
    pub response: Response,
    /// The ratio changed: the app calls `CropTool::refit` with [`CropRatioChange::editing`].
    pub ratio: Option<CropRatioChange>,
    /// The Straighten button was pressed: the app calls `CropTool::toggle_straighten`.
    pub straighten: bool,
    /// New Crop Preset…: the app opens its dialog with [`CropSettings::preset_name`].
    pub new_preset: bool,
    /// Delete Crop Preset…: the app opens its dialog.
    pub delete_preset: bool,
    /// The gear menu's Custom… or the shield swatch: the app opens its colour picker with this request and sets
    /// `CropShield::custom_color` from the answer.
    pub pick_color: Option<ColorRequest>,
    /// Any option of the settings changed this pass: the app persists them.
    pub settings_changed: bool,
    /// The help button was pressed this pass: the host opens a `HostedAlert` (the `fill_help_*` words of
    /// [`CropBarWords`]).
    pub help: bool,
    /// A value typed into W x H x Resolution outside its range: the cell keeps the limit and the host raises the
    /// range alert.
    pub out_of_range: Option<craft_ui::text::RangeAlert>,
    /// Reset was pressed this pass.
    pub reset: bool,
    /// Selection was picked in the preset list: the app calls `CropTool::take_selection_frame`.
    pub selection: bool,
}

impl Outcome for CropBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Crop tool's options, shown as children of the app's `Bar::new(label, BarKind::Options)`; ✓ and ⊘ are the bar's
/// `commit_cancel`.
///
/// - Order (Photoshop's): the preset list ([`CropPresetList`]: Selection when offered, Ratio, W x H x Resolution, the
///   ratio presets, the size presets, the user's presets, Front Image, a rule, New Crop Preset…, Delete Crop
///   Preset…); the W and H ratio fields (0 to 99,999 with 3 decimals) with the swap button between them, or in
///   W × H × Resolution mode the W, H and Resolution fields with the px/in or px/cm select; Clear; the labelled
///   Straighten toggle (pressed while armed); the frame's angle is edited on the canvas; the overlay menu ([`CropOverlayMenu`]); the gear popover
///   ([`CropOptionsMenu`]); the Delete Cropped Pixels check box; Fill and its select; the help button when a help
///   label is given; Reset.
/// - W and H keep their lengths with the unit they were entered in ([`CropLength`](super::CropLength)); a cleared
///   field is unset.
/// - A scrub or a step of a ratio field refits live; Escape puts the ratio back.
/// - Delete Crop Preset… is disabled when no user preset is given; New Crop Preset… when [`CropSettings::preset_name`]
///   is `None`; Original Ratio and Front Image without a document.
/// - Fill's first entry reads Transparent while Delete Cropped Pixels is off and Background while it is on.
/// - **Keyboard and AccessKit:** the list is a select named "Crop ratio" with the chosen entry as its value; the
///   fields are named Width, Height and Resolution; the Straighten toggle has its pressed state; the menus
///   open with Enter or Space.
#[must_use = "show the bar's children with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct CropBar<'a> {
    settings: &'a mut CropSettings,
    document: &'a CropDocument,
    /// The user's saved crop presets, oldest first.
    #[prop]
    user_presets: &'a [CropUserPreset],
    /// The Straighten toggle's pressed state (`CropTool::straighten_armed`).
    #[prop]
    straighten: bool,
    /// The help button after Fill; `Some` shows it, named by the label.
    #[prop(into)]
    help: Option<Label>,
    /// Whether Reset is enabled (`CropTool::modified`).
    #[prop]
    can_reset: bool,
    /// Offers Selection as the preset list's first entry.
    #[prop]
    selection_entry: bool,
    /// Shows Selection as the chosen preset (`CropTool::shows_selection_entry`).
    #[prop]
    selection_chosen: bool,
}

impl<'a> CropBar<'a> {
    /// The bar editing `settings`, over `document` (Original Ratio's numbers and Front Image).
    pub fn new(settings: &'a mut CropSettings, document: &'a CropDocument) -> Self {
        Self {
            settings,
            document,
            user_presets: &[],
            straighten: false,
            help: None,
            can_reset: false,
            selection_entry: false,
            selection_chosen: false,
        }
    }

    /// Shows the bar's children.
    pub fn show(self, ui: &mut Ui) -> CropBarOutput {
        let Self { settings, document, user_presets, straighten, help, can_reset, selection_entry, selection_chosen } = self;
        let env = Env::of(ui);
        let words = env.words::<CropBarWords>();
        let units = env.units();
        let has_document = document.size.width > 0.0 && document.size.height > 0.0;
        let before = settings.clone();
        let mut report = Report::default();
        let (mut help_pressed, mut reset_pressed) = (false, false);
        let mut out_of_range = None;
        // The options are the bar's members, spaced as the bar spaces them.
        let shown = Row::new().gap(Gap::Item).show(ui, |ui, cx| {
            let list = CropPresetList::new(settings, document)
                .user_presets(user_presets)
                .selection_entry(selection_entry)
                .selection_chosen(selection_chosen)
                .show(ui);
            report.ratio = list.ratio;
            report.new_preset = list.new_preset;
            report.delete_preset = list.delete_preset;
            report.selection = list.selection;
            if settings.ratio == CropRatio::Size {
                let cells = size_cells::show(ui, &mut settings.size, SizeLimits::CROP, document, &units, &words);
                if cells.changed {
                    report.ratio = Some(CropRatioChange { editing: false });
                }
                // The body can run more than once in a pass: a later run does not drop an alert an earlier one raised.
                if cells.out_of_range.is_some() {
                    out_of_range = cells.out_of_range;
                }
            } else {
                ratio_fields(ui, cx, settings, &words, document, has_document, &units, &mut report);
            }
            Divider::vertical().show(ui);
            if clicked(&Button::new(words.clear.clone()).show(ui)) {
                settings.clear();
                report.ratio = Some(CropRatioChange { editing: false });
            }
            let toggle = ToggleButton::new(words.straighten_name.clone(), Bind::given(straighten))
                .icon(IconId::new("ruler"))
                .tooltip(words.straighten.clone())
                .show(ui);
            report.straighten |= toggle.value().is_some();
            CropOverlayMenu::new(settings).show(ui);
            report.pick_color = CropOptionsMenu::new(settings).show(ui).pick_color;
            Checkbox::new(Bind::bound(&mut settings.delete_pixels)).label(words.delete_pixels.clone()).show(ui);
            let background = if settings.delete_pixels { words.fill_background.clone() } else { words.fill_transparent.clone() };
            let fills = [Entry::new(CropFill::Background, background), Entry::new(CropFill::ContentAware, words.fill_content_aware.clone())];
            Field::new(words.fill_name.clone()).id_salt("fill").show(ui, |ui, _| {
                let pick = Select::new(words.fill_name.clone(), Bind::given(settings.fill), &fills).show(ui);
                if let Some(fill) = pick.value().copied() {
                    settings.fill = fill;
                }
            });
            if let Some(label) = help.clone() {
                help_pressed = clicked(&IconButton::new(IconId::new("info")).label(label).show(ui));
            }
            reset_pressed = clicked(&IconButton::new(IconId::new("undo_2")).label(words.reset_tip.clone()).enabled(can_reset).show(ui));
        });
        let changed = *settings != before;
        CropBarOutput {
            response: shown.output,
            ratio: report.ratio,
            straighten: report.straighten,
            new_preset: report.new_preset,
            delete_preset: report.delete_preset,
            pick_color: report.pick_color,
            settings_changed: changed,
            help: help_pressed,
            out_of_range,
            reset: reset_pressed,
            selection: report.selection,
        }
    }
}

/// What the cells reported this pass.
#[derive(Default)]
struct Report {
    ratio: Option<CropRatioChange>,
    straighten: bool,
    new_preset: bool,
    delete_preset: bool,
    pick_color: Option<ColorRequest>,
    selection: bool,
}

/// The ratio and its fields when a scrub of a ratio field began, for Escape.
#[derive(Clone, Default)]
struct RatioStart(Option<(CropRatio, [f64; 2])>);

/// The two ratio fields with the swap button between them.
#[expect(clippy::too_many_arguments, reason = "the bar's inputs")]
fn ratio_fields(
    ui: &mut Ui,
    cx: &mut BodyCx<'_>,
    settings: &mut CropSettings,
    words: &CropBarWords,
    document: &CropDocument,
    has_document: bool,
    units: &Units,
    report: &mut Report,
) {
    let shown = settings.shown_ratio_fields(document);
    let mut changed = false;
    let mut editing = false;
    ratio_field(ui, cx, 0, &words.width, shown, settings, units, &mut changed, &mut editing);
    let original = settings.ratio == CropRatio::Original;
    let swap = IconButton::new(IconId::new("swap_horizontal")).label(words.swap.clone()).enabled(has_document || !original).show(ui);
    ratio_field(ui, cx, 1, &words.height, shown, settings, units, &mut changed, &mut editing);
    if clicked(&swap) {
        if original {
            settings.swap(document);
        } else {
            settings.swap_fields();
        }
        changed = true;
    }
    if changed {
        report.ratio = Some(CropRatioChange { editing });
    }
}

/// One ratio field: `shown` is both fields' numbers as drawn; an empty field is 0.
#[expect(clippy::too_many_arguments, reason = "the bar's inputs")]
fn ratio_field(
    ui: &mut Ui,
    cx: &mut BodyCx<'_>,
    side: usize,
    caption: &Label,
    shown: [f64; 2],
    settings: &mut CropSettings,
    units: &Units,
    changed: &mut bool,
    editing: &mut bool,
) {
    let current = shown.get(side).copied().unwrap_or(0.0);
    let salt = if side == 0 { "ratio-width" } else { "ratio-height" };
    let mut events = EditEvents::new();
    Field::new(caption.clone()).side(SideLabel::new().kind(LabelKind::Hidden)).id_salt(salt).show(ui, |ui, _| {
        let codec = NumberCodec::new(units.reader(None), RATIO_DECIMALS);
        let field = NumberField::new(caption.clone(), Bind::given((current > 0.0).then_some(current)))
            .codec(codec)
            .range(0.0..=RATIO_MAX)
            .step(RATIO_STEP)
            .automatic(true)
            .show(ui);
        events = field.edit.events.clone();
    });
    // Returns whether the value changed the settings.
    let apply = |settings: &mut CropSettings, value: Option<f64>| {
        let new = value.unwrap_or(0.0);
        if (new - current).abs() < 0.5 * 10f64.powi(-(RATIO_DECIMALS as i32)) {
            return false;
        }
        // The other side keeps what the bar shows, so an Original ratio becomes its sides.
        if let Some(other) = settings.ratio_fields.get_mut(1 - side) {
            *other = shown.get(1 - side).copied().unwrap_or(0.0);
        }
        settings.set_ratio_field(side, new);
        true
    };
    for event in events {
        match event {
            EditEvent::Begin { .. } => {
                cx.memory::<RatioStart>().0 = Some((settings.ratio, settings.ratio_fields));
                *editing = true;
            }
            EditEvent::Update { value, .. } => {
                *editing = true;
                *changed |= apply(settings, value);
            }
            EditEvent::Commit { after: value, .. } | EditEvent::Set { after: value, .. } => {
                cx.memory::<RatioStart>().0 = None;
                *changed |= apply(settings, value);
            }
            EditEvent::Cancel { .. } => {
                if let Some((ratio, fields)) = cx.memory::<RatioStart>().0.take() {
                    settings.ratio = ratio;
                    settings.ratio_fields = fields;
                    *changed = true;
                }
            }
            _ => {}
        }
    }
}
