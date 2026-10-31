//! Reading face records from font files: the table directory and the `name`, `OS/2` and `fvar`
//! tables, never the whole file.

use std::path::{Path, PathBuf};

use skrifa::raw::{FileRef, TableProvider};
use skrifa::{MetadataProvider, string::StringId};

/// Where a face comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// Installed on the system: a platform font folder or the platform's font service.
    System,
    /// The suite's font store ([`crate::font_store_dir`]).
    Store,
    /// A folder the app added ([`crate::Catalog::set_app_dirs`]).
    App,
    /// Bundled with the app and added as bytes.
    Bundled,
    /// Added by the user to this app.
    User,
    /// Embedded in, or shipped beside, an open document.
    Document,
}

/// A name in one language, from the font's `name` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalizedName {
    /// The record's BCP 47 language tag (`ja`, `zh-Hant`), when the font says.
    pub language: Option<String>,
    /// The name.
    pub name: String,
}

/// One face of an installed or added font: one face of a file, or one named instance of a
/// variable font.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceInfo {
    /// The family: the typographic family name (name ID 16), else the family name (ID 1), in
    /// English when the font has an English record.
    pub family: String,
    /// The style: the typographic subfamily (ID 17), else the subfamily (ID 2), else `Regular`.
    /// A named instance's subfamily name.
    pub style: String,
    /// The legacy family name (ID 1) when it differs from [`FaceInfo::family`] (`Roboto Light`
    /// in the family `Roboto`).
    pub legacy_family: Option<String>,
    /// The legacy style (ID 2) that goes with [`FaceInfo::legacy_family`].
    pub legacy_style: Option<String>,
    /// The family's names in other languages (`ヒラギノ角ゴシック`).
    pub localized_families: Vec<LocalizedName>,
    /// The full name (ID 4) of the face itself; `None` for a named instance.
    pub full_name: Option<String>,
    /// The PostScript name (ID 6, or a named instance's own), as PDF and other documents name fonts.
    pub postscript_name: Option<String>,
    /// The weight, 1 to 1000 (400 regular, 700 bold): `usWeightClass`, a named instance's `wght`,
    /// else the weight the style name implies.
    pub weight: f32,
    /// The width as a percentage of normal (100; 75 condensed, 125 expanded): `usWidthClass`, a
    /// named instance's `wdth`, else the width the style name implies.
    pub width: f32,
    /// Italic or oblique.
    pub italic: bool,
    /// A named instance's axis settings (tag, user units); empty for the face itself.
    pub variations: Vec<([u8; 4], f32)>,
    /// The face has variation axes.
    pub variable: bool,
    /// The OS/2 `fsType` embedding permissions (0 without an OS/2 table).
    pub fs_type: u16,
    /// The OS/2 code page bits (`ulCodePageRange1`, `ulCodePageRange2`).
    pub code_pages: [u32; 2],
    /// The file the face is in; `None` for a face added as bytes.
    pub path: Option<PathBuf>,
    /// The face's number in its file: its index in a collection (0 in a single font), or for a
    /// Mac suitcase font the number of its `sfnt` resource.
    pub index: u32,
    /// Where it comes from.
    pub source: Source,
    /// For a face added as bytes: which of the catalog's added blobs holds it.
    pub(crate) blob: Option<u64>,
    /// A confined folder's file ([`crate::Catalog::set_confined_dirs`]), recorded by its real
    /// path: on Unix its loads don't follow a link put in its place after the scan.
    pub(crate) confined: bool,
    /// The size and modification time of the file the face is in when the scan or
    /// [`crate::Catalog::add_file`] listed it, so a file replaced in place changes its family's
    /// signature ([`crate::Catalog::changes_since`]) and a [`crate::FaceCache`] makes its faces
    /// again.
    pub(crate) file: Option<FileStamp>,
}

/// A file's size and modification time: when either differs, the file changed.
pub(crate) type FileStamp = (u64, Option<std::time::SystemTime>);

pub(crate) fn file_stamp(meta: &std::fs::Metadata) -> FileStamp {
    (meta.len(), meta.modified().ok())
}

/// A face record with the normalized names it answers to.
#[derive(Clone, Debug)]
pub(crate) struct Face {
    pub info: FaceInfo,
    pub keys: Keys,
}

/// Every name a face answers to besides its family and style, normalized ([`norm`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct Keys {
    /// Its family in every language, typographic else legacy.
    pub families: Vec<String>,
    /// Its style in every language, and a variable font's default instance's name.
    pub styles: Vec<String>,
    /// Legacy family and style pairs (`Hiragino Sans W3` / `Regular`), per language record.
    pub legacy: Vec<(String, String)>,
    pub full_names: Vec<String>,
    pub postscript: Vec<String>,
}

const MAX_AXES: usize = 64;
const MAX_INSTANCES: usize = 512;
/// The most faces read from a (possibly damaged) collection.
pub(crate) const MAX_FACES: u32 = 256;
/// The largest table the scan reads.
const MAX_TABLE: u32 = 1 << 20;
/// Tables holding outlines a renderer draws. A face with none of them (outlines in Apple's `hvgl`
/// table, bitmaps only) draws nothing and is left out.
const OUTLINE_TABLES: [&[u8; 4]; 4] = [b"glyf", b"CFF ", b"CFF2", b"VARC"];

pub(crate) fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The weight a style name implies (400 when it names none).
pub fn style_weight(style: &str) -> f32 {
    const TABLE: &[(&str, f32)] = &[
        ("extralight", 200.0),
        ("ultralight", 200.0),
        ("semibold", 600.0),
        ("demibold", 600.0),
        ("extrabold", 800.0),
        ("ultrabold", 800.0),
        ("hairline", 100.0),
        ("thin", 100.0),
        ("light", 300.0),
        ("medium", 500.0),
        ("bold", 700.0),
        ("black", 900.0),
        ("heavy", 900.0),
    ];
    let s = norm(style);
    TABLE.iter().find(|(k, _)| s.contains(k)).map_or(400.0, |(_, w)| *w)
}

/// Whether a style name says italic or oblique.
pub fn style_italic(style: &str) -> bool {
    let s = norm(style);
    s.contains("italic") || s.contains("oblique") || s == "it"
}

/// The width a style name implies, in percent of normal (100 when it names none).
pub fn style_width(style: &str) -> f32 {
    const TABLE: &[(&str, f32)] = &[
        ("ultracondensed", 50.0),
        ("extracondensed", 62.5),
        ("semicondensed", 87.5),
        ("condensed", 75.0),
        ("narrow", 75.0),
        ("ultraexpanded", 200.0),
        ("extraexpanded", 150.0),
        ("semiexpanded", 112.5),
        ("expanded", 125.0),
        ("extended", 125.0),
        ("wide", 125.0),
    ];
    let s = norm(style);
    TABLE.iter().find(|(k, _)| s.contains(k)).map_or(100.0, |(_, w)| *w)
}

/// `usWidthClass` (1–9) as a percentage of normal.
fn width_class_percent(class: u16) -> Option<f32> {
    const PERCENT: [f32; 9] = [50.0, 62.5, 75.0, 87.5, 100.0, 112.5, 125.0, 150.0, 200.0];
    PERCENT.get(usize::from(class).checked_sub(1)?).copied()
}

fn name(font: &skrifa::FontRef<'_>, ids: &[StringId]) -> Option<String> {
    ids.iter().find_map(|id| font.localized_strings(*id).english_or_first().map(|s| s.to_string()).filter(|s| !s.is_empty()))
}

/// Every string of `id`, with its language.
fn localized(font: &skrifa::FontRef<'_>, id: StringId) -> Vec<(Option<String>, String)> {
    font.localized_strings(id).map(|s| (s.language().map(str::to_owned), s.to_string())).filter(|(_, s)| !s.is_empty()).collect()
}

fn all_names(font: &skrifa::FontRef<'_>, id: StringId) -> Vec<String> {
    localized(font, id).into_iter().map(|(_, s)| s).collect()
}

fn sort_dedup(v: &mut Vec<String>) {
    v.retain(|k| !k.is_empty());
    v.sort();
    v.dedup();
}

/// The faces `font` (face `index` of its file) offers: the face itself, then the named instances
/// of a variable font. Empty when it has no family name.
pub(crate) fn font_faces(font: &skrifa::FontRef<'_>, index: u32, path: Option<&Path>, source: Source) -> Vec<Face> {
    let Some(family) = name(font, &[StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME]) else { return vec![] };
    let style = name(font, &[StringId::TYPOGRAPHIC_SUBFAMILY_NAME, StringId::SUBFAMILY_NAME]).unwrap_or_else(|| "Regular".into());
    let typo_family = localized(font, StringId::TYPOGRAPHIC_FAMILY_NAME);
    let typo_style = all_names(font, StringId::TYPOGRAPHIC_SUBFAMILY_NAME);
    let legacy_family = localized(font, StringId::FAMILY_NAME);
    let legacy_style = localized(font, StringId::SUBFAMILY_NAME);

    let family_records = if typo_family.is_empty() { &legacy_family } else { &typo_family };
    let mut localized_families: Vec<LocalizedName> = Vec::new();
    for (language, n) in family_records {
        if *n != family && !localized_families.iter().any(|l| l.name == *n) {
            localized_families.push(LocalizedName { language: language.clone(), name: n.clone() });
        }
    }
    let mut families: Vec<String> = std::iter::once(family.as_str()).chain(family_records.iter().map(|(_, f)| f.as_str())).map(norm).collect();
    // A legacy style names the face beside its legacy family only when the font has a
    // typographic style: every weight of `Hiragino Sans` is `Regular` in its own legacy family.
    let style_names: Vec<String> = if typo_style.is_empty() { legacy_style.iter().map(|(_, s)| s.clone()).collect() } else { typo_style };
    let mut styles: Vec<String> = std::iter::once(style.as_str()).chain(style_names.iter().map(String::as_str)).map(norm).collect();
    let mut legacy: Vec<(String, String)> = if typo_family.is_empty() {
        vec![]
    } else {
        legacy_family
            .iter()
            .flat_map(|(lf, f)| legacy_style.iter().filter(move |(ls, _)| ls == lf).map(move |(_, s)| (norm(f), norm(s))))
            .filter(|(f, s)| !f.is_empty() && !s.is_empty())
            .collect()
    };
    legacy.sort();
    legacy.dedup();
    let mut postscript: Vec<String> = all_names(font, StringId::POSTSCRIPT_NAME).iter().map(|n| norm(n)).collect();
    let mut full_names: Vec<String> = all_names(font, StringId::FULL_NAME).iter().map(|n| norm(n)).collect();
    for v in [&mut families, &mut styles, &mut full_names, &mut postscript] {
        sort_dedup(v);
    }

    let os2 = font.os2().ok();
    let weight = os2
        .as_ref()
        .map(|t| t.us_weight_class())
        .map(|w| if (1..10).contains(&w) { w * 100 } else { w })
        .filter(|w| (1..=1000).contains(w))
        .map_or_else(|| style_weight(&style), f32::from);
    let width = os2.as_ref().and_then(|t| width_class_percent(t.us_width_class())).unwrap_or_else(|| style_width(&style));
    let selection_italic = os2.as_ref().is_some_and(|t| {
        let f = t.fs_selection();
        f.contains(skrifa::raw::tables::os2::SelectionFlags::ITALIC) || f.contains(skrifa::raw::tables::os2::SelectionFlags::OBLIQUE)
    });
    let fs_type = os2.as_ref().map_or(0, |t| t.fs_type());
    let code_pages = os2.as_ref().map_or([0, 0], |t| [t.ul_code_page_range_1().unwrap_or(0), t.ul_code_page_range_2().unwrap_or(0)]);
    let axes = font.axes();
    let legacy_pair = (!typo_family.is_empty())
        .then(|| name(font, &[StringId::FAMILY_NAME]))
        .flatten()
        .filter(|f| *f != family)
        .map(|f| (f, name(font, &[StringId::SUBFAMILY_NAME])));

    let info = FaceInfo {
        family: family.clone(),
        style: style.clone(),
        legacy_family: legacy_pair.as_ref().map(|(f, _)| f.clone()),
        legacy_style: legacy_pair.and_then(|(_, s)| s),
        localized_families,
        full_name: name(font, &[StringId::FULL_NAME]),
        postscript_name: name(font, &[StringId::POSTSCRIPT_NAME]),
        weight,
        width,
        italic: selection_italic || style_italic(&style),
        variations: vec![],
        variable: !axes.is_empty(),
        fs_type,
        code_pages,
        path: path.map(Path::to_path_buf),
        index,
        source,
        blob: None,
        confined: false,
        file: None,
    };
    let mut keys = Keys { families, styles, legacy, full_names, postscript };
    let instances = named_instances(font, &info, &mut keys);
    std::iter::once(Face { info, keys }).chain(instances).collect()
}

/// The named instances of a variable face (its `fvar` table) as faces of its family, without the
/// default instance (the face itself) and without names already taken. The default instance's
/// name, when it isn't the face's style (a face `Regular` whose default instance is `Book`), names
/// the face too. Damaged tables give fewer instances or none.
fn named_instances(font: &skrifa::FontRef<'_>, face: &FaceInfo, keys: &mut Keys) -> Vec<Face> {
    let axes = font.axes();
    if axes.is_empty() || axes.len() > MAX_AXES {
        return vec![];
    }
    let tags: Vec<[u8; 4]> = axes.iter().map(|a| a.tag().to_be_bytes()).collect();
    let defaults: Vec<f32> = axes.iter().map(|a| a.default_value()).collect();
    // Instances without a PostScript name of their own are named `<prefix>-<style>` (Adobe
    // Technical Note #5902).
    let prefix = name(font, &[StringId::VARIATIONS_POSTSCRIPT_NAME_PREFIX]).unwrap_or_else(|| face.family.replace(' ', ""));
    let mut taken = vec![norm(&face.style)];
    let mut out = Vec::new();
    for inst in font.named_instances().iter().take(MAX_INSTANCES) {
        let coords: Vec<f32> = inst.user_coords().take(MAX_AXES).collect();
        if coords.len() != tags.len() || coords.iter().any(|c| !c.is_finite()) {
            continue;
        }
        let Some(style) = name(font, &[inst.subfamily_name_id()]) else { continue };
        let ns = norm(&style);
        if ns.is_empty() || taken.contains(&ns) {
            continue;
        }
        let ps_default = format!("{prefix}-{}", style.replace(' ', ""));
        taken.push(ns.clone());
        if coords.iter().zip(&defaults).all(|(c, d)| (c - d).abs() < 1e-3) {
            keys.styles.push(ns);
            keys.postscript.push(norm(&ps_default));
            continue;
        }
        let axis = |tag: &[u8; 4]| tags.iter().zip(&coords).find(|(t, _)| *t == tag).map(|(_, v)| *v);
        let weight = axis(b"wght").map_or_else(|| style_weight(&style), |w| w.clamp(1.0, 1000.0));
        let width = axis(b"wdth").map_or_else(|| style_width(&style), |w| w.clamp(25.0, 400.0));
        let italic = style_italic(&style) || axis(b"ital").is_some_and(|v| v >= 0.5) || axis(b"slnt").is_some_and(|v| v != 0.0);
        let own_ps = inst.postscript_name_id().and_then(|id| name(font, &[id]));
        let mut styles: Vec<String> = std::iter::once(style.clone()).chain(all_names(font, inst.subfamily_name_id())).map(|s| norm(&s)).collect();
        let mut postscript: Vec<String> =
            inst.postscript_name_id().map(|id| all_names(font, id)).unwrap_or_default().iter().map(|n| norm(n)).collect();
        postscript.push(norm(&ps_default));
        sort_dedup(&mut styles);
        sort_dedup(&mut postscript);
        let info = FaceInfo {
            style,
            legacy_family: None,
            legacy_style: None,
            full_name: None,
            postscript_name: Some(own_ps.unwrap_or(ps_default)),
            weight,
            width,
            italic,
            variations: tags.iter().copied().zip(coords.iter().copied()).collect(),
            ..face.clone()
        };
        let keys = Keys { families: keys.families.clone(), styles, legacy: vec![], full_names: vec![], postscript };
        out.push(Face { info, keys });
    }
    sort_dedup(&mut keys.styles);
    sort_dedup(&mut keys.postscript);
    out
}

/// How many faces `data` (a font file or collection) holds; 0 when it isn't one.
pub(crate) fn face_count(data: &[u8]) -> u32 {
    match FileRef::new(data) {
        Ok(FileRef::Font(_)) => 1,
        Ok(FileRef::Collection(c)) => c.len(),
        Err(_) => 0,
    }
}

/// The faces of every font in `data`, at most `max_faces` font faces read.
pub(crate) fn data_faces(data: &[u8], max_faces: u32, path: Option<&Path>, source: Source) -> Vec<Face> {
    (0..face_count(data).min(max_faces))
        .filter_map(|i| Some((i, skrifa::FontRef::from_index(data, i).ok()?)))
        .filter(|(_, f)| OUTLINE_TABLES.iter().any(|t| f.table_data(skrifa::Tag::new(t)).is_some()))
        .flat_map(|(i, f)| font_faces(&f, i, path, source))
        .collect()
}

/// The faces of the font file at `path`, reading only its table directories and `name`, `fvar`
/// and `OS/2` tables, at most `max_read` bytes in all: a scan opens thousands of files, many of
/// them megabytes long. A Mac suitcase font's `sfnt` resources are read instead.
pub(crate) fn file_faces(path: &Path, max_read: u64, source: Source) -> Vec<Face> {
    opened_file_faces(path, max_read, source, false)
}

/// [`file_faces`], opening the file with [`open_regular`]'s `no_follow`. With `no_follow`, a
/// suitcase gives no faces: its resource fork is opened through the file's path, which follows
/// a link in its place.
pub(crate) fn opened_file_faces(path: &Path, max_read: u64, source: Source, no_follow: bool) -> Vec<Face> {
    if crate::suitcase::is_suitcase(path) {
        if no_follow {
            return vec![];
        }
        if crate::suitcase::fork_len(path) > max_read {
            return vec![];
        }
        let mut faces = Vec::new();
        for (resource, font) in (0u32..).zip(crate::suitcase::read_fonts(path)) {
            for mut face in data_faces(&font, 1, Some(path), source) {
                face.info.index = resource;
                faces.push(face);
            }
        }
        return faces;
    }
    match open_regular(path, no_follow) {
        Ok(file) => reader_faces(file, MAX_FACES, max_read, path, source),
        Err(_) => vec![],
    }
}

pub(crate) fn not_a_regular_file() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a regular file")
}

/// `path` opened for reading, when it is a regular file (after following links). The check is
/// on the open file, so a FIFO or device swapped in after an earlier check is refused too; on
/// Unix the open doesn't wait for a FIFO's writer (`O_NONBLOCK`, which reads of a regular file
/// ignore). With `no_follow`, on Unix a link as the last component is refused (`O_NOFOLLOW`);
/// links in the folders above it are still followed.
pub(crate) fn open_regular(path: &Path, no_follow: bool) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NONBLOCK | if no_follow { libc::O_NOFOLLOW } else { 0 });
    #[cfg(not(unix))]
    let _ = no_follow;
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(not_a_regular_file());
    }
    Ok(file)
}

fn reader_faces(mut file: impl std::io::Read + std::io::Seek, max_faces: u32, max_read: u64, path: &Path, source: Source) -> Vec<Face> {
    use std::io::SeekFrom;
    let mut read = 0u64;
    let file_len = file.seek(SeekFrom::End(0)).unwrap_or(0);
    let mut read_at = |offset: u64, len: usize| -> Option<Vec<u8>> {
        read = read.checked_add(u64::try_from(len).ok()?).filter(|r| *r <= max_read)?;
        let mut buf = vec![0; len];
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut buf).ok()?;
        Some(buf)
    };
    let be32 = |b: &[u8], at: usize| b.get(at..at + 4).and_then(|s| s.try_into().ok()).map(u32::from_be_bytes);
    let Some(head) = read_at(0, 12) else { return vec![] };
    let starts: Vec<u32> = if head.starts_with(b"ttcf") {
        // A damaged count is capped by what the file can hold.
        let fits = u32::try_from(file_len.saturating_sub(12) / 4).unwrap_or(u32::MAX);
        let n = be32(&head, 8).unwrap_or(0).min(max_faces).min(fits) as usize;
        read_at(12, n * 4).map(|b| b.as_chunks::<4>().0.iter().map(|c| u32::from_be_bytes(*c)).collect()).unwrap_or_default()
    } else {
        vec![0]
    };
    (0u32..)
        .zip(starts)
        .filter_map(|(index, start)| {
            let dir = read_at(start.into(), 12)?;
            let tables = usize::from(u16::from_be_bytes(dir.get(4..6)?.try_into().ok()?));
            let records = read_at(u64::from(start) + 12, tables * 16)?;
            let records: Vec<&[u8]> = records.as_chunks::<16>().0.iter().map(|r| r.as_slice()).collect();
            if !records.iter().any(|r| OUTLINE_TABLES.iter().any(|t| r.starts_with(*t))) {
                return None;
            }
            let mut table = |tag: &[u8; 4]| -> Option<Vec<u8>> {
                let rec = records.iter().find(|r| r.starts_with(tag))?;
                let (offset, len) = (be32(rec, 8)?, be32(rec, 12)?);
                if len > MAX_TABLE {
                    return None;
                }
                read_at(offset.into(), len as usize)
            };
            let name = table(b"name")?;
            let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![(b"name", name)];
            for tag in [b"fvar", b"OS/2"] {
                if let Some(t) = table(tag) {
                    tables.push((tag, t));
                }
            }
            let refs: Vec<(&[u8; 4], &[u8])> = tables.iter().map(|(t, d)| (*t, d.as_slice())).collect();
            let font = sfnt_of(&refs)?;
            Some(font_faces(&skrifa::FontRef::new(&font).ok()?, index, Some(path), source))
        })
        .flatten()
        .collect()
}

/// A font file holding `tables` (tag, data), sorted by tag as table directories are.
pub(crate) fn sfnt_of(tables: &[(&[u8; 4], &[u8])]) -> Option<Vec<u8>> {
    let mut tables = tables.to_vec();
    tables.sort_by_key(|t| *t.0);
    let n = u16::try_from(tables.len()).ok()?;
    let mut font = Vec::new();
    font.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    for v in [n, 0, 0, 0] {
        font.extend_from_slice(&v.to_be_bytes());
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        font.extend_from_slice(*tag);
        for v in [0, u32::try_from(offset).ok()?, u32::try_from(data.len()).ok()?] {
            font.extend_from_slice(&v.to_be_bytes());
        }
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    Some(font)
}

/// Whether `path` has a font file's extension (`.ttf`, `.otf`, `.ttc`, `.otc`, in any case), as
/// the scan picks files in a folder.
pub fn is_font_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["ttf", "otf", "ttc", "otc"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}
