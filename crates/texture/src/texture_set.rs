//! Texture sets recognized from image file names.
//!
//! [`recognize`] takes a list of file names, such as a folder listing or the entries of an
//! archive, groups the image names into [`TextureSet`]s by their shared stem, and gives each name
//! the [`MapRole`] that its suffix or prefix names: `Brick_BaseColor.png`, `brick_nor_gl_4k.exr`
//! and `brick_ARM.png` are the base color, an OpenGL normal map and a packed occlusion,
//! roughness and metallic map of the set `Brick`. The roles are the textures of the glTF 2.0
//! metallic-roughness material, section 3.9
//! (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>), and the matching inputs of
//! MaterialX's `standard_surface`
//! (<https://github.com/AcademySoftwareFoundation/MaterialX/blob/main/libraries/bxdf/standard_surface.mtlx>),
//! plus height and glossiness.
//!
//! The module reads names only. It opens no file, joins or resolves no path and accepts names in
//! any Unicode. A name is split at its last `/` or `\`, and only the part after it, the leaf, is
//! read; the part before it is the folder, compared with `\` and `/` taken as equal. When the
//! name starts with its only separator, as in `/brick_albedo.png`, the folder is that separator.
//! [`NameLimits`] caps the number of names and the bytes of each, and the memory used grows only
//! with the names read.
//!
//! # Rules
//!
//! 1. Only a leaf whose extension is in [`IMAGE_EXTENSIONS`], in any ASCII case, is a map.
//!    Archive metadata (a `__MACOSX` folder, a leaf starting with `._`) and preview images (a
//!    word such as `preview`, `thumb` or `swatch`) are ignored, each with its [`IgnoreReason`].
//! 2. The stem (the leaf without its extension) is split into words at every character that is
//!    not a letter or digit, at a change from an ASCII lower-case to an upper-case letter
//!    (`BaseColor`), before the last letter of an upper-case run followed by a lower-case letter
//!    (`ORMMap`), and between ASCII digits and other characters (`diff4k`). A number directly
//!    followed by `k`, `px` or `bit` stays one word (`4k`). Words are compared after ASCII
//!    lower-casing; other characters are compared as they are.
//! 3. Resolution words (`1k` to `16k`, `16px` to `16384px`, a bare power of two from 128 to
//!    16384), bit depths (`8bit`, `16bit`, `32bit`), normal conventions (`gl`, `ogl`, `opengl`,
//!    `open gl`; `dx`, `directx`, `direct x`, `d3d` written without separators) and a four-digit
//!    UDIM tile from 1001 to 1999 are taken out wherever they are. A bare `1024` is a resolution,
//!    or tile 1024 when another name of its set has a tile; a name with both a bare `1024` and a
//!    tile is [`IgnoreReason::SeveralTiles`]. A normal map's convention also comes from
//!    `normalgl` and `normaldx`, in its role words and in normal words right next to them; a
//!    name that states both conventions is [`IgnoreReason::ConflictingNormalConvention`].
//! 4. The role is named by the last remaining words (three, two or one), or by the first ones when
//!    the last do not name a role. Words right next to them that name the same role, as in
//!    `diffuse_color`, go with them. The words left over are the set's stem. A different role named
//!    right next to the role words makes the name [`IgnoreReason::SeveralRoles`], except an
//!    occlusion, a roughness and a metallic word in that order, which name
//!    [`MapRole::PackedOrm`]. A single letter is never a role, except a trailing `n` (normal).
//! 5. Names with the same folder and the same stem words form one set, in the order of their
//!    first name. Two names of one set with the same role and tile are both kept and listed in
//!    [`TextureSet::duplicates`]; none of them is chosen. An identical repeated name is ignored
//!    as [`IgnoreReason::DuplicateName`].

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::normal::NormalConvention;

/// Default [`NameLimits::max_names`].
pub const DEFAULT_MAX_NAMES: usize = 4096;
/// Default [`NameLimits::max_name_bytes`].
pub const DEFAULT_MAX_NAME_BYTES: usize = 1024;

/// File extensions of the names read as images, compared without regard to ASCII case.
pub const IMAGE_EXTENSIONS: [&str; 10] = ["png", "jpg", "jpeg", "tga", "tif", "tiff", "bmp", "webp", "exr", "hdr"];

/// The map an image file name names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum MapRole {
    /// Base color, albedo or diffuse color: glTF `baseColorTexture`, MaterialX `base_color`.
    BaseColor,
    /// A tangent-space normal map: glTF `normalTexture`, MaterialX `normal`.
    Normal,
    /// Roughness: the green channel of glTF `metallicRoughnessTexture`, MaterialX
    /// `specular_roughness`.
    Roughness,
    /// Metallic: the blue channel of glTF `metallicRoughnessTexture`, MaterialX `metalness`.
    Metallic,
    /// Ambient occlusion: glTF `occlusionTexture`, which is read from red.
    AmbientOcclusion,
    /// Height or displacement.
    Height,
    /// Emissive color: glTF `emissiveTexture`, MaterialX `emission_color`.
    Emissive,
    /// Opacity or alpha: MaterialX `opacity`.
    Opacity,
    /// Glossiness or smoothness, the inverse of roughness. The name is labeled only; no value is
    /// inverted.
    Glossiness,
    /// One image with occlusion in red, roughness in green and metallic in blue (`ORM`, `ARM`).
    PackedOrm,
}

/// Caps on the names [`recognize`] reads. `#[non_exhaustive]`: start from
/// [`NameLimits::default`] and set the fields you change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct NameLimits {
    /// Most names in one call. Default [`DEFAULT_MAX_NAMES`]. To find a longer list, [`recognize`]
    /// takes one name past the limit from the iterator and does not read it.
    pub max_names: usize,
    /// Most UTF-8 bytes in one name, folder included. Default [`DEFAULT_MAX_NAME_BYTES`].
    pub max_name_bytes: usize,
}

impl Default for NameLimits {
    fn default() -> Self {
        Self { max_names: DEFAULT_MAX_NAMES, max_name_bytes: DEFAULT_MAX_NAME_BYTES }
    }
}

/// Why [`recognize`] read no list.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum NameError {
    /// The list holds more than [`NameLimits::max_names`] names. The first name past the limit is
    /// taken from the iterator but not read, and no later name is taken.
    #[error("the list holds more than {limit} names")]
    TooManyNames { limit: usize },
    /// The name at `index` is longer than [`NameLimits::max_name_bytes`].
    #[error("name {index} is {bytes} bytes long, over the limit of {limit}")]
    NameTooLong { index: usize, bytes: usize, limit: usize },
}

/// Why a name is in no texture set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IgnoreReason {
    /// The same name came earlier in the list.
    DuplicateName,
    /// The name ends with a separator, or its leaf has no letter or digit before the extension.
    NoFileName,
    /// The leaf has no extension, or one that is not in [`IMAGE_EXTENSIONS`].
    NotAnImage,
    /// A `__MACOSX` folder or a leaf starting with `._`, as archivers write them.
    ArchiveMetadata,
    /// A preview, thumbnail or swatch image.
    Preview,
    /// No role word at the end or the start of the stem.
    UnknownMap,
    /// A known map that has no [`MapRole`], such as cavity or specular.
    UnsupportedMap,
    /// Two different roles named next to each other, such as `albedo_normal`.
    SeveralRoles,
    /// Two different UDIM tiles in one name.
    SeveralTiles,
    /// A normal map named both OpenGL and DirectX.
    ConflictingNormalConvention,
}

/// A name that is in no texture set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredName<'a> {
    /// Position of the name in the list.
    pub index: usize,
    /// The name as given.
    pub name: &'a str,
    pub reason: IgnoreReason,
}

/// A name of a texture set and the map it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapName<'a> {
    /// Position of the name in the list.
    pub index: usize,
    /// The name as given.
    pub name: &'a str,
    pub role: MapRole,
    /// For a normal map, the convention its name states; `None` when it states none, and for
    /// every other role.
    pub convention: Option<NormalConvention>,
    /// The UDIM tile its name states.
    pub tile: Option<u16>,
}

/// Maps of one set that name the same role and tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateRole {
    pub role: MapRole,
    pub tile: Option<u16>,
    /// Positions in [`TextureSet::maps`], in list order. There are at least two.
    pub maps: Vec<usize>,
}

/// The maps whose names share a folder and a stem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureSet<'a> {
    /// The folder part of the set's first name, without its last separator; empty for a bare
    /// name, and the separator alone for a name such as `/brick_albedo.png`.
    pub folder: &'a str,
    /// The stem words of the set's first name, as written there: without its role words and the
    /// words that rule 3 takes out, so `brick_4k_wall_albedo.png` gives `brick_wall`. Empty for a
    /// name such as `normal.png`. Names whose stem words differ only in ASCII case or in the
    /// separators between them are in the same set.
    pub stem: String,
    /// The maps in list order.
    pub maps: Vec<MapName<'a>>,
    /// Every role and tile that more than one map names, in the order of their first map.
    pub duplicates: Vec<DuplicateRole>,
}

impl<'a> TextureSet<'a> {
    /// The maps that name `role`, in list order.
    pub fn maps_for(&self, role: MapRole) -> impl Iterator<Item = &MapName<'a>> + '_ {
        self.maps.iter().filter(move |map| map.role == role)
    }
}

/// The texture sets of a list of names and the names that are in none.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Recognition<'a> {
    /// The sets in the order of their first name.
    pub sets: Vec<TextureSet<'a>>,
    /// The names that are in no set, in list order.
    pub ignored: Vec<IgnoredName<'a>>,
}

/// Groups the image file names of `names` into texture sets and names each one's map role. Reads
/// the names one at a time and stops at the first that breaks `limits`.
///
/// Returns [`NameError::TooManyNames`] when `names` has more than [`NameLimits::max_names`]
/// entries; to find that out it takes one entry past the limit from the iterator, which a lazy
/// iterator has to produce, and does not read it. Returns [`NameError::NameTooLong`] for the first
/// name over
/// [`NameLimits::max_name_bytes`]. An empty list gives an empty [`Recognition`].
pub fn recognize<'a>(names: impl IntoIterator<Item = &'a str>, limits: &NameLimits) -> Result<Recognition<'a>, NameError> {
    let mut seen: HashSet<&'a str> = HashSet::new();
    let mut groups: HashMap<(String, String), usize> = HashMap::new();
    let mut sets: Vec<SetBuilder<'a>> = Vec::new();
    let mut ignored = Vec::new();
    for (index, name) in names.into_iter().enumerate() {
        if index >= limits.max_names {
            return Err(NameError::TooManyNames { limit: limits.max_names });
        }
        if name.len() > limits.max_name_bytes {
            return Err(NameError::NameTooLong { index, bytes: name.len(), limit: limits.max_name_bytes });
        }
        if !seen.insert(name) {
            ignored.push(IgnoredName { index, name, reason: IgnoreReason::DuplicateName });
            continue;
        }
        match classify(name) {
            Err(reason) => ignored.push(IgnoredName { index, name, reason }),
            Ok(parsed) => {
                let key = (parsed.folder.replace('\\', "/"), parsed.key.clone());
                let at = *groups.entry(key).or_insert_with(|| {
                    sets.push(SetBuilder::new(parsed.folder, parsed.stem.clone()));
                    sets.len() - 1
                });
                if let Some(set) = sets.get_mut(at) {
                    set.push(index, name, &parsed);
                }
            }
        }
    }
    Ok(Recognition { sets: sets.into_iter().map(SetBuilder::finish).collect(), ignored })
}

/// A set being built, with a flag per map for a bare `1024` read as a resolution.
struct SetBuilder<'a> {
    set: TextureSet<'a>,
    bare_1024: Vec<bool>,
}

impl<'a> SetBuilder<'a> {
    fn new(folder: &'a str, stem: String) -> Self {
        Self { set: TextureSet { folder, stem, maps: Vec::new(), duplicates: Vec::new() }, bare_1024: Vec::new() }
    }

    fn push(&mut self, index: usize, name: &'a str, parsed: &Parsed<'a>) {
        self.set.maps.push(MapName { index, name, role: parsed.role, convention: parsed.convention, tile: parsed.tile });
        self.bare_1024.push(parsed.bare_1024);
    }

    fn finish(mut self) -> TextureSet<'a> {
        if self.set.maps.iter().any(|map| map.tile.is_some()) {
            for (map, bare) in self.set.maps.iter_mut().zip(&self.bare_1024) {
                if *bare && map.tile.is_none() {
                    map.tile = Some(1024);
                }
            }
        }
        self.set.duplicates = duplicates(&self.set.maps);
        self.set
    }
}

/// Every role and tile that more than one of `maps` names.
fn duplicates(maps: &[MapName<'_>]) -> Vec<DuplicateRole> {
    let mut slots: HashMap<(MapRole, Option<u16>), usize> = HashMap::new();
    let mut found: Vec<DuplicateRole> = Vec::new();
    for (position, map) in maps.iter().enumerate() {
        let at = *slots.entry((map.role, map.tile)).or_insert_with(|| {
            found.push(DuplicateRole { role: map.role, tile: map.tile, maps: Vec::new() });
            found.len() - 1
        });
        if let Some(slot) = found.get_mut(at) {
            slot.maps.push(position);
        }
    }
    found.retain(|slot| slot.maps.len() > 1);
    found
}

/// What one name says.
struct Parsed<'a> {
    folder: &'a str,
    stem: String,
    /// The stem words joined by `_`, the set's grouping key.
    key: String,
    role: MapRole,
    convention: Option<NormalConvention>,
    tile: Option<u16>,
    bare_1024: bool,
}

/// A role word, or a known map that has no [`MapRole`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Word {
    Map(MapRole),
    Unsupported,
}

/// Every single role word. The README's table lists the same words.
const WORDS: &[(Word, &[&str])] = &[
    (Word::Map(MapRole::BaseColor), &["basecolor", "basecolour", "albedo", "diffuse", "diff", "color", "colour", "col", "bc", "basemap", "maintex"]),
    (Word::Map(MapRole::Normal), &["normal", "normalmap", "normalgl", "normaldx", "nrm", "nor", "norm", "nm", "n", "bumpmap"]),
    (Word::Map(MapRole::Roughness), &["roughness", "rough", "rgh"]),
    (Word::Map(MapRole::Metallic), &["metallic", "metalness", "metal", "met", "mtl"]),
    (Word::Map(MapRole::AmbientOcclusion), &["ambientocclusion", "ao", "occlusion", "occ", "ambocc"]),
    (Word::Map(MapRole::Height), &["height", "heightmap", "displacement", "disp", "displace", "hgt", "bump"]),
    (Word::Map(MapRole::Emissive), &["emissive", "emission", "emissivecolor", "emissionmap", "emit", "glow"]),
    (Word::Map(MapRole::Opacity), &["opacity", "alpha", "transparency", "cutout"]),
    (Word::Map(MapRole::Glossiness), &["glossiness", "gloss", "smoothness", "smooth"]),
    (Word::Map(MapRole::PackedOrm), &["orm", "arm", "aorm"]),
    (Word::Unsupported, &["cavity", "curvature", "specular", "thickness", "sheen", "ior"]),
];

/// Every two-word role. The README's table lists the same pairs.
const PAIRS: &[(MapRole, &[[&str; 2]])] = &[
    (MapRole::BaseColor, &[["base", "color"], ["base", "colour"], ["base", "map"], ["main", "tex"]]),
    (MapRole::Normal, &[["normal", "map"], ["bump", "map"]]),
    (MapRole::AmbientOcclusion, &[["ambient", "occlusion"]]),
    (MapRole::Height, &[["height", "map"]]),
    (MapRole::Emissive, &[["emissive", "color"], ["emissive", "colour"], ["emission", "map"]]),
];

/// Words that mark a preview image.
const PREVIEW_WORDS: [&str; 10] = ["preview", "previews", "thumb", "thumbs", "thumbnail", "sphere", "render", "swatch", "icon", "screenshot"];

fn word(text: &str) -> Option<Word> {
    WORDS.iter().find(|(_, words)| words.contains(&text)).map(|(word, _)| *word)
}

fn pair(first: &str, second: &str) -> Option<MapRole> {
    PAIRS.iter().find(|(_, pairs)| pairs.iter().any(|[a, b]| *a == first && *b == second)).map(|(role, _)| *role)
}

/// The role that the words of `span` name together.
fn span_word(words: &[&str], span: Range<usize>) -> Option<Word> {
    match words.get(span)? {
        [a, b, c] => {
            let packed = word(a) == Some(Word::Map(MapRole::AmbientOcclusion))
                && word(b) == Some(Word::Map(MapRole::Roughness))
                && word(c) == Some(Word::Map(MapRole::Metallic));
            packed.then_some(Word::Map(MapRole::PackedOrm))
        }
        [a, b] => pair(a, b).map(Word::Map),
        [a] => word(a),
        _ => None,
    }
}

/// The longest role span of one to three words at the end of `words`, or at the start when
/// `at_end` is false. A lone `n` counts only when `lone_n` is true.
fn edge_span(words: &[&str], at_end: bool, lone_n: bool) -> Option<(Word, Range<usize>)> {
    let n = words.len();
    (1..=n.min(3)).rev().find_map(|k| {
        let span = if at_end { n - k..n } else { 0..k };
        if k == 1 && !lone_n && words.get(span.start) == Some(&"n") {
            return None;
        }
        span_word(words, span.clone()).map(|w| (w, span))
    })
}

/// The role of `words` and the positions of its role words, together with the words right next to
/// them that name the same role, or why the words name none.
fn find_role(words: &[&str]) -> Result<(MapRole, Range<usize>), IgnoreReason> {
    let (found, mut span) = edge_span(words, true, true).or_else(|| edge_span(words, false, false)).ok_or(IgnoreReason::UnknownMap)?;
    let at_end = span.end == words.len();
    // The words on the stem side of the role words, and a role they name right next to them.
    loop {
        let side = if at_end { words.get(..span.start) } else { words.get(span.end..) }.unwrap_or_default();
        match edge_span(side, at_end, false) {
            Some((w, _)) if w != found => return Err(IgnoreReason::SeveralRoles),
            Some((_, next)) if at_end => span.start = next.start,
            Some((_, next)) => span.end += next.len(),
            None => break,
        }
    }
    match found {
        Word::Map(role) => Ok((role, span)),
        Word::Unsupported => Err(IgnoreReason::UnsupportedMap),
    }
}

/// What a name says, or why it is in no set.
fn classify(name: &str) -> Result<Parsed<'_>, IgnoreReason> {
    let (folder, leaf) = match name.rfind(['/', '\\']) {
        // A name under the root keeps its separator as the folder.
        Some(0) => name.split_at(1),
        Some(at) => (name.get(..at).unwrap_or_default(), name.get(at + 1..).unwrap_or_default()),
        None => ("", name),
    };
    if name.split(['/', '\\']).any(|part| part == "__MACOSX") || leaf.starts_with("._") {
        return Err(IgnoreReason::ArchiveMetadata);
    }
    if leaf.is_empty() {
        return Err(IgnoreReason::NoFileName);
    }
    let (stem, extension) = leaf.rsplit_once('.').ok_or(IgnoreReason::NotAnImage)?;
    if !IMAGE_EXTENSIONS.iter().any(|known| known.eq_ignore_ascii_case(extension)) {
        return Err(IgnoreReason::NotAnImage);
    }
    let tokens = tokens(stem);
    if tokens.is_empty() {
        return Err(IgnoreReason::NoFileName);
    }
    if tokens.iter().any(|t| PREVIEW_WORDS.contains(&t.text.as_str())) {
        return Err(IgnoreReason::Preview);
    }
    let extras = Extras::split(&tokens);
    let words: Vec<&str> = extras.kept.iter().filter_map(|&i| tokens.get(i)).map(|t| t.text.as_str()).collect();
    let (role, span) = find_role(&words)?;
    let rest = if span.start == 0 { extras.kept.get(span.end..) } else { extras.kept.get(..span.start) }.unwrap_or_default();
    let stem_text = stem_without_extras(stem, &tokens, rest);
    let key = rest.iter().filter_map(|&i| tokens.get(i)).map(|t| t.text.as_str()).collect::<Vec<_>>().join("_");
    // The conventions that the role words with one glued on state.
    let glued: Vec<NormalConvention> = words
        .get(span)
        .unwrap_or_default()
        .iter()
        .filter_map(|w| match *w {
            "normalgl" => Some(NormalConvention::OpenGl),
            "normaldx" => Some(NormalConvention::DirectX),
            _ => None,
        })
        .collect();
    let convention = if role == MapRole::Normal { extras.convention(&glued)? } else { None };
    let tile = extras.tile()?;
    // A bare 1024 next to a tile is a second tile.
    if extras.bare_1024 && tile.is_some() {
        return Err(IgnoreReason::SeveralTiles);
    }
    Ok(Parsed { folder, stem: stem_text, key, role, convention, tile, bare_1024: extras.bare_1024 })
}

/// The words of `stem` at the token positions `rest` (ascending) as written, each joined to the
/// one before by the text between them. Where extra words were taken out between two of them,
/// the join is the text before the first extra word, so `brick_4k_wall` gives `brick_wall`.
fn stem_without_extras(stem: &str, tokens: &[Token], rest: &[usize]) -> String {
    let mut out = String::new();
    let mut previous: Option<usize> = None;
    for &i in rest {
        let Some(token) = tokens.get(i) else { continue };
        if let Some(p) = previous
            && let (Some(before), Some(next)) = (tokens.get(p), tokens.get(p + 1))
        {
            out.push_str(stem.get(before.end..next.start).unwrap_or_default());
        }
        out.push_str(stem.get(token.start..token.end).unwrap_or_default());
        previous = Some(i);
    }
    out
}

/// A word of a stem: its byte range in the stem and its text, ASCII lower-cased.
struct Token {
    start: usize,
    end: usize,
    text: String,
}

/// Whether a word ends between `previous` and `c`, given the character after `c`.
fn splits(previous: char, c: char, next: Option<char>) -> bool {
    (previous.is_ascii_lowercase() && c.is_ascii_uppercase())
        || (previous.is_ascii_uppercase() && c.is_ascii_uppercase() && next.is_some_and(|n| n.is_ascii_lowercase()))
        || previous.is_ascii_digit() != c.is_ascii_digit()
}

/// The words of `stem`.
fn tokens(stem: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = stem.char_indices().collect();
    let mut out: Vec<Token> = Vec::new();
    let mut current: Option<Token> = None;
    let mut previous: Option<char> = None;
    for (n, &(i, c)) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            out.extend(current.take());
            previous = None;
            continue;
        }
        let next = chars.get(n + 1).map(|&(_, next)| next);
        if previous.is_some_and(|p| splits(p, c, next)) {
            out.extend(current.take());
        }
        let token = current.get_or_insert_with(|| Token { start: i, end: i, text: String::new() });
        token.text.push(c.to_ascii_lowercase());
        token.end = i + c.len_utf8();
        previous = Some(c);
    }
    out.extend(current);
    join_units(out)
}

/// `tokens` with each number joined to a `k`, `px` or `bit` written right after it.
fn join_units(tokens: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    for token in tokens {
        if let Some(last) = out.last_mut()
            && last.end == token.start
            && last.text.bytes().all(|b| b.is_ascii_digit())
            && matches!(token.text.as_str(), "k" | "px" | "bit")
        {
            last.text.push_str(&token.text);
            last.end = token.end;
            continue;
        }
        out.push(token);
    }
    out
}

/// A word that is taken out of the stem.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Extra {
    Size,
    Convention(NormalConvention),
    Tile(u16),
    Bare1024,
}

/// A number of at most five ASCII digits.
fn number(text: &str) -> Option<u32> {
    if text.is_empty() || text.len() > 5 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn single_extra(text: &str) -> Option<Extra> {
    match text {
        "gl" | "ogl" | "opengl" => return Some(Extra::Convention(NormalConvention::OpenGl)),
        "dx" | "directx" => return Some(Extra::Convention(NormalConvention::DirectX)),
        "8bit" | "16bit" | "32bit" => return Some(Extra::Size),
        _ => {}
    }
    if let Some(n) = text.strip_suffix('k') {
        return number(n).filter(|n| (1..=16).contains(n)).map(|_| Extra::Size);
    }
    if let Some(n) = text.strip_suffix("px") {
        return number(n).filter(|n| (16..=16384).contains(n)).map(|_| Extra::Size);
    }
    let n = number(text)?;
    if text.len() == 4 && n == 1024 {
        Some(Extra::Bare1024)
    } else if text.len() == 4 && (1001..=1999).contains(&n) {
        u16::try_from(n).ok().map(Extra::Tile)
    } else {
        ((128..=16384).contains(&n) && n.is_power_of_two()).then_some(Extra::Size)
    }
}

/// The extra word starting at `tokens[i]` and how many tokens it spans.
fn extra_at(tokens: &[Token], i: usize) -> Option<(Extra, usize)> {
    let text = |k: usize| tokens.get(i + k).map(|t| t.text.as_str());
    // Whether tokens[i + k] ends where the next token starts.
    let touches = |k: usize| matches!((tokens.get(i + k), tokens.get(i + k + 1)), (Some(a), Some(b)) if a.end == b.start);
    match (text(0)?, text(1), text(2)) {
        ("open", Some("gl"), _) => Some((Extra::Convention(NormalConvention::OpenGl), 2)),
        ("direct", Some("x"), _) => Some((Extra::Convention(NormalConvention::DirectX), 2)),
        ("d", Some("3"), Some("d")) if touches(0) && touches(1) => Some((Extra::Convention(NormalConvention::DirectX), 3)),
        (first, _, _) => single_extra(first).map(|extra| (extra, 1)),
    }
}

/// The extra words of a stem and the positions of the other words.
struct Extras {
    kept: Vec<usize>,
    found: Vec<Extra>,
    bare_1024: bool,
}

impl Extras {
    fn split(tokens: &[Token]) -> Self {
        let mut extras = Extras { kept: Vec::new(), found: Vec::new(), bare_1024: false };
        let mut i = 0;
        while i < tokens.len() {
            match extra_at(tokens, i) {
                Some((Extra::Bare1024, len)) => {
                    extras.bare_1024 = true;
                    i += len;
                }
                Some((extra, len)) => {
                    extras.found.push(extra);
                    i += len;
                }
                None => {
                    extras.kept.push(i);
                    i += 1;
                }
            }
        }
        extras
    }

    /// The normal convention the words and `glued` state, or an error when they state both.
    fn convention(&self, glued: &[NormalConvention]) -> Result<Option<NormalConvention>, IgnoreReason> {
        let stated = self.found.iter().filter_map(|extra| match extra {
            Extra::Convention(c) => Some(*c),
            _ => None,
        });
        let mut result = None;
        for c in glued.iter().copied().chain(stated) {
            match result {
                Some(r) if r != c => return Err(IgnoreReason::ConflictingNormalConvention),
                _ => result = Some(c),
            }
        }
        Ok(result)
    }

    /// The UDIM tile the words state, or an error when they state two.
    fn tile(&self) -> Result<Option<u16>, IgnoreReason> {
        let mut result = None;
        for extra in &self.found {
            if let Extra::Tile(t) = extra {
                match result {
                    Some(r) if r != *t => return Err(IgnoreReason::SeveralTiles),
                    _ => result = Some(*t),
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<'a>(names: &[&'a str]) -> Recognition<'a> {
        match recognize(names.iter().copied(), &NameLimits::default()) {
            Ok(found) => found,
            Err(e) => panic!("recognition failed: {e}"),
        }
    }

    /// The role and stem of one name, or why it is ignored.
    fn one(name: &str) -> Result<(MapRole, String), IgnoreReason> {
        let found = run(&[name]);
        match (found.sets.first(), found.ignored.first()) {
            (Some(set), None) => match set.maps.as_slice() {
                [map] => Ok((map.role, set.stem.to_string())),
                maps => panic!("{name}: {} maps", maps.len()),
            },
            (None, Some(ignored)) => Err(ignored.reason),
            _ => panic!("{name}: {found:?}"),
        }
    }

    fn role_named(label: &str) -> Option<Word> {
        Some(match label {
            "`BaseColor`" => Word::Map(MapRole::BaseColor),
            "`Normal`" => Word::Map(MapRole::Normal),
            "`Roughness`" => Word::Map(MapRole::Roughness),
            "`Metallic`" => Word::Map(MapRole::Metallic),
            "`AmbientOcclusion`" => Word::Map(MapRole::AmbientOcclusion),
            "`Height`" => Word::Map(MapRole::Height),
            "`Emissive`" => Word::Map(MapRole::Emissive),
            "`Opacity`" => Word::Map(MapRole::Opacity),
            "`Glossiness`" => Word::Map(MapRole::Glossiness),
            "`PackedOrm`" => Word::Map(MapRole::PackedOrm),
            "`UnsupportedMap` (ignored)" => Word::Unsupported,
            _ => return None,
        })
    }

    fn backticked(cell: &str) -> Vec<String> {
        cell.split('`').skip(1).step_by(2).map(str::to_string).collect()
    }

    /// The README's word table: each role with its words and its pairs.
    fn readme_table() -> Vec<(Word, Vec<String>, Vec<String>)> {
        let readme = include_str!("../README.md");
        let section = readme.split("## Texture-set names").nth(1).unwrap_or_default();
        let section = section.split("\n## ").next().unwrap_or_default();
        let mut rows = Vec::new();
        for line in section.lines() {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            if let [_, label, words, pairs, _] = cells.as_slice()
                && let Some(role) = role_named(label)
            {
                rows.push((role, backticked(words), backticked(pairs)));
            }
        }
        rows
    }

    fn expect(word: Word) -> Result<MapRole, IgnoreReason> {
        match word {
            Word::Map(role) => Ok(role),
            Word::Unsupported => Err(IgnoreReason::UnsupportedMap),
        }
    }

    #[test]
    fn readme_table_lists_every_word_and_its_role() {
        let table = readme_table();
        assert_eq!(table.len(), WORDS.len(), "README rows: {table:?}");
        for ((role, words, pairs), (code_role, code_words)) in table.iter().zip(WORDS) {
            assert_eq!(role, code_role);
            let code_words: Vec<String> = code_words.iter().map(|w| w.to_string()).collect();
            assert_eq!(words, &code_words, "{role:?}");
            let code_pairs: Vec<String> =
                PAIRS.iter().filter(|(r, _)| Word::Map(*r) == *role).flat_map(|(_, pairs)| pairs.iter().map(|[a, b]| format!("{a} {b}"))).collect();
            assert_eq!(pairs, &code_pairs, "{role:?}");
            for word in words.iter().chain(pairs) {
                let name = format!("brick_{}.png", word.replace(' ', "_"));
                assert_eq!(one(&name), expect(*role).map(|r| (r, "brick".to_string())), "{name}");
            }
        }
    }

    /// `word` (one or more parts) in `case`, joined by `separator` or by case changes.
    fn spell(parts: &[&str], case: usize, separator: &str) -> String {
        let spelled: Vec<String> = parts
            .iter()
            .map(|p| match case {
                0 => p.to_string(),
                1 => p.to_ascii_uppercase(),
                _ => p.get(..1).unwrap_or_default().to_ascii_uppercase() + p.get(1..).unwrap_or_default(),
            })
            .collect();
        spelled.join(separator)
    }

    #[test]
    fn every_word_in_any_case_separator_position_and_suffix() {
        let mut spellings: Vec<(Vec<&str>, Word)> = Vec::new();
        for (word, words) in WORDS {
            spellings.extend(words.iter().map(|w| (vec![*w], *word)));
        }
        for (role, pairs) in PAIRS {
            spellings.extend(pairs.iter().map(|p| (p.to_vec(), Word::Map(*role))));
        }
        spellings.push((vec!["ao", "rough", "metal"], Word::Map(MapRole::PackedOrm)));
        spellings.push((vec!["occlusion", "roughness", "metallic"], Word::Map(MapRole::PackedOrm)));
        let stems = ["brick", "Old Wall", "rock2", "ziegel_ß", "砖墙", "écorce"];
        let extras = ["", "_4k", "_2048", "-16bit", ".1001", "_512px"];
        let mut checked = 0;
        for (parts, word) in &spellings {
            for stem in stems {
                for case in 0..3 {
                    for separator in ["_", "-", " ", "."] {
                        for extra in extras {
                            let spelled = spell(parts, case, separator);
                            let expected = expect(*word).map(|r| (r, stem.to_string()));
                            let suffix = format!("{stem}{separator}{spelled}{extra}.PNG");
                            assert_eq!(one(&suffix), expected, "{suffix}");
                            if parts.as_slice() != ["n"] {
                                let prefix = format!("{spelled}{separator}{stem}{extra}.png");
                                assert_eq!(one(&prefix), expected, "{prefix}");
                            }
                            checked += 1;
                        }
                    }
                    // Written right after an ASCII stem that ends in lower case, a word in title or
                    // upper case starts at its case change.
                    if case > 0 && stem.is_ascii() && stem.ends_with(|c: char| c.is_ascii_lowercase()) {
                        let glued = format!("{stem}{}.tga", spell(parts, 2, ""));
                        assert_eq!(one(&glued), expect(*word).map(|r| (r, stem.to_string())), "{glued}");
                    }
                }
            }
        }
        assert!(checked > 10_000, "{checked} names checked");
    }

    #[test]
    fn a_role_at_the_start_and_role_words_inside_the_stem() {
        assert_eq!(one("albedo_brick.png"), Ok((MapRole::BaseColor, "brick".to_string())));
        assert_eq!(one("metal_plate_roughness.png"), Ok((MapRole::Roughness, "metal_plate".to_string())));
        assert_eq!(one("T_Brick_BC_4k.tga"), Ok((MapRole::BaseColor, "T_Brick".to_string())));
        assert_eq!(one("n_brick.png"), Err(IgnoreReason::UnknownMap));
        assert_eq!(one("brick_n.png"), Ok((MapRole::Normal, "brick".to_string())));
        for letter in ["r", "m", "d", "h"] {
            assert_eq!(one(&format!("brick_{letter}.png")), Err(IgnoreReason::UnknownMap), "{letter}");
        }
    }

    #[test]
    fn normal_conventions_come_from_the_name() {
        let convention = |name: &str| run(&[name]).sets.first().and_then(|s| s.maps.first()).and_then(|m| m.convention);
        for name in ["brick_normal_gl.png", "brick_NormalGL.png", "brick_nor_opengl_4k.exr", "brick_normal-open-gl.png", "brick_normalgl.png"] {
            assert_eq!(convention(name), Some(NormalConvention::OpenGl), "{name}");
        }
        for name in ["brick_normal_dx.png", "Brick_NormalDX.png", "brick_nrm_directx.png", "brick_normal_d3d.png", "brick_normal_direct_x.png"] {
            assert_eq!(convention(name), Some(NormalConvention::DirectX), "{name}");
        }
        assert_eq!(convention("brick_normal.png"), None);
        assert_eq!(convention("brick_albedo_dx.png"), None, "only a normal map has a convention");
        assert_eq!(one("brick_normal_gl_dx.png"), Err(IgnoreReason::ConflictingNormalConvention));
        assert_eq!(one("brick_NormalGL_dx.png"), Err(IgnoreReason::ConflictingNormalConvention));
        let found = run(&["brick_albedo_dx.png", "brick_normal_dx.png", "brick_normal_gl.png"]);
        assert_eq!(found.sets.len(), 1, "convention words are not part of the stem");
        assert!(found.sets.iter().all(|s| s.duplicates.iter().any(|d| d.role == MapRole::Normal && d.maps == [1, 2])));
    }

    #[test]
    fn a_normal_word_next_to_the_role_words_states_its_convention() {
        let convention = |name: &str| run(&[name]).sets.first().and_then(|s| s.maps.first()).and_then(|m| m.convention);
        assert_eq!(one("brick_normalgl_normaldx.png"), Err(IgnoreReason::ConflictingNormalConvention));
        assert_eq!(one("normalgl_normaldx_brick.png"), Err(IgnoreReason::ConflictingNormalConvention));
        assert_eq!(convention("brick_normaldx_normal.png"), Some(NormalConvention::DirectX));
        assert_eq!(convention("brick_normalgl_normal_gl.png"), Some(NormalConvention::OpenGl));
        assert_eq!(convention("brick_normalgl_normalgl_4k.png"), Some(NormalConvention::OpenGl));
        // The words next to the role words that name the same role are not in the stem.
        let found = run(&[
            "brick_normal.png",
            "brick_normal_normaldx.png",
            "NormalGL_Normal_brick.png",
            "brick_diffuse_color.png",
            "brick_diffuse_color_albedo_bc.png",
            "albedo_col_diffuse_brick.png",
        ]);
        let summary: Vec<(&str, Vec<usize>)> = found.sets.iter().map(|s| (s.stem.as_str(), s.maps.iter().map(|m| m.index).collect())).collect();
        assert_eq!(summary, [("brick", vec![0, 1, 2, 3, 4, 5])], "{:?}", found.ignored);
        assert_eq!(one("brick_normal_nrm_normaldx_normalgl.png"), Err(IgnoreReason::ConflictingNormalConvention));
        assert_eq!(one("brick_normal_diffuse_color_albedo.png"), Err(IgnoreReason::SeveralRoles));
    }

    #[test]
    fn d3d_is_read_only_when_its_letters_touch() {
        let found = run(&["panel_d_3_d_normal_gl.png"]);
        let maps: Vec<(&str, MapRole, Option<NormalConvention>)> =
            found.sets.iter().flat_map(|s| s.maps.iter().map(|m| (s.stem.as_str(), m.role, m.convention))).collect();
        assert_eq!(maps, [("panel_d_3_d", MapRole::Normal, Some(NormalConvention::OpenGl))], "{:?}", found.ignored);
        assert_eq!(one("panel_D3D_normal.png"), Ok((MapRole::Normal, "panel".to_string())));
    }

    #[test]
    fn several_sets_in_one_folder() {
        let found = run(&[
            "textures/brick_albedo.png",
            "textures/wood_BaseColor.png",
            "textures/Brick-Normal.png",
            "textures\\wood_normal_4k.png",
            "other/brick_albedo.png",
            "textures/brick_4k_roughness.png",
        ]);
        let summary: Vec<(&str, &str, Vec<usize>)> =
            found.sets.iter().map(|s| (s.folder, s.stem.as_str(), s.maps.iter().map(|m| m.index).collect())).collect();
        assert_eq!(summary, [("textures", "brick", vec![0, 2, 5]), ("textures", "wood", vec![1, 3]), ("other", "brick", vec![4])]);
        assert!(found.ignored.is_empty());
        assert!(found.sets.iter().all(|s| s.duplicates.is_empty()));
    }

    #[test]
    fn size_and_convention_words_are_not_in_the_stem() {
        let found = run(&["brick_4k_wall_albedo.png", "Brick4kWall_Normal_GL.png", "brick-d3d-wall_height.png", "brick_wall_2048_roughness.png"]);
        let stems: Vec<(&str, usize)> = found.sets.iter().map(|s| (s.stem.as_str(), s.maps.len())).collect();
        assert_eq!(stems, [("brick_wall", 4)]);
        let stem = |name: &str| one(name).map(|(_, stem)| stem);
        assert_eq!(stem("Brick4kWall_Normal_GL.png"), Ok("BrickWall".to_string()));
        assert_eq!(stem("brick-open-gl-wall_normal.png"), Ok("brick-wall".to_string()));
        assert_eq!(stem("4k_brick_albedo.png"), Ok("brick".to_string()));
    }

    #[test]
    fn a_duplicate_role_is_reported_not_chosen() {
        let found = run(&["brick_albedo.png", "brick_normal.png", "brick_diffuse_4k.jpg", "brick_albedo.png", "brick_col.tga"]);
        let [set] = found.sets.as_slice() else { panic!("{found:?}") };
        assert_eq!(set.maps_for(MapRole::BaseColor).map(|m| m.index).collect::<Vec<_>>(), [0, 2, 4]);
        assert_eq!(set.duplicates, [DuplicateRole { role: MapRole::BaseColor, tile: None, maps: vec![0, 2, 3] }]);
        assert_eq!(found.ignored, [IgnoredName { index: 3, name: "brick_albedo.png", reason: IgnoreReason::DuplicateName }]);
    }

    #[test]
    fn unknown_unsupported_and_other_names_are_ignored_with_a_reason() {
        let cases = [
            ("brick_wood.png", IgnoreReason::UnknownMap),
            ("brick_normal_v2.png", IgnoreReason::UnknownMap),
            ("4k.png", IgnoreReason::UnknownMap),
            ("brick_cavity.png", IgnoreReason::UnsupportedMap),
            ("brick_albedo_normal.png", IgnoreReason::SeveralRoles),
            ("brick_rough_metal.png", IgnoreReason::SeveralRoles),
            ("brick_normal_map_roughness.png", IgnoreReason::SeveralRoles),
            ("roughness_normal_map_brick.png", IgnoreReason::SeveralRoles),
            ("brick_ao_rough_metal_albedo.png", IgnoreReason::SeveralRoles),
            ("brick_albedo", IgnoreReason::NotAnImage),
            ("brick_albedo.psd", IgnoreReason::NotAnImage),
            ("readme.txt", IgnoreReason::NotAnImage),
            ("brick_albedo_preview.png", IgnoreReason::Preview),
            ("Thumbnail_brick_albedo.jpg", IgnoreReason::Preview),
            ("__MACOSX/brick/brick_albedo.png", IgnoreReason::ArchiveMetadata),
            ("brick/._brick_albedo.png", IgnoreReason::ArchiveMetadata),
        ];
        for (name, reason) in cases {
            assert_eq!(one(name), Err(reason), "{name}");
        }
    }

    #[test]
    fn only_a_leaf_starting_with_dot_underscore_is_archive_metadata() {
        assert_eq!(one("assets/._materials/brick_albedo.png"), Ok((MapRole::BaseColor, "brick".to_string())));
        assert_eq!(one("assets\\._materials\\._brick_albedo.png"), Err(IgnoreReason::ArchiveMetadata));
        assert_eq!(one("a/__MACOSX/b/brick_albedo.png"), Err(IgnoreReason::ArchiveMetadata));
    }

    #[test]
    fn a_rooted_name_is_not_in_the_set_of_a_bare_name() {
        let found = run(&["brick_albedo.png", "/brick_normal.png", "\\brick_roughness.png"]);
        let summary: Vec<(&str, &str, Vec<usize>)> =
            found.sets.iter().map(|s| (s.folder, s.stem.as_str(), s.maps.iter().map(|m| m.index).collect())).collect();
        assert_eq!(summary, [("", "brick", vec![0]), ("/", "brick", vec![1, 2])]);
    }

    #[test]
    fn an_empty_list_gives_nothing() {
        assert_eq!(run(&[]), Recognition::default());
    }

    #[test]
    fn names_with_no_stem() {
        let found = run(&["normal.png", "folder/BaseColor.jpg"]);
        let stems: Vec<(&str, &str, usize)> = found.sets.iter().map(|s| (s.folder, s.stem.as_str(), s.maps.len())).collect();
        assert_eq!(stems, [("", "", 1), ("folder", "", 1)]);
        for name in [".png", "textures/", "textures\\", "", "___.png", "brick/-.exr"] {
            assert_eq!(one(name), Err(IgnoreReason::NoFileName), "{name:?}");
        }
    }

    #[test]
    fn udim_tiles_stay_apart() {
        let found = run(&["brick_albedo.1001.exr", "brick_albedo.1002.exr", "brick_normal_1001.exr", "brick_albedo_1001_4k.exr"]);
        let [set] = found.sets.as_slice() else { panic!("{found:?}") };
        let tiles: Vec<Option<u16>> = set.maps.iter().map(|m| m.tile).collect();
        assert_eq!(tiles, [Some(1001), Some(1002), Some(1001), Some(1001)]);
        assert_eq!(set.duplicates, [DuplicateRole { role: MapRole::BaseColor, tile: Some(1001), maps: vec![0, 3] }]);
        assert_eq!(one("brick_albedo_1001_1002.png"), Err(IgnoreReason::SeveralTiles));
        assert_eq!(one("brick_albedo_1024_1001.exr"), Err(IgnoreReason::SeveralTiles));
        assert_eq!(one("brick_albedo_2000.png"), Err(IgnoreReason::UnknownMap), "2000 is neither a tile nor a size");
        let tile_1024 = |names: &[&str]| run(names).sets.iter().flat_map(|s| s.maps.iter().map(|m| m.tile)).collect::<Vec<_>>();
        assert_eq!(tile_1024(&["brick_albedo_1024.png"]), [None], "a bare 1024 alone is a resolution");
        assert_eq!(tile_1024(&["brick_albedo_1023.png", "brick_albedo_1024.png"]), [Some(1023), Some(1024)]);
    }

    #[test]
    fn packed_and_separate_maps_have_their_own_roles() {
        let found =
            run(&["brick_ao.png", "brick_roughness.png", "brick_metallic.png", "brick_ARM.png", "brick_ao_rough_metal.png", "brick_gloss.png"]);
        let [set] = found.sets.as_slice() else { panic!("{found:?}") };
        let roles: Vec<MapRole> = set.maps.iter().map(|m| m.role).collect();
        assert_eq!(
            roles,
            [MapRole::AmbientOcclusion, MapRole::Roughness, MapRole::Metallic, MapRole::PackedOrm, MapRole::PackedOrm, MapRole::Glossiness]
        );
        assert_eq!(set.duplicates, [DuplicateRole { role: MapRole::PackedOrm, tile: None, maps: vec![3, 4] }]);
    }

    #[test]
    fn unicode_names_are_kept_as_written() {
        let found = run(&[
            "Ziegel Ä/砖墙_BaseColor.png",
            "Ziegel Ä\\砖墙-normal.png",
            "ÉCORCE_albedo.png",
            "écorce_normal.png",
            "\u{202e}gnp_brick_ao.png",
            "🧱_metal.png",
        ]);
        let stems: Vec<(&str, &str)> = found.sets.iter().map(|s| (s.folder, s.stem.as_str())).collect();
        assert_eq!(stems, [("Ziegel Ä", "砖墙"), ("", "ÉCORCE"), ("", "écorce"), ("", "gnp_brick"), ("", "")], "ASCII case only is folded");
        assert!(found.ignored.is_empty());
    }

    #[test]
    fn the_name_count_limit_is_checked() {
        // Six digits are never a size or a tile, so every name is its own set.
        let names: Vec<String> = (0..DEFAULT_MAX_NAMES).map(|i| format!("set{i:06}_albedo.png")).collect();
        let Ok(found) = recognize(names.iter().map(String::as_str), &NameLimits::default()) else { panic!("at the limit") };
        assert_eq!(found.sets.len(), DEFAULT_MAX_NAMES);
        let past = names.iter().map(String::as_str).chain(["one_more_albedo.png"]);
        assert_eq!(recognize(past, &NameLimits::default()), Err(NameError::TooManyNames { limit: DEFAULT_MAX_NAMES }));
        // An endless list stops at the limit.
        assert_eq!(recognize(std::iter::repeat("a_albedo.png"), &NameLimits::default()), Err(NameError::TooManyNames { limit: DEFAULT_MAX_NAMES }));
        let none = NameLimits { max_names: 0, ..NameLimits::default() };
        assert_eq!(recognize([], &none), Ok(Recognition::default()));
        assert_eq!(recognize(["a_albedo.png"], &none), Err(NameError::TooManyNames { limit: 0 }));
    }

    #[test]
    fn the_name_length_limit_counts_bytes() {
        // "é" is two bytes: 2 × 506 + 12 = 1024.
        let at = format!("{}x_normal.png", "é".repeat(506));
        assert_eq!(at.len(), DEFAULT_MAX_NAME_BYTES);
        assert_eq!(one(&at).map(|(role, _)| role), Ok(MapRole::Normal));
        let over = format!("x{at}");
        let names = ["brick_albedo.png", over.as_str()];
        assert_eq!(
            recognize(names, &NameLimits::default()),
            Err(NameError::NameTooLong { index: 1, bytes: DEFAULT_MAX_NAME_BYTES + 1, limit: DEFAULT_MAX_NAME_BYTES })
        );
        let short = NameLimits { max_name_bytes: 15, ..NameLimits::default() };
        assert_eq!(recognize(["a_albedo.png", "brick_albedo.png"], &short), Err(NameError::NameTooLong { index: 1, bytes: 16, limit: 15 }));
    }

    #[test]
    fn hostile_names_at_the_limits() {
        let tokens = "a_".repeat(506) + "albedo.png";
        let tiles = "1001_1002_".repeat(100) + "normal.png";
        let glued = "aB".repeat(506) + "_albedo.png";
        let separators = "/\\".repeat(510) + ".png";
        let mut names = vec![tokens.as_str(), tiles.as_str(), glued.as_str(), separators.as_str()];
        assert!(names.iter().all(|n| n.len() <= DEFAULT_MAX_NAME_BYTES));
        let filler: Vec<String> = (0..DEFAULT_MAX_NAMES - names.len()).map(|i| format!("{}_{i}_normal_dx_4k.exr", "z".repeat(1000))).collect();
        names.extend(filler.iter().map(String::as_str));
        let Ok(found) = recognize(names.iter().copied(), &NameLimits::default()) else { panic!("hostile names") };
        let maps: usize = found.sets.iter().map(|s| s.maps.len()).sum();
        assert_eq!(maps + found.ignored.len(), DEFAULT_MAX_NAMES, "{:?}", found.ignored);
        assert_eq!(
            found.ignored.iter().map(|i| (i.index, i.reason)).collect::<Vec<_>>(),
            [(1, IgnoreReason::SeveralTiles), (3, IgnoreReason::NoFileName)]
        );
    }
}
