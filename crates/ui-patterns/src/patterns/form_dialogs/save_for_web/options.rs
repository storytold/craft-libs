//! The settings column of Save for Web: Preset, Optimize to File Size…, Format, the format's options, the Color
//! Table, Matte, Convert to sRGB, Metadata and Image Size.

use super::{
    ColorTable, ColorTableEdit, ColorTableEvent, Dither, Live, MATTE_COLOR_KEY, MAX_PERCENT, OptimizeRequest, OptimizeScope, OptimizeStart,
    PALETTE_COLOR_KEY, RESAMPLE_METHODS, SaveForWebOptions, SaveForWebWords, SizingWords, TableMarks, TableOrder, WebAntiAlias, WebDocument,
    WebFormat, WebMetadata, WebPreset, WebPresetEvent, WebSlices, hex, percent_of,
};
use crate::runtime::requests::ColorRequest;

use crate::buttons::{Button, IconButton};
use crate::choice::{CellMarks, Checkbox, Swatch, SwatchTable};
use crate::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Trigger, clicked};

use crate::display::{Shade, Text};
use crate::form::{Field, FieldGroup, SliderField, SliderFieldLayout};

use super::model::set_pixels;
use crate::layout_components::Row;
use crate::menu::{Menu, MenuItem};
use crate::patterns::form_dialogs::{LinkedSize, Resample, resize_from_scale};
use crate::patterns::presets::{
    NameCheck, NamedPresetEvent, NamedPresetRow, PickerField, PresetError, PresetFns, PresetPicker, PresetRow, PresetSource,
};
use crate::pickers::{Select, UnitSelect};

use crate::runtime::env::{Env, Label, UnitKind, Units};

use crate::sections::Section;
use crate::style::TypeRole;
use crate::text::{HexField, NumberField};

use craft_behavior::{IconId, NumberCodec};
use craft_theme::{Rgba, Srgba};

use egui::Ui;
use std::ops::RangeInclusive;

/// The range of a percent slider.
const PERCENT: RangeInclusive<f64> = 0.0..=100.0;
/// The range of Quality.
const QUALITY: RangeInclusive<f64> = 0.0..=100.0;
/// The range of Colors.
const COLORS: RangeInclusive<f64> = 2.0..=256.0;
/// The range of the Desired File Size, in K.
const DESIRED_K: RangeInclusive<f64> = 1.0..=100_000.0;
/// The least the Percent row takes.
const LEAST_PERCENT: f64 = 1.0;
/// Opaque alpha of the matte colour.
const OPAQUE: u8 = 255;
/// The spacing of the web-safe palette's channel values.
const WEB_STEP: u8 = 51;

/// A row: `label` beside the control `body` shows.
fn row(ui: &mut Ui, label: &Label, key: &'static str, mut body: impl FnMut(&mut Ui)) {
    Field::new(label.clone()).id_salt(key).show(ui, |ui, _| body(ui));
}

/// A whole-number field named `name`.
fn whole(units: &Units, name: Label, value: f64, range: RangeInclusive<f64>) -> NumberField<'static> {
    NumberField::new(name, Bind::given(Some(value))).codec(NumberCodec::new(units.reader(None), 0)).step(1.0).range(range)
}

/// A slider row named `label` over `range`, in percent when `percent`; the whole number it reports.
fn slider(ui: &mut Ui, label: &Label, key: &'static str, value: u8, range: RangeInclusive<f64>, percent: bool) -> Option<u8> {
    let mut picked = None;
    row(ui, label, key, |ui| {
        let mut slider = SliderField::new(label.clone(), Bind::given(f64::from(value)), range.clone())
            .step(1.0)
            .id_salt("slider")
            .layout(SliderFieldLayout::Inline);
        if percent {
            slider = slider.unit_kind(UnitKind::Percent);
        }
        picked = slider.show(ui).value().cloned().map(|v| v.round().clamp(*range.start(), *range.end()) as u8);
    });
    picked
}

/// A check box row.
fn check(ui: &mut Ui, label: &Label, key: &'static str, value: bool) -> Option<bool> {
    Checkbox::new(Bind::given(value)).label(label.clone()).id_salt(key).show(ui).value().cloned()
}

/// What the settings column reads besides the settings.
pub(super) struct Kit<'k> {
    pub(super) words: &'k SaveForWebWords,
    pub(super) sizing: &'k SizingWords,
    pub(super) units: &'k Units,
    pub(super) presets: &'k [WebPreset],
    /// The palette of the shown pane's encode, when it has one.
    pub(super) palette: Option<&'k [[u8; 3]]>,
    /// The shown pane's encode failed: the Color Table says so and offers the generated table back.
    pub(super) failed: bool,
    /// What the host's colour table says, for the cell marks.
    pub(super) marks: Option<&'k TableMarks>,
    /// The formats the Format list offers.
    pub(super) formats: &'k [WebFormat],
    /// The palette methods the Color Table list offers.
    pub(super) color_tables: &'k [ColorTable],
    pub(super) document: &'k WebDocument,
    /// "Optimize to File Size…" shows.
    pub(super) optimize: bool,
    /// The document has slices: the Slices choice shows.
    pub(super) has_slices: bool,
}

/// What the settings column asks of the dialog besides edits to the settings.
#[derive(Default)]
pub(super) struct Asked {
    /// The matte or palette colour request.
    pub(super) color: Option<ColorRequest>,
    pub(super) presets: Vec<WebPresetEvent>,
    pub(super) table: Vec<ColorTableEvent>,
    /// The Optimize rows opened: the host answers the opening size.
    pub(super) optimize_open: bool,
    pub(super) optimize_request: Option<OptimizeRequest>,
}

/// The settings column over `options`; the Output size `live.link`, the Color Table's selection, the Preset row's
/// picker and the Optimize rows live in `live`.
pub(super) fn show(ui: &mut Ui, kit: &Kit<'_>, options: &mut SaveForWebOptions, live: &mut Live, asked: &mut Asked) {
    let Kit { words, sizing, units, has_slices, document, .. } = *kit;
    let vector = matches!(document, WebDocument::Vector { .. });
    FieldGroup::new(words.settings.clone()).show(ui, |ui, _| {
        preset_row(ui, kit, options, live, asked);
        if kit.optimize {
            optimize_rows(ui, kit, options, live, asked);
        }
        format_row(ui, kit, options);
        format_options(ui, kit, options);
        if options.format.indexed() {
            color_table_section(ui, kit, live, asked);
        }
        if options.has_matte() {
            matte_row(ui, words, options, &mut asked.color);
        }
        if let Some(on) = check(ui, &words.convert_srgb, "srgb", options.convert_srgb) {
            options.convert_srgb = on;
        }
        metadata_row(ui, words, options);
        if vector {
            output_row(ui, words, options);
        }
        if has_slices {
            slices_rows(ui, words, options, vector);
        }
        size_section(ui, words, sizing, units, &mut live.link, options, document);
    });
}

/// Sets `options` to what `preset` sets, keeping what belongs to the document: the size and its link, the resample
/// method, the slices and the artboard.
pub(super) fn apply_preset(options: &mut SaveForWebOptions, preset: &WebPreset) {
    *options = SaveForWebOptions {
        percent: options.percent,
        size: options.size,
        constrain_proportions: options.constrain_proportions,
        resample: options.resample,
        slices: options.slices.clone(),
        artboard: options.artboard,
        ..preset.options.clone()
    };
}

/// One preset row per name, keyed by the name.
fn preset_rows(names: &&[String], _: &Env) -> Vec<PresetRow<String>> {
    names.iter().map(|name| PresetRow::item(name.clone(), name.clone(), 0)).collect()
}

/// The Preset row: pick, save under a new name, rename and delete. The host stores the presets, so the row reports
/// events in `asked` and the host gives the list back.
fn preset_row(ui: &mut Ui, kit: &Kit<'_>, options: &mut SaveForWebOptions, live: &mut Live, asked: &mut Asked) {
    let Kit { words, presets, .. } = *kit;
    let names: Vec<String> = presets.iter().map(|preset| preset.name.clone()).collect();
    let rows = PresetSource::new(names.as_slice(), PresetFns::new(preset_rows));
    let held = presets.iter().find(|preset| preset.is_set_in(options)).map(|preset| preset.name.clone());
    let picker = PresetPicker::new(words.preset.clone(), Bind::given(held), &rows, Bind::bound(&mut live.preset))
        .field(PickerField::Name)
        .placeholder(words.unnamed.clone());
    let named = NamedPresetRow::new(words.preset.clone(), picker)
        .suggest_name(words.new_preset_name.as_str().to_owned())
        .check_name(NameCheck::new(names.as_slice(), |names, typed| {
            names.iter().any(|name| name.eq_ignore_ascii_case(typed)).then_some(PresetError::NameTaken)
        }))
        .built_in(|key| presets.iter().any(|preset| preset.built_in && preset.name == *key))
        .show(ui);
    for event in named.events {
        match event {
            NamedPresetEvent::Apply(key) => {
                if let Some(preset) = presets.iter().find(|preset| preset.name == key) {
                    apply_preset(options, preset);
                }
            }
            NamedPresetEvent::Create { name, .. } => {
                asked.presets.push(WebPresetEvent::Saved(WebPreset::new(name, options.without_document_fields())));
            }
            NamedPresetEvent::Rename { key, text, .. } => asked.presets.push(WebPresetEvent::Renamed { from: key, to: text }),
            NamedPresetEvent::Delete(key) => asked.presets.push(WebPresetEvent::Deleted(key)),
            _ => {}
        }
    }
}

/// "Optimize to File Size…" and, once opened, its rows; OK, or Enter in the Desired File Size field, files the search.
/// While the rows are closed, the host's last answer shows as a line while the settings are the ones it found.
fn optimize_rows(ui: &mut Ui, kit: &Kit<'_>, options: &SaveForWebOptions, live: &mut Live, asked: &mut Asked) {
    let Kit { words, units, .. } = *kit;
    if !live.optimize.open {
        if clicked(&Button::new(words.optimize_open.clone()).id_salt("optimize").show(ui)) {
            live.optimize.open();
            asked.optimize_open = true;
        }
        if let Some(answer) = live.optimize.answer.as_ref().filter(|answer| answer.options.as_ref().is_none_or(|found| found == options)) {
            let word = if answer.achieved { &words.optimize_reached } else { &words.optimize_missed };
            let line = word.fill(&[("size", units.file_size(answer.bytes).as_str()), ("target", units.file_size(answer.target_bytes).as_str())]);
            Text::new(Label::from(line)).role(TypeRole::Caption).shade(Shade::Faint).id_salt("optimize-result").show(ui);
        }
        return;
    }
    row(ui, &words.optimize_size, "optimize-size", |ui| {
        let edit = whole(units, words.optimize_size.clone(), live.optimize.desired_k(), DESIRED_K).submits().show(ui).edit;
        if let Some(k) = edit.value().cloned().flatten() {
            live.optimize.k = Some(k);
        }
        if edit.submitted && edit.submit_mods == craft_keymap::Mods::NONE {
            asked.optimize_request = Some(live.optimize.request(options));
        }
    });
    let starts = [Entry::new(OptimizeStart::Current, words.start_current.clone()), Entry::new(OptimizeStart::Auto, words.start_auto.clone())];
    row(ui, &words.optimize_start, "optimize-start", |ui| {
        if let Some(start) = Select::new(words.optimize_start.clone(), Bind::given(live.optimize.start), &starts).show(ui).value().copied() {
            live.optimize.start = start;
        }
    });
    let scopes = [
        Entry::new(OptimizeScope::AllSlices, words.all_slices.clone()),
        Entry::new(OptimizeScope::EachSlice, words.scope_each.clone()),
        Entry::new(OptimizeScope::CurrentSlice, words.scope_current.clone()),
    ];
    row(ui, &words.optimize_use, "optimize-use", |ui| {
        if let Some(scope) = Select::new(words.optimize_use.clone(), Bind::given(live.optimize.scope), &scopes).show(ui).value().copied() {
            live.optimize.scope = scope;
        }
    });
    if let Some(reason) = &live.optimize.refused {
        Text::new(reason.clone()).role(TypeRole::Caption).shade(Shade::Faint).id_salt("optimize-refused").show(ui);
    }
    Row::new().id_salt("optimize-buttons").show(ui, |ui, _| {
        if clicked(&Button::new(words.optimize_ok.clone()).id_salt("optimize-ok").default_stop().show(ui)) {
            asked.optimize_request = Some(live.optimize.request(options));
        }
        if clicked(&Button::new(words.optimize_cancel.clone()).id_salt("optimize-cancel").show(ui)) {
            live.optimize.close();
        }
    });
}

fn format_row(ui: &mut Ui, kit: &Kit<'_>, options: &mut SaveForWebOptions) {
    let words = kit.words;
    let entries: Vec<Entry<WebFormat>> = kit.formats.iter().map(|format| Entry::new(*format, Label::from(format.name()))).collect();
    row(ui, &words.format, "format", |ui| {
        if let Some(format) = Select::new(words.format.clone(), Bind::given(options.format), &entries).show(ui).value().cloned() {
            options.format = format;
        }
    });
}

/// The rows of the chosen format.
fn format_options(ui: &mut Ui, kit: &Kit<'_>, options: &mut SaveForWebOptions) {
    let Kit { words, units, .. } = *kit;
    match options.format {
        WebFormat::Gif | WebFormat::Png8 => {
            color_table_row(ui, kit, options);
            row(ui, &words.colors, "colors", |ui| {
                let field = whole(units, words.colors.clone(), f64::from(options.colors), COLORS);
                if let Some(v) = field.show(ui).edit.value().cloned().flatten() {
                    options.colors = v.round().clamp(*COLORS.start(), *COLORS.end()) as u16;
                }
            });
            dither_row(ui, words, options, true);
            if let Some(amount) = slider(ui, &words.amount, "amount", options.dither_amount, PERCENT, true) {
                options.dither_amount = amount;
            }
            if let Some(on) = check(ui, &words.transparency, "transparency", options.transparency) {
                options.transparency = on;
            }
            if let Some(on) = check(ui, &words.interlaced, "interlaced", options.interlaced) {
                options.interlaced = on;
            }
            if let Some(snap) = slider(ui, &words.web_snap, "snap", options.web_snap, PERCENT, true) {
                options.web_snap = snap;
            }
            if options.format == WebFormat::Gif
                && let Some(lossy) = slider(ui, &words.lossy, "lossy", options.lossy, PERCENT, true)
            {
                options.lossy = lossy;
            }
        }
        WebFormat::Jpeg => {
            quality_row(ui, words, options);
            if let Some(on) = check(ui, &words.progressive, "progressive", options.progressive) {
                options.progressive = on;
            }
            if let Some(on) = check(ui, &words.optimized_box, "optimized", options.optimized) {
                options.optimized = on;
            }
            if let Some(on) = check(ui, &words.embed_profile, "profile", options.embed_profile) {
                options.embed_profile = on;
            }
        }
        WebFormat::WebP => {
            if let Some(on) = check(ui, &words.lossless, "lossless", options.lossless) {
                options.lossless = on;
            }
            if options.has_quality() {
                quality_row(ui, words, options);
            }
            if let Some(on) = check(ui, &words.transparency, "transparency", options.transparency) {
                options.transparency = on;
            }
            if let Some(on) = check(ui, &words.embed_profile, "profile", options.embed_profile) {
                options.embed_profile = on;
            }
        }
        WebFormat::Wbmp => dither_row(ui, words, options, false),
        WebFormat::Png24 => {
            if let Some(on) = check(ui, &words.transparency, "transparency", options.transparency) {
                options.transparency = on;
            }
            if let Some(on) = check(ui, &words.interlaced, "interlaced", options.interlaced) {
                options.interlaced = on;
            }
        }
    }
}

fn quality_row(ui: &mut Ui, words: &SaveForWebWords, options: &mut SaveForWebOptions) {
    if let Some(quality) = slider(ui, &words.quality, "quality", options.quality, QUALITY, false) {
        options.quality = quality;
    }
}

/// The word of the palette method `table`.
fn color_table_name(words: &SaveForWebWords, table: ColorTable) -> Label {
    match table {
        ColorTable::Perceptual => words.perceptual.clone(),
        ColorTable::Selective => words.selective.clone(),
        ColorTable::Adaptive => words.adaptive.clone(),
        ColorTable::Restrictive => words.restrictive.clone(),
        ColorTable::Exact => words.exact.clone(),
        ColorTable::BlackWhite => words.black_white.clone(),
        ColorTable::Grayscale => words.grayscale.clone(),
    }
}

fn color_table_row(ui: &mut Ui, kit: &Kit<'_>, options: &mut SaveForWebOptions) {
    let words = kit.words;
    let entries: Vec<Entry<ColorTable>> = kit.color_tables.iter().map(|table| Entry::new(*table, color_table_name(words, *table))).collect();
    row(ui, &words.color_table, "table", |ui| {
        if let Some(table) = Select::new(words.color_table.clone(), Bind::given(options.color_table), &entries).show(ui).value().cloned() {
            options.color_table = table;
        }
    });
}

/// A palette entry as a swatch colour.
fn srgba([r, g, b]: [u8; 3]) -> Srgba {
    Srgba::from(Rgba([r, g, b, OPAQUE]))
}

/// The marks of the cell showing `color`: the host's locked, transparent and shifted colours, and whether it is web-safe.
fn cell_marks(marks: Option<&TableMarks>, color: Option<[u8; 3]>) -> CellMarks {
    let Some(color) = color else { return CellMarks::default() };
    let mut cell = CellMarks::default().web_safe(color.iter().all(|channel| channel % WEB_STEP == 0));
    if let Some(marks) = marks {
        cell = cell.locked(marks.locked.contains(&color)).transparent(marks.transparent.contains(&color));
        if let Some((from, _)) = marks.shifted.iter().find(|(_, to)| *to == color) {
            cell = cell.source(srgba(*from));
        }
    }
    cell
}

/// The Color Table: the shown pane's palette, one cell per index, with icon buttons, a menu and the Delete key for the
/// selected colours. Opening a cell asks the host for its colour, keyed [`PALETTE_COLOR_KEY`]; the colour waits in
/// `live.editing` for the answer. While the shown pane's encode failed, the note and Restore Generated Table show.
fn color_table_section(ui: &mut Ui, kit: &Kit<'_>, live: &mut Live, asked: &mut Asked) {
    let Kit { words, marks, failed, .. } = *kit;
    let Some(palette) = kit.palette.filter(|palette| !palette.is_empty()) else {
        if failed {
            Section::fixed(words.color_table.clone()).id_salt("palette").show(ui, |ui, _| {
                Text::new(words.table_failed.clone()).role(TypeRole::Caption).shade(Shade::Faint).id_salt("table-failed").show(ui);
                if clicked(&Button::new(words.restore_table.clone()).id_salt("restore").show(ui)) {
                    asked.table.push(ColorTableEvent { edit: ColorTableEdit::Restore, colors: Vec::new() });
                }
            });
        }
        return;
    };
    let entries: Vec<(Label, Srgba)> =
        palette.iter().enumerate().map(|(index, color)| (Label::from(format!("{index} {}", hex(*color))), srgba(*color))).collect();
    Section::fixed(words.color_table.clone()).id_salt("palette").show(ui, |ui, _| {
        let shown = SwatchTable::new(words.color_table.clone(), &entries)
            .per_row(16)
            .tips(true)
            .multiple(true)
            .marks(|at: &usize| cell_marks(marks, palette.get(*at).copied()))
            .selection(Bind::bound(&mut live.table))
            .show_all(ui);
        let selected: Vec<[u8; 3]> = shown.chosen.iter().filter_map(|at| palette.get(*at).copied()).collect();
        if let Some(color) = shown.open.and_then(|index| palette.get(index).copied()) {
            asked.color = Some(ColorRequest::new(PALETTE_COLOR_KEY, srgba(color)));
            live.editing = Some(color);
        }
        let deleted: Vec<[u8; 3]> = shown.delete.iter().filter_map(|at| palette.get(*at).copied()).collect();
        if !deleted.is_empty() {
            asked.table.push(ColorTableEvent { edit: ColorTableEdit::Delete, colors: deleted });
        }
        let tools = [
            ("web_colors", &words.web_shift, ColorTableEdit::WebShift),
            ("undo", &words.unshift, ColorTableEdit::Unshift),
            ("lock", &words.lock, ColorTableEdit::Lock),
            ("grid", &words.make_transparent, ColorTableEdit::Transparent),
            ("trash", &words.delete_color, ColorTableEdit::Delete),
        ];
        Row::new().id_salt("table-tools").show(ui, |ui, _| {
            for (icon, label, edit) in &tools {
                let button = IconButton::new(IconId::new(icon)).label((*label).clone()).enabled(!selected.is_empty()).id_salt(*icon).show(ui);
                if clicked(&button) {
                    asked.table.push(ColorTableEvent { edit: edit.clone(), colors: selected.clone() });
                }
            }
            let more = IconButton::new(IconId::new("more")).label(words.table_menu.clone()).id_salt("menu").show(ui);
            let orders = [
                (&words.order_unsorted, TableOrder::Unsorted),
                (&words.order_hue, TableOrder::Hue),
                (&words.order_luminance, TableOrder::Luminance),
                (&words.order_popularity, TableOrder::Popularity),
            ];
            let _ = Menu::new(words.table_menu.clone()).opened_by(&more, Trigger::Click).show(ui, |ui, _| {
                for (label, order) in orders {
                    if MenuItem::new(label.clone()).show(ui).ran {
                        asked.table.push(ColorTableEvent { edit: ColorTableEdit::Sort(order), colors: Vec::new() });
                    }
                }
                if MenuItem::new(words.unshift_all.clone()).show(ui).ran {
                    asked.table.push(ColorTableEvent { edit: ColorTableEdit::UnshiftAll, colors: Vec::new() });
                }
                if MenuItem::new(words.delete_color.clone()).enabled(!selected.is_empty()).show(ui).ran {
                    asked.table.push(ColorTableEvent { edit: ColorTableEdit::Delete, colors: selected.clone() });
                }
            });
        });
    });
}

/// The Slices choice (All Slices, All User Slices, Selected Slices) and, for a raster document, the Save HTML and
/// Images check box.
fn slices_rows(ui: &mut Ui, words: &SaveForWebWords, options: &mut SaveForWebOptions, vector: bool) {
    let entries = [
        Entry::new(0usize, words.all_slices.clone()),
        Entry::new(1usize, words.user_slices.clone()),
        Entry::new(2usize, words.selected_slices.clone()),
    ];
    let current = match options.slices {
        WebSlices::All => 0usize,
        WebSlices::User => 1,
        _ => 2,
    };
    row(ui, &words.slices, "slices", |ui| {
        if let Some(kind) = Select::new(words.slices.clone(), Bind::given(current), &entries).show(ui).value().cloned() {
            options.slices = match kind {
                0 => WebSlices::All,
                1 => WebSlices::User,
                _ => WebSlices::Selected(options.slices.selected().to_vec()),
            };
        }
    });
    if !vector && let Some(on) = check(ui, &words.save_html, "html", options.html) {
        options.html = on;
    }
}

/// A vector document's Output row: images alone, or HTML and images.
fn output_row(ui: &mut Ui, words: &SaveForWebWords, options: &mut SaveForWebOptions) {
    let entries = [Entry::new(false, words.output_images.clone()), Entry::new(true, words.output_html.clone())];
    row(ui, &words.output, "output", |ui| {
        if let Some(html) = Select::new(words.output.clone(), Bind::given(options.html), &entries).show(ui).value().copied() {
            options.html = html;
        }
    });
}

/// The Dither list; WBMP has no Noise.
fn dither_row(ui: &mut Ui, words: &SaveForWebWords, options: &mut SaveForWebOptions, noise: bool) {
    let mut entries = vec![
        Entry::new(Dither::None, words.no_dither.clone()),
        Entry::new(Dither::Diffusion, words.diffusion.clone()),
        Entry::new(Dither::Pattern, words.pattern.clone()),
    ];
    if noise {
        entries.push(Entry::new(Dither::Noise, words.noise.clone()));
    }
    row(ui, &words.dither, "dither", |ui| {
        if let Some(dither) = Select::new(words.dither.clone(), Bind::given(options.dither), &entries).show(ui).value().cloned() {
            options.dither = dither;
        }
    });
}

/// The Matte button, which asks the app's colour picker, and the matte as hexadecimal text.
fn matte_row(ui: &mut Ui, words: &SaveForWebWords, options: &mut SaveForWebOptions, ask: &mut Option<ColorRequest>) {
    let [r, g, b] = options.matte;
    let color = Rgba([r, g, b, OPAQUE]);
    row(ui, &words.matte, "matte", |ui| {
        Row::new().show(ui, |ui, _| {
            let swatch = Swatch::new(words.matte.clone(), Srgba::from(color)).picker(MATTE_COLOR_KEY).id_salt("choose").show(ui);
            if let Some(request) = swatch.pick_color {
                *ask = Some(request);
            }
            if let Some(Rgba([r, g, b, _])) = HexField::hex(words.matte_hex.clone(), Bind::given(color)).id_salt("hex").show(ui).value().cloned() {
                options.matte = [r, g, b];
            }
        });
    });
}

fn metadata_row(ui: &mut Ui, words: &SaveForWebWords, options: &mut SaveForWebOptions) {
    let entries = [
        Entry::new(WebMetadata::None, words.metadata_none.clone()),
        Entry::new(WebMetadata::Copyright, words.metadata_copyright.clone()),
        Entry::new(WebMetadata::CopyrightAndContact, words.metadata_contact.clone()),
        Entry::new(WebMetadata::All, words.metadata_all.clone()),
    ];
    row(ui, &words.metadata, "metadata", |ui| {
        if let Some(metadata) = Select::new(words.metadata.clone(), Bind::given(options.metadata), &entries).show(ui).value().cloned() {
            options.metadata = metadata;
        }
    });
}

/// Image Size: Width and Height in pixels or percent, the Percent row and, for a raster document, the Resample method
/// or, for a vector one, Anti-aliasing, Clip to Artboard and the Artboard.
fn size_section(
    ui: &mut Ui,
    words: &SaveForWebWords,
    sizing: &SizingWords,
    units: &Units,
    link: &mut LinkedSize,
    options: &mut SaveForWebOptions,
    document: &WebDocument,
) {
    let kinds = [Entry::new(UnitKind::Pixels, units.name(UnitKind::Pixels)), Entry::new(UnitKind::Percent, units.name(UnitKind::Percent))];
    Section::fixed(words.image_size.clone()).id_salt("size").show(ui, |ui, _| {
        row(ui, &words.width, "width", |ui| {
            let range = link.width_range();
            let field = UnitSelect::new(words.width.clone(), Bind::given(Some(link.width())), Bind::given(link.unit()), &kinds)
                .unit_label(words.size_unit.clone())
                .range(range)
                .show(ui);
            if let Some(v) = field.edit.value().cloned().flatten() {
                link.set_width(v);
                options.set_exact_size(link);
            }
            if let Some(unit) = field.unit.value().copied() {
                link.set_unit(unit);
            }
        });
        row(ui, &words.height, "height", |ui| {
            let range = link.height_range();
            let field = UnitSelect::new(words.height.clone(), Bind::given(Some(link.height())), Bind::given(link.unit()), &kinds)
                .unit_label(words.size_unit.clone())
                .range(range)
                .show(ui);
            if let Some(v) = field.edit.value().cloned().flatten() {
                link.set_height(v);
                options.set_exact_size(link);
            }
            if let Some(unit) = field.unit.value().copied() {
                link.set_unit(unit);
            }
        });
        row(ui, &words.percent, "percent", |ui| {
            let percent = percent_of(link.pixels(), link.original());
            let field = NumberField::new(words.percent.clone(), Bind::given(Some(percent)))
                .codec(NumberCodec::new(units.reader(Some(UnitKind::Percent)), 2))
                .range(LEAST_PERCENT..=MAX_PERCENT);
            if let Some(v) = field.show(ui).edit.value().cloned().flatten()
                && let Ok(size) = resize_from_scale(link.original(), v)
            {
                set_pixels(link, size);
                options.percent = v;
                options.size = None;
            }
        });
        if let Some(on) = check(ui, &words.constrain, "constrain", link.is_linked()) {
            options.constrain(link, on);
        }
        match document {
            WebDocument::Raster => {
                let methods = Resample::entries(sizing, &RESAMPLE_METHODS);
                row(ui, &sizing.resample_method, "resample", |ui| {
                    if let Some(method) =
                        Select::new(sizing.resample_method.clone(), Bind::given(options.resample), &methods).show(ui).value().copied()
                    {
                        options.resample = method;
                    }
                });
            }
            WebDocument::Vector { artboards } => vector_rows(ui, words, options, artboards),
        }
    });
}

/// A vector document's rows of Image Size: Anti-aliasing, Clip to Artboard and, while clipped with more than one
/// artboard, the Artboard.
fn vector_rows(ui: &mut Ui, words: &SaveForWebWords, options: &mut SaveForWebOptions, artboards: &[String]) {
    let entries = [
        Entry::new(WebAntiAlias::None, words.anti_alias_none.clone()),
        Entry::new(WebAntiAlias::Art, words.anti_alias_art.clone()),
        Entry::new(WebAntiAlias::Type, words.anti_alias_type.clone()),
    ];
    row(ui, &words.anti_alias, "anti-alias", |ui| {
        if let Some(mode) = Select::new(words.anti_alias.clone(), Bind::given(options.anti_alias), &entries).show(ui).value().cloned() {
            options.anti_alias = mode;
        }
    });
    if let Some(on) = check(ui, &words.clip_to_artboard, "clip", options.clip_to_artboard) {
        options.clip_to_artboard = on;
    }
    if options.clip_to_artboard && artboards.len() > 1 {
        let entries: Vec<Entry<usize>> = artboards.iter().enumerate().map(|(at, name)| Entry::new(at, Label::from(name.clone()))).collect();
        row(ui, &words.artboard, "artboard", |ui| {
            let current = options.artboard.unwrap_or(0);
            if let Some(at) = Select::new(words.artboard.clone(), Bind::given(current), &entries).show(ui).value().copied() {
                options.artboard = Some(at);
            }
        });
    }
}
