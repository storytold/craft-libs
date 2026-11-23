//! The Summary's list of options that differ from the chosen preset, and the artboard range.

use super::PdfSection;
use super::words::PdfExportWords;
use crate::runtime::env::Label;
use craft_pdf::settings::{Choice, ImageCompression, PdfSettings};

/// The value of one setting as the Summary compares and words it.
#[derive(Clone, Debug, PartialEq)]
enum Shown {
    Flag(bool),
    Number(f64),
    Text(String),
    /// A choice of the closed list `kind` names, with its stable id and craft-pdf's English label.
    Choice {
        kind: &'static str,
        id: &'static str,
        label: &'static str,
    },
}

fn choice<T: Choice + PartialEq>(kind: &'static str, value: T) -> Shown {
    let at = T::ALL.iter().position(|candidate| *candidate == value).unwrap_or_default();
    Shown::Choice { kind, id: T::IDS.get(at).copied().unwrap_or_default(), label: T::LABELS.get(at).copied().unwrap_or_default() }
}

/// The paths of the settings of one kind of image, in the order `image` lists them.
struct ImagePaths {
    downsample: &'static str,
    ppi: &'static str,
    above_ppi: &'static str,
    compression: &'static str,
    quality: &'static str,
}

const COLOR: ImagePaths = ImagePaths {
    downsample: "compression.color.downsample",
    ppi: "compression.color.ppi",
    above_ppi: "compression.color.abovePpi",
    compression: "compression.color.compression",
    quality: "compression.color.quality",
};

const GRAY: ImagePaths = ImagePaths {
    downsample: "compression.gray.downsample",
    ppi: "compression.gray.ppi",
    above_ppi: "compression.gray.abovePpi",
    compression: "compression.gray.compression",
    quality: "compression.gray.quality",
};

fn image(out: &mut Vec<(&'static str, Shown)>, paths: &ImagePaths, image: &ImageCompression) {
    out.push((paths.downsample, choice("downsample", image.downsample)));
    out.push((paths.ppi, Shown::Number(image.ppi)));
    out.push((paths.above_ppi, Shown::Number(image.above_ppi)));
    out.push((paths.compression, choice("imageCodec", image.compression)));
    out.push((paths.quality, choice("jpegQuality", image.quality)));
}

/// Every setting of `settings` that has a row, by its path (`compression.color.ppi`). Passwords are not among them.
fn leaves(settings: &PdfSettings) -> Vec<(&'static str, Shown)> {
    let mut out = vec![
        ("standard", choice("standard", settings.standard)),
        ("compatibility", choice("compatibility", settings.compatibility)),
        ("preserveEditing", Shown::Flag(settings.preserve_editing)),
        ("thumbnails", Shown::Flag(settings.thumbnails)),
        ("fastWebView", Shown::Flag(settings.fast_web_view)),
        ("viewAfterSaving", Shown::Flag(settings.view_after_saving)),
        ("createLayers", Shown::Flag(settings.create_layers)),
        ("includeNonPrinting", Shown::Flag(settings.include_non_printing)),
        ("flattenerPreset", Shown::Text(settings.flattener_preset.clone())),
    ];
    image(&mut out, &COLOR, &settings.compression.color);
    image(&mut out, &GRAY, &settings.compression.gray);
    let mono = &settings.compression.mono;
    out.push(("compression.mono.downsample", choice("downsample", mono.downsample)));
    out.push(("compression.mono.ppi", Shown::Number(mono.ppi)));
    out.push(("compression.mono.abovePpi", Shown::Number(mono.above_ppi)));
    out.push(("compression.mono.compression", choice("monoCodec", mono.compression)));
    out.push(("compression.compressText", Shown::Flag(settings.compression.compress_text)));
    let marks = &settings.marks;
    out.extend([
        ("marks.trim", Shown::Flag(marks.trim)),
        ("marks.registration", Shown::Flag(marks.registration)),
        ("marks.colorBars", Shown::Flag(marks.color_bars)),
        ("marks.pageInfo", Shown::Flag(marks.page_info)),
        ("marks.kind", choice("markKind", marks.kind)),
        ("marks.weight", Shown::Number(marks.weight)),
        ("marks.offset", Shown::Number(marks.offset)),
    ]);
    let bleed = &settings.bleed;
    out.extend([
        ("bleed.useDocument", Shown::Flag(bleed.use_document)),
        ("bleed.top", Shown::Number(bleed.top)),
        ("bleed.bottom", Shown::Number(bleed.bottom)),
        ("bleed.left", Shown::Number(bleed.left)),
        ("bleed.right", Shown::Number(bleed.right)),
    ]);
    let output = &settings.output;
    out.extend([
        ("output.conversion", choice("colorConversion", output.conversion)),
        ("output.destination", Shown::Text(output.destination.clone())),
        ("output.profiles", choice("profileInclusion", output.profiles)),
        ("output.outputIntent", Shown::Text(output.output_intent.clone())),
        ("output.outputCondition", Shown::Text(output.output_condition.clone())),
        ("output.outputConditionId", Shown::Text(output.output_condition_id.clone())),
        ("output.registry", Shown::Text(output.registry.clone())),
        ("output.trapped", Shown::Flag(output.trapped)),
    ]);
    let advanced = &settings.advanced;
    out.extend([
        ("advanced.fontSubsetPercent", Shown::Number(advanced.font_subset_percent)),
        ("advanced.outlineText", Shown::Flag(advanced.outline_text)),
        ("advanced.overprint", choice("overprint", advanced.overprint)),
    ]);
    let security = &settings.security;
    out.extend([
        ("security.printing", choice("printing", security.printing)),
        ("security.changes", choice("changes", security.changes)),
        ("security.copy", Shown::Flag(security.copy)),
        ("security.screenReader", Shown::Flag(security.screen_reader)),
        ("security.plaintextMetadata", Shown::Flag(security.plaintext_metadata)),
    ]);
    out
}

/// The section a setting belongs to.
fn section_of(path: &str) -> PdfSection {
    match path.split('.').next().unwrap_or_default() {
        "compression" => PdfSection::Compression,
        "marks" | "bleed" => PdfSection::Marks,
        "output" => PdfSection::Output,
        "advanced" => PdfSection::Advanced,
        "security" => PdfSection::Security,
        _ => PdfSection::General,
    }
}

/// The words of one setting, by the last segment of its path; `None` for a setting without a row.
fn leaf_label<'a>(words: &'a PdfExportWords, path: &str) -> Option<&'a Label> {
    let leaf = path.rsplit('.').next().unwrap_or(path);
    Some(match leaf {
        "standard" => &words.standard,
        "compatibility" => &words.compatibility,
        "preserveEditing" => &words.preserve_editing,
        "thumbnails" => &words.thumbnails,
        "fastWebView" => &words.fast_web_view,
        "viewAfterSaving" => &words.view_after_saving,
        "createLayers" => &words.create_layers,
        "includeNonPrinting" => &words.include_non_printing,
        "flattenerPreset" => &words.flattener_preset,
        "downsample" => &words.downsampling,
        "ppi" => &words.to,
        "abovePpi" => &words.above,
        "compression" => &words.compression,
        "quality" => &words.quality,
        "compressText" => &words.compress_text,
        "trim" => &words.trim_marks,
        "registration" => &words.registration_marks,
        "colorBars" => &words.color_bars,
        "pageInfo" => &words.page_information,
        "kind" => &words.mark_type,
        "weight" => &words.mark_weight,
        "offset" => &words.mark_offset,
        "useDocument" => &words.use_document_bleed,
        "top" => &words.top,
        "bottom" => &words.bottom,
        "left" => &words.left,
        "right" => &words.right,
        "conversion" => &words.color_conversion,
        "destination" => &words.destination,
        "profiles" => &words.profile_inclusion,
        "outputIntent" => &words.intent_profile,
        "outputCondition" => &words.output_condition,
        "outputConditionId" => &words.condition_identifier,
        "registry" => &words.registry_name,
        "trapped" => &words.trapped,
        "fontSubsetPercent" => &words.subset_below,
        "outlineText" => &words.outline_text,
        "overprint" => &words.overprint,
        "printing" => &words.printing_allowed,
        "changes" => &words.changes_allowed,
        "copy" => &words.allow_copy,
        "screenReader" => &words.allow_reader,
        "plaintextMetadata" => &words.plaintext_metadata,
        _ => return None,
    })
}

/// `compression.color.abovePpi` as "Compression › Color images › above": the section, the group (`color`, `gray`,
/// `mono` images), then the setting.
pub(super) fn option_label(words: &PdfExportWords, path: &str) -> String {
    let mut parts = vec![section_of(path).title(words).as_str().to_owned()];
    let keys: Vec<&str> = path.split('.').collect();
    if let [_, group, _] = keys.as_slice() {
        parts.push(
            match *group {
                "color" => &words.color_images,
                "gray" => &words.gray_images,
                _ => &words.mono_images,
            }
            .as_str()
            .to_owned(),
        );
    }
    parts.push(leaf_label(words, path).map_or_else(|| humanize(keys.last().copied().unwrap_or(path)), |label| label.as_str().to_owned()));
    parts.join(words.path_separator.as_str())
}

/// `abovePpi` as "Above Ppi", for a setting without words.
fn humanize(key: &str) -> String {
    let mut out = String::new();
    for (at, c) in key.chars().enumerate() {
        if at == 0 {
            out.extend(c.to_uppercase());
        } else {
            if c.is_uppercase() {
                out.push(' ');
            }
            out.push(c);
        }
    }
    out
}

fn value_text(words: &PdfExportWords, shown: &Shown) -> String {
    match shown {
        Shown::Flag(true) => words.on.as_str().to_owned(),
        Shown::Flag(false) => words.off.as_str().to_owned(),
        Shown::Number(number) => number.to_string(),
        Shown::Text(text) => text.clone(),
        Shown::Choice { kind, id, label } => words.choice(kind, id, label).as_str().to_owned(),
    }
}

/// One "Section › Setting: value" line for every setting of `settings` that differs from `base`, in the order of the
/// sections. Passwords are never part of the comparison.
pub(crate) fn differences(words: &PdfExportWords, settings: &PdfSettings, base: &PdfSettings) -> Vec<String> {
    let base = leaves(base);
    let mut diff: Vec<(&'static str, Shown)> = leaves(settings)
        .into_iter()
        .filter(|(path, shown)| base.iter().find(|(other, _)| other == path).is_none_or(|(_, held)| held != shown))
        .collect();
    diff.sort_by_key(|(path, _)| section_of(path));
    diff.iter()
        .map(|(path, shown)| words.difference.fill(&[("setting", option_label(words, path).as_str()), ("value", value_text(words, shown).as_str())]))
        .collect()
}

/// Why an artboard range was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RangeError {
    /// `part` is not a number from 1 to the artboard count.
    #[non_exhaustive]
    NotANumber {
        /// The part of the range.
        part: String,
    },
    /// A part of only a hyphen.
    #[non_exhaustive]
    NoNumber {
        /// The part of the range.
        part: String,
    },
    /// A part that runs from a larger number to a smaller one.
    #[non_exhaustive]
    Backwards {
        /// The part of the range.
        part: String,
    },
    /// No part at all.
    Empty,
}

/// Whether `text` (`"1-3, 5"`, 1-based; `all` for every artboard) names artboards of a document of `count`: each number
/// lies in `1..=count`, `3-` runs to the last, `-2` from the first, no part runs backwards.
pub(crate) fn check_range(text: &str, count: usize) -> Result<(), RangeError> {
    if text.trim().eq_ignore_ascii_case("all") {
        return Ok(());
    }
    let number = |part: &str, open: usize| -> Result<usize, RangeError> {
        let part = part.trim();
        if part.is_empty() {
            return Ok(open);
        }
        part.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| part.parse().ok())
            .flatten()
            .filter(|n| (1..=count).contains(n))
            .ok_or_else(|| RangeError::NotANumber { part: part.to_owned() })
    };
    let mut any = false;
    for part in text.split(',').map(str::trim).filter(|part| !part.is_empty()) {
        let (from, to) = match part.replace('\u{2013}', "-").split_once('-') {
            Some((a, b)) if a.trim().is_empty() && b.trim().is_empty() => return Err(RangeError::NoNumber { part: part.to_owned() }),
            Some((a, b)) => (number(a, 1)?, number(b, count)?),
            None => {
                let n = number(part, 0)?;
                (n, n)
            }
        };
        if from > to {
            return Err(RangeError::Backwards { part: part.to_owned() });
        }
        any = true;
    }
    if any { Ok(()) } else { Err(RangeError::Empty) }
}
