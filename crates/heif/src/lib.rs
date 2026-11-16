//! `craft-heif`: the optional HEIF / HEIC (iPhone and Mac photo) decoder.
//!
//! `heic-rs`, a pure-Rust HEIF container and HEVC still-picture decoder, reads the container and
//! reconstructs the coded pictures (bit-exact against libde265 on iPhone photos); this crate turns
//! them into RGB itself ([`ycc`]) so the colours match libheif: it honours the HEVC VUI's range
//! and matrix ([`vui`]) where heic-rs assumes BT.709 limited range, and upsamples chroma like
//! libheif. Single pictures and grid-tiled photos, 8- to 16-bit, monochrome, alpha auxiliary
//! images, ICC, EXIF and XMP. The API is plain data ([`Info`], [`Decoded`], [`Error`]) so the
//! crate knows nothing about any app; each app's codec layer adapts it behind its own optional
//! `heif` feature. Moved from LightCraft's `lightcraft-heif`; it also replaces PhotoCraft's
//! `photocraft-heif`.
//!
//! HEIF records orientation in the container (`irot`/`imir`, plus a `clap` crop), not in EXIF.
//! [`Options::apply_transforms`] applies them; with it off, the pixels come back as coded.
//! Auxiliary images other than alpha (HDR gain maps, depth maps) are ignored.
//!
//! Never panics: heic-rs is young, so every call into it runs under `catch_unwind` and a panic
//! inside it becomes [`Error::Malformed`].

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(feature = "decode")]
mod census;
#[cfg(feature = "metadata")]
pub mod items;
#[cfg(feature = "decode")]
mod orient;
/// Lossless synthetic HEIF files for tests, here and in the crates above (feature `testdata`).
#[cfg(any(test, feature = "testdata"))]
#[doc(hidden)]
pub mod testdata;
#[cfg(feature = "decode")]
mod vui;
#[cfg(feature = "decode")]
mod ycc;

#[cfg(feature = "decode")]
use std::borrow::Cow;

#[cfg(feature = "decode")]
use heic_rs::context::Context;
#[cfg(feature = "decode")]
use heic_rs::hevc::Frame;
#[cfg(feature = "decode")]
use heic_rs::props::ItemProps;
#[cfg(feature = "decode")]
use heic_rs::props::colr::Range;

/// What a HEIF file declares, read from the container alone (no pixel is decoded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    /// Size after the container transforms (rotation, mirror, crop): what a viewer shows.
    pub width: u32,
    pub height: u32,
    /// Size as coded, before the transforms.
    pub coded_width: u32,
    pub coded_height: u32,
    /// The primary image has an alpha auxiliary image.
    pub has_alpha: bool,
    /// Bits per sample as coded (8 for most photos, 10 for HDR-capable ones).
    pub bit_depth: u8,
}

/// Decode settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Refuse images with more pixels than this (checked before decoding).
    pub max_pixels: u64,
    /// Apply the container's rotation, mirror and crop (`irot`/`imir`/`clap`).
    pub apply_transforms: bool,
    /// Whose chroma upsampling to match.
    pub chroma: Chroma,
}

impl Default for Options {
    fn default() -> Self {
        Options { max_pixels: 1 << 28, apply_transforms: true, chroma: Chroma::Libheif }
    }
}

/// How 4:2:0 and 4:2:2 chroma is brought to full resolution. The two differ only next to sharp
/// colour edges, where Apple's blends the neighbouring column and, for 4:2:0, the neighbouring row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chroma {
    /// libheif: each chroma sample covers its luma block, blended only after an odd `clap` crop.
    Libheif,
    /// Apple's decoder (and so Photoshop on macOS): a left-sited bilinear filter.
    Apple,
}

/// How the file describes its colour: the code points of ITU-T H.273, from the `colr` `nclx`
/// box, else the HEVC VUI, else the standard defaults (2 = unspecified, limited range).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colour {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}

/// A decoded image: interleaved RGB or RGBA, 16 bits per sample whatever the coded depth,
/// row-major, no padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// Four channels (RGBA) instead of three (RGB).
    pub has_alpha: bool,
    /// Bits per sample as coded.
    pub bit_depth: u8,
    pub data: Vec<u16>,
    pub colour: Colour,
    /// The primary image's ICC profile.
    pub icc: Option<Vec<u8>>,
    /// EXIF as a TIFF structure (without HEIF's offset header). Its Orientation tag only mirrors
    /// what the container says; it is returned verbatim.
    pub exif: Option<Vec<u8>>,
    /// The XMP packet.
    pub xmp: Option<String>,
}

/// Why a file could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A valid file using something this decoder does not handle (image sequences, overlays…).
    Unsupported(&'static str),
    /// The image is larger than [`Options::max_pixels`] or holds an oversized box.
    Limit(String),
    /// Broken or truncated data (or a decoder bug, reported the same way).
    Malformed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unsupported(m) => write!(f, "unsupported HEIF: {m}"),
            Error::Limit(m) => write!(f, "limit exceeded: {m}"),
            Error::Malformed(m) => write!(f, "malformed HEIF: {m}"),
        }
    }
}

impl std::error::Error for Error {}

/// The primary image's EXIF and XMP packet, read from the box structure alone ([`items::read`]):
/// what a build without the decoder (`--no-default-features --features metadata`) has.
#[cfg(all(feature = "metadata", not(feature = "decode")))]
pub fn read_metadata(bytes: &[u8]) -> Result<(Option<Vec<u8>>, Option<String>), Error> {
    items::read(bytes)
}

#[cfg(any(feature = "decode", feature = "metadata"))]
const SEQUENCE: &str = "this file holds an image sequence; only HEIF still images can be opened";

#[cfg(feature = "decode")]
/// heic-rs 0.1.1 reconstructs 4:2:0 and monochrome pictures exactly but fails on the 4:2:2 and
/// 4:4:4 ones some cameras write (Canon and Sony "HIF"); those failures are reported as this.
const CHROMA_422_444: &str = "4:2:2 and 4:4:4 HEIF images (as some Canon and Sony cameras write) can't be decoded yet";

#[cfg(feature = "decode")]
pub(crate) fn err(e: heic_rs::Error) -> Error {
    match e {
        heic_rs::Error::Unsupported(what) => Error::Unsupported(what),
        // The primary item is neither an HEVC picture nor a grid of them.
        heic_rs::Error::MissingBox("hvcC") => Error::Unsupported("the image is an overlay, an identity derivation or not HEVC-coded"),
        heic_rs::Error::PixelLimit { .. } | heic_rs::Error::BoxTooLarge { .. } => Error::Limit(e.to_string()),
        e => Error::Malformed(e.to_string()),
    }
}

#[cfg(feature = "decode")]
/// Runs `f`, turning a heic-rs panic into an error. PhotoCraft's fuzzing found two
/// out-of-range slices in heic-rs 0.1.1 on malformed files: in its box parser (`boxes.rs:130`)
/// and during reconstruction (`hevc/decode/recon.rs:70`).
fn guarded<T>(f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(Error::Malformed("the HEIF decoder failed on this file".into())))
}

#[cfg(feature = "decode")]
/// Reads the container's declared size, depth and alpha without decoding pixels, so callers can
/// check their limits first. Also refuses a file cut short: every coded picture of the primary
/// image must lie within `bytes`.
pub fn probe(bytes: &[u8]) -> Result<Info, Error> {
    guarded(|| probe_unguarded(bytes))
}

#[cfg(feature = "decode")]
fn probe_unguarded(bytes: &[u8]) -> Result<Info, Error> {
    census::cap(bytes)?;
    // No `meta` box: no still image, only a `moov` image sequence (a video track).
    let open_error = |e| match e {
        heic_rs::Error::MissingBox("meta") => Error::Unsupported(SEQUENCE),
        e => err(e),
    };
    let ctx = Context::open(bytes).map_err(open_error)?;
    let id = ctx.meta.primary;
    // heic-rs 0.1.4 caps the size of what it assembles, not the work: an item built from other
    // items (`iloc` construction method 2) is rebuilt once per extent, so a chain of such items
    // multiplies at every level while each result stays small. Count those visits from the
    // `iloc` table before reconstructing anything: the primary item first (`heic_rs::probe` and
    // `Context::grid` both rebuild its descriptor), then its alpha plane and, when that is a grid,
    // its tiles, then every tile reference, repeats included.
    let mut visits = 0_usize;
    item_visits(&ctx, id, 0, &mut visits)?;
    // `heic_rs::probe` reads an item without `ispe` from its parameter sets
    check_item_parameter_sets(&ctx.props(id).map_err(err)?)?;
    let info = heic_rs::probe(bytes).map_err(open_error)?;
    let alpha = ctx.alpha_item(id).map_err(err)?;
    if let Some(alpha) = alpha {
        item_visits(&ctx, alpha, 0, &mut visits)?;
        // an alpha plane may be a grid too: its tiles are rebuilt when it is decoded
        if let Some((_, tiles)) = ctx.grid(alpha).map_err(err)? {
            for &tile in tiles.iter().filter(|&&tile| tile != alpha) {
                item_visits(&ctx, tile, 0, &mut visits)?;
            }
        }
    }
    let mut coded = match ctx.grid(id).map_err(err)? {
        Some((_, tiles)) => tiles,
        None => vec![id],
    };
    for &item in coded.iter().filter(|&&item| item != id) {
        item_visits(&ctx, item, 0, &mut visits)?;
    }
    // A `dimg` listing may name the same tile more than once, and reconstructable items are
    // moreover rebuilt for every extent that references them in heic-rs 0.1.4. For the size
    // check each unique item counts once: the total reconstruction (all of it assembled from
    // `bytes` by heic-rs) is held below one copy of the file, above which a repeat or chain
    // wants more bytes than exist. Order doesn't matter here: this only checks that every
    // tile's bytes lie in the file.
    coded.sort_unstable();
    coded.dedup();
    let mut reconstructed = 0_usize;
    for item in coded {
        reconstructed = reconstructed.saturating_add(ctx.item_data(item).map_err(err)?.len());
        if reconstructed > bytes.len() {
            return Err(Error::Limit(format!(
                "the container holds {reconstructed} bytes of item data, over the {} bytes of the file itself",
                bytes.len()
            )));
        }
    }
    Ok(Info {
        width: info.width,
        height: info.height,
        coded_width: info.coded_width,
        coded_height: info.coded_height,
        has_alpha: info.has_alpha,
        bit_depth: info.bit_depth,
    })
}

#[cfg(feature = "decode")]
/// Most extent visits heic-rs may spend reconstructing the primary image's coded items; real
/// files need one per tile (plus a few for the rare derived item).
const MAX_ITEM_VISITS: usize = 4096;
#[cfg(feature = "decode")]
/// Deepest chain of items built from items we follow (heic-rs refuses beyond eight itself).
const MAX_ITEM_CHAIN: u32 = 8;

#[cfg(feature = "decode")]
/// Adds to `visits` the extents heic-rs would read to reconstruct `id`, following items built
/// from other items, and fails once that passes [`MAX_ITEM_VISITS`] or the chain is too deep.
fn item_visits(ctx: &Context<'_>, id: u32, depth: u32, visits: &mut usize) -> Result<(), Error> {
    use heic_rs::meta::iloc::Construction;
    if depth > MAX_ITEM_CHAIN {
        return Err(Error::Limit(format!("items built from items nest deeper than {MAX_ITEM_CHAIN} levels")));
    }
    let Some(loc) = ctx.meta.location(id) else { return Ok(()) };
    *visits = visits.saturating_add(loc.extents.len().max(1));
    if *visits > MAX_ITEM_VISITS {
        return Err(Error::Limit(format!("reconstructing the image takes over {MAX_ITEM_VISITS} item extent reads")));
    }
    if loc.construction == Construction::Item {
        for e in &loc.extents {
            if let Ok(source) = u32::try_from(e.index)
                && source != id
            {
                item_visits(ctx, source, depth + 1, visits)?;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "decode")]
/// The primary image's EXIF (a TIFF structure, as in [`Decoded::exif`]) and XMP packet, read
/// from the container alone: no picture is decoded, so cataloguing a photo costs about a
/// [`probe`]. `(None, None)` for a file that carries neither.
pub fn read_metadata(bytes: &[u8]) -> Result<(Option<Vec<u8>>, Option<String>), Error> {
    guarded(|| {
        census::cap(bytes)?;
        let ctx = Context::open(bytes).map_err(|e| match e {
            heic_rs::Error::MissingBox("meta") => Error::Unsupported(SEQUENCE),
            e => err(e),
        })?;
        Ok(metadata(&ctx))
    })
}

#[cfg(feature = "decode")]
/// Decodes the primary image, with its metadata.
pub fn decode(bytes: &[u8], options: &Options) -> Result<Decoded, Error> {
    guarded(|| decode_unguarded(bytes, options))
}

#[cfg(feature = "decode")]
/// Decodes the smallest thumbnail image the file carries whose long edge is at least
/// `min_edge`, if any (iPhones store one of about 320 × 240). Transforms are applied as for the
/// primary image; the metadata (and, when the thumbnail has no profile of its own, the ICC
/// profile) are the primary image's.
pub fn decode_thumbnail(bytes: &[u8], min_edge: u32, options: &Options) -> Result<Option<Decoded>, Error> {
    guarded(|| {
        census::cap(bytes)?;
        let ctx = Context::open(bytes).map_err(err)?;
        let primary = ctx.meta.primary;
        let mut best: Option<(u32, u32)> = None;
        // A thumbnail whose own reconstruction would cost too much is not a candidate; the visits
        // of all the candidates share one budget.
        let mut candidate_visits = 0_usize;
        for r in ctx.meta.refs.iter().filter(|r| r.kind == heic_rs::meta::iref::RefKind::Thmb && r.to.contains(&primary)) {
            if item_visits(&ctx, r.from, 0, &mut candidate_visits).is_err() {
                continue;
            }
            let Ok(p) = ctx.props(r.from) else { continue };
            if check_item_parameter_sets(&p).is_err() {
                continue;
            }
            // A grid whose tile would fail the same check must not win and then fail in
            // `decode_item`, with no fallback to a larger valid thumbnail.
            let grid = match ctx.grid(r.from) {
                Ok(grid) => grid,
                // an unreadable grid fails in `decode_item` too: not a candidate
                Err(_) => continue,
            };
            if let Some((_, tiles)) = grid {
                // every reference is rebuilt and decoded, repeats included, so each is charged;
                // the properties are checked once per distinct tile
                let mut checked = std::collections::HashSet::new();
                let tiles_ok = tiles.iter().all(|&t| {
                    item_visits(&ctx, t, 0, &mut candidate_visits).is_ok()
                        && (!checked.insert(t) || ctx.props(t).is_ok_and(|tp| check_item_parameter_sets(&tp).is_ok()))
                });
                if !tiles_ok {
                    continue;
                }
            }
            let Ok((w, h)) = ctx.coded_size(r.from, &p) else { continue };
            let edge = w.max(h);
            if edge >= min_edge && best.is_none_or(|(_, e)| edge < e) {
                best = Some((r.from, edge));
            }
        }
        let Some((id, _)) = best else { return Ok(None) };
        // The same pre-scan as `probe`: the thumbnail's own descriptor and every tile reference
        // are counted before heic-rs rebuilds any of them.
        let mut visits = 0_usize;
        item_visits(&ctx, id, 0, &mut visits)?;
        if let Some((_, tiles)) = ctx.grid(id).map_err(err)? {
            for &tile in &tiles {
                item_visits(&ctx, tile, 0, &mut visits)?;
            }
        }
        let mut decoded = decode_item(&ctx, id, options, false)?;
        // `item_icc` rebuilds the primary grid's descriptor: only when that is within the visit
        // budget (the thumbnail path never counted it).
        if decoded.icc.is_none() && item_visits(&ctx, primary, 0, &mut 0).is_ok() {
            decoded.icc = item_icc(&ctx, primary);
        }
        (decoded.exif, decoded.xmp) = metadata(&ctx);
        Ok(Some(decoded))
    })
}

#[cfg(feature = "decode")]
/// An item's ICC profile; a grid whose own properties carry none takes its first tile's (as on
/// some iPhones).
fn item_icc(ctx: &Context<'_>, id: u32) -> Option<Vec<u8>> {
    let p = ctx.props(id).ok()?;
    let icc = p.icc.or_else(|| {
        let (_, tiles) = ctx.grid(id).ok()??;
        ctx.props(*tiles.first()?).ok()?.icc
    });
    icc.map(<[u8]>::to_vec)
}

#[cfg(feature = "decode")]
fn decode_unguarded(bytes: &[u8], options: &Options) -> Result<Decoded, Error> {
    // Refuses sequences with a clear message before anything else is read.
    probe_unguarded(bytes)?;
    let ctx = Context::open(bytes).map_err(err)?;
    let mut decoded = decode_item(&ctx, ctx.meta.primary, options, true)?;
    let (exif, xmp) = metadata(&ctx);
    decoded.exif = exif;
    decoded.xmp = xmp;
    Ok(decoded)
}

#[cfg(feature = "decode")]
/// Decodes one image item (a coded picture or a grid of them) to RGB(A), alpha included when
/// `with_alpha` and the item has an alpha auxiliary image.
fn decode_item(ctx: &Context<'_>, id: u32, options: &Options, with_alpha: bool) -> Result<Decoded, Error> {
    let p = ctx.props(id).map_err(err)?;
    // heic-rs 0.1.4 keeps every associated transform and its [`orient::apply`] allocates the
    // full picture again for each: a file could chain the repetition and multiply decode CPU
    // without changing any checked size. Real files carry at most one `clap`, `irot` and
    // `imir`, so refuse anything approaching [`MAX_TRANSFORMS`] (CodeRabbit #44).
    if p.transforms.len() > MAX_TRANSFORMS {
        return Err(Error::Limit(format!("the image carries {} transform properties, over the {MAX_TRANSFORMS}-property cap", p.transforms.len())));
    }
    let limit = options.max_pixels;
    // The declared sizes are refused before anything is decoded or allocated (an item without
    // `ispe` takes its size from its parameter sets, checked first).
    check_item_parameter_sets(&p)?;
    let (cw, ch) = ctx.coded_size(id, &p).map_err(err)?;
    heic_rs::image::check_pixels(cw, ch, limit).map_err(err)?;
    let (tw, th) = heic_rs::transform::transformed_size(cw, ch, &p.transforms).map_err(err)?;
    heic_rs::image::check_pixels(tw, th, limit).map_err(err)?;

    let coded = Coded::decode(ctx, id, &p, limit)?;
    let planes = coded.planes()?;
    let colour = colour(&p, &coded.first_props);
    let signal = ycc::Signal { matrix: colour.matrix, full_range: colour.full_range };
    let alpha_id = if with_alpha { ctx.alpha_item(id).map_err(err)? } else { None };
    let (w, h) = planes.size();
    // A grid's canvas dimensions come from its payload, not the checked `ispe` (CodeRabbit
    // #48): check the real one, before [`ycc::to_rgb16`] allocates for it.
    match (u32::try_from(w), u32::try_from(h)) {
        (Ok(wu), Ok(hu)) => {
            heic_rs::image::check_pixels(wu, hu, limit).map_err(err)?;
        }
        _ => return Err(Error::Limit(format!("the image canvas is {w}x{h}, beyond the {limit}-pixel limit"))),
    }
    // The transforms apply to the picture at its declared size, so the `clap` parity that
    // decides the upsampling is measured there, not on the coded picture.
    let (dw, dh) = (cw as usize, ch as usize);
    let mode = match options.chroma {
        Chroma::Apple => ycc::Upsampling::Sited,
        Chroma::Libheif if options.apply_transforms && orient::odd_crop(dw, dh, &p.transforms)? => ycc::Upsampling::Bilinear,
        Chroma::Libheif => ycc::Upsampling::Nearest,
    };
    let data = ycc::to_rgb16(&planes, signal, alpha_id.is_some(), mode)
        .ok_or_else(|| Error::Malformed("the decoded pictures don't cover the image".into()))?;
    let bit_depth = planes.bit_depth();
    let has_alpha = alpha_id.is_some();
    let mut img = orient::Pixels { width: w, height: h, ch: if has_alpha { 4 } else { 3 }, data };
    // A coded picture is a whole number of coding blocks and may be larger than the image the
    // container declares (Sony codes 1920 x 1080 as 1920 x 1088 and writes no SPS conformance
    // window). Crop to `ispe` before the alpha merge and the transforms, both of which are
    // defined on that size, so decoding agrees with [`probe`] instead of handing back the
    // encoder's padding.
    img = orient::to_declared(img, dw, dh)?;
    if let Some(aux) = alpha_id {
        let ap = ctx.props(aux).map_err(err)?;
        // The alpha is a coded item of its own: it declares its own size and may pad it to whole
        // coding blocks differently from the picture. What has to agree is what the two declare,
        // not how either is coded; the padding comes off both the same way. Checked before the
        // alpha is decoded, so a mismatch costs nothing.
        if ctx.coded_size(aux, &ap).map_err(err)? != (cw, ch) {
            return Err(Error::Unsupported("an alpha image of a different size than the picture"));
        }
        let alpha = Coded::decode(ctx, aux, &ap, limit)?;
        let alpha_planes = alpha.planes()?;
        // `put_alpha` refuses a plane that does not cover the declared size.
        ycc::put_alpha(&alpha_planes, &mut img.data, dw).ok_or_else(|| Error::Malformed("the alpha image is incomplete".into()))?;
    }
    if options.apply_transforms {
        img = orient::apply(img, &p.transforms)?;
    }
    Ok(Decoded {
        width: u32::try_from(img.width).map_err(|_| Error::Limit("image too wide".into()))?,
        height: u32::try_from(img.height).map_err(|_| Error::Limit("image too tall".into()))?,
        has_alpha,
        bit_depth,
        data: img.data,
        colour,
        // A grid whose own properties carry no profile takes its tiles' (as on some iPhones).
        icc: p.icc.or(coded.first_props.icc).map(<[u8]>::to_vec),
        exif: None,
        xmp: None,
    })
}

#[cfg(feature = "decode")]
/// How many times `max_pixels` the coded pictures of one item may hold together: a grid's tiles
/// cover its canvas rounded up to whole tiles, so legitimate grids sit near 1.
const GRID_BUDGET: u64 = 2;

#[cfg(feature = "decode")]
/// The most transform properties (`clap`/`irot`/`imir`) one item may carry. Real files have at
/// most one of each, so anything approaching this is a reordered or hostile association list.
const MAX_TRANSFORMS: usize = 12;

#[cfg(feature = "decode")]
/// The most parameter sets one `hvcC` may carry: one per id H.265 allows (16 VPS, 16 SPS, 64 PPS).
/// heic-rs 0.1.4 replaces a repeated id by scanning every set kept so far, so more sets (repeats,
/// or ids past the standard's range) only make that collection quadratic.
const MAX_PARAMETER_SETS: usize = 16 + 16 + 64;
#[cfg(feature = "decode")]
/// Highest `pps_pic_parameter_set_id` H.265 allows (7.4.3.3.1).
const MAX_PPS_ID: u32 = 63;
#[cfg(feature = "decode")]
/// Most tile columns and rows a picture may have: H.265's level limits (Table A.8, levels 6 to
/// 6.2). heic-rs 0.1.4 builds its tile tables by walking the tile boundaries for every coding tree
/// block, so its work grows with blocks × tiles; these keep that a small multiple of the blocks.
const MAX_TILE_COLS: u32 = 20;
#[cfg(feature = "decode")]
const MAX_TILE_ROWS: u32 = 22;

#[cfg(feature = "decode")]
/// Refuses parameter sets that would make heic-rs's set collection or tile tables cost far more
/// than the picture: too many sets, a PPS id out of range, or more tiles than any level allows.
fn check_parameter_sets(sets: &[&[u8]]) -> Result<(), Error> {
    if sets.len() > MAX_PARAMETER_SETS {
        return Err(Error::Limit(format!("the hvcC record carries {} parameter sets, over the {MAX_PARAMETER_SETS}-set cap", sets.len())));
    }
    let layouts = vui::pps_layouts(sets).ok_or_else(|| Error::Malformed("unreadable picture parameter set".into()))?;
    for p in layouts {
        if p.id > MAX_PPS_ID {
            return Err(Error::Malformed(format!("picture parameter set id {} is out of range", p.id)));
        }
        if p.tile_cols > MAX_TILE_COLS || p.tile_rows > MAX_TILE_ROWS {
            return Err(Error::Limit(format!(
                "a picture of {} × {} tiles, over the {MAX_TILE_COLS} × {MAX_TILE_ROWS} H.265 allows",
                p.tile_cols, p.tile_rows
            )));
        }
    }
    Ok(())
}

#[cfg(feature = "decode")]
/// [`check_parameter_sets`] of a coded item's `hvcC`, if it has one.
fn check_item_parameter_sets(p: &ItemProps<'_>) -> Result<(), Error> {
    p.hvcc.as_ref().map_or(Ok(()), |h| check_parameter_sets(&h.parameter_sets()))
}

#[cfg(feature = "decode")]
/// The decoded pictures of an item: one, or a grid's tiles in `dimg` order.
struct Coded<'a> {
    frames: Vec<Frame>,
    grid: Option<heic_rs::grid::Grid>,
    /// The properties of the first coded picture (the item itself, or the first tile): where the
    /// HEVC configuration and its VUI live.
    first_props: ItemProps<'a>,
    /// A failed 4:2:2 / 4:4:4 picture, for the error message.
    chroma_format: u8,
}

#[cfg(feature = "decode")]
impl<'a> Coded<'a> {
    fn decode(ctx: &Context<'a>, id: u32, p: &ItemProps<'a>, limit: u64) -> Result<Coded<'a>, Error> {
        let (grid, ids) = match ctx.grid(id).map_err(err)? {
            Some((g, tiles)) => (Some(g), tiles),
            None => (None, vec![id]),
        };
        let first = *ids.first().ok_or_else(|| Error::Malformed("grid lists no tiles".into()))?;
        let first_props = if first == id { p.clone() } else { ctx.props(first).map_err(err)? };
        let chroma_format = first_props.hvcc.as_ref().map_or(1, |c| c.chroma_format);
        // #1518: the decoder allocates each picture at its SPS's uncropped size, which a
        // conformance window (or a slice picking another SPS) can make far larger than the
        // declared one, and a grid holds all its tiles at once. Both are refused before decoding.
        let mut total = 0u64;
        // A grid may list one tile thousands of times: each distinct tile is read and checked
        // once, so the work follows the file's items, not the grid's cell count.
        let mut largest_of: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
        for t in &ids {
            if let Some(largest) = largest_of.get(t) {
                total = total.saturating_add(*largest);
                continue;
            }
            let tp = ctx.props(*t).map_err(err)?;
            let hvcc = tp.hvcc.as_ref().ok_or(heic_rs::Error::MissingBox("hvcC")).map_err(err)?;
            check_parameter_sets(&hvcc.parameter_sets())?;
            let sizes = vui::coded_sizes(&hvcc.parameter_sets()).ok_or_else(|| Error::Malformed("unreadable sequence parameter set".into()))?;
            let mut largest = 0u64;
            for (w, h) in sizes {
                heic_rs::image::check_pixels(w, h, limit).map_err(err)?;
                largest = largest.max(u64::from(w) * u64::from(h));
            }
            largest_of.insert(*t, largest);
            total = total.saturating_add(largest);
        }
        if total > limit.saturating_mul(GRID_BUDGET) {
            return Err(Error::Limit(format!("the coded pictures hold {total} pixels, over the {limit}-pixel limit")));
        }
        let decode_one = |t: &u32| -> Result<Frame, Error> {
            let tp = ctx.props(*t).map_err(err)?;
            let hvcc = tp.hvcc.as_ref().ok_or(heic_rs::Error::MissingBox("hvcC")).map_err(err)?;
            let data = ctx.item_data(*t).map_err(err)?;
            let nals = hvcc.split_nals(&data).map_err(err)?;
            let params = hvcc.parameter_sets();
            if params.is_empty() {
                return Err(Error::Malformed("hvcC carries no parameter sets".into()));
            }
            let frame = heic_rs::hevc::decode_still(&params, &nals).map_err(err)?;
            frame.validate().map_err(err)?;
            heic_rs::image::check_pixels(frame.width, frame.height, limit).map_err(err)?;
            Ok(frame)
        };
        #[cfg(not(target_arch = "wasm32"))]
        let frames: Result<Vec<Frame>, Error> = {
            use rayon::prelude::*;
            ids.par_iter().map(decode_one).collect()
        };
        #[cfg(target_arch = "wasm32")]
        let frames: Result<Vec<Frame>, Error> = ids.iter().map(decode_one).collect();
        let frames = frames.map_err(|e| match e {
            Error::Malformed(_) if chroma_format >= 2 => Error::Unsupported(CHROMA_422_444),
            e => e,
        })?;
        Ok(Coded { frames, grid, first_props, chroma_format })
    }

    fn planes(&self) -> Result<ycc::Planes<'_>, Error> {
        match (&self.grid, self.frames.as_slice()) {
            (Some(g), tiles) => heic_rs::grid::Mosaic::new(g, tiles).map(ycc::Planes::Grid).map_err(err),
            (None, [frame]) => Ok(ycc::Planes::Picture(frame)),
            (None, _) => Err(Error::Malformed(format!("expected one coded picture (chroma format {})", self.chroma_format))),
        }
    }
}

#[cfg(feature = "decode")]
/// The colour description in force: `nclx` on the item (or, for a grid without one, on its
/// first tile), else the VUI of its (first) coded picture, else the H.265 defaults.
fn colour(p: &ItemProps<'_>, coded: &ItemProps<'_>) -> Colour {
    if let Some(n) = p.nclx.or(coded.nclx) {
        return Colour { primaries: n.primaries, transfer: n.transfer, matrix: n.matrix_code, full_range: n.range == Range::Full };
    }
    let s = coded.hvcc.as_ref().and_then(|c| vui::signal(&c.parameter_sets())).unwrap_or_default();
    Colour { primaries: s.primaries.into(), transfer: s.transfer.into(), matrix: s.matrix.into(), full_range: s.full_range }
}

#[cfg(feature = "decode")]
/// The primary image's EXIF (TIFF structure) and XMP packet. Metadata is a courtesy: a broken
/// metadata item never fails a decode whose pixels came out fine.
fn metadata(ctx: &Context<'_>) -> (Option<Vec<u8>>, Option<String>) {
    let id = ctx.meta.primary;
    // Metadata is best effort, so an item that would cost too much to rebuild is skipped, not an error.
    let mut visits = 0_usize;
    let cheap = |item: Option<u32>, visits: &mut usize| item.is_none_or(|item| item_visits(ctx, item, 0, visits).is_ok());
    let exif = if cheap(ctx.exif_item(id), &mut visits) { ctx.exif(id).ok().flatten().map(<[u8]>::to_vec) } else { None };
    let xmp_item = ctx.xmp_item(id).filter(|&x| cheap(Some(x), &mut visits));
    let xmp = xmp_item.and_then(|x| ctx.item_data(x).ok()).and_then(|d| match d {
        Cow::Borrowed(b) => String::from_utf8(b.to_vec()).ok(),
        Cow::Owned(b) => String::from_utf8(b).ok(),
    });
    (exif, xmp.map(|x| x.trim_end_matches('\0').to_string()))
}

#[cfg(test)]
#[cfg(feature = "decode")]
mod tests_icc;
#[cfg(test)]
#[cfg(feature = "decode")]
mod tests_synthetic;

#[cfg(test)]
#[cfg(feature = "decode")]
mod tests {
    use super::*;

    /// Found by PhotoCraft's `decode_heif` fuzz target: a malformed box makes heic-rs 0.1.1 slice
    /// out of range (`boxes.rs:130`). It must be an error, not a crash.
    const HEIC_RS_BOX_PANIC: [u8; 72] = [
        0x00, 0x00, 0x00, 0x24, 0x66, 0x74, 0x79, 0x70, 0x68, 0x65, 0x69, 0x63, 0x00, 0x00, 0x00, 0x00, 0x6d, 0x69, 0x66, 0x31, 0x4d, 0x69, 0x50,
        0x72, 0x6d, 0x69, 0x61, 0x66, 0x4d, 0x69, 0x48, 0x42, 0x72, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x6d, 0x65, 0x74, 0x61, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x75, 0x75, 0x69, 0x64, 0x00, 0x00, 0x00, 0x00, 0x03, 0x08, 0x08, 0x08, 0x00, 0x03, 0x01, 0x03, 0x70,
        0x00, 0xa8, 0x00,
    ];

    #[test]
    fn a_heic_rs_panic_is_a_malformed_error() {
        for apply_transforms in [true, false] {
            let r = decode(&HEIC_RS_BOX_PANIC, &Options { apply_transforms, ..Default::default() });
            assert!(matches!(r, Err(Error::Malformed(_))), "{r:?}");
        }
    }

    #[test]
    fn image_sequence_is_unsupported_with_a_clear_message() {
        // An ftyp and nothing else: a HEIF image sequence would have a moov here instead of a meta.
        let bytes = b"\0\0\0\x18ftypmsf1\0\0\0\0msf1hevc";
        for r in [probe(bytes).map(|_| ()), decode(bytes, &Options::default()).map(|_| ()), read_metadata(bytes).map(|_| ())] {
            assert!(matches!(&r, Err(Error::Unsupported(m)) if m.contains("sequence")), "{r:?}");
        }
    }

    #[test]
    fn garbage_is_an_error() {
        for bytes in [&b""[..], b"\0\0\0\x18ftypheic\0\0\0\0mif1heic", &[0xFF; 64]] {
            assert!(probe(bytes).is_err());
            assert!(decode(bytes, &Options::default()).is_err());
            assert!(decode_thumbnail(bytes, 1, &Options::default()).is_err());
            assert!(read_metadata(bytes).is_err());
        }
    }

    #[test]
    fn errors_display_their_kind() {
        assert!(Error::Unsupported("x").to_string().contains("unsupported"));
        assert!(Error::Limit("x".into()).to_string().contains("limit"));
        assert!(Error::Malformed("x".into()).to_string().contains("malformed"));
    }
}
