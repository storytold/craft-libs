//! Borrowed, merged-only PSD reader that skips layer records and decodes selected planes.
use crate::{Head, HeadView, PsdError, Result};

mod rows;
pub use rows::{ChannelRows, EncodedRow, EncodedRows};

/// Dimensions in the fixed header fields, for reporting a preflight limit error.
/// This does not validate the PSD signature or remaining header.
pub fn declared_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let h = bytes.get(14..18)?;
    let w = bytes.get(18..22)?;
    Some((u32::from_be_bytes([w[0], w[1], w[2], w[3]]), u32::from_be_bytes([h[0], h[1], h[2], h[3]])))
}

/// The file head and merged-composite location. Layer records are not parsed or allocated.
pub struct Merged<'a> {
    /// Header, color-mode data, and image resources.
    pub head: Head,
    /// Whether the layer count marks merged transparency.
    pub transparency: bool,
    /// Merged image compression identifier.
    pub compression: u16,
    bytes: &'a [u8],
    data_offset: usize,
}

fn take<'a>(bytes: &'a [u8], at: &mut usize, len: usize) -> Result<&'a [u8]> {
    let end = at.checked_add(len).ok_or(PsdError::LimitExceeded("offset"))?;
    let data = bytes.get(*at..end).ok_or_else(|| PsdError::invalid("truncated"))?;
    *at = end;
    Ok(data)
}
fn u16_at(bytes: &[u8], at: &mut usize) -> Result<u16> {
    let b = take(bytes, at, 2)?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}
fn u32_at(bytes: &[u8], at: &mut usize) -> Result<u32> {
    let b = take(bytes, at, 4)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}
fn u64_at(bytes: &[u8], at: &mut usize) -> Result<u64> {
    let b = take(bytes, at, 8)?;
    Ok(u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

fn has_merged_transparency(layer: &[u8], version: crate::Version) -> bool {
    let mut at = 0usize;
    let info_len = if version.is_psb() { u64_at(layer, &mut at).ok() } else { u32_at(layer, &mut at).ok().map(u64::from) };
    let Some(info_len) = info_len else { return false };
    if info_len >= 2 {
        return layer.get(at..at + 2).is_some_and(|b| i16::from_be_bytes([b[0], b[1]]) < 0);
    }
    let Ok(info_len) = usize::try_from(info_len) else { return false };
    let Some(next) = at.checked_add(info_len) else { return false };
    at = next;
    let Ok(mask_len) = u32_at(layer, &mut at) else { return false };
    let Some(next) = at.checked_add(mask_len as usize) else { return false };
    at = next;
    while let Some(head) = at.checked_add(12).and_then(|end| layer.get(at..end)) {
        let sig = &head[..4];
        let key = &head[4..8];
        if sig != b"8BIM" && sig != b"8B64" {
            return false;
        }
        let Ok(key_arr): std::result::Result<&[u8; 4], _> = key.try_into() else { return false };
        let len = if crate::tagged::uses_long_length(version, key_arr) {
            let Ok(l) = u64_at(layer, &mut { at + 8 }) else { return false };
            let Ok(l) = usize::try_from(l) else { return false };
            at += 16;
            l
        } else {
            at += 12;
            u32::from_be_bytes([head[8], head[9], head[10], head[11]]) as usize
        };
        if (key == b"Lr16" || key == b"Lr32") && layer.get(at..at + 2).is_some_and(|b| i16::from_be_bytes([b[0], b[1]]) < 0) {
            return true;
        }
        if matches!(key, b"Mtrn" | b"MTrn" | b"Mt16" | b"Mt32") {
            return true;
        }
        // Global tagged blocks pad their data to 4 bytes (per psd-tools and craft-psd).
        let Some(next) = len.checked_add(3).map(|v| v & !3).and_then(|p| at.checked_add(p)) else { return false };
        at = next;
    }
    false
}

impl<'a> Merged<'a> {
    /// Open the merged section with a pixel limit. This skips layer records, preserving the ability
    /// to read a composite when its layer data is not understood. No image planes are allocated.
    pub fn open(bytes: &'a [u8], max_pixels: u64) -> Result<Self> {
        Self::check_pixel_limit(bytes, max_pixels)?;
        Self::from_head(bytes, Head::from_bytes(bytes)?, max_pixels)
    }

    /// Open the merged section while omitting all image resources, including valid records.
    ///
    /// This validates the header and declared section boundaries but skips the resource records.
    /// Callers choose when to use it, for example after detecting damaged resources during a strict
    /// open. The returned reader borrows the original file and only copies its color-mode data;
    /// no resource payloads or image planes are allocated.
    pub fn open_without_resources(bytes: &'a [u8], max_pixels: u64) -> Result<Self> {
        Self::check_pixel_limit(bytes, max_pixels)?;
        let view = HeadView::from_bytes(bytes)?;
        let mut color_mode_data = Vec::new();
        color_mode_data.try_reserve_exact(view.color_mode_data.len()).map_err(|_| PsdError::OutOfMemory("color-mode data"))?;
        color_mode_data.extend_from_slice(view.color_mode_data);
        let head = Head { header: view.header, color_mode_data, resources: Vec::new(), layer_section_offset: view.layer_section_offset };
        Self::from_head(bytes, head, max_pixels)
    }

    fn check_pixel_limit(bytes: &[u8], max_pixels: u64) -> Result<()> {
        // Refuse a declared oversized canvas before copying color-mode data or resources.
        if let Some((w, h)) = declared_dimensions(bytes)
            && u64::from(w) * u64::from(h) > max_pixels
        {
            return Err(PsdError::LimitExceeded("pixel count"));
        }
        Ok(())
    }

    fn from_head(bytes: &'a [u8], head: Head, max_pixels: u64) -> Result<Self> {
        let h = &head.header;
        if u64::from(h.width) * u64::from(h.height) > max_pixels {
            return Err(PsdError::LimitExceeded("pixel count"));
        }
        let psb = h.version.is_psb();
        let mut at = usize::try_from(head.layer_section_offset).map_err(|_| PsdError::LimitExceeded("offset"))?;
        let len = if psb { u64_at(bytes, &mut at)? } else { u64::from(u32_at(bytes, &mut at)?) };
        let layer = take(bytes, &mut at, usize::try_from(len).map_err(|_| PsdError::LimitExceeded("layer section"))?)?;
        let transparency = has_merged_transparency(layer, h.version);
        let compression = u16_at(bytes, &mut at)?;
        Ok(Self { head, transparency, compression, bytes, data_offset: at })
    }

    /// Check that all declared raw data or RLE row payloads fit without decoding them.
    pub fn validate_all(&self) -> Result<()> {
        let h = &self.head.header;
        let bpc = match h.depth {
            8 => 1usize,
            16 => 2,
            32 => 4,
            _ => return Err(PsdError::Unsupported("merged depth".into())),
        };
        let channels = usize::from(h.channels);
        let height = h.height as usize;
        let mut at = self.data_offset;
        match self.compression {
            0 => {
                let len = (h.width as usize)
                    .checked_mul(height)
                    .and_then(|n| n.checked_mul(bpc))
                    .and_then(|n| n.checked_mul(channels))
                    .ok_or(PsdError::LimitExceeded("merged data"))?;
                take(self.bytes, &mut at, len)?;
            }
            1 => {
                let rows = channels.checked_mul(height).ok_or(PsdError::LimitExceeded("row count"))?;
                let entry = if h.version.is_psb() { 4 } else { 2 };
                let table_len = rows.checked_mul(entry).ok_or(PsdError::LimitExceeded("row table"))?;
                let table = take(self.bytes, &mut at, table_len)?;
                let mut total = 0usize;
                for chunk in table.chunks_exact(entry) {
                    let len = if entry == 2 {
                        usize::from(u16::from_be_bytes([chunk[0], chunk[1]]))
                    } else {
                        u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as usize
                    };
                    total = total.saturating_add(len);
                }
                take(self.bytes, &mut at, total)?;
            }
            _ => return Err(PsdError::Unsupported("merged compression".into())),
        }
        Ok(())
    }

    /// Decode only the first `used` planes, leaving spot channels untouched. Eight-, sixteen-, and
    /// thirty-two-bit raw or PackBits samples are returned as planar big-endian bytes. Before any
    /// RLE plane allocation, encoded row lengths must fit the input and the PackBits expansion bound.
    pub fn read_planes(&self, used: usize) -> Result<Vec<Vec<u8>>> {
        let h = &self.head.header;
        let channels = usize::from(h.channels);
        if used > channels {
            return Err(PsdError::invalid("too few channels"));
        }
        let bpc = match h.depth {
            8 => 1usize,
            16 => 2,
            32 => 4,
            _ => return Err(PsdError::Unsupported("merged depth".into())),
        };
        let width = h.width as usize;
        let height = h.height as usize;
        let row = width.checked_mul(bpc).ok_or(PsdError::LimitExceeded("row size"))?;
        let plane = row.checked_mul(height).ok_or(PsdError::LimitExceeded("plane size"))?;
        let mut at = self.data_offset;
        let mut planes = Vec::new();
        planes.try_reserve_exact(used).map_err(|_| PsdError::OutOfMemory("planes"))?;
        match self.compression {
            0 => {
                for _ in 0..used {
                    planes.push(take(self.bytes, &mut at, plane)?.to_vec());
                }
            }
            1 => {
                let entry = if h.version.is_psb() { 4 } else { 2 };
                let table_len = channels.checked_mul(height).and_then(|v| v.checked_mul(entry)).ok_or(PsdError::LimitExceeded("row table"))?;
                let table = take(self.bytes, &mut at, table_len)?;
                let count = |i: usize| -> usize {
                    let pos = i.saturating_mul(entry);
                    match table.get(pos..pos.saturating_add(entry)) {
                        Some(&[a, b]) => usize::from(u16::from_be_bytes([a, b])),
                        Some(&[a, b, c, d]) => u32::from_be_bytes([a, b, c, d]) as usize,
                        _ => 0,
                    }
                };
                let min_row = row.div_ceil(128).saturating_mul(2);
                let mut total = 0usize;
                for i in 0..used.saturating_mul(height) {
                    let n = count(i);
                    if n < min_row {
                        return Err(PsdError::invalid("RLE row too short for its width"));
                    }
                    total = total.saturating_add(n);
                }
                if total > self.bytes.len().saturating_sub(at) {
                    return Err(PsdError::invalid("truncated"));
                }
                for c in 0..used {
                    let mut decoded = Vec::new();
                    decoded.try_reserve_exact(plane).map_err(|_| PsdError::OutOfMemory("plane"))?;
                    decoded.resize(plane, 0);
                    for (y, row) in decoded.chunks_exact_mut(row).enumerate() {
                        let encoded = take(self.bytes, &mut at, count(c * height + y))?;
                        unpackbits(encoded, row)?;
                    }
                    planes.push(decoded);
                }
            }
            _ => return Err(PsdError::Unsupported("merged compression".into())),
        }
        Ok(planes)
    }
}

/// Decode one PackBits row. Truncated packets leave the pre-zeroed remainder unchanged; packets
/// that overrun the output are rejected. Trailing encoded data after the row is ignored.
pub fn unpackbits(mut src: &[u8], row: &mut [u8]) -> Result<()> {
    let mut at = 0;
    while at < row.len() {
        let Some((&h, rest)) = src.split_first() else { break };
        src = rest;
        let h = h as i8;
        if h >= 0 {
            let n = h as usize + 1;
            let lit = src.get(..n).unwrap_or(src);
            let dst = row.get_mut(at..at + lit.len()).ok_or_else(|| PsdError::invalid("RLE row overruns its width"))?;
            dst.copy_from_slice(lit);
            at += lit.len();
            src = src.get(n..).unwrap_or(&[]);
        } else if h != -128 {
            let n = (1 - h as isize) as usize;
            let Some((&b, rest)) = src.split_first() else { break };
            src = rest;
            row.get_mut(at..at + n).ok_or_else(|| PsdError::invalid("RLE row overruns its width"))?.fill(b);
            at += n;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(channels: u16, compression: u16) -> Vec<u8> {
        let mut b = b"8BPS\0\x01\0\0\0\0\0\0".to_vec();
        b.extend_from_slice(&channels.to_be_bytes());
        b.extend_from_slice(&2u32.to_be_bytes());
        b.extend_from_slice(&2u32.to_be_bytes());
        b.extend_from_slice(&16u16.to_be_bytes());
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&compression.to_be_bytes());
        b
    }

    #[test]
    fn reads_only_selected_rle_plane_without_extra_channel_data() {
        let mut b = head(2, 1);
        for _ in 0..4 {
            b.extend_from_slice(&2u16.to_be_bytes());
        }
        b.extend_from_slice(&[0xfd, 0x80, 0xfd, 0x80]);
        let merged = Merged::open(&b, 4).unwrap();
        assert_eq!(merged.read_planes(1).unwrap(), vec![vec![0x80; 8]]);
        assert!(merged.read_planes(2).is_err());
        assert!(merged.validate_all().is_err());
        assert!(Merged::open(&b, 3).is_err());
    }

    #[test]
    fn skips_invalid_layer_contents_and_checks_rle_packets() {
        let mut b = head(1, 0);
        let layer_offset = 34;
        b.splice(layer_offset..layer_offset + 4, [0, 0, 0, 3, 0xff, 0xfe, 0xfd]);
        b.extend_from_slice(&[1; 8]);
        let merged = Merged::open(&b, 4).unwrap();
        assert_eq!(merged.read_planes(1).unwrap(), vec![vec![1; 8]]);
        assert!(merged.validate_all().is_ok());
        let mut row = [0; 4];
        unpackbits(&[3, 7, 8], &mut row).unwrap();
        assert_eq!(row, [7, 8, 0, 0]);
        assert!(unpackbits(&[0x81, 1], &mut row).is_err());
    }

    #[test]
    fn detects_lr16_and_mt16_merged_transparency() {
        let with_layer = |layer: &[u8]| {
            let mut b = head(2, 0);
            b.splice(34..38, (layer.len() as u32).to_be_bytes().iter().copied().chain(layer.iter().copied()));
            b
        };
        let mut lr16 = Vec::new();
        lr16.extend_from_slice(&0u32.to_be_bytes());
        lr16.extend_from_slice(&0u32.to_be_bytes());
        lr16.extend_from_slice(b"8BIMPatt");
        lr16.extend_from_slice(&6u32.to_be_bytes());
        lr16.extend_from_slice(&[1, 2, 3, 4, 5, 6, 0, 0]);
        lr16.extend_from_slice(b"8BIMLr16");
        lr16.extend_from_slice(&2u32.to_be_bytes());
        lr16.extend_from_slice(&(-1i16).to_be_bytes());
        assert!(Merged::open(&with_layer(&lr16), 4).unwrap().transparency);

        let n = lr16.len();
        lr16[n - 2..].copy_from_slice(&1i16.to_be_bytes());
        assert!(!Merged::open(&with_layer(&lr16), 4).unwrap().transparency);

        let mut mt16 = vec![0; 8];
        mt16.extend_from_slice(b"8BIMMt16\0\0\0\0");
        assert!(Merged::open(&with_layer(&mt16), 4).unwrap().transparency);
    }

    #[test]
    fn resource_skipping_open_reads_psd_raw_and_psb_packbits_from_original_bytes() {
        for (version, compression) in [(1, 0), (2, 1)] {
            let valid_resources = recovery_resource();
            let mut damaged_resources = valid_resources.clone();
            damaged_resources.extend_from_slice(b"BAD!");
            let (bytes, data_offset) = recovery_file(version, compression, &damaged_resources);
            let (control_bytes, _) = recovery_file(version, compression, &[]);
            let control = Merged::open(&control_bytes, 4).unwrap();

            assert!(Head::from_bytes(&bytes).is_err());
            assert!(Merged::open(&bytes, 4).is_err());
            let recovered = Merged::open_without_resources(&bytes, 4).unwrap();
            assert!(std::ptr::eq(recovered.bytes, bytes.as_slice()));
            assert_eq!(recovered.data_offset, data_offset);
            assert_eq!(recovered.head.header, control.head.header);
            assert_eq!(recovered.head.color_mode_data, [9, 8]);
            assert!(recovered.head.resources.is_empty());
            assert_eq!(recovered.compression, compression);
            assert!(recovered.transparency);
            assert_eq!(recovered.transparency, control.transparency);
            assert_eq!(recovered.read_planes(2).unwrap(), vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]]);
            assert_eq!(recovered.read_planes(2).unwrap(), control.read_planes(2).unwrap());
            recovered.validate_all().unwrap();

            let (valid_bytes, _) = recovery_file(version, compression, &valid_resources);
            let strict = Merged::open(&valid_bytes, 4).unwrap();
            assert_eq!(strict.head.resources.len(), 1);
            assert_eq!(strict.head.resources[0].id, 1039);
            assert_eq!(strict.head.resources[0].data, [7, 6]);
            let skipped = Merged::open_without_resources(&valid_bytes, 4).unwrap();
            assert!(skipped.head.resources.is_empty());
            assert_eq!(skipped.read_planes(2).unwrap(), strict.read_planes(2).unwrap());
        }
    }

    #[test]
    fn resource_skipping_open_ignores_truncated_resource_payload() {
        for version in [1, 2] {
            let mut resources = recovery_resource();
            resources[8..12].copy_from_slice(&100u32.to_be_bytes());
            let (bytes, _) = recovery_file(version, 0, &resources);
            assert!(Merged::open(&bytes, 4).is_err());
            let recovered = Merged::open_without_resources(&bytes, 4).unwrap();
            assert!(recovered.head.resources.is_empty());
            assert_eq!(recovered.read_planes(1).unwrap(), vec![vec![1, 2, 3, 4]]);
        }
    }

    #[test]
    fn resource_skipping_open_checks_pixel_limit_before_malformed_head() {
        let (mut bytes, _) = recovery_file(1, 0, b"BAD!");
        bytes.truncate(22);
        bytes[..4].copy_from_slice(b"BAD!");
        assert!(matches!(Merged::open_without_resources(&bytes, 3), Err(PsdError::LimitExceeded("pixel count"))));
        assert!(matches!(Merged::open(&bytes, 3), Err(PsdError::LimitExceeded("pixel count"))));
    }

    #[test]
    fn resource_skipping_open_rejects_out_of_bounds_sections_and_tail_framing() {
        for version in [1, 2] {
            let (bytes, data_offset) = recovery_file(version, 0, b"BAD!");
            let mut invalid_header = bytes.clone();
            invalid_header[..4].copy_from_slice(b"BAD!");
            assert!(Merged::open_without_resources(&invalid_header, 4).is_err());

            let mut color_outside_file = bytes.clone();
            color_outside_file[26..30].copy_from_slice(&u32::MAX.to_be_bytes());
            assert!(Merged::open_without_resources(&color_outside_file, 4).is_err());
            let mut resources_outside_file = bytes.clone();
            resources_outside_file[32..36].copy_from_slice(&u32::MAX.to_be_bytes());
            assert!(Merged::open_without_resources(&resources_outside_file, 4).is_err());

            let layer_offset = crate::HeadView::from_bytes(&bytes).unwrap().layer_section_offset as usize;
            let length_width = if version == 2 { 8 } else { 4 };
            assert!(Merged::open_without_resources(&bytes[..layer_offset + length_width - 1], 4).is_err());
            let mut layer_outside_file = bytes.clone();
            layer_outside_file[layer_offset..layer_offset + length_width].fill(0xff);
            assert!(Merged::open_without_resources(&layer_outside_file, 4).is_err());
            assert!(Merged::open_without_resources(&bytes[..data_offset - 1], 4).is_err());
        }
    }

    #[test]
    fn resource_skipping_open_preserves_selected_plane_and_full_validation_distinction() {
        for (version, compression) in [(1, 0), (2, 1)] {
            let (mut bytes, _) = recovery_file(version, compression, b"BAD!");
            bytes.truncate(bytes.len() - if compression == 0 { 4 } else { 6 });
            let recovered = Merged::open_without_resources(&bytes, 4).unwrap();
            assert_eq!(recovered.read_planes(1).unwrap(), vec![vec![1, 2, 3, 4]]);
            assert!(recovered.read_planes(2).is_err());
            assert!(recovered.validate_all().is_err());
        }
    }

    fn recovery_resource() -> Vec<u8> {
        let mut bytes = b"8BIM".to_vec();
        bytes.extend_from_slice(&1039u16.to_be_bytes());
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&2u32.to_be_bytes());
        bytes.extend_from_slice(&[7, 6]);
        bytes
    }

    fn recovery_file(version: u16, compression: u16, resources: &[u8]) -> (Vec<u8>, usize) {
        let mut bytes = b"8BPS".to_vec();
        bytes.extend_from_slice(&version.to_be_bytes());
        bytes.extend_from_slice(&[0; 6]);
        bytes.extend_from_slice(&2u16.to_be_bytes());
        bytes.extend_from_slice(&2u32.to_be_bytes());
        bytes.extend_from_slice(&2u32.to_be_bytes());
        bytes.extend_from_slice(&8u16.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&2u32.to_be_bytes());
        bytes.extend_from_slice(&[9, 8]);
        bytes.extend_from_slice(&(resources.len() as u32).to_be_bytes());
        bytes.extend_from_slice(resources);
        let mut layer = Vec::new();
        if version == 2 {
            layer.extend_from_slice(&2u64.to_be_bytes());
        } else {
            layer.extend_from_slice(&2u32.to_be_bytes());
        }
        layer.extend_from_slice(&(-1i16).to_be_bytes());
        if version == 2 {
            bytes.extend_from_slice(&(layer.len() as u64).to_be_bytes());
        } else {
            bytes.extend_from_slice(&(layer.len() as u32).to_be_bytes());
        }
        bytes.extend_from_slice(&layer);
        bytes.extend_from_slice(&compression.to_be_bytes());
        let data_offset = bytes.len();
        if compression == 0 {
            bytes.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        } else {
            for _ in 0..4 {
                if version == 2 {
                    bytes.extend_from_slice(&3u32.to_be_bytes());
                } else {
                    bytes.extend_from_slice(&3u16.to_be_bytes());
                }
            }
            for row in [[1, 2], [3, 4], [5, 6], [7, 8]] {
                bytes.push(1);
                bytes.extend_from_slice(&row);
            }
        }
        (bytes, data_offset)
    }
}
