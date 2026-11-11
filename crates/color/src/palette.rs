//! Swatch files (F04.14): Photoshop's `.aco` swatches and `.act` colour tables, swatch exchange
//! `.ase` files, GIMP `.gpl` palettes and CSS custom properties, read into [`SwatchGroup`]s of
//! [`SwatchEntry`]s and written back.
//!
//! Each colour stays in its own model ([`SwatchColor`]): a CMYK swatch keeps its inks and a Lab
//! swatch its values through a read and a write. What a format cannot hold is reported, not
//! dropped silently ([`SwatchReport`]).
//!
//! ```no_run
//! use craft_color::palette::{SwatchFormat, read_swatches, write_swatches};
//! # let bytes: Vec<u8> = Vec::new();
//! let (groups, report) = read_swatches(&bytes, "Brand.ase")?;
//! let (aco, report) = write_swatches(&groups, SwatchFormat::Aco)?;
//! # Ok::<(), craft_color::palette::SwatchError>(())
//! ```

use craft_cms::convert::CmykConverter;
use craft_theme::{Hsb, Lab, Srgba};

/// A swatch file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SwatchFormat {
    /// Photoshop swatches (`.aco`, binary, versions 1 and 2): RGB, HSB, CMYK, Lab and grey
    /// colours with names, no groups. Colour-book colours are kept as they are.
    Aco,
    /// Photoshop colour table (`.act`, 768 bytes of RGB plus an optional count and transparent
    /// index): 8-bit RGB, no names. Recognised by its file name only: the bytes have no
    /// signature.
    Act,
    /// Adobe swatch exchange (`.ase`, binary): RGB, CMYK, Lab and grey colours, global, spot or
    /// process, in groups.
    Ase,
    /// GIMP palette (`.gpl`, text): 8-bit RGB with names; groups as `# Group:` comment headers
    /// (VectorCraft's convention).
    Gpl,
    /// CSS custom properties on `:root` (`--brand-blue: #3e63dd;`), one comment per group.
    /// Written only.
    Css,
}

impl SwatchFormat {
    /// Every format, in the Export menu's order.
    pub const ALL: [SwatchFormat; 5] = [SwatchFormat::Aco, SwatchFormat::Act, SwatchFormat::Ase, SwatchFormat::Gpl, SwatchFormat::Css];

    /// The file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            SwatchFormat::Aco => "aco",
            SwatchFormat::Act => "act",
            SwatchFormat::Ase => "ase",
            SwatchFormat::Gpl => "gpl",
            SwatchFormat::Css => "css",
        }
    }

    /// The format a file name's extension names (case-insensitive), if any.
    // pc: crates/engine/src/cmd/swatch_cmds.rs:377 `Format::parse`, :394 `ext_format`.
    pub fn from_name(file_name: &str) -> Option<SwatchFormat> {
        let extension = std::path::Path::new(file_name).extension()?.to_str()?.to_ascii_lowercase();
        SwatchFormat::ALL.into_iter().find(|f| f.extension() == extension)
    }

    /// Whether [`read_swatches`] reads the format (every one but CSS).
    pub fn can_read(self) -> bool {
        self != SwatchFormat::Css
    }

    /// The format of `bytes`: the `ASEF` signature, `GIMP Palette` on the first line, an `.aco`
    /// version header that parses; `.act` only by `file_name` (768 or 772 bytes).
    // inv-vectorcraft: crates/color/src/palette_io.rs:346 `sniff`, :365 `sniff_bytes`; pc:
    // crates/engine/src/cmd/swatch_cmds.rs:394 `ext_format` for the name fallback.
    pub fn sniff(bytes: &[u8], file_name: &str) -> Option<SwatchFormat> {
        let by_name = SwatchFormat::from_name(file_name);
        let act_length = bytes.len() == ACT_PALETTE_LEN || bytes.len() == ACT_PALETTE_LEN + ACT_TRAILER_LEN;
        if act_length && by_name == Some(SwatchFormat::Act) {
            return Some(SwatchFormat::Act);
        }
        if bytes.starts_with(ASE_SIGNATURE) {
            return Some(SwatchFormat::Ase);
        }
        if is_gpl(bytes) {
            return Some(SwatchFormat::Gpl);
        }
        // An `.act` whose first colour is `0, 1, x` starts like an `.aco`: at `.act` lengths the
        // header counts only when its sections fill the file exactly.
        if matches!(bytes.get(..2), Some([0, 1] | [0, 2]))
            && let Ok((_, len)) = parse_aco(bytes)
            && (!act_length || len == bytes.len())
        {
            return Some(SwatchFormat::Aco);
        }
        by_name
    }
}

/// A swatch colour in its own model.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "model", content = "values", rename_all = "camelCase")]
pub enum SwatchColor {
    /// Encoded RGB, 0–1 each, in the RGB working space.
    Rgb([f32; 3]),
    /// Hue 0–360°, saturation and brightness 0–1 (a view of an RGB colour).
    Hsb([f32; 3]),
    /// Cyan, magenta, yellow and black ink, 0–1 each, in the CMYK working space.
    Cmyk([f32; 4]),
    /// L* 0–100, a*, b* (D50).
    Lab([f32; 3]),
    /// Black ink 0–1 (0 white), as Photoshop's K.
    Gray(f32),
    /// A colour space this crate does not model (colour-book colours in an `.aco` file), kept
    /// as stored so it survives a round trip. Shows as black.
    Book {
        /// The colour-book space id stored in the file.
        space: u16,
        /// The four stored channel values, untouched.
        values: [u16; 4],
    },
}

impl SwatchColor {
    /// The colour as encoded sRGB for drawing: CMYK through `cmyk` (the document's converter),
    /// Lab through D50, HSB and grey directly; a [`Book`](SwatchColor::Book) colour is black.
    // pc: crates/ui-egui/src/panels/swatches_ui.rs:40 (`conv.rgb(&s.color)` per swatch).
    pub fn to_srgb(&self, cmyk: &CmykConverter) -> Srgba {
        match *self {
            SwatchColor::Rgb(v) => Srgba::from_rgb_array(v.map(unit)),
            SwatchColor::Hsb(v) => Srgba::from_rgb_array(hsb_to_rgb(v)),
            SwatchColor::Cmyk(v) => Srgba::from_rgb_array(cmyk.to_srgb(v.map(unit)).map(unit)),
            SwatchColor::Lab([l, a, b]) => {
                let (l, a, b) = (finite(l), finite(a), finite(b));
                // ICC v4 Lab encoding (the Lab D50 profile's device values).
                let encoded = [unit(l / 100.0), unit((a + 128.0) / 255.0), unit((b + 128.0) / 255.0)];
                match profile_to_srgb(&working_transforms().lab, &encoded) {
                    Some(rgb) => Srgba::from_rgb_array(rgb),
                    None => Srgba::from_lab(Lab { l, a, b }),
                }
            }
            SwatchColor::Gray(k) => {
                let level = 1.0 - unit(k);
                Srgba::from_rgb_array(profile_to_srgb(&working_transforms().gray, &[level]).unwrap_or([level; 3]))
            }
            SwatchColor::Book { .. } => Srgba::rgb(0.0, 0.0, 0.0),
        }
    }
}

/// How an `.ase` file marks a colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SwatchKind {
    #[default]
    #[allow(missing_docs)]
    Process,
    #[allow(missing_docs)]
    Global,
    #[allow(missing_docs)]
    Spot,
}

/// One named swatch.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SwatchEntry {
    /// The swatch's label.
    pub name: String,
    /// The swatch's colour.
    pub color: SwatchColor,
    #[serde(default)]
    /// How the file marks the colour; `Process` unless the file says otherwise.
    pub kind: SwatchKind,
}

impl SwatchEntry {
    /// A process swatch.
    pub fn new(name: impl Into<String>, color: SwatchColor) -> SwatchEntry {
        SwatchEntry { name: name.into(), color, kind: SwatchKind::Process }
    }
}

/// A named group of swatches, as the Swatches panel shows them. A file without groups reads as
/// one group named after the file.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SwatchGroup {
    /// The group's label.
    pub name: String,
    /// The group's swatches in display order.
    pub entries: Vec<SwatchEntry>,
}

/// What a read or a write could not keep. Each item is a sentence the app can show; the
/// variants let it word them itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SwatchReport {
    /// What was dropped or changed, in the order it was found.
    pub notes: Vec<SwatchNote>,
}

/// One thing a read or a write could not keep.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SwatchNote {
    /// Colour-book colours (`.aco` colours in a space this crate does not model) read as black,
    /// or left out of a format that cannot hold them, with their count.
    BookColors(usize),
    /// `.ase` colours in a colour model this crate does not read were skipped, with their count.
    Skipped(usize),
    /// The format has no groups: the swatches were written as one list.
    GroupsFlattened,
    /// The format has only 8-bit RGB: CMYK, Lab and grey colours were converted, with their count.
    ConvertedToRgb(usize),
    /// The format has no spot or global colours: they were written as process colours, with their
    /// count.
    KindsDropped(usize),
    /// `.act` holds at most 256 colours: the rest were left out, with their count.
    Truncated(usize),
    /// Names were cut to what the format holds (`.ase`: 1024 UTF-16 units).
    NamesCut(usize),
}

/// Why a swatch file could not be read or written.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SwatchError {
    /// Neither the content nor the name says what the file is.
    UnknownFormat,
    /// The format is write-only (CSS).
    CannotRead(SwatchFormat),
    /// The file is cut short or malformed, with the parser's message.
    Malformed(SwatchFormat, String),
    /// Nothing to write (no swatches in any group).
    Empty,
}

impl std::fmt::Display for SwatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SwatchError::UnknownFormat => f.write_str("not a swatch file"),
            SwatchError::CannotRead(format) => write!(f, ".{} files can be written, not read", format.extension()),
            SwatchError::Malformed(format, message) => write!(f, "not a readable .{} file: {message}", format.extension()),
            SwatchError::Empty => f.write_str("there are no swatches to write"),
        }
    }
}

impl std::error::Error for SwatchError {}

/// Reads a swatch file: the format from the content ([`SwatchFormat::sniff`]), `.act` from
/// `file_name`. Ungrouped colours go into a group named after the file (its stem); a swatch
/// without a name is "Swatch N". Names are cut to one line.
// pc: crates/engine/src/cmd/swatch_cmds.rs:491 `read_file` (`.aco`, `.act`, `.ase` with
// :406 `from_aco`, :435 `from_ase`), crates/psd/src/aco.rs:150 `parse`, crates/psd/src/ase.rs:180
// `parse`, crates/psd/src/act.rs:46 `parse`; inv-vectorcraft: crates/color/src/palette_io.rs:376
// `read_gpl`. The binary parsers move here (craft-color has no psd crate).
pub fn read_swatches(bytes: &[u8], file_name: &str) -> Result<(Vec<SwatchGroup>, SwatchReport), SwatchError> {
    let format = SwatchFormat::sniff(bytes, file_name).ok_or(SwatchError::UnknownFormat)?;
    let stem = stem_of(file_name);
    let mut report = SwatchReport::default();
    let mut groups: Vec<SwatchGroup> = Vec::new();
    match format {
        SwatchFormat::Css => return Err(SwatchError::CannotRead(format)),
        SwatchFormat::Aco => {
            let (swatches, _) = parse_aco(bytes).map_err(|e| malformed(format, e))?;
            let books = swatches.iter().filter(|(_, c)| matches!(c, AcoColor::Other { .. })).count();
            push_note(&mut report, books, SwatchNote::BookColors);
            let entries =
                swatches.into_iter().enumerate().map(|(i, (name, color))| SwatchEntry::new(entry_name(i, &name), from_aco(color))).collect();
            groups.push(SwatchGroup { name: stem, entries });
        }
        SwatchFormat::Act => {
            let used = act_used(bytes).map_err(|e| malformed(format, e))?;
            let entries = used
                .iter()
                .enumerate()
                .map(|(i, c)| SwatchEntry::new(format!("Swatch {}", i + 1), SwatchColor::Rgb(c.map(|v| f32::from(v) / 255.0))))
                .collect();
            groups.push(SwatchGroup { name: stem, entries });
        }
        SwatchFormat::Ase => {
            let items = parse_ase(bytes).map_err(|e| malformed(format, e))?;
            let (mut n, mut skipped) = (0usize, 0usize);
            let mut convert = |s: &AseSwatch| {
                n += 1;
                let entry = from_ase(&s.color).map(|color| SwatchEntry { name: entry_name(n - 1, &s.name), color, kind: s.kind });
                skipped += usize::from(entry.is_none());
                entry
            };
            let mut loose = Vec::new();
            for item in &items {
                match item {
                    AseItem::Swatch(s) => loose.extend(convert(s)),
                    AseItem::Group(name, swatches) => {
                        let entries = swatches.iter().filter_map(&mut convert).collect();
                        let name = clip_name(name);
                        groups.push(SwatchGroup { name: if name.is_empty() { stem.clone() } else { name }, entries });
                    }
                }
            }
            if !loose.is_empty() || groups.is_empty() {
                groups.insert(0, SwatchGroup { name: stem, entries: loose });
            }
            push_note(&mut report, skipped, SwatchNote::Skipped);
        }
        SwatchFormat::Gpl => groups = read_gpl(bytes, file_name),
    }
    if count_swatches(&groups) == 0 {
        return Err(malformed(format, "the file holds no swatches"));
    }
    Ok((groups, report))
}

/// Writes `groups` as `format`, with what the format could not hold.
// pc: crates/engine/src/cmd/swatch_cmds.rs:566 `write_file` (`.aco` with :418 `to_aco`, `.ase`
// with :447 `to_ase`, `.act`), crates/psd/src/aco.rs:201 `write` (version 1 then 2),
// crates/psd/src/act.rs:63 `write`; inv-vectorcraft: crates/color/src/palette_io.rs:128
// `write_gpl`, :182 `write_css` with :148 `css_property`.
pub fn write_swatches(groups: &[SwatchGroup], format: SwatchFormat) -> Result<(Vec<u8>, SwatchReport), SwatchError> {
    if count_swatches(groups) == 0 {
        return Err(SwatchError::Empty);
    }
    let mut report = SwatchReport::default();
    let all = || groups.iter().flat_map(|g| g.entries.iter());
    let kinds = all().filter(|e| e.kind != SwatchKind::Process).count();
    let flattened = groups.len() > 1;
    match format {
        SwatchFormat::Aco => {
            if count_swatches(groups) > ACO_MAX_SWATCHES {
                return Err(malformed(format, "an .aco file holds at most 65535 swatches"));
            }
            let entries: Vec<(String, AcoColor)> = all().map(|e| (e.name.clone(), to_aco(&e.color))).collect();
            if flattened {
                report.notes.push(SwatchNote::GroupsFlattened);
            }
            push_note(&mut report, kinds, SwatchNote::KindsDropped);
            push_note(&mut report, names_cut(groups, false), SwatchNote::NamesCut);
            Ok((write_aco(&entries), report))
        }
        SwatchFormat::Act => {
            let (colors, books, converted) = rgb_colors(groups);
            let mut triplets: Vec<[u8; 3]> = colors.iter().flatten().map(|(_, c, _)| *c).collect();
            if triplets.is_empty() {
                return Err(SwatchError::Empty);
            }
            let left_out = triplets.len().saturating_sub(ACT_COLORS);
            triplets.truncate(ACT_COLORS);
            if flattened {
                report.notes.push(SwatchNote::GroupsFlattened);
            }
            push_note(&mut report, books, SwatchNote::BookColors);
            push_note(&mut report, converted, SwatchNote::ConvertedToRgb);
            push_note(&mut report, kinds, SwatchNote::KindsDropped);
            push_note(&mut report, left_out, SwatchNote::Truncated);
            let mut out: Vec<u8> = triplets.iter().flatten().copied().collect();
            out.resize(ACT_PALETTE_LEN, 0);
            out.extend((triplets.len() as u16).to_be_bytes());
            out.extend(0xFFFFu16.to_be_bytes());
            Ok((out, report))
        }
        SwatchFormat::Ase => {
            if count_swatches(groups) > ASE_MAX_SWATCHES || groups.len() > ASE_MAX_GROUPS {
                return Err(malformed(format, "too many swatches or groups for an .ase file"));
            }
            let mut body = Vec::new();
            let (mut blocks, mut books) = (0u32, 0usize);
            for g in groups {
                let mut name = Vec::new();
                put_ase_name(&mut name, &g.name);
                put_ase_block(&mut body, ASE_GROUP_START, &name);
                blocks += 2;
                for e in &g.entries {
                    match to_ase(&e.color) {
                        Some(color) => {
                            put_ase_color(&mut body, &e.name, &color, e.kind);
                            blocks += 1;
                        }
                        None => books += 1,
                    }
                }
                put_ase_block(&mut body, ASE_GROUP_END, &[]);
            }
            push_note(&mut report, books, SwatchNote::BookColors);
            push_note(&mut report, names_cut(groups, true), SwatchNote::NamesCut);
            let mut out = Vec::with_capacity(12 + body.len());
            out.extend(ASE_SIGNATURE);
            out.extend(1u16.to_be_bytes());
            out.extend(0u16.to_be_bytes());
            out.extend(blocks.to_be_bytes());
            out.extend(body);
            Ok((out, report))
        }
        SwatchFormat::Gpl | SwatchFormat::Css => {
            let (colors, books, converted) = rgb_colors(groups);
            push_note(&mut report, books, SwatchNote::BookColors);
            push_note(&mut report, converted, SwatchNote::ConvertedToRgb);
            let text = if format == SwatchFormat::Gpl {
                push_note(&mut report, kinds, SwatchNote::KindsDropped);
                let plain: Vec<Vec<(String, [u8; 3])>> = colors.iter().map(|l| l.iter().map(|(n, c, _)| (n.clone(), *c)).collect()).collect();
                write_gpl(groups, &plain)
            } else {
                write_css(groups, &colors)
            };
            Ok((text.into_bytes(), report))
        }
    }
}

/// PhotoCraft's default swatches (46 RGB colours in five groups: Grays, the exact RGB primaries
/// and secondaries, Pastel, Pure and Dark hues), what Reset Swatches restores.
// pc: crates/engine/src/presets/swatches.rs:294 `builtin`.
pub fn default_swatches() -> Vec<SwatchGroup> {
    let rgb = |name: &str, c: [u8; 3]| SwatchEntry::new(name, SwatchColor::Rgb(c.map(|v| f32::from(v) / 255.0)));
    let grays = [
        ("Black", 0),
        ("90% Gray", 26),
        ("80% Gray", 51),
        ("70% Gray", 77),
        ("60% Gray", 102),
        ("50% Gray", 128),
        ("40% Gray", 153),
        ("30% Gray", 179),
        ("20% Gray", 204),
        ("White", 255),
    ];
    let hues = ["Red", "Orange", "Yellow", "Yellow Green", "Green", "Cyan Green", "Cyan", "Blue", "Violet", "Magenta"];
    let pastel: [[u8; 3]; 10] = [
        [236, 128, 128],
        [244, 176, 132],
        [250, 224, 128],
        [214, 240, 128],
        [150, 232, 150],
        [128, 232, 200],
        [128, 220, 240],
        [128, 176, 244],
        [168, 144, 244],
        [232, 144, 232],
    ];
    let pure: [[u8; 3]; 10] = [
        [230, 40, 40],
        [245, 120, 30],
        [250, 210, 30],
        [160, 220, 40],
        [40, 200, 80],
        [30, 200, 170],
        [30, 170, 230],
        [40, 100, 230],
        [120, 70, 220],
        [210, 50, 180],
    ];
    let dark: [[u8; 3]; 10] = [
        [120, 20, 20],
        [130, 60, 10],
        [130, 110, 10],
        [80, 120, 20],
        [20, 100, 40],
        [10, 100, 90],
        [10, 80, 120],
        [20, 50, 120],
        [60, 30, 110],
        [110, 20, 90],
    ];
    let row = |prefix: &str, colors: [[u8; 3]; 10]| -> Vec<SwatchEntry> {
        hues.iter().zip(colors).map(|(hue, c)| rgb(&if prefix.is_empty() { (*hue).to_string() } else { format!("{prefix} {hue}") }, c)).collect()
    };
    // Photoshop's order: red, yellow, green, cyan, blue, magenta.
    let exact: [(&str, [u8; 3]); 6] = [
        ("RGB Red", [255, 0, 0]),
        ("RGB Yellow", [255, 255, 0]),
        ("RGB Green", [0, 255, 0]),
        ("RGB Cyan", [0, 255, 255]),
        ("RGB Blue", [0, 0, 255]),
        ("RGB Magenta", [255, 0, 255]),
    ];
    let group = |name: &str, entries: Vec<SwatchEntry>| SwatchGroup { name: name.into(), entries };
    vec![
        group("Grays", grays.iter().map(|(n, v)| rgb(n, [*v; 3])).collect()),
        group("RGB", exact.iter().map(|(n, c)| rgb(n, *c)).collect()),
        group("Pastel", row("Pastel", pastel)),
        group("Pure", row("", pure)),
        group("Dark", row("Dark", dark)),
    ]
}

mod aco;
mod ase;
mod reader;
#[cfg(test)]
mod tests;
mod text;

use aco::*;
use ase::*;
use reader::*;
use text::*;
