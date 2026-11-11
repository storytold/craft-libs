//! Box writers shared by the progressive and fragmented muxers: `ftyp`, `mvhd`, `trak`, `stbl`, `udta`.

use super::*;

pub(super) const MATRIX: [i32; 9] = [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000];

pub(super) fn write_ftyp(b: &mut BoxBuf, brand: Brand) {
    let m = b.start(b"ftyp");
    match brand {
        Brand::Mp4 => {
            b.bytes(b"isom");
            b.u32(0x200);
            for c in [b"isom", b"iso2", b"avc1", b"mp41", b"mp42"] {
                b.bytes(c);
            }
        }
        Brand::Mov => {
            b.bytes(b"qt  ");
            b.u32(0x200);
            b.bytes(b"qt  ");
        }
    }
    b.end(m);
}

pub(super) fn movie_duration(tracks: &[WTrack], movie_ts: u32) -> u64 {
    tracks.iter().map(|t| presentation_duration(t, movie_ts)).max().unwrap_or(0)
}

pub(super) fn effective_edits(t: &WTrack, movie_ts: u32) -> Vec<Edit> {
    if !t.cfg.edits.is_empty() {
        return t.cfg.edits.clone();
    }
    if let Some(start) = t.cfg.media_start {
        // presentation runs from `start` to the end of the last sample in composition order (with
        // B-frame reordering that is past the end of the decode timeline)
        let rest = t.t.comp_end.max(t.t.media_duration).saturating_sub(start.max(0) as u64);
        return vec![Edit { segment_duration: rescale(rest, t.cfg.timescale, movie_ts), media_time: start, media_rate: 0x10000 }];
    }
    Vec::new()
}

pub(super) fn presentation_duration(t: &WTrack, movie_ts: u32) -> u64 {
    let e = effective_edits(t, movie_ts);
    if e.is_empty() { rescale(t.t.media_duration, t.cfg.timescale, movie_ts) } else { e.iter().map(|e| e.segment_duration).sum() }
}

pub(super) fn write_mvhd(b: &mut BoxBuf, timescale: u32, duration: u64, next_id: u32) {
    let v1 = duration > u32::MAX as u64;
    let m = b.start_full(b"mvhd", v1 as u8, 0);
    if v1 {
        b.u64(0);
        b.u64(0);
        b.u32(timescale);
        b.u64(duration);
    } else {
        b.u32(0);
        b.u32(0);
        b.u32(timescale);
        b.u32(duration as u32);
    }
    b.u32(0x10000);
    b.u16(0x100);
    b.zeros(10);
    for v in MATRIX {
        b.i32(v);
    }
    b.zeros(24);
    b.u32(next_id);
    b.end(m);
}

pub(super) fn write_trak(b: &mut BoxBuf, t: &WTrack, movie_ts: u32, qt: bool, shift: u64, force_co64: bool) -> Result<()> {
    let trak = b.start(b"trak");
    let dur = presentation_duration(t, movie_ts);
    let v1 = dur > u32::MAX as u64;
    // a chapter track is in the movie but disabled: players list it as chapters, never show it
    let m = b.start_full(b"tkhd", v1 as u8, if t.handler == Handler::Text { 2 } else { 3 });
    if v1 {
        b.u64(0);
        b.u64(0);
        b.u32(t.id);
        b.u32(0);
        b.u64(dur);
    } else {
        b.u32(0);
        b.u32(0);
        b.u32(t.id);
        b.u32(0);
        b.u32(dur as u32);
    }
    b.zeros(8);
    b.i16(0);
    b.u16(0);
    b.i16(if t.handler == Handler::Audio { 0x100 } else { 0 });
    b.u16(0);
    for v in MATRIX {
        b.i32(v);
    }
    let (w, h) = match &t.cfg.entry.video {
        Some(v) => {
            let w = match v.pixel_aspect {
                Some((hs, vs)) if hs > 0 && vs > 0 => (v.width as u64 * hs as u64 / vs as u64) as u32,
                _ => v.width as u32,
            };
            (w, v.height as u32)
        }
        None => (0, 0),
    };
    b.u32(w << 16);
    b.u32(h << 16);
    b.end(m);
    let edits = effective_edits(t, movie_ts);
    if !edits.is_empty() {
        let e = b.start(b"edts");
        let v1 = edits.iter().any(|e| e.segment_duration > u32::MAX as u64 || e.media_time > i32::MAX as i64 || e.media_time < i32::MIN as i64);
        let l = b.start_full(b"elst", v1 as u8, 0);
        b.u32(edits.len() as u32);
        for ed in &edits {
            if v1 {
                b.u64(ed.segment_duration);
                b.i64(ed.media_time);
            } else {
                b.u32(ed.segment_duration as u32);
                b.i32(ed.media_time as i32);
            }
            b.i32(ed.media_rate);
        }
        b.end(l);
        b.end(e);
    }
    if t.tmcd_ref.is_some() || t.chap_ref.is_some() {
        let r = b.start(b"tref");
        for (kind, id) in [(b"tmcd", t.tmcd_ref), (b"chap", t.chap_ref)] {
            if let Some(id) = id {
                let m = b.start(kind);
                b.u32(id);
                b.end(m);
            }
        }
        b.end(r);
    }
    let mdia = b.start(b"mdia");
    let mv1 = t.t.media_duration > u32::MAX as u64;
    let m = b.start_full(b"mdhd", mv1 as u8, 0);
    if mv1 {
        b.u64(0);
        b.u64(0);
        b.u32(t.cfg.timescale);
        b.u64(t.t.media_duration);
    } else {
        b.u32(0);
        b.u32(0);
        b.u32(t.cfg.timescale);
        b.u32(t.t.media_duration as u32);
    }
    b.u16(pack_language(&t.cfg.language));
    b.u16(0);
    b.end(m);
    let (htype, default_name) = match t.handler {
        Handler::Video => (b"vide", "VideoHandler"),
        Handler::Audio => (b"soun", "SoundHandler"),
        Handler::Timecode => (b"tmcd", "TimeCodeHandler"),
        Handler::Text => (b"text", "Chapters"),
        Handler::Other => (b"meta", "MetaHandler"),
    };
    let name = t.cfg.handler_name.clone().unwrap_or_else(|| default_name.into());
    let m = b.start_full(b"hdlr", 0, 0);
    b.bytes(if qt { b"mhlr" } else { &[0, 0, 0, 0] });
    b.bytes(htype);
    b.zeros(12);
    if qt {
        let n = name.len().min(255);
        b.u8(n as u8);
        b.bytes(&name.as_bytes()[..n]);
    } else {
        b.bytes(name.as_bytes());
        b.u8(0);
    }
    b.end(m);
    let minf = b.start(b"minf");
    match t.handler {
        Handler::Video => {
            let m = b.start_full(b"vmhd", 0, 1);
            b.zeros(8);
            b.end(m);
        }
        Handler::Audio => {
            let m = b.start_full(b"smhd", 0, 0);
            b.zeros(4);
            b.end(m);
        }
        Handler::Timecode => {
            let g = b.start(b"gmhd");
            let m = b.start_full(b"gmin", 0, 0);
            b.u16(0x40);
            b.u16(0x8000);
            b.u16(0x8000);
            b.u16(0x8000);
            b.i16(0);
            b.u16(0);
            b.end(m);
            let tm = b.start(b"tmcd");
            let m = b.start_full(b"tcmi", 0, 0);
            b.u16(0);
            b.u16(0);
            b.u16(12);
            b.u16(0);
            b.bytes(&[0xFF; 6]);
            b.zeros(6);
            b.u8(0);
            b.end(m);
            b.end(tm);
            b.end(g);
        }
        Handler::Text => {
            // QTFF "Base media information header" with a text media information atom (a matrix)
            let g = b.start(b"gmhd");
            let m = b.start_full(b"gmin", 0, 0);
            b.u16(0x40);
            b.u16(0x8000);
            b.u16(0x8000);
            b.u16(0x8000);
            b.i16(0);
            b.u16(0);
            b.end(m);
            let tx = b.start(b"text");
            for v in MATRIX {
                b.i32(v);
            }
            b.end(tx);
            b.end(g);
        }
        Handler::Other => {
            let m = b.start_full(b"nmhd", 0, 0);
            b.end(m);
        }
    }
    if qt {
        // QuickTime data handler reference.
        let m = b.start_full(b"hdlr", 0, 0);
        b.bytes(b"dhlr");
        b.bytes(b"url ");
        b.zeros(12);
        b.u8(0);
        b.end(m);
    }
    let d = b.start(b"dinf");
    let r = b.start_full(b"dref", 0, 0);
    b.u32(1);
    let u = b.start_full(b"url ", 0, 1);
    b.end(u);
    b.end(r);
    b.end(d);
    write_stbl(b, t, qt, shift, force_co64)?;
    b.end(minf);
    b.end(mdia);
    b.end(trak);
    Ok(())
}

pub(super) fn write_stbl(b: &mut BoxBuf, t: &WTrack, qt: bool, shift: u64, force_co64: bool) -> Result<()> {
    let stbl = b.start(b"stbl");
    let m = b.start_full(b"stsd", 0, 0);
    b.u32(1);
    t.cfg.entry.write(b, qt)?;
    b.end(m);
    let tb = &t.t;
    let m = b.start_full(b"stts", 0, 0);
    b.u32(tb.stts.len() as u32);
    for &(c, d) in &tb.stts {
        b.u32(c);
        b.u32(d);
    }
    b.end(m);
    if tb.ctts.iter().any(|&(_, o)| o != 0) {
        let neg = tb.ctts.iter().any(|&(_, o)| o < 0);
        let m = b.start_full(b"ctts", neg as u8, 0);
        b.u32(tb.ctts.len() as u32);
        for &(c, o) in &tb.ctts {
            b.u32(c);
            b.i32(o);
        }
        b.end(m);
    }
    if !tb.all_sync {
        let m = b.start_full(b"stss", 0, 0);
        b.u32(tb.sync.len() as u32);
        for &s in &tb.sync {
            b.u32(s);
        }
        b.end(m);
    }
    let m = b.start_full(b"stsc", 0, 0);
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for (i, &(_, n)) in tb.chunks.iter().enumerate() {
        if runs.last().is_none_or(|r| r.1 != n) {
            runs.push((i as u32 + 1, n));
        }
    }
    b.u32(runs.len() as u32);
    for (first, n) in runs {
        b.u32(first);
        b.u32(n);
        b.u32(1);
    }
    b.end(m);
    let sample_count = u32::try_from(tb.sample_count).map_err(|_| too_many_samples())?;
    let m = b.start_full(b"stsz", 0, 0);
    match tb.const_size {
        Some(s) => {
            b.u32(s);
            b.u32(sample_count);
        }
        None => {
            let first = tb.sizes.first().copied().unwrap_or(0);
            // A uniform size of 0 would mean "table follows" (ISO/IEC 14496-12 §8.7.3), so
            // all-empty samples must still be written as a table.
            if first != 0 && !tb.sizes.is_empty() && tb.sizes.iter().all(|&s| s == first) {
                b.u32(first);
                b.u32(sample_count);
            } else {
                b.u32(0);
                b.u32(sample_count);
                for &s in &tb.sizes {
                    b.u32(s);
                }
            }
        }
    }
    b.end(m);
    let co64 = force_co64 || tb.chunks.iter().any(|&(o, _)| o + shift > u32::MAX as u64);
    let m = b.start_full(if co64 { b"co64" } else { b"stco" }, 0, 0);
    b.u32(tb.chunks.len() as u32);
    for &(o, _) in &tb.chunks {
        if co64 {
            b.u64(o + shift);
        } else {
            b.u32((o + shift) as u32);
        }
    }
    b.end(m);
    b.end(stbl);
    Ok(())
}

pub(super) fn write_udta(b: &mut BoxBuf, md: &[(String, String)], chapters: &[Chapter], movie_ts: u32) {
    let items: Vec<([u8; 4], &str)> = md
        .iter()
        .filter_map(|(k, v)| {
            let mut chars = k.chars();
            if chars.next()? != '©' {
                return None;
            }
            let rest: Vec<u8> = chars.map(|c| c as u32 as u8).collect();
            (rest.len() == 3).then(|| ([0xA9, rest[0], rest[1], rest[2]], v.as_str()))
        })
        .collect();
    if items.is_empty() && chapters.is_empty() {
        return;
    }
    let u = b.start(b"udta");
    for (k, v) in items {
        let m = b.start(&k);
        b.u16(v.len() as u16);
        b.u16(0x55C4);
        b.bytes(v.as_bytes());
        b.end(m);
    }
    if !chapters.is_empty() {
        write_chpl(b, chapters, movie_ts);
    }
    b.end(u);
}
