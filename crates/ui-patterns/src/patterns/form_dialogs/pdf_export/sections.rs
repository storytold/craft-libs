//! The seven sections of Save PDF and the rows they share.

use super::PdfSection;
use super::words::PdfExportWords;
use crate::choice::{Checkbox, Radio, RadioGroup};
use crate::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use crate::display::{Shade, Text};
use crate::form::{Field, FieldGroup};
use crate::layout_components::Stack;
use crate::notices::{Notice, NoticeLayout};
use crate::pickers::Select;
use crate::runtime::env::{Label, UnitKind, Units};
use crate::sections::Section;
use crate::style::TypeRole;
use crate::text::{NumberField, TextField};
use craft_behavior::Mixed;
use craft_pdf::settings::{Choice, ColorConversion, Compatibility, DOCUMENT_CMYK, Encryption, ImageCodec, ImageCompression, PdfSettings, Standard};
use craft_theme::variant::Tone;
use egui::Ui;
use std::ops::RangeInclusive;

/// Trim mark weights the Trim mark weight list offers, in points.
const WEIGHTS: [f64; 3] = [0.125, 0.25, 0.5];

/// The longest length the bleed and the mark offset take, in points.
const MAX_LENGTH: f64 = 72.0;
/// The range of a resolution, in pixels per inch.
const PPI: RangeInclusive<f64> = 9.0..=2400.0;
/// The range of the font subset threshold, in percent.
const PERCENT: RangeInclusive<f64> = 0.0..=100.0;

/// The artboards to write: all, or the typed range ("1-3, 5").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ArtboardChoice {
    pub range_only: bool,
    pub range: String,
}

/// The two check boxes that turn a password on. A password that is set keeps its box checked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PasswordSwitches {
    pub open: bool,
    pub permissions: bool,
}

/// What the sections read and edit in one pass.
pub(super) struct Sections<'a> {
    pub words: &'a PdfExportWords,
    pub units: &'a Units,
    /// The unit lengths in points are shown in.
    pub length: UnitKind,
    /// The colour profiles the app knows, for Destination and Output intent profile.
    pub profiles: &'a [String],
    /// The names of the saved flattener presets.
    pub flatteners: &'a [String],
    /// The document's bleed in points: top, bottom, left, right.
    pub document_bleed: [f64; 4],
    pub work: &'a mut PdfSettings,
    pub artboards: &'a mut ArtboardChoice,
    pub switches: &'a mut PasswordSwitches,
    /// Why the typed artboard range is refused, already worded.
    pub range_error: Option<&'a str>,
    /// The warnings of the Summary, already worded.
    pub warnings: &'a [String],
    /// The options that differ from the chosen preset, already worded.
    pub changed: &'a [String],
    /// The chosen preset's name as shown.
    pub preset_name: &'a Label,
}

impl Sections<'_> {
    pub(super) fn show(&mut self, ui: &mut Ui, section: PdfSection) {
        match section {
            PdfSection::General => self.general(ui),
            PdfSection::Compression => self.compression(ui),
            PdfSection::Marks => self.marks_and_bleeds(ui),
            PdfSection::Output => self.output(ui),
            PdfSection::Advanced => self.advanced(ui),
            PdfSection::Security => self.security(ui),
            PdfSection::Summary => self.summary(ui),
        }
    }

    fn general(&mut self, ui: &mut Ui) {
        let words = self.words;
        let plain = self.work.standard == Standard::None;
        // PDF layers need PDF 1.5, and PDF/X-1a and PDF/X-3 have none.
        let layers = self.work.standard.allows_layers() && self.work.compatibility.has_layers();
        let work = &mut *self.work;
        group(ui, "options", &words.options, |ui| {
            flag(ui, "preserve", &words.preserve_editing, &mut work.preserve_editing, plain);
            flag(ui, "thumbnails", &words.thumbnails, &mut work.thumbnails, true);
            flag(ui, "fast", &words.fast_web_view, &mut work.fast_web_view, true);
            flag(ui, "view", &words.view_after_saving, &mut work.view_after_saving, true);
            flag(ui, "layers", &words.create_layers, &mut work.create_layers, layers);
            flag(ui, "nonprinting", &words.include_non_printing, &mut work.include_non_printing, true);
        });
        let artboards = &mut *self.artboards;
        let range_error = self.range_error;
        group(ui, "artboards", &words.artboards, |ui| {
            let chosen = RadioGroup::new(words.artboards.clone(), Bind::given(!artboards.range_only)).show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    Radio::new(true).label(words.all.clone()).show(ui);
                    Radio::new(false).label(words.range.clone()).show(ui);
                });
            });
            if let Some(all) = chosen.output.value().cloned() {
                artboards.range_only = !all;
            }
            field(ui, "range", &words.range, artboards.range_only, |ui| {
                let typed = TextField::new(words.range.clone(), Bind::given(artboards.range.clone()))
                    .placeholder(words.range_hint.clone())
                    .enabled(artboards.range_only)
                    .show(ui);
                if let Some(text) = typed.value().cloned() {
                    artboards.range = text;
                }
            });
            if let Some(message) = range_error.filter(|_| artboards.range_only) {
                Notice::new(Tone::Bad, Label::from(message.to_owned())).layout(NoticeLayout::Inline).show(ui);
            }
        });
    }

    fn compression(&mut self, ui: &mut Ui) {
        let words = self.words;
        let compression = &mut self.work.compression;
        for (id, title, image) in [("color", &words.color_images, &mut compression.color), ("gray", &words.gray_images, &mut compression.gray)] {
            group(ui, id, title, |ui| image_rows(ui, words, image));
        }
        let mono = &mut compression.mono;
        group(ui, "mono", &words.mono_images, |ui| {
            resample_rows(ui, words, &mut mono.downsample, &mut mono.ppi, &mut mono.above_ppi);
            pick(ui, "codec", "monoCodec", &words.compression, words, &mut mono.compression, true);
        });
        let compress_text = &mut compression.compress_text;
        group(ui, "text", &words.compress_text, |ui| flag(ui, "compress", &words.compress_text, compress_text, true));
    }

    fn marks_and_bleeds(&mut self, ui: &mut Ui) {
        let (words, units, length) = (self.words, self.units, self.length);
        let marks = &mut self.work.marks;
        group(ui, "marks", &words.marks, |ui| {
            let count = [marks.trim, marks.registration, marks.color_bars, marks.page_info].iter().filter(|on| **on).count();
            let all = match count {
                0 => Mixed::Same(false),
                4 => Mixed::Same(true),
                _ => Mixed::Mixed,
            };
            let all_marks = Checkbox::new(Bind::given(all)).label(words.all_marks.clone()).id_salt("all").show(ui);
            if let Some(on) = all_marks.value().cloned() {
                (marks.trim, marks.registration, marks.color_bars, marks.page_info) = (on, on, on, on);
            }
            flag(ui, "trim", &words.trim_marks, &mut marks.trim, true);
            flag(ui, "registration", &words.registration_marks, &mut marks.registration, true);
            flag(ui, "bars", &words.color_bars, &mut marks.color_bars, true);
            flag(ui, "info", &words.page_information, &mut marks.page_info, true);
            pick(ui, "kind", "markKind", &words.mark_type, words, &mut marks.kind, true);
            let mut weights: Vec<f64> = WEIGHTS.to_vec();
            if !weights.iter().any(|weight| (weight - marks.weight).abs() < f64::EPSILON) {
                weights.push(marks.weight);
            }
            let entries: Vec<Entry<usize>> =
                weights.iter().enumerate().map(|(at, weight)| Entry::new(at, Label::from(units.show(UnitKind::Points, *weight)))).collect();
            let current = weights.iter().position(|weight| (weight - marks.weight).abs() < f64::EPSILON).unwrap_or_default();
            field(ui, "weight", &words.mark_weight, true, |ui| {
                let chosen = Select::new(words.mark_weight.clone(), Bind::given(current), &entries).show(ui);
                if let Some(at) = chosen.value().cloned().and_then(|at| weights.get(at).copied()) {
                    marks.weight = at;
                }
            });
            length_row(ui, "offset", &words.mark_offset, &mut marks.offset, length, true);
        });
        let bleed = &mut self.work.bleed;
        let document = self.document_bleed;
        group(ui, "bleeds", &words.bleeds, |ui| {
            flag(ui, "document", &words.use_document_bleed, &mut bleed.use_document, true);
            // With the document's bleed the fields show it.
            let use_document = bleed.use_document;
            let mut shown = if use_document { document } else { bleed.values() };
            let sides = [("top", &words.top), ("bottom", &words.bottom), ("left", &words.left), ("right", &words.right)];
            for (side, (id, label)) in sides.into_iter().enumerate() {
                if let Some(points) = shown.get_mut(side) {
                    length_row(ui, id, label, points, length, !use_document);
                }
            }
            if !use_document {
                [bleed.top, bleed.bottom, bleed.left, bleed.right] = shown;
            }
        });
    }

    fn output(&mut self, ui: &mut Ui) {
        let (words, profiles) = (self.words, self.profiles);
        let standard = self.work.standard;
        let work = &mut *self.work;
        let cmyk_only = standard.cmyk_only();
        group(ui, "color", &words.color, |ui| {
            let output = &mut work.output;
            pick(ui, "conversion", "colorConversion", &words.color_conversion, words, &mut output.conversion, true);
            // PDF/X-1a converts to CMYK even without a conversion.
            let converting = output.conversion != ColorConversion::None || cmyk_only;
            let mut destinations = vec![(String::new(), words.document_profile.clone()), (DOCUMENT_CMYK.to_owned(), Label::from(DOCUMENT_CMYK))];
            destinations.extend(profiles.iter().map(|name| (name.clone(), Label::from(name.clone()))));
            name_row(ui, "destination", &words.destination, &mut output.destination, destinations, converting);
            // A standard decides whether colours are tagged.
            pick(ui, "profiles", "profileInclusion", &words.profile_inclusion, words, &mut output.profiles, standard == Standard::None);
            if cmyk_only {
                note(ui, &words.note_cmyk_only);
            } else if standard != Standard::None {
                note(ui, &words.note_tagged);
            }
        });
        // PDF/A files carry their own output intent; PDF/X files always have one.
        let own = standard != Standard::PdfA2b;
        let blank = if standard.is_pdfx() { &words.cmyk_in_effect } else { &words.no_profile };
        group(ui, "intent", &words.output_intent, |ui| {
            let output = &mut work.output;
            let mut intents = vec![(String::new(), blank.clone())];
            intents.extend(profiles.iter().map(|name| (name.clone(), Label::from(name.clone()))));
            name_row(ui, "profile", &words.intent_profile, &mut output.output_intent, intents, own);
            text_row(ui, "condition", &words.output_condition, &mut output.output_condition, own);
            text_row(ui, "conditionId", &words.condition_identifier, &mut output.output_condition_id, own);
            text_row(ui, "registry", &words.registry_name, &mut output.registry, own);
            flag(ui, "trapped", &words.trapped, &mut output.trapped, own);
            if !own {
                note(ui, &words.note_pdfa_intent);
            }
        });
    }

    fn advanced(&mut self, ui: &mut Ui) {
        let (words, flatteners) = (self.words, self.flatteners);
        // PDF 1.3 files (PDF/X-1a and PDF/X-3 ones too) have no transparency: it is flattened.
        let flat = self.work.pdf13();
        let work = &mut *self.work;
        group(ui, "fonts", &words.fonts, |ui| {
            let advanced = &mut work.advanced;
            let outlined = advanced.outline_text;
            number_row(
                ui,
                "subset",
                &words.subset_below,
                &mut advanced.font_subset_percent,
                NumberSetup { kind: UnitKind::Percent, range: PERCENT, enabled: !outlined },
            );
            note(ui, &words.of_characters);
            flag(ui, "outline", &words.outline_text, &mut advanced.outline_text, true);
        });
        group(ui, "overprint", &words.overprint_flattener, |ui| {
            pick(ui, "overprint", "overprint", &words.overprint, words, &mut work.advanced.overprint, true);
            let mut presets = vec![
                ("High Resolution".to_owned(), words.flatten_high.clone()),
                ("Medium Resolution".to_owned(), words.flatten_medium.clone()),
                ("Low Resolution".to_owned(), words.flatten_low.clone()),
            ];
            presets.extend(flatteners.iter().map(|name| (name.clone(), Label::from(name.clone()))));
            // The empty name and the ids `high`, `medium` and `low` name the built-in presets.
            let built_in = ["high", "medium", "low"];
            let mut current = match work.flattener_preset.trim() {
                "" => "High Resolution".to_owned(),
                id if built_in.iter().any(|name| name.eq_ignore_ascii_case(id)) => {
                    let at = built_in.iter().position(|name| name.eq_ignore_ascii_case(id)).unwrap_or_default();
                    presets.get(at).map(|(name, _)| name.clone()).unwrap_or_default()
                }
                name => name.to_owned(),
            };
            let before = current.clone();
            name_row(ui, "flattener", &words.flattener_preset, &mut current, presets, flat);
            if current != before {
                work.flattener_preset = current;
            }
            if !flat {
                note(ui, &words.note_flatten);
            }
        });
    }

    fn security(&mut self, ui: &mut Ui) {
        let words = self.words;
        let plain = self.work.standard == Standard::None;
        let compatibility = self.work.compatibility;
        let security = &mut self.work.security;
        let switches = &mut *self.switches;
        let mut open = false;
        let mut restrict = false;
        if !plain {
            note(ui, &words.note_standard_password);
        }
        group(ui, "open", &words.open_password_heading, |ui| {
            open = password_switch(ui, "require", &words.require_open_password, &mut switches.open, &mut security.open_password, plain);
            password_row(ui, "open-password", &words.open_password, &mut security.open_password, open);
        });
        group(ui, "permissions", &words.permissions, |ui| {
            restrict =
                password_switch(ui, "restrict", &words.restrict_permissions, &mut switches.permissions, &mut security.permissions_password, plain);
            password_row(ui, "permissions-password", &words.permissions_password, &mut security.permissions_password, restrict);
            pick(ui, "printing", "printing", &words.printing_allowed, words, &mut security.printing, restrict);
            pick(ui, "changes", "changes", &words.changes_allowed, words, &mut security.changes, restrict);
            flag(ui, "copy", &words.allow_copy, &mut security.copy, restrict);
            // What can be copied can be read out.
            let mut reader = security.screen_reader || security.copy;
            flag(ui, "reader", &words.allow_reader, &mut reader, restrict && !security.copy);
            if !security.copy {
                security.screen_reader = reader;
            }
            let plaintext_ok = (open || restrict) && !matches!(compatibility, Compatibility::Pdf13 | Compatibility::Pdf14);
            flag(ui, "plaintext", &words.plaintext_metadata, &mut security.plaintext_metadata, plaintext_ok);
            let name = Encryption::for_compatibility(compatibility).label();
            note_text(ui, words.encryption_note.fill(&[("name", name)]));
        });
    }

    fn summary(&mut self, ui: &mut Ui) {
        let words = self.words;
        let work = &*self.work;
        let standard = words.choice("standard", work.standard.id(), work.standard.label());
        let compatibility = words.choice("compatibility", work.compatibility.id(), work.compatibility.label());
        let encryption = work.encryption().map_or_else(|| words.no_encryption.clone(), |e| Label::from(e.label()));
        group(ui, "summary", &words.options, |ui| {
            value_row(ui, "standard", &words.standard, &standard);
            value_row(ui, "compatibility", &words.compatibility, &compatibility);
            value_row(ui, "encryption", &words.encryption, &encryption);
        });
        Section::fixed(words.summary_changes.clone()).id_salt("changes").show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                for line in self.changed {
                    Text::new(Label::from(line.clone())).show(ui);
                }
                if self.changed.is_empty() {
                    note_text(ui, words.summary_matches.fill(&[("preset", self.preset_name.as_str())]));
                }
            });
        });
        Section::fixed(words.summary_warnings.clone()).id_salt("warnings").show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                if self.warnings.is_empty() {
                    note(ui, &words.no_warnings);
                }
                for warning in self.warnings {
                    Text::new(Label::from(warning.clone())).show(ui);
                }
            });
        });
    }
}

/// The settings of one kind of image: downsampling, resolution, compression and, for lossy codecs, quality.
fn image_rows(ui: &mut Ui, words: &PdfExportWords, image: &mut ImageCompression) {
    resample_rows(ui, words, &mut image.downsample, &mut image.ppi, &mut image.above_ppi);
    pick(ui, "codec", "imageCodec", &words.compression, words, &mut image.compression, true);
    let lossy = !matches!(image.compression, ImageCodec::None | ImageCodec::Zip);
    pick(ui, "quality", "jpegQuality", &words.quality, words, &mut image.quality, lossy);
}

/// Downsampling, to and above rows of one kind of image.
fn resample_rows(ui: &mut Ui, words: &PdfExportWords, downsample: &mut craft_pdf::settings::Downsample, ppi: &mut f64, above: &mut f64) {
    pick(ui, "downsample", "downsample", &words.downsampling, words, downsample, true);
    let enabled = *downsample != craft_pdf::settings::Downsample::None;
    for (id, label, value) in [("ppi", &words.to, ppi), ("above", &words.above, above)] {
        number_row(ui, id, label, value, NumberSetup { kind: UnitKind::PixelsPerInch, range: PPI, enabled });
    }
}

/// A titled group of rows: a section whose body is one field group.
fn group(ui: &mut Ui, id: &'static str, title: &Label, mut rows: impl FnMut(&mut Ui)) {
    Section::fixed(title.clone()).id_salt(id).show(ui, |ui, _| {
        FieldGroup::new(title.clone()).show(ui, |ui, _| rows(ui));
    });
}

/// `body` in a field named `label`, greyed while not `enabled`.
fn field(ui: &mut Ui, id: &'static str, label: &Label, enabled: bool, mut body: impl FnMut(&mut Ui)) {
    Field::new(label.clone()).id_salt(id).enabled(enabled).show(ui, |ui, _| body(ui));
}

/// A muted note under the rows.
fn note(ui: &mut Ui, text: &Label) {
    Text::new(text.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
}

fn note_text(ui: &mut Ui, text: String) {
    Text::new(Label::from(text)).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
}

/// A check box row; `enabled` off greys it.
fn flag(ui: &mut Ui, id: &'static str, label: &Label, value: &mut bool, enabled: bool) {
    let shown = Checkbox::new(Bind::given(*value)).label(label.clone()).id_salt(id).enabled(enabled).show(ui);
    if let Some(on) = shown.value().cloned() {
        *value = on;
    }
}

/// A list of the choices of `T`, worded through `words`.
fn pick<T: Choice + PartialEq + Send + Sync>(
    ui: &mut Ui,
    id: &'static str,
    kind: &str,
    label: &Label,
    words: &PdfExportWords,
    value: &mut T,
    enabled: bool,
) {
    let entries: Vec<Entry<T>> = T::ALL
        .iter()
        .zip(T::IDS)
        .zip(T::LABELS)
        .map(|((choice, choice_id), choice_label)| Entry::new(*choice, words.choice(kind, choice_id, choice_label)))
        .collect();
    field(ui, id, label, enabled, |ui| {
        let chosen = Select::new(label.clone(), Bind::given(*value), &entries).show(ui);
        if let Some(next) = chosen.value().cloned() {
            *value = next;
        }
    });
}

/// A list of names (`value`, `label`); a current value that is not among them is added.
fn name_row(ui: &mut Ui, id: &'static str, label: &Label, current: &mut String, mut names: Vec<(String, Label)>, enabled: bool) {
    if !names.iter().any(|(value, _)| value == current) {
        names.push((current.clone(), Label::from(current.clone())));
    }
    let entries: Vec<Entry<String>> = names.into_iter().map(|(value, text)| Entry::new(value, text)).collect();
    field(ui, id, label, enabled, |ui| {
        let chosen = Select::new(label.clone(), Bind::given(current.clone()), &entries).show(ui);
        if let Some(next) = chosen.value().cloned() {
            *current = next;
        }
    });
}

fn text_row(ui: &mut Ui, id: &'static str, label: &Label, value: &mut String, enabled: bool) {
    field(ui, id, label, enabled, |ui| {
        let typed = TextField::new(label.clone(), Bind::given(value.clone())).show(ui);
        if let Some(text) = typed.value().cloned() {
            *value = text;
        }
    });
}

fn password_row(ui: &mut Ui, id: &'static str, label: &Label, value: &mut String, enabled: bool) {
    field(ui, id, label, enabled, |ui| {
        let typed = TextField::new(label.clone(), Bind::given(value.clone())).masked().show(ui);
        if let Some(text) = typed.value().cloned() {
            *value = text;
        }
    });
}

/// The check box that turns a password on. Unchecking it clears the password. Returns whether the password applies.
fn password_switch(ui: &mut Ui, id: &'static str, label: &Label, switch: &mut bool, password: &mut String, enabled: bool) -> bool {
    let mut on = *switch || !password.is_empty();
    flag(ui, id, label, &mut on, enabled);
    if on != (*switch || !password.is_empty()) {
        *switch = on;
        if !on {
            password.clear();
        }
    }
    on && enabled
}

/// A label and a value that is shown, not edited.
fn value_row(ui: &mut Ui, id: &'static str, label: &Label, value: &Label) {
    field(ui, id, label, true, |ui| {
        Text::new(value.clone()).show(ui);
    });
}

/// How a number row reads and limits its value.
struct NumberSetup {
    kind: UnitKind,
    range: RangeInclusive<f64>,
    enabled: bool,
}

/// A number of `setup.kind` in a field.
fn number_row(ui: &mut Ui, id: &'static str, label: &Label, value: &mut f64, setup: NumberSetup) {
    let NumberSetup { kind, range, enabled } = setup;
    field(ui, id, label, enabled, |ui| {
        let number = NumberField::new(label.clone(), Bind::given(Some(*value))).unit_kind(kind).range(range.clone()).show(ui);
        if let Some(typed) = number.edit.value().cloned().flatten() {
            *value = typed;
        }
    });
}

/// A length stored in points and shown in `unit`: a length kind, else points.
fn length_row(ui: &mut Ui, id: &'static str, label: &Label, points: &mut f64, unit: UnitKind, enabled: bool) {
    let (kind, scale) = match unit.per_inch(UnitKind::POINTS_PER_INCH) {
        Some(per_inch) => (unit, per_inch / UnitKind::POINTS_PER_INCH),
        None => (UnitKind::Points, 1.0),
    };
    let mut shown = *points * scale;
    number_row(ui, id, label, &mut shown, NumberSetup { kind, range: 0.0..=MAX_LENGTH * scale, enabled });
    if (shown - *points * scale).abs() > f64::EPSILON {
        *points = shown / scale;
    }
}
