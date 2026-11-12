//! Photoshop's layered-TIFF conventions, observed from files Photoshop writes and reads.
//!
//! A layered TIFF is an ordinary TIFF whose IFD holds the flattened composite, plus the two tags
//! [`TAG_IMAGE_SOURCE_DATA`] ([`ImageSourceData`]) and [`TAG_IMAGE_RESOURCES`] (the image
//! resources). Reading one is: decode the composite, read the two tags, and rebuild a
//! [`PsdFile`] with [`ImageSourceData::into_psd`], the composite as its merged image
//! ([`interleaved_to_planar`]). Writing one is the reverse ([`planar_to_interleaved`],
//! [`ImageSourceData::for_tiff`], [`ifd_channel_resources`]).
//!
//! Photoshop is strict about a few things a PSD reader would not notice:
//!
//! - `LMsk` must hold its 14-byte record: a zero-length block makes Photoshop report that the
//!   layer data cannot be read.
//! - It writes an empty `Patt` first and a default `FMsk` (filter mask) last, and a set of
//!   blocks on every layer ([`photoshop_layer_blocks`]).
//! - The IFD's extra samples are paired with the channel-name resources in order, ignoring the
//!   `ExtraSamples` kind. When the composite's transparency is not stored in the IFD, those
//!   resources must leave it out too ([`ifd_channel_resources`]).

use crate::error::{PsdError, Result};
use crate::file::{GlobalLayerMask, PsdFile};
use crate::layer::LayerFlags;
use crate::resources::ImageResource;
use crate::tagged::TaggedBlock;

use super::ImageSourceData;

/// The TIFF tag holding [`ImageSourceData`] (`ImageSourceData`, type UNDEFINED).
pub const TAG_IMAGE_SOURCE_DATA: u16 = 37724;
/// The TIFF tag holding the image resources (type UNDEFINED or BYTE).
pub const TAG_IMAGE_RESOURCES: u16 = 34377;

/// The filter-mask block (`FMsk`) Photoshop writes in a layered TIFF: the same red 50% overlay
/// as [`GlobalLayerMask::photoshop_default`], without the kind byte.
pub const FILTER_MASK_DEFAULT: [u8; 12] = [0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 50];

/// Image resources about the extra channels (spot and alpha channels).
pub mod ids {
    /// Alpha channel names, Pascal strings.
    pub const ALPHA_NAMES: u16 = 1006;
    /// Quick Mask information: the channel id (u16) and whether it was empty.
    pub const QUICK_MASK_INFO: u16 = 1022;
    /// Alpha channel names, Unicode strings.
    pub const UNICODE_ALPHA_NAMES: u16 = 1045;
    /// Alpha identifiers: one u32 per extra channel.
    pub const ALPHA_IDENTIFIERS: u16 = 1053;
    /// DisplayInfo: a 4-byte version, then one 13-byte record per extra channel.
    pub const DISPLAY_INFO: u16 = 1077;
}

/// The size of one DisplayInfo (1077) record.
const DISPLAY_RECORD: usize = 13;

impl ImageSourceData {
    /// The layer data of a layered TIFF saved from `file`, adjusted the way Photoshop writes it.
    /// The layers, global mask and global blocks move out of `file`, which keeps its header,
    /// resources and merged image.
    ///
    /// `ifd_alpha`: the IFD stores the composite's transparency as its first extra sample (Save
    /// Transparency on).
    pub fn for_tiff(file: &mut PsdFile, ifd_alpha: bool) -> ImageSourceData {
        // Photoshop pairs alpha-name resources with samples past the colour channels and ignores
        // the ExtraSamples kind. Layer transparency stays in each layer's channel -1.
        let merged_alpha = file.layer_info.as_ref().is_some_and(|l| l.merged_alpha);
        let color = usize::from(file.header.color_mode.color_channels().unwrap_or(0));
        let spot_extras = usize::from(file.header.channels) > color + usize::from(merged_alpha);
        let mut isd = ImageSourceData::take_from_psd(file);
        if ifd_alpha && spot_extras {
            // Save Transparency next to spot or alpha channels, as Photoshop (27.11) writes it: a
            // positive layer count and an `MTrn` block first in the layer data mark the first
            // extra sample as the merged transparency.
            if let Some(info) = &mut isd.layer_info {
                info.merged_alpha = false;
            }
        } else if !ifd_alpha {
            // A positive layer count: the merged image's first extra sample, if any, is not
            // transparency (Photoshop's own spot TIFF does this; a negative count would claim the
            // spot plane is alpha).
            if let Some(info) = &mut isd.layer_info {
                info.merged_alpha = false;
            }
            // A kept merged-transparency marker would make the first spot read back as
            // transparency; the IFD leaves that sample out.
            isd.global_blocks.retain(|b| !b.is_merged_transparency_marker());
        }
        if let Some(info) = &mut isd.layer_info {
            for layer in &mut info.layers {
                // Photoshop sets flag bit 3 on every record it writes, group dividers included.
                layer.flags.0 |= LayerFlags::BIT4_USEFUL;
                photoshop_layer_blocks(&mut layer.blocks);
            }
        }
        // Photoshop reads a fixed 14-byte record from `LMsk`.
        if let Some(mask) = &mut isd.global_layer_mask
            && mask.data.len() < 14
        {
            *mask = GlobalLayerMask::photoshop_default();
        }
        if !isd.global_blocks.iter().any(|b| matches!(&b.key, b"Patt" | b"Pat2" | b"Pat3")) {
            isd.global_blocks.insert(0, TaggedBlock::new(*b"Patt", Vec::new()));
        }
        if !isd.global_blocks.iter().any(|b| &b.key == b"FMsk") {
            isd.global_blocks.push(TaggedBlock::new(*b"FMsk", FILTER_MASK_DEFAULT.to_vec()));
        }
        // Photoshop writes the marker first, ahead of `Patt`.
        if ifd_alpha && spot_extras && !isd.global_blocks.iter().any(TaggedBlock::is_merged_transparency_marker) {
            isd.global_blocks.insert(0, TaggedBlock::new(*b"MTrn", Vec::new()));
        }
        isd
    }

    /// Drops the global blocks [`Self::for_tiff`] adds that carry nothing: an empty `Patt`,
    /// `Pat2` or `Pat3` and the default `FMsk`. A reader that keeps global blocks for a later
    /// save does this on open, so they do not pile up on every round trip.
    pub fn drop_photoshop_padding(&mut self) {
        self.global_blocks.retain(|b| {
            let empty_pattern = matches!(&b.key, b"Patt" | b"Pat2" | b"Pat3") && b.data.is_empty();
            let default_filter_mask = &b.key == b"FMsk" && b.data.as_slice() == FILTER_MASK_DEFAULT;
            !empty_pattern && !default_filter_mask
        });
    }
}

/// Adds the blocks Photoshop writes on every layer of a layered TIFF when the layer does not
/// carry them already: `lnsr` (`layr`), `clbl` (on), `infx` (off), `knko` (0), `lspf` (none
/// locked) and `lclr` (no label), after `lyid` (or `luni`), as Photoshop orders them.
pub fn photoshop_layer_blocks(blocks: &mut Vec<TaggedBlock>) {
    let has = |k: &[u8; 4]| blocks.iter().any(|b| &b.key == k);
    let mut add = Vec::new();
    if !has(b"lnsr") {
        add.push(TaggedBlock::name_source(*b"layr"));
    }
    if !has(b"clbl") {
        add.push(TaggedBlock::blend_clipped_as_group(true));
    }
    if !has(b"infx") {
        add.push(TaggedBlock::blend_interior_elements(false));
    }
    if !has(b"knko") {
        add.push(TaggedBlock::knockout(0));
    }
    if !has(b"lspf") {
        add.push(TaggedBlock::protection(0));
    }
    if !has(b"lclr") {
        add.push(TaggedBlock::sheet_color(0));
    }
    let at = blocks
        .iter()
        .position(|b| &b.key == b"lyid")
        .map(|i| i + 1)
        .or_else(|| blocks.iter().position(|b| &b.key == b"luni").map(|i| i + 1))
        .unwrap_or(blocks.len());
    for (i, block) in add.into_iter().enumerate() {
        blocks.insert(at.saturating_add(i), block);
    }
}

/// A block [`strip_foreign_order_blocks`] removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrippedBlock {
    /// The index of the layer in [`crate::LayerInfo::layers`] it was on; `None` for a global
    /// block.
    pub layer: Option<usize>,
    /// The block's key.
    pub key: [u8; 4],
    /// Bytes of data it held.
    pub len: usize,
}

/// Removes the blocks kept verbatim from a little-endian TIFF because their layout is unknown
/// ([`TaggedBlock::is_foreign_order`]) before `file` is written as a PSD: their bytes are in the
/// wrong order for a PSD and cannot be converted. A save back to a little-endian TIFF keeps
/// them. Returns what was removed, for the caller to report.
pub fn strip_foreign_order_blocks(file: &mut PsdFile) -> Vec<StrippedBlock> {
    let mut out = Vec::new();
    let mut strip = |blocks: &mut Vec<TaggedBlock>, layer: Option<usize>| {
        blocks.retain(|b| {
            let keep = !b.is_foreign_order();
            if !keep {
                out.push(StrippedBlock { layer, key: b.key, len: b.data.len() });
            }
            keep
        });
    };
    if let Some(info) = &mut file.layer_info {
        for (i, l) in info.layers.iter_mut().enumerate() {
            strip(&mut l.blocks, Some(i));
        }
    }
    strip(&mut file.global_blocks, None);
    out
}

/// How a layered TIFF's IFD stores the PSD merged image's channels: colour, then (when
/// `write_alpha`) the transparency, then the first `kept` of the `extra` spot and alpha
/// channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IfdChannels {
    /// Colour channels: 1, 3 or 4.
    pub color: usize,
    /// The PSD merged image has a transparency plane (after the colour planes).
    pub merged_alpha: bool,
    /// The IFD stores it as the first extra sample.
    pub write_alpha: bool,
    /// Spot and alpha channels in the merged image.
    pub extra: usize,
    /// How many of them the IFD stores (a TIFF writer may cap the extra samples).
    pub kept: usize,
}

/// Adjusts the PSD model's channel resources to the samples the IFD stores ([`IfdChannels`]),
/// as Photoshop pairs them: the alpha identifiers (1053), the Quick Mask channel id (1022), the
/// names (1006, 1045) and DisplayInfo (1077). A resource too short for what it should hold is
/// left as it is.
pub fn ifd_channel_resources(resources: &mut Vec<ImageResource>, c: IfdChannels) {
    let kept = c.kept.min(c.extra);
    // One identifier per extra sample actually stored. The model's 1053 lists the merged
    // transparency too, which the IFD leaves out without it.
    if !c.write_alpha {
        resources.retain(|r| r.id != ids::ALPHA_IDENTIFIERS);
        if kept > 0 {
            resources.push(ImageResource::new(ids::ALPHA_IDENTIFIERS, alpha_identifiers(false, kept)));
        }
    }
    let dropped_alpha = c.merged_alpha && !c.write_alpha;
    // 1022 names a channel of the merged image, which counts the transparency plane. An IFD
    // without it does not, so the id moves back by one.
    if dropped_alpha
        && kept > 0
        && let Some(r) = resources.iter_mut().find(|r| r.id == ids::QUICK_MASK_INFO)
        && let Some(b) = r.data.get_mut(..2)
    {
        let id = u16::from_be_bytes([b[0], b[1]]).saturating_sub(1);
        b.copy_from_slice(&id.to_be_bytes());
    }
    // The name and DisplayInfo resources list the merged transparency first; the IFD leaves
    // that plane out, so they must too.
    if dropped_alpha {
        for r in resources.iter_mut() {
            r.data = match r.id {
                ids::ALPHA_NAMES => skip_pascal(&r.data, 1),
                ids::UNICODE_ALPHA_NAMES => skip_unicode(&r.data, 1),
                ids::DISPLAY_INFO => skip_display_records(&r.data, 1),
                _ => continue,
            };
        }
    }
    // Past the cap, the resources describe only the channels stored, and a Quick Mask among the
    // rest is dropped. With the transparency written, the lists hold it ahead of the channels.
    if c.extra > kept {
        let entries = kept + usize::from(c.write_alpha);
        for r in resources.iter_mut() {
            r.data = match r.id {
                ids::ALPHA_NAMES => first_pascal(&r.data, entries),
                ids::UNICODE_ALPHA_NAMES => first_unicode(&r.data, entries),
                ids::DISPLAY_INFO => first_display_records(&r.data, entries),
                _ => continue,
            };
        }
        let end = c.color.saturating_add(entries);
        resources.retain(|r| r.id != ids::QUICK_MASK_INFO || r.data.get(..2).is_none_or(|b| usize::from(u16::from_be_bytes([b[0], b[1]])) < end));
    }
}

/// Resource 1053 as Photoshop numbers it: 0 for the transparency (when `transparency`), then 3,
/// 4, 5, ... for `count` extra channels.
pub fn alpha_identifiers(transparency: bool, count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(count.saturating_add(1).saturating_mul(4));
    if transparency {
        out.extend_from_slice(&0u32.to_be_bytes());
    }
    for i in 0..count {
        let id = 3u32.saturating_add(u32::try_from(i).unwrap_or(u32::MAX));
        out.extend_from_slice(&id.to_be_bytes());
    }
    out
}

/// The byte offset past the first `n` Pascal strings, or `None` when `data` has fewer.
fn pascal_end(data: &[u8], n: usize) -> Option<usize> {
    let mut at = 0usize;
    for _ in 0..n {
        at = at.saturating_add(1).saturating_add(usize::from(*data.get(at)?));
    }
    (at <= data.len()).then_some(at)
}

/// The byte offset past the first `n` Unicode strings (u32 length, then UTF-16BE).
fn unicode_end(data: &[u8], n: usize) -> Option<usize> {
    let mut at = 0usize;
    for _ in 0..n {
        let b = data.get(at..at.checked_add(4)?)?;
        let len = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
        at = at.saturating_add(4).saturating_add(len.saturating_mul(2));
    }
    (at <= data.len()).then_some(at)
}

fn first_pascal(data: &[u8], n: usize) -> Vec<u8> {
    pascal_end(data, n).and_then(|end| data.get(..end)).unwrap_or(data).to_vec()
}

fn skip_pascal(data: &[u8], n: usize) -> Vec<u8> {
    pascal_end(data, n).and_then(|end| data.get(end..)).unwrap_or(data).to_vec()
}

fn first_unicode(data: &[u8], n: usize) -> Vec<u8> {
    unicode_end(data, n).and_then(|end| data.get(..end)).unwrap_or(data).to_vec()
}

fn skip_unicode(data: &[u8], n: usize) -> Vec<u8> {
    unicode_end(data, n).and_then(|end| data.get(end..)).unwrap_or(data).to_vec()
}

fn first_display_records(data: &[u8], n: usize) -> Vec<u8> {
    let end = 4usize.saturating_add(n.saturating_mul(DISPLAY_RECORD));
    data.get(..end).unwrap_or(data).to_vec()
}

fn skip_display_records(data: &[u8], n: usize) -> Vec<u8> {
    let start = 4usize.saturating_add(n.saturating_mul(DISPLAY_RECORD));
    match (data.get(..4), data.get(start..)) {
        (Some(version), Some(rest)) => [version, rest].concat(),
        _ => data.to_vec(),
    }
}

/// The samples of an interleaved image in native byte order as the planes of a PSD merged
/// image: planar, big-endian, the first `invert` channels inverted (PSD stores CMYK ink as
/// `max - value`). `bps` is 1 (u8), 2 (u16) or 4 (f32). An error when `data` does not hold
/// `pixels × channels` samples.
pub fn interleaved_to_planar(data: &[u8], pixels: usize, channels: usize, bps: usize, invert: usize) -> Result<Vec<u8>> {
    let len = sample_bytes(pixels, channels, bps)?;
    if data.len() != len {
        return Err(PsdError::Invalid(format!("{} sample bytes where {len} were expected", data.len())));
    }
    let mut out = reserve(len)?;
    out.resize(len, 0);
    let plane = pixels.saturating_mul(bps);
    for (c, dst_plane) in out.chunks_exact_mut(plane.max(1)).enumerate().take(channels) {
        let flip = c < invert;
        for (dst, px) in dst_plane.chunks_exact_mut(bps).zip(data.chunks_exact(channels * bps)) {
            let src = px.get(c * bps..(c + 1) * bps).unwrap_or_default();
            convert(src, dst, flip, false);
        }
    }
    Ok(out)
}

/// The first `channels` planes of a PSD merged image (planar, big-endian) as interleaved
/// samples in native byte order, the first `invert` channels un-inverted. An error when `planes`
/// is shorter than those planes.
pub fn planar_to_interleaved(planes: &[u8], pixels: usize, channels: usize, bps: usize, invert: usize) -> Result<Vec<u8>> {
    let len = sample_bytes(pixels, channels, bps)?;
    let src = planes.get(..len).ok_or_else(|| PsdError::Invalid("the merged image is shorter than its header says".into()))?;
    let mut out = reserve(len)?;
    out.resize(len, 0);
    let plane = pixels.saturating_mul(bps);
    for (c, src_plane) in src.chunks_exact(plane.max(1)).enumerate().take(channels) {
        let flip = c < invert;
        for (s, px) in src_plane.chunks_exact(bps).zip(out.chunks_exact_mut(channels * bps)) {
            if let Some(dst) = px.get_mut(c * bps..(c + 1) * bps) {
                convert(s, dst, flip, true);
            }
        }
    }
    Ok(out)
}

/// One plane between big-endian (PSD) and native byte order, either way (the swap is its own
/// inverse): for a TIFF extra sample that becomes a merged-image plane, or the reverse.
pub fn swap_plane(src: &[u8], bps: usize) -> Result<Vec<u8>> {
    if !matches!(bps, 1 | 2 | 4) || !src.len().is_multiple_of(bps) {
        return Err(PsdError::Invalid("an extra channel plane is incomplete".into()));
    }
    let mut out = reserve(src.len())?;
    out.resize(src.len(), 0);
    for (s, d) in src.chunks_exact(bps).zip(out.chunks_exact_mut(bps)) {
        convert(s, d, false, true);
    }
    Ok(out)
}

fn sample_bytes(pixels: usize, channels: usize, bps: usize) -> Result<usize> {
    if !matches!(bps, 1 | 2 | 4) || channels == 0 {
        return Err(PsdError::Unsupported(format!("{channels} channels of {bps}-byte samples")));
    }
    pixels.checked_mul(channels).and_then(|n| n.checked_mul(bps)).ok_or(PsdError::LimitExceeded("image size"))
}

fn reserve(len: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.try_reserve_exact(len).map_err(|_| PsdError::OutOfMemory("layered TIFF samples"))?;
    Ok(out)
}

/// One sample between native and big-endian order (`from_be`: big-endian to native), inverted
/// when `flip`: `255 - v`, `65535 - v`, `1.0 - v`.
fn convert(src: &[u8], dst: &mut [u8], flip: bool, from_be: bool) {
    match (src, dst) {
        ([v], [d]) => *d = if flip { 255 - *v } else { *v },
        ([a, b], d @ [_, _]) => {
            let v = if from_be { u16::from_be_bytes([*a, *b]) } else { u16::from_ne_bytes([*a, *b]) };
            let v = if flip { 65535 - v } else { v };
            d.copy_from_slice(&if from_be { v.to_ne_bytes() } else { v.to_be_bytes() });
        }
        ([a, b, c, e], d @ [_, _, _, _]) => {
            let v = if from_be { f32::from_be_bytes([*a, *b, *c, *e]) } else { f32::from_ne_bytes([*a, *b, *c, *e]) };
            let v = if flip { 1.0 - v } else { v };
            d.copy_from_slice(&if from_be { v.to_ne_bytes() } else { v.to_be_bytes() });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
