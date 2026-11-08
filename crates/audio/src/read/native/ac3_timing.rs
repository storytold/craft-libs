//! Bounded AC-3/E-AC-3 timing from syncframe headers, without decoding or copying packets.

use super::{AudioError, Result, Src, ac3, io_error, isobmff};
use craft_media::ByteSource;

/// At most 8 MiB of additional packet-start metadata, independent of PCM duration.
const MAX_INDEX_PACKETS: usize = 1 << 20;
use crate::track::MAX_AC3_PACKET_SYNCFRAMES;

pub(super) struct Index {
    pub starts: Vec<u64>,
    pub total: u64,
}

/// Return exact primary-frame starts when all headers agree with the established stream rate.
/// A damaged header leaves container timing authoritative; I/O and resource failures propagate.
pub(super) fn index(src: &Src<'_>, samples: &[isobmff::Sample], rate: u32) -> Result<Option<Index>> {
    if samples.len() > MAX_INDEX_PACKETS {
        return Err(AudioError::TooLarge("AC-3 packet timing index exceeds its limit".into()));
    }
    let mut starts = Vec::new();
    starts.try_reserve_exact(samples.len()).map_err(|_| AudioError::TooLarge("AC-3 timing index allocation failed".into()))?;
    let mut total = 0u64;
    for sample in samples {
        starts.push(total);
        let Some(frames) = packet_frames(src, sample.offset, u64::from(sample.size), rate)? else { return Ok(None) };
        total = total.checked_add(frames).ok_or_else(|| AudioError::TooLarge("AC-3 timeline overflow".into()))?;
    }
    Ok(Some(Index { starts, total }))
}

fn packet_frames(src: &Src<'_>, offset: u64, size: u64, rate: u32) -> Result<Option<u64>> {
    prefixed_packet_frames(src, offset, size, &[], rate)
}

/// Read only the small header of each syncframe, combining a stripped prefix with stored bytes.
fn prefixed_packet_frames(src: &Src<'_>, offset: u64, stored_size: u64, prefix: &[u8], rate: u32) -> Result<Option<u64>> {
    let size = stored_size.checked_add(prefix.len() as u64).ok_or_else(|| AudioError::TooLarge("AC-3 packet size overflow".into()))?;
    let mut consumed = 0u64;
    let mut frames = 0u64;
    let mut count = 0usize;
    while consumed < size {
        if count == MAX_AC3_PACKET_SYNCFRAMES {
            return Err(AudioError::TooLarge("AC-3 packet contains too many syncframes".into()));
        }
        count += 1;
        let bytes = (size - consumed).min(16) as usize;
        let mut header = [0u8; 16];
        let prefixed = prefix.len().saturating_sub(consumed as usize).min(bytes);
        if prefixed > 0 {
            header[..prefixed].copy_from_slice(&prefix[consumed as usize..consumed as usize + prefixed]);
        }
        if prefixed < bytes {
            let stored_at = (consumed + prefixed as u64).saturating_sub(prefix.len() as u64);
            let position = offset.checked_add(stored_at).ok_or_else(|| AudioError::Malformed("AC-3 packet offset overflow".into()))?;
            src.read_at(position, &mut header[prefixed..bytes]).map_err(io_error)?;
        }
        let Ok(header) = ac3::parse_header(&header[..bytes]) else { return Ok(None) };
        if header.sample_rate != rate || header.frame_bytes == 0 || header.frame_bytes as u64 > size - consumed {
            return Ok(None);
        }
        if header.is_primary() {
            frames += header.samples() as u64;
            if frames > super::MAX_PACKET_FRAMES {
                return Err(AudioError::TooLarge("AC-3 packet duration exceeds its limit".into()));
            }
        }
        consumed += header.frame_bytes as u64;
    }
    Ok(Some(frames))
}

/// Matroska sample records omit DiscardPadding, so read its bounded BlockGroup metadata too.
/// Payloads are skipped; neither full encoded packets nor decoded PCM are retained here.
pub(super) fn mkv_index(src: &Src<'_>, track: &super::matroska::Track, rate: u32) -> Result<Option<Index>> {
    if track.samples.len() > MAX_INDEX_PACKETS {
        return Err(AudioError::TooLarge("AC-3 packet timing index exceeds its limit".into()));
    }
    let mut starts = Vec::new();
    starts.try_reserve_exact(track.samples.len()).map_err(|_| AudioError::TooLarge("AC-3 timing index allocation failed".into()))?;
    let prefix = track.stripped_header().unwrap_or_default();
    let mut total = 0u64;
    let mut last_padding = None;
    let mut exact = true;
    for sample in &track.samples {
        starts.push(total);
        let padding = match last_padding {
            Some((offset, padding)) if offset == sample.block_offset => padding,
            _ => {
                let padding = block_padding(src, sample.block_offset)?;
                last_padding = Some((sample.block_offset, padding));
                padding
            }
        };
        if padding > 0 && sample.lace > 0 {
            return Err(AudioError::Unsupported("AC-3 Matroska laced blocks with DiscardPadding".into()));
        }
        // Losing exact packet timing must not bypass unsupported padding in later blocks.
        // Continue the bounded metadata scan even after falling back to container timestamps.
        if !exact {
            continue;
        }
        let Some(frames) = prefixed_packet_frames(src, sample.offset, u64::from(sample.size), prefix, rate)? else {
            exact = false;
            continue;
        };
        let trim = super::rescale_round(padding, 1_000_000_000, u64::from(rate));
        total = total.checked_add(frames.saturating_sub(trim)).ok_or_else(|| AudioError::TooLarge("AC-3 timeline overflow".into()))?;
    }
    Ok(exact.then_some(Index { starts, total }))
}

/// Keep an explicit final block trim without turning rounded container ticks into lost PCM.
/// Packet starts already account for interior DiscardPadding; using the final packet's own
/// duration avoids reintroducing timestamp rounding or counting earlier padding twice.
pub(super) fn mkv_presentation_frames(index: &Index, track: &super::matroska::Track, rate: u32, skip: u64) -> u64 {
    let mut total = index.total;
    let (num, den) = track.timebase;
    if den > 0
        && let (Some(sample), Some(&start)) = (track.samples.last(), index.starts.last())
        && sample.duration > 0
    {
        let scale = u128::from(num) * u128::from(rate);
        let declared = (u128::from(sample.duration).saturating_mul(scale).saturating_add(u128::from(den) / 2)) / u128::from(den);
        let declared = u64::try_from(declared).unwrap_or(u64::MAX);
        // A duration can be the difference of two rounded tick endpoints: allow one tick,
        // rounded upward to a whole sample, rather than a fixed codec-sized tolerance.
        let rounding = u64::try_from(scale.div_ceil(u128::from(den))).unwrap_or(u64::MAX);
        if total.saturating_sub(start).saturating_sub(declared) > rounding {
            total = start.saturating_add(declared).min(total);
        }
    }
    total.saturating_sub(skip)
}

/// Parse a finite EBML element envelope with at most twelve header bytes.
fn element(src: &Src<'_>, offset: u64, limit: u64) -> Result<(u32, u64, u64)> {
    use super::matroska::ebml;
    let mut bytes = [0u8; 12];
    let len = limit.saturating_sub(offset).min(12) as usize;
    src.read_at(offset, &mut bytes[..len]).map_err(io_error)?;
    let (id, id_len) = ebml::read_id(&bytes[..len]).ok_or_else(|| AudioError::Malformed("invalid Matroska block element ID".into()))?;
    let (size, size_len) = ebml::read_size(&bytes[id_len..len]).ok_or_else(|| AudioError::Malformed("invalid Matroska block element size".into()))?;
    let start = offset.checked_add((id_len + size_len) as u64).ok_or_else(|| AudioError::Malformed("Matroska block offset overflow".into()))?;
    let end = size
        .known()
        .and_then(|n| start.checked_add(n))
        .filter(|&n| n <= limit)
        .ok_or_else(|| AudioError::Malformed("invalid Matroska block bounds".into()))?;
    Ok((id, start, end))
}

fn block_padding(src: &Src<'_>, offset: u64) -> Result<u64> {
    use super::matroska::element_ids::{BLOCK_GROUP, DISCARD_PADDING, SIMPLE_BLOCK};
    let (id, mut at, end) = element(src, offset, src.size())?;
    if id == SIMPLE_BLOCK {
        return Ok(0);
    }
    if id != BLOCK_GROUP {
        return Err(AudioError::Malformed("unexpected Matroska block type".into()));
    }
    let mut padding = 0;
    for _ in 0..4096 {
        if at == end {
            return Ok(padding);
        }
        let (id, start, next) = element(src, at, end)?;
        if id == DISCARD_PADDING {
            let length = (next - start) as usize;
            if !(1..=8).contains(&length) {
                return Err(AudioError::Malformed("invalid Matroska discard padding".into()));
            }
            let mut bytes = [0u8; 8];
            src.read_at(start, &mut bytes[8 - length..]).map_err(io_error)?;
            // The native reader trims packet ends only. Treating a beginning trim as zero
            // would silently present padding and make the advertised exact timeline wrong.
            if bytes[8 - length] & 0x80 != 0 {
                return Err(AudioError::Unsupported("AC-3 Matroska negative DiscardPadding".into()));
            }
            padding = u64::from_be_bytes(bytes);
        }
        at = next;
    }
    Err(AudioError::TooLarge("too many Matroska block metadata elements".into()))
}

#[cfg(all(test, feature = "ac3-encode", feature = "eac3-encode"))]
#[path = "ac3_timing_tests.rs"]
mod tests;
