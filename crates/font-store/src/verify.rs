//! Checks a downloaded or installed file against its index entry.

use sha2::{Digest, Sha256};

use skrifa::raw::FileRef;
use skrifa::raw::TableProvider;
use skrifa::string::StringId;
use skrifa::{FontRef, MetadataProvider};

use crate::index::{Family, FontFile, squash};
use crate::source::MAX_STR;

/// Tables of which a drawable face has at least one.
const OUTLINE_TABLES: [&[u8; 4]; 4] = [b"glyf", b"CFF ", b"CFF2", b"VARC"];

/// Why a file doesn't match its index entry.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    /// The size differs.
    #[error("the file is {actual} bytes, not {expected}")]
    Size {
        /// The index's size.
        expected: u64,
        /// The file's.
        actual: u64,
    },
    /// The sha256 differs.
    #[error("the file's sha256 doesn't match the index")]
    Sha256,
    /// It doesn't parse as a font with outlines.
    #[error("the file is not a font, or a damaged one")]
    NotAFont,
    /// The font doesn't name itself the family the entry belongs to.
    #[error("the file is the family {found}, not {expected}")]
    Family {
        /// The entry's family.
        expected: String,
        /// The file's, or `(none)`.
        found: String,
    },
    /// The font isn't the style the entry claims.
    #[error("the file is {found}, not {expected}")]
    Style {
        /// What the entry claims, of the attributes that differ: `weight 700, width 100%`; or
        /// `a variable font` or `a static font` when the file is not the kind the entry lists.
        expected: String,
        /// What the file is, of the same attributes, or `a static font` or `a variable font`.
        found: String,
    },
    /// The file has no variation axis the entry claims, or not its whole range.
    #[error("the file's `{tag}` axis is {found}, not {expected}")]
    Axis {
        /// The axis tag.
        tag: String,
        /// The entry's range.
        expected: String,
        /// The file's range, or `none`.
        found: String,
    },
    /// The entry gives PostScript names and the file's is not one of them.
    #[error("the file's PostScript name is {found}, not one the entry lists")]
    PostScript {
        /// The file's, or `(none)`.
        found: String,
    },
    /// The file is a collection, which an entry can't name a face of.
    #[error("the file is a font collection, and an index entry can't name one of its faces")]
    Collection,
}

/// The sha256 of `bytes`, in lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The size, then the sha256, of `bytes` against `file`.
fn check_bytes(file: &FontFile, bytes: &[u8]) -> Result<(), VerifyError> {
    let actual = bytes.len() as u64;
    if actual != file.size {
        return Err(VerifyError::Size { expected: file.size, actual });
    }
    if sha256_hex(bytes) != file.sha256 {
        return Err(VerifyError::Sha256);
    }
    Ok(())
}

/// Checks `bytes` against `file`: its size, its sha256, then that every face in it parses
/// and has outlines.
pub fn verify_font(file: &FontFile, bytes: &[u8]) -> Result<(), VerifyError> {
    check_bytes(file, bytes)?;
    if !is_font(bytes) {
        return Err(VerifyError::NotAFont);
    }
    Ok(())
}

/// Whether a face has a `name` table and outlines.
fn is_drawable(font: &FontRef<'_>) -> bool {
    font.table_data(skrifa::Tag::new(b"name")).is_some() && OUTLINE_TABLES.iter().any(|t| font.table_data(skrifa::Tag::new(t)).is_some())
}

/// Whether `bytes` is a font file or collection whose every face has a `name` table and outlines.
pub fn is_font(bytes: &[u8]) -> bool {
    let Ok(file) = FileRef::new(bytes) else { return false };
    let mut faces = 0usize;
    for font in file.fonts() {
        let Ok(font) = font else { return false };
        if !is_drawable(&font) {
            return false;
        }
        faces += 1;
    }
    faces > 0
}

/// Name records decoded per name ID. A `name` table holds at most 65,535 records (the format's
/// limit, so scanning their headers is bounded) and a hostile one can repeat an ID in all of them.
const MAX_NAME_RECORDS: usize = 64;
/// How closely a width percentage must agree.
const WIDTH_TOLERANCE: f32 = 0.5;
/// How closely an axis range must agree.
const AXIS_TOLERANCE: f32 = 0.01;
/// The percentage of each `usWidthClass` from 1 to 9.
const WIDTHS: [f32; 9] = [50.0, 62.5, 75.0, 87.5, 100.0, 112.5, 125.0, 150.0, 200.0];

/// The records of name ID `id` that can be compared. The headers scanned are at most 65,535 (the
/// format's limit). Each record is decoded to at most `MAX_STR + 1` characters, and only the
/// first [`MAX_NAME_RECORDS`] that decode to something are kept, so records that decode to
/// nothing don't use them up: at most 64 x 1,025 characters per ID. A record longer than
/// `MAX_STR` characters is left out, never cut short (a cut name could match).
fn names(font: &FontRef<'_>, id: u16) -> Vec<String> {
    font.localized_strings(StringId::new(id))
        .map(|s| s.chars().take(MAX_STR + 1).collect::<String>())
        .filter(|s| !s.is_empty())
        .take(MAX_NAME_RECORDS)
        .filter(|s| s.chars().count() <= MAX_STR)
        .collect()
}

/// [`verify_font`] for one font, then that the font really is what `family`'s index entry says,
/// whatever the index's author made of it. The host supplies the index and the sha256 together,
/// so this catches an index that describes the wrong file by its family, axes or PostScript name,
/// and, with no PostScript names listed, by its weight, width or italic flag. It is not a security
/// boundary.
///
/// The bytes must be the entry's size and sha256 and one font with a `name` table and outlines
/// (a collection is not checked and is [`VerifyError::Collection`]). Then
///
/// 1. the font names itself the entry's family: `family.family` equals, ignoring case, spaces,
///    hyphens, underscores and other punctuation (`Saira Extra Condensed` is `Saira
///    ExtraCondensed`), a non-empty typographic family name (name ID 16) or legacy family name
///    (ID 1) of the font. A font that names itself another family fails even when its
///    PostScript name is listed: a build registers a face by the font's own names;
/// 2. its variation axes are the entry's: a font is variable exactly when the entry lists axes,
///    each with at least the range the entry gives;
/// 3. when the entry lists PostScript names, the font's (ID 6) is one of them, exactly, and
///    nothing more is compared: the PostScript name is the style's identity. When it lists none,
///    the entry's weight, width and italic flag each equal one of the readings of the font's
///    raw `OS/2` and `head` fields that the index generators make (see the README). A comparison
///    is skipped for what the entry says a variable font covers: weight with a `wght` axis,
///    width with a `wdth` axis, the italic flag with an `ital` axis.
///
/// The work is bounded: one sha256 pass, one table directory, and for each of the name IDs 16, 1
/// and 6, at most 65,535 record headers scanned, at most 64 records decoded and at most 1,025
/// characters of each; only names of at most 1,024 characters are compared.
pub fn verify_face(family: &Family, file: &FontFile, bytes: &[u8]) -> Result<(), VerifyError> {
    check_bytes(file, bytes)?;
    if bytes.starts_with(b"ttcf") {
        return Err(VerifyError::Collection);
    }
    let font = match FileRef::new(bytes) {
        Ok(FileRef::Font(font)) if is_drawable(&font) => font,
        _ => return Err(VerifyError::NotAFont),
    };

    let found: Vec<String> = [16, 1].into_iter().flat_map(|id| names(&font, id)).collect();
    let wanted = squash(&family.family);
    // A name with nothing to compare (no letters or digits) must be equal as written.
    let same = |n: &String| if wanted.is_empty() { *n == family.family } else { squash(n) == wanted };
    if !found.iter().any(same) {
        let found = found.first().cloned().unwrap_or_else(|| "(none)".into());
        return Err(VerifyError::Family { expected: family.family.clone(), found });
    }

    let axes = font.axes();
    if axes.is_empty() != file.axes.is_empty() {
        let describe = |variable: bool| if variable { "a variable font" } else { "a static font" };
        return Err(VerifyError::Style { expected: describe(!file.axes.is_empty()).into(), found: describe(!axes.is_empty()).into() });
    }
    for want in &file.axes {
        let have = axes.iter().find(|a| a.tag().to_be_bytes() == want.tag.as_bytes());
        if !have.as_ref().is_some_and(|a| a.min_value() <= want.min + AXIS_TOLERANCE && a.max_value() >= want.max - AXIS_TOLERANCE) {
            let found = have.map_or_else(|| "none".to_string(), |a| format!("{}..{}", a.min_value(), a.max_value()));
            return Err(VerifyError::Axis { tag: want.tag.clone(), expected: format!("{}..{}", want.min, want.max), found });
        }
    }

    if !file.postscript.is_empty() {
        let have = names(&font, 6);
        if have.iter().any(|n| file.postscript.iter().any(|p| p == n)) {
            return Ok(());
        }
        return Err(VerifyError::PostScript { found: have.first().cloned().unwrap_or_else(|| "(none)".into()) });
    }
    check_style(file, &font)
}

/// The weight, width and italic flag of the entry against the readings of the font's raw
/// fields. Only for an entry with no PostScript names.
fn check_style(file: &FontFile, font: &FontRef<'_>) -> Result<(), VerifyError> {
    let covers = |tag: &str| file.axes.iter().any(|a| a.tag == tag);
    let os2 = font.os2().ok();
    let mac_style = font.head().map(|h| h.mac_style().bits()).unwrap_or(0);
    let (mac_bold, mac_italic) = (mac_style & 1 != 0, mac_style & 2 != 0);

    // The weights the readings give: the class, or the CSS weight of a legacy class.
    let weights: Vec<u16> = match &os2 {
        Some(os2) => {
            let class = os2.us_weight_class().clamp(1, 1000);
            match class {
                250 => vec![class, 100],
                275 => vec![class, 200],
                _ => vec![class],
            }
        }
        None => vec![if mac_bold { 700 } else { 400 }],
    };
    // The widths: the class's percentage, and 100 for a class the table has no row for.
    let widths: Vec<f32> = match &os2 {
        Some(os2) => {
            let class = os2.us_width_class();
            let mut v = vec![WIDTHS[usize::from(class.clamp(1, 9)) - 1]];
            if !(1..=9).contains(&class) {
                v.push(100.0);
            }
            v
        }
        None => vec![100.0],
    };
    // The italic flags: with an `OS/2` table, `fsSelection` bit 0 (italic) or bit 9 (oblique),
    // or bit 0 or `head`'s italic bit; without one, the italic bit.
    let italics: Vec<bool> = match &os2 {
        Some(os2) => {
            let sel = os2.fs_selection().bits();
            vec![sel & 1 != 0 || sel & (1 << 9) != 0, sel & 1 != 0 || mac_italic]
        }
        None => vec![mac_italic],
    };

    // Only the attributes compared and failed are named: one an axis exempts is not blamed.
    let join = |v: Vec<String>| v.join(" or ");
    let (mut expected, mut found) = (Vec::new(), Vec::new());
    if !covers("wght") && !weights.contains(&file.weight) {
        expected.push(format!("weight {}", file.weight));
        found.push(format!("weight {}", join(weights.iter().map(u16::to_string).collect())));
    }
    if !covers("wdth") && !widths.iter().any(|w| (w - file.width).abs() <= WIDTH_TOLERANCE) {
        expected.push(format!("width {}%", file.width));
        found.push(format!("width {}%", join(widths.iter().map(f32::to_string).collect())));
    }
    if !covers("ital") && !italics.contains(&file.is_italic()) {
        let word = |i: bool| if i { "italic" } else { "upright" };
        expected.push(word(file.is_italic()).to_string());
        found.push({
            let mut words: Vec<String> = italics.iter().map(|&i| word(i).to_string()).collect();
            words.dedup();
            join(words)
        });
    }
    if !expected.is_empty() {
        return Err(VerifyError::Style { expected: expected.join(", "), found: found.join(", ") });
    }
    Ok(())
}
