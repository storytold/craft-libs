//! Photoshop layer data embedded in TIFF files (layered TIFF).
//!
//! Photoshop keeps a layered TIFF's layers in two private tags, described by Adobe's
//! *Photoshop TIFF Technical Notes* (March 2002) together with the PSD specification:
//!
//! * **37724 `ImageSourceData`**: the string `"Adobe Photoshop Document Data Block\0"`
//!   (`"Adobe Photoshop Document Data V0002\0"` for PSB-sized data, whose long keys carry
//!   8-byte lengths) followed by tagged blocks in the usual `8BIM` + key + length + data form,
//!   each padded to a multiple of 4. `Layr` (8-bit), `Lr16` and `Lr32` hold the layer info, the
//!   same structure as a PSD's layer info section; `LMsk` holds the global layer mask; every other
//!   block is a global additional-layer-information block (`Patt`, `Anno`, `FMsk`, `Txt2`, …).
//! * **34377**: the PSD image resources (`8BIM` resource blocks, without the section length).
//!
//! **Byte order.** Adobe's note is silent, but Photoshop files show (and open-source readers such as psdtags
//! implement) that the block structure follows the TIFF's byte order: in an Intel-order (`II`)
//! file every integer and double is little-endian and every four-character signature and key is
//! stored reversed (`MIB8`, `ryaL`), while the pixel samples inside channel data stay big-endian
//! as in a PSD, and RLE row-count tables follow the container. [`ImageSourceData::from_bytes`]
//! normalizes either order to the big-endian PSD model, and [`ImageSourceData::to_bytes`] writes
//! either order, by transcoding every structure it knows field by field ([`transcode`]).
//! A block the transcoder does not know is never dropped on the way in: its bytes are kept
//! verbatim with the signature `MIB8` (`46B8` for `8B64`) marking that they are in little-endian
//! layout ([`TaggedBlock::is_foreign_order`]), so a later save to a little-endian TIFF writes it
//! back exactly. Only a save to the *other* byte order has to drop such a block, with a warning:
//! a byte-swapped block passed through unchanged would be read as garbage by every other
//! application (and would end the block list for readers that stop at a bad signature).

use crate::descriptor::MAX_DEPTH;
use crate::error::{PsdError, Result};
use crate::file::{GlobalLayerMask, LayerInfoPlacement, PsdFile};
use crate::header::{Header, Version};
use crate::image_data::ImageData;
use crate::io::{Reader, WriteExt};
use crate::layer::{LayerInfo, LayerMask, mask_parameter_bytes};
use crate::resources::ImageResource;
use crate::tagged::{TaggedBlock, read_blocks, uses_long_length};

/// The byte order of a TIFF container (and so of the Photoshop data inside it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ByteOrder {
    /// Motorola order (`MM`); the PSD byte order.
    #[default]
    Big,
    /// Intel order (`II`).
    Little,
}

impl ByteOrder {
    fn is_little(self) -> bool {
        matches!(self, ByteOrder::Little)
    }
}

/// Header string of 37724 data with PSD-sized (4-byte) lengths.
pub const SIGNATURE: &[u8; 36] = b"Adobe Photoshop Document Data Block\0";
/// Header string of 37724 data with PSB-sized (8-byte) lengths for the long keys.
pub const SIGNATURE_PSB: &[u8; 36] = b"Adobe Photoshop Document Data V0002\0";

/// The layer-info block key Photoshop uses for a bit depth.
pub fn layer_info_key(depth: u16) -> [u8; 4] {
    match depth {
        16 => *b"Lr16",
        32 => *b"Lr32",
        _ => *b"Layr",
    }
}

/// The contents of an `ImageSourceData` tag, in the big-endian PSD model.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ImageSourceData {
    /// PSD (4-byte lengths) or PSB (8-byte lengths for the long keys).
    pub version: Version,
    /// The byte order the data was read in (what [`Self::to_bytes`] should normally write back).
    pub byte_order: ByteOrder,
    /// The layers, from the `Layr` / `Lr16` / `Lr32` block.
    pub layer_info: Option<LayerInfo>,
    /// Which key held the layer info (`Layr` when there was none).
    pub layer_info_key: [u8; 4],
    /// The `LMsk` block.
    pub global_layer_mask: Option<GlobalLayerMask>,
    /// Every other block, in stored order.
    pub global_blocks: Vec<TaggedBlock>,
    /// Bytes after the last block that do not form a block.
    pub trailing: Vec<u8>,
}

impl Default for ImageSourceData {
    fn default() -> Self {
        ImageSourceData {
            version: Version::Psd,
            byte_order: ByteOrder::Big,
            layer_info: None,
            layer_info_key: *b"Layr",
            global_layer_mask: None,
            global_blocks: Vec::new(),
            trailing: Vec::new(),
        }
    }
}

fn block_order(rest: &[u8]) -> Result<ByteOrder> {
    match rest.get(..4) {
        None | Some(b"8BIM") | Some(b"8B64") => Ok(ByteOrder::Big),
        Some(b"MIB8") | Some(b"46B8") => Ok(ByteOrder::Little),
        Some(other) => Err(PsdError::InvalidSignature {
            expected: "8BIM or MIB8 (Photoshop image source data block)",
            found: [other[0], other[1], other[2], other[3]],
        }),
    }
}

impl ImageSourceData {
    /// Parses a 37724 tag. Returns the data together with warnings about blocks that were
    /// dropped because they could not be converted from the file's byte order.
    pub fn from_bytes(data: &[u8]) -> Result<(Self, Vec<String>)> {
        let (version, rest) = match data.get(..36) {
            Some(s) if s == SIGNATURE => (Version::Psd, &data[36..]),
            Some(s) if s == SIGNATURE_PSB => (Version::Psb, &data[36..]),
            _ => return Err(PsdError::invalid("not Photoshop image source data (missing \"Adobe Photoshop Document Data Block\")")),
        };
        let byte_order = block_order(rest)?;
        let mut warnings = Vec::new();
        let big;
        let rest = if byte_order.is_little() {
            let (b, w) = transcode::transcode_blocks(rest, version, ByteOrder::Little, ByteOrder::Big)?;
            warnings = w;
            big = b;
            &big[..]
        } else {
            rest
        };
        let (mut blocks, trailing) = read_blocks(&mut Reader::new(rest), version)?;
        // Blocks in a TIFF are padded to 4 (`to_bytes` re-pads them), so zero padding is the
        // canonical default here, as even-length padding is in a PSD.
        for b in &mut blocks {
            if b.padding.as_ref().is_some_and(|p| p.len() <= 3 && p.iter().all(|&x| x == 0)) {
                b.padding = None;
            }
        }
        let mut layer_info = None;
        let mut layer_info_key = *b"Layr";
        if let Some(i) = blocks.iter().position(|b| matches!(&b.key, b"Layr" | b"Lr16" | b"Lr32")) {
            let b = blocks.remove(i);
            layer_info = Some(LayerInfo::read_body(&b.data, version)?);
            layer_info_key = b.key;
        }
        let global_layer_mask = blocks.iter().position(|b| &b.key == b"LMsk").map(|i| GlobalLayerMask { data: blocks.remove(i).data });
        Ok((ImageSourceData { version, byte_order, layer_info, layer_info_key, global_layer_mask, global_blocks: blocks, trailing }, warnings))
    }

    /// Serializes the data in `order`. Returns the bytes and warnings about blocks that were
    /// dropped because they could not be converted to a little-endian layout.
    pub fn to_bytes(&self, order: ByteOrder) -> Result<(Vec<u8>, Vec<String>)> {
        Self::finish(self.body()?, self.version, order)
    }

    /// [`Self::to_bytes`] that frees the layers once their block is written, before the bytes
    /// are converted to `order` (a copy of them all): a layered TIFF save holds them one time
    /// fewer (photocraft#6607).
    pub fn into_bytes(self, order: ByteOrder) -> Result<(Vec<u8>, Vec<String>)> {
        let (body, version) = (self.body()?, self.version);
        drop(self);
        Self::finish(body, version, order)
    }

    /// The signature, then the blocks in big-endian order.
    fn body(&self) -> Result<Vec<u8>> {
        let version = self.version;
        let signature = if version.is_psb() { SIGNATURE_PSB } else { SIGNATURE };
        let len_field = |key: &[u8; 4]| if uses_long_length(version, key) { 8u64 } else { 4 };
        // Reserved at its exact length, as the buffer holds all the layers' pixels: one that is a
        // few bytes short doubles at its last append, copying them once more (photocraft#6607).
        let mut len = signature.len() as u64;
        let info_pad = match &self.layer_info {
            Some(info) => {
                let pad = info.pad_len(version, 4)?;
                len = len.saturating_add(8 + len_field(&self.layer_info_key)).saturating_add(info.unpadded_len(version)?).saturating_add(pad as u64);
                pad
            }
            None => 0,
        };
        if let Some(m) = &self.global_layer_mask {
            let pad = TaggedBlock::default_padding(m.data.len(), 4);
            len = len.saturating_add(8 + len_field(b"LMsk")).saturating_add(m.data.len() as u64).saturating_add(pad as u64);
        }
        len = self.global_blocks.iter().fold(len, |n, b| n.saturating_add(b.encoded_len_aligned(version, 4)));
        len = len.saturating_add(self.trailing.len() as u64);
        let no_memory = || PsdError::OutOfMemory("not enough memory for the layer data");
        let mut body = Vec::new();
        body.try_reserve_exact(usize::try_from(len).map_err(|_| no_memory())?).map_err(|_| no_memory())?;
        body.put(signature);
        if let Some(info) = &self.layer_info {
            // Written straight into the body, padded to 4 inside its length (as a padded copy
            // would be): a separate buffer for it would hold all the pixels a second time.
            let long = uses_long_length(version, &self.layer_info_key);
            body.put(b"8BIM");
            body.put(&self.layer_info_key);
            let at = body.begin_len(long);
            info.write_unpadded(&mut body, version)?;
            body.resize(body.len() + info_pad, 0);
            body.end_len(at, long)?;
        }
        // Blocks without explicit padding are padded to 4 as they are written (alignment 4), so
        // the global mask and blocks are written from the model, without a copy of their data.
        // The signature stays (a foreign-order marker included).
        if let Some(m) = &self.global_layer_mask {
            TaggedBlock::write_parts(&mut body, *b"8BIM", *b"LMsk", &m.data, None, version, 4)?;
        }
        for b in &self.global_blocks {
            b.write(&mut body, version, 4)?;
        }
        body.put(&self.trailing);
        Ok(body)
    }

    /// The `body` (signature included) converted to `order`.
    fn finish(body: Vec<u8>, version: Version, order: ByteOrder) -> Result<(Vec<u8>, Vec<String>)> {
        if !order.is_little() {
            return Ok((body, Vec::new()));
        }
        let signature = if version.is_psb() { SIGNATURE_PSB } else { SIGNATURE };
        let blocks = body.get(signature.len()..).ok_or_else(|| PsdError::invalid("image source data without its signature"))?;
        // Converted behind the signature, into one buffer sized for both.
        transcode::transcode_blocks_after(signature, blocks, version, ByteOrder::Big, ByteOrder::Little)
    }

    /// The layers, global mask and global blocks of a PSD as image source data (byte order Big).
    pub fn from_psd(file: &PsdFile) -> Self {
        let key = match &file.layer_info_placement {
            LayerInfoPlacement::GlobalBlock { key, .. } => *key,
            LayerInfoPlacement::Section => layer_info_key(file.header.depth),
        };
        ImageSourceData {
            version: file.header.version,
            byte_order: ByteOrder::Big,
            layer_info: file.layer_info.clone(),
            layer_info_key: key,
            global_layer_mask: file.global_layer_mask.clone(),
            global_blocks: file.global_blocks.clone(),
            trailing: file.layer_mask_trailing.clone(),
        }
    }

    /// [`Self::from_psd`] that moves the layers, global mask and global blocks out of `file`
    /// instead of copying them: a layered TIFF save would otherwise hold every layer's pixels
    /// twice (photocraft#6607). `file` keeps its header, resources and merged image.
    pub fn take_from_psd(file: &mut PsdFile) -> Self {
        let key = match &file.layer_info_placement {
            LayerInfoPlacement::GlobalBlock { key, .. } => *key,
            LayerInfoPlacement::Section => layer_info_key(file.header.depth),
        };
        ImageSourceData {
            version: file.header.version,
            byte_order: ByteOrder::Big,
            layer_info: file.layer_info.take(),
            layer_info_key: key,
            global_layer_mask: file.global_layer_mask.take(),
            global_blocks: std::mem::take(&mut file.global_blocks),
            trailing: std::mem::take(&mut file.layer_mask_trailing),
        }
    }

    /// Assembles a PSD model from this data plus the parts a TIFF stores natively: the header
    /// (dimensions, depth, colour mode, channel count), the image resources (tag 34377) and the
    /// merged image (the TIFF's pixels).
    pub fn into_psd(self, header: Header, resources: Vec<ImageResource>, image_data: ImageData) -> PsdFile {
        let placement = if matches!(&self.layer_info_key, b"Lr16" | b"Lr32") && self.layer_info.is_some() {
            LayerInfoPlacement::GlobalBlock { index: 0, signature: *b"8BIM", key: self.layer_info_key, padding: None }
        } else {
            LayerInfoPlacement::Section
        };
        PsdFile {
            header,
            color_mode_data: Vec::new(),
            resources,
            layer_info: self.layer_info,
            layer_info_placement: placement,
            global_layer_mask: self.global_layer_mask,
            global_blocks: self.global_blocks,
            layer_mask_trailing: self.trailing,
            image_data,
            write_as_psb: false,
        }
    }
}

/// A block padded to a multiple of 4, as Photoshop writes them in TIFFs.
#[cfg(test)]
fn padded_block(key: [u8; 4], data: Vec<u8>) -> TaggedBlock {
    let pad = (4 - data.len() % 4) % 4;
    TaggedBlock { signature: *b"8BIM", key, data, padding: Some(vec![0; pad]), stored: None }
}

/// Parses a 34377 tag (image resources). Little-endian resource blocks (`MIB8`) are converted;
/// resource payloads other than the typed numeric ones are kept as stored.
pub fn resources_from_bytes(data: &[u8]) -> Result<(Vec<ImageResource>, Vec<String>)> {
    let order = block_order(data)?;
    let mut warnings = Vec::new();
    let big;
    let data = if order.is_little() {
        let (b, w) = transcode::transcode_resources(data, ByteOrder::Little, ByteOrder::Big)?;
        warnings = w;
        big = b;
        &big[..]
    } else {
        data
    };
    let mut r = Reader::new(data);
    let mut out = Vec::new();
    while !r.is_empty() {
        // Photoshop pads the tag to an even length; a lone trailing byte is not a resource.
        if r.remaining() < 12 && r.peek_rest().iter().all(|&b| b == 0) {
            break;
        }
        out.push(ImageResource::read(&mut r)?);
    }
    Ok((out, warnings))
}

/// Serializes image resources for a 34377 tag (big-endian, which every reader expects).
pub fn resources_to_bytes(resources: &[ImageResource]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for r in resources {
        r.write(&mut out)?;
    }
    Ok(out)
}

/// What Photoshop writes beside a layered TIFF's layer data, and how the TIFF's samples line up
/// with the PSD model's channels.
pub mod layered;

/// Field-by-field byte-order conversion of Photoshop structures.
///
/// Every function reads one structure in the source order and writes it in the target order,
/// so output lengths equal input lengths and every length field stays valid. Layouts follow the
/// Adobe PSD specification; the walker only needs to know where the integers, doubles and
/// four-character keys are, not what they mean.
pub mod transcode;

#[cfg(test)]
mod tests;
