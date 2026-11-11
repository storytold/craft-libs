//! Block and frame parsing, lacing expansion and cluster scanning.

use super::io::{Hdr, Io};
use super::{FrameBudget, MAX_LACED_BODY, warn};
use crate::matroska::ebml;
use crate::matroska::ebml::{Lacing, Size, parse_lacing};
use crate::matroska::error::Result;
use crate::matroska::ids::{
    BLOCK, BLOCK_ADD_ID, BLOCK_ADDITIONAL, BLOCK_ADDITIONS, BLOCK_DURATION, BLOCK_GROUP, BLOCK_MORE, DISCARD_PADDING, REFERENCE_BLOCK, SIMPLE_BLOCK,
    TIMESTAMP, is_top_level,
};
use crate::matroska::meta::ClusterInfo;
use crate::matroska::track::Track;
use crate::source::ByteSource;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub(super) struct BlockRef {
    pub(super) track: usize,
    pub(super) block_offset: u64,
    /// Timestamp in ticks (cluster + relative).
    pub(super) ts: i64,
    /// Absolute (offset, size) of each frame.
    pub(super) frames: Vec<(u64, u32)>,
    pub(super) block_duration: Option<u64>,
    pub(super) keyframe: bool,
    pub(super) discardable: bool,
    pub(super) invisible: bool,
    pub(super) discard_padding: Option<i64>,
    /// (offset, size) of the `BlockAdditional` with `BlockAddID` 1 (WebM: the VP8/VP9 alpha
    /// layer), kept for unlaced blocks only.
    pub(super) addition: Option<(u64, u32)>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FrameRef {
    pub(super) track: usize,
    pub(super) offset: u64,
    pub(super) size: u32,
    pub(super) pts: i64,
    pub(super) pts_ns: i64,
    pub(super) duration: u64,
    pub(super) duration_ns: u64,
    pub(super) keyframe: bool,
    pub(super) discardable: bool,
    pub(super) invisible: bool,
    pub(super) discard_padding: Option<i64>,
    pub(super) block_offset: u64,
    pub(super) lace: u16,
    pub(super) addition: Option<(u64, u32)>,
}

/// Track number to track index. A duplicate number resolves to its first track.
pub(super) type TrackIndex = HashMap<u64, usize>;

pub(super) fn track_index(tracks: &[Track]) -> TrackIndex {
    let mut m = HashMap::with_capacity(tracks.len());
    for (i, t) in tracks.iter().enumerate() {
        m.entry(t.number).or_insert(i);
    }
    m
}

/// A parsed block header: (track index, relative ts, flags, frames as (offset, size)).
pub(super) type BlockHeader = (usize, i16, u8, Vec<(u64, u32)>);

/// Parse the Block/SimpleBlock header at `data_start` (`len` bytes).
/// Returns `None` for unknown tracks / damage.
pub(super) fn parse_block<S: ByteSource + ?Sized>(io: &mut Io<'_, S>, data_start: u64, len: u64, lookup: &TrackIndex) -> Result<Option<BlockHeader>> {
    let head = io.peek(data_start, 16.min(len as usize))?;
    let Some((tn, tl, _)) = ebml::read_vint(head) else { return Ok(None) };
    if head.len() < tl + 3 {
        return Ok(None);
    }
    let rel = i16::from_be_bytes([head[tl], head[tl + 1]]);
    let flags = head[tl + 2];
    let Some(&track) = lookup.get(&tn) else { return Ok(None) };
    let hdr = (tl + 3) as u64;
    let lacing = Lacing::from_flags(flags);
    let frames = if lacing == Lacing::None {
        let Ok(size) = u32::try_from(len.saturating_sub(hdr)) else { return Ok(None) };
        vec![(data_start + hdr, size)]
    } else {
        let body_len = len.saturating_sub(hdr);
        if body_len > MAX_LACED_BODY {
            return Ok(None);
        }
        let body = io.read_vec(data_start + hdr, body_len)?;
        let Ok(laced) = parse_lacing(lacing, &body) else { return Ok(None) };
        let mut frames = Vec::with_capacity(laced.len());
        for (o, l) in laced {
            let Ok(size) = u32::try_from(l) else { return Ok(None) };
            frames.push((data_start + hdr + o as u64, size));
        }
        frames
    };
    Ok(Some((track, rel, flags, frames)))
}

/// The `BlockAdditional` with `BlockAddID` 1 (the default ID) in a `BlockAdditions` element:
/// (offset, size) of its data. Other IDs, empty additions and damaged children are skipped.
pub(super) fn block_addition<S: ByteSource + ?Sized>(io: &mut Io<'_, S>, start: u64, end: u64) -> Result<Option<(u64, u32)>> {
    let mut pos = start;
    while pos < end {
        let Some(more) = io.header(pos, end)? else { break };
        let me = more.end(end);
        if more.id == BLOCK_MORE {
            let (mut id, mut data) = (1u64, None);
            let mut p = more.data_start();
            while p < me {
                let Some(c) = io.header(p, me)? else { break };
                let ce = c.end(me);
                let len = ce.saturating_sub(c.data_start());
                match c.id {
                    BLOCK_ADD_ID => id = ebml::uint(io.peek(c.data_start(), len.min(8) as usize)?),
                    BLOCK_ADDITIONAL => data = u32::try_from(len).ok().filter(|&n| n > 0).map(|n| (c.data_start(), n)),
                    _ => {}
                }
                p = ce;
            }
            if id == 1 && data.is_some() {
                return Ok(data);
            }
        }
        pos = me;
    }
    Ok(None)
}

/// Cluster timestamps are unsigned; a hostile value beyond `i64::MAX` saturates, so adding a
/// block's relative timestamp cannot overflow.
pub(super) fn sat_i64(v: i128) -> i64 {
    v.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

pub(super) fn cluster_timestamp(raw: u64) -> i64 {
    i64::try_from(raw).unwrap_or(i64::MAX)
}

/// Parse one SimpleBlock or BlockGroup element.
pub(super) fn parse_block_elem<S: ByteSource + ?Sized>(
    io: &mut Io<'_, S>,
    h: &Hdr,
    end: u64,
    cluster_ts: i64,
    lookup: &TrackIndex,
) -> Result<Option<BlockRef>> {
    let ds = h.data_start();
    match h.id {
        SIMPLE_BLOCK => {
            let Some((track, rel, flags, frames)) = parse_block(io, ds, end - ds, lookup)? else { return Ok(None) };
            Ok(Some(BlockRef {
                track,
                block_offset: h.pos,
                ts: cluster_ts.saturating_add(i64::from(rel)),
                frames,
                block_duration: None,
                keyframe: flags & 0x80 != 0,
                discardable: flags & 0x01 != 0,
                invisible: flags & 0x08 != 0,
                discard_padding: None,
                addition: None,
            }))
        }
        BLOCK_GROUP => {
            let mut pos = ds;
            let mut block = None;
            let (mut dur, mut has_ref, mut pad, mut addition) = (None, false, None, None);
            while pos < end {
                let Some(c) = io.header(pos, end)? else { break };
                let ce = c.end(end);
                match c.id {
                    BLOCK => block = parse_block(io, c.data_start(), ce - c.data_start(), lookup)?,
                    BLOCK_ADDITIONS => addition = block_addition(io, c.data_start(), ce)?,
                    BLOCK_DURATION => dur = Some(ebml::uint(io.peek(c.data_start(), (ce - c.data_start()) as usize)?)),
                    REFERENCE_BLOCK => has_ref = true,
                    DISCARD_PADDING => pad = Some(ebml::int(io.peek(c.data_start(), (ce - c.data_start()) as usize)?)),
                    _ => {}
                }
                pos = ce;
            }
            let Some((track, rel, flags, frames)) = block else { return Ok(None) };
            // with lacing, an addition belongs to the block, not to one of its frames
            let addition = addition.filter(|_| frames.len() == 1);
            Ok(Some(BlockRef {
                track,
                block_offset: h.pos,
                ts: cluster_ts.saturating_add(i64::from(rel)),
                frames,
                block_duration: dur,
                keyframe: !has_ref,
                discardable: false,
                invisible: flags & 0x08 != 0,
                discard_padding: pad,
                addition,
            }))
        }
        _ => Ok(None),
    }
}

/// Expand a block into frames with per-lace timestamps.
pub(super) fn expand(b: &BlockRef, t: &Track, scale: u64, skip: u16, out: &mut impl Extend<FrameRef>) {
    let n = b.frames.len() as u64;
    // total block duration in ns
    let block_ns: Option<u128> = match (b.block_duration, t.default_duration_ns) {
        (Some(d), _) => Some(d as u128 * scale as u128),
        (None, Some(dd)) => Some(dd as u128 * n as u128),
        _ => None,
    };
    let scale_i = scale as i128;
    // round half away from zero
    let to_ticks = |ns: i128| -> i64 { sat_i64(if ns >= 0 { (ns + scale_i / 2) / scale_i } else { (ns - scale_i / 2) / scale_i }) };
    // CodecDelay is subtracted from every timestamp (RFC 9559 §5.1.4.1.25); in ticks it is rounded
    // once, so tick timestamps stay on the block grid.
    let delay_ns = t.codec_delay_ns as i128;
    let delay_ticks = to_ticks(delay_ns);
    // `parse_info` caps the scale at 2^32, which keeps these products within i128; results
    // outside i64 saturate.
    let base_ns = b.ts as i128 * scale_i;
    out.extend(b.frames.iter().enumerate().skip(skip as usize).map(|(i, &(offset, size))| {
        let (start, end) = match block_ns {
            Some(total) => ((total * i as u128 / n as u128) as i128, Some((total * (i as u128 + 1) / n as u128) as i128)),
            None => (0, None),
        };
        let pts_ns = base_ns + start - delay_ns;
        let pts = to_ticks(base_ns + start).saturating_sub(delay_ticks);
        let duration = end.map_or(0, |e| to_ticks(e).saturating_sub(to_ticks(start)).max(0) as u64);
        FrameRef {
            track: b.track,
            offset,
            size,
            pts,
            pts_ns: sat_i64(pts_ns),
            duration,
            duration_ns: end.map_or(0, |e| u64::try_from((e - start).max(0)).unwrap_or(u64::MAX)),
            keyframe: b.keyframe,
            discardable: b.discardable,
            invisible: b.invisible,
            discard_padding: b.discard_padding,
            block_offset: b.block_offset,
            lace: i as u16,
            addition: b.addition,
        }
    }));
}

/// Scan a whole cluster (headers only), returning its info and blocks.
pub(super) fn scan_cluster<S: ByteSource + ?Sized>(
    io: &mut Io<'_, S>,
    h: &Hdr,
    limit: u64,
    lookup: &TrackIndex,
    budget: &mut FrameBudget,
    warnings: &mut Vec<String>,
) -> Result<(ClusterInfo, Vec<BlockRef>)> {
    let ds = h.data_start();
    let known_end = match h.size {
        Size::Known(_) => Some(h.end(limit)),
        Size::Unknown => None,
    };
    let lim = known_end.unwrap_or(limit);
    let mut ts: i64 = 0;
    let mut blocks = Vec::new();
    let mut pos = ds;
    let end = loop {
        if pos >= lim {
            break lim;
        }
        let Some(c) = io.header(pos, lim)? else {
            warn(warnings, format!("damaged cluster at {}: undecodable element at {pos}", h.pos));
            break io.resync(pos + 1, limit)?.unwrap_or(limit);
        };
        if known_end.is_none() && is_top_level(c.id) {
            break pos;
        }
        if c.size == Size::Unknown || c.overruns(lim) {
            warn(warnings, format!("damaged cluster at {}: element 0x{:X} at {pos} overruns", h.pos, c.id));
            if c.overruns(lim) && matches!(c.id, SIMPLE_BLOCK | BLOCK_GROUP) && io.len <= c.data_start() + c.size.known().unwrap_or(0) {
                break lim; // truncated file: drop the partial block
            }
            break io.resync(pos + 1, limit)?.unwrap_or(limit);
        }
        let ce = c.end(lim);
        match c.id {
            TIMESTAMP => ts = cluster_timestamp(ebml::uint(io.peek(c.data_start(), (ce - c.data_start()) as usize)?)),
            SIMPLE_BLOCK | BLOCK_GROUP => {
                if let Some(b) = parse_block_elem(io, &c, ce, ts, lookup)? {
                    budget.take(&b.frames)?;
                    blocks.push(b);
                }
            }
            _ => {}
        }
        pos = ce;
    };
    Ok((ClusterInfo { offset: h.pos, data_start: ds, end, timestamp: ts.max(0) as u64 }, blocks))
}
