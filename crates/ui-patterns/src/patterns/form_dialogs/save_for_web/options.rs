//! The settings column of Save for Web: Preset, Format, the format's options, Matte, Convert to sRGB, Metadata and
//! Image Size.

use super::{
    ColorTable, Dither, MATTE_COLOR_KEY, MAX_PERCENT, RESAMPLE_METHODS, SaveForWebOptions, SaveForWebWords, SizingWords, WebFormat, WebMetadata,
    WebPreset, percent_of,
};
use crate::runtime::requests::ColorRequest;

use crate::buttons::Button;
use crate::choice::Checkbox;
use crate::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, clicked};

use crate::form::{Field, FieldGroup, SliderField, SliderFieldLayout};

use crate::layout_components::Row;
use crate::patterns::form_dialogs::{LinkedSize, Resample, resize_from_scale};
use crate::pickers::{Select, UnitSelect};

use crate::runtime::env::{Label, UnitKind, Units};

use crate::sections::Section;
use crate::text::{HexField, NumberField};

use craft_behavior::NumberCodec;
use craft_theme::{Rgba, Srgba};

use egui::Ui;
use std::ops::RangeInclusive;

/// The range of a percent slider.
const PERCENT: RangeInclusive<f64> = 0.0..=100.0;
/// The range of Quality.
const QUALITY: RangeInclusive<f64> = 0.0..=100.0;
/// The range of Colors.
const COLORS: RangeInclusive<f64> = 2.0..=256.0;
/// The least the Percent row takes.
const LEAST_PERCENT: f64 = 1.0;
/// Opaque alpha of the matte colour.
const OPAQUE: u8 = 255;

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
}

/// The settings column over `options` and the output size `link`; the matte request when Matte was pressed goes to
/// `ask`.
pub(super) fn show(ui: &mut Ui, kit: &Kit<'_>, options: &mut SaveForWebOptions, link: &mut LinkedSize, ask: &mut Option<ColorRequest>) {
    let Kit { words, sizing, units, presets } = *kit;
    FieldGroup::new(words.settings.clone()).show(ui, |ui, _| {
        if !presets.is_empty() {
            preset_row(ui, words, presets, options);
        }
        format_row(ui, words, options);
        format_options(ui, words, units, options);
        if options.has_matte() {
            matte_row(ui, words, options, ask);
        }
        if let Some(on) = check(ui, &words.convert_srgb, "srgb", options.convert_srgb) {
            options.convert_srgb = on;
        }
        metadata_row(ui, words, options);
        size_section(ui, words, sizing, units, link, &mut options.resample);
    });
}

fn preset_row(ui: &mut Ui, words: &SaveForWebWords, presets: &[WebPreset], options: &mut SaveForWebOptions) {
    let current = presets.iter().position(|preset| preset.is_set_in(options));
    let mut entries = vec![Entry::new(None, words.unnamed.clone())];
    entries.extend(presets.iter().enumerate().map(|(at, preset)| Entry::new(Some(at), Label::from(preset.name.clone()))));
    row(ui, &words.preset, "preset", |ui| {
        if let Some(Some(at)) = Select::new(words.preset.clone(), Bind::given(current), &entries).show(ui).value().cloned() {
            let (percent, resample) = (options.percent, options.resample);
            *options = SaveForWebOptions { percent, resample, ..presets[at].options.clone() };
        }
    });
}

fn format_row(ui: &mut Ui, words: &SaveForWebWords, options: &mut SaveForWebOptions) {
    let entries: Vec<Entry<WebFormat>> = WebFormat::ALL.iter().map(|format| Entry::new(*format, Label::from(format.name()))).collect();
    row(ui, &words.format, "format", |ui| {
        if let Some(format) = Select::new(words.format.clone(), Bind::given(options.format), &entries).show(ui).value().cloned() {
            options.format = format;
        }
    });
}

/// The rows of the chosen format.
fn format_options(ui: &mut Ui, words: &SaveForWebWords, units: &Units, options: &mut SaveForWebOptions) {
    match options.format {
        WebFormat::Gif | WebFormat::Png8 => {
            color_table_row(ui, words, options);
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

fn color_table_row(ui: &mut Ui, words: &SaveForWebWords, options: &mut SaveForWebOptions) {
    let entries = [
        Entry::new(ColorTable::Perceptual, words.perceptual.clone()),
        Entry::new(ColorTable::Selective, words.selective.clone()),
        Entry::new(ColorTable::Adaptive, words.adaptive.clone()),
        Entry::new(ColorTable::Restrictive, words.restrictive.clone()),
        Entry::new(ColorTable::Exact, words.exact.clone()),
    ];
    row(ui, &words.color_table, "table", |ui| {
        if let Some(table) = Select::new(words.color_table.clone(), Bind::given(options.color_table), &entries).show(ui).value().cloned() {
            options.color_table = table;
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
            if clicked(&Button::new(Label::from(color.to_hex())).id_salt("choose").show(ui)) {
                *ask = Some(ColorRequest::new(MATTE_COLOR_KEY, Srgba::from(color)).title(words.matte.clone()));
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

/// Image Size: Width and Height in pixels or percent, the Percent row and the Resample method.
fn size_section(ui: &mut Ui, words: &SaveForWebWords, sizing: &SizingWords, units: &Units, link: &mut LinkedSize, resample: &mut Resample) {
    let kinds = [
        Entry::new(UnitKind::Pixels, Label::from(units.symbol(UnitKind::Pixels))),
        Entry::new(UnitKind::Percent, Label::from(units.symbol(UnitKind::Percent))),
    ];
    Section::fixed(words.image_size.clone()).id_salt("size").show(ui, |ui, _| {
        row(ui, &words.width, "width", |ui| {
            let range = link.width_range();
            let field = UnitSelect::new(words.width.clone(), Bind::given(Some(link.width())), Bind::given(link.unit()), &kinds)
                .unit_label(words.size_unit.clone())
                .range(range)
                .show(ui);
            if let Some(v) = field.edit.value().cloned().flatten() {
                link.set_width(v);
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
            if let Some(v) = field.show(ui).edit.value().cloned().flatten() {
                set_percent(link, v);
            }
        });
        let methods = Resample::entries(sizing, &RESAMPLE_METHODS);
        row(ui, &sizing.resample_method, "resample", |ui| {
            if let Some(method) = Select::new(sizing.resample_method.clone(), Bind::given(*resample), &methods).show(ui).value().copied() {
                *resample = method;
            }
        });
    });
}

/// Sets the output size to `percent` of the document, whatever unit the fields show.
fn set_percent(link: &mut LinkedSize, percent: f64) {
    if let Ok(size) = resize_from_scale(link.original(), percent) {
        let unit = link.unit();
        link.set_unit(UnitKind::Pixels);
        link.set_width(f64::from(size.width));
        link.set_unit(unit);
    }
}
