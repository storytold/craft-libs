//! Every entry of the coefficient code tables, and the first two escape modes, against ffmpeg:
//! a stream coded with the tables must decode exactly as the same coefficients sent with the
//! fixed-length escape. Skipped when ffmpeg is absent.

use std::process::Command;

use craft_media::bitstream::BitWriter;

use super::headers::{self, Sequence, Timing, VopHeader};
use super::picture::{Mb, Mode, write_mb};
use super::tables::{self, Tcoef, ZIGZAG};

/// One `(last, run, level)` event of interest for each block: every table entry, every
/// `LMAX + 1` (escape mode 1) and every `RMAX + 1` (escape mode 2), with alternating signs.
fn events(t: &Tcoef) -> Vec<(bool, usize, i32)> {
    let mut out = Vec::new();
    for last in [false, true] {
        for run in 0..64 {
            let lmax = t.lmax(last, run);
            for level in 1..=lmax {
                out.push((last, run, level as i32));
            }
            if lmax > 0 {
                out.push((last, run, lmax as i32 + 1));
            }
        }
        for level in 1..28 {
            if let Some(rmax) = t.rmax(last, level) {
                out.push((last, rmax + 1, level as i32));
            }
        }
    }
    for (i, e) in out.iter_mut().enumerate() {
        if i % 2 == 1 {
            e.2 = -e.2;
        }
    }
    out
}

/// A block holding `event` from scan position `start`; an event with `last` clear is followed by
/// a closing level at scan position 63.
fn block(event: (bool, usize, i32), start: usize) -> [i16; 64] {
    let (last, run, level) = event;
    let mut b = [0i16; 64];
    b[ZIGZAG[start + run]] = level as i16;
    if !last {
        b[ZIGZAG[63]] = 1;
    }
    b
}

/// Macroblocks carrying `events` six to a macroblock, padded to `n`.
fn macroblocks(events: &[(bool, usize, i32)], intra: bool, n: usize) -> Vec<Mb> {
    let mut mbs = Vec::new();
    for chunk in events.chunks(6) {
        let mut mb = Mb { mode: if intra { Mode::Intra } else { Mode::Inter }, ..Mb::skip() };
        for (b, &e) in chunk.iter().enumerate() {
            mb.levels[b] = block(e, usize::from(intra));
            mb.cbp |= 1 << (5 - b);
        }
        mbs.push(mb);
    }
    while mbs.len() < n {
        mbs.push(Mb::skip());
    }
    mbs
}

/// An I-VOP with the intra events, then a P-VOP with the inter events (zero vectors).
fn stream(width: u32, height: u32, intra: &[Mb], inter: &[Mb], escape_only: bool) -> Vec<u8> {
    let timing = Timing { resolution: 25, increment: 1 };
    let mut w = BitWriter::new();
    w.write_bytes(&headers::sequence_headers(&Sequence { width, height, timing, profile_level: 3, sar: (1, 1), color: None }));
    for (k, mbs) in [intra, inter].into_iter().enumerate() {
        let i = k == 0;
        let h = VopHeader { intra: i, seconds: 0, time_increment: k as u32, rounding: false, quant: 1, fcode: 1 };
        headers::vop_header(&mut w, &h, timing);
        for mb in mbs {
            let mut mb = mb.clone();
            if i {
                mb.mode = Mode::Intra;
            }
            write_mb(&mut w, &mb, i, 1, escape_only);
        }
        headers::stuffing(&mut w);
    }
    w.finish()
}

#[test]
fn code_tables_decode_like_the_fixed_length_escape() {
    let Some(ff) = craft_media_testkit::ffmpeg_or_skip("mpeg4 code tables") else { return };
    let (ei, ep) = (events(tables::intra()), events(tables::inter()));
    let mb_w = 8usize;
    let n = ei.len().max(ep.len()).div_ceil(6);
    let mb_h = n.div_ceil(mb_w);
    let (w, h) = ((mb_w * 16) as u32, (mb_h * 16) as u32);
    let mut intra = macroblocks(&ei, true, mb_w * mb_h);
    for mb in &mut intra {
        mb.mode = Mode::Intra;
    }
    let inter = macroblocks(&ep, false, mb_w * mb_h);
    let dir = craft_media_testkit::private_dir("mpeg4-tables");
    let decode = |name: &str, escape_only: bool| -> Vec<u8> {
        let path = dir.join(name);
        std::fs::write(&path, stream(w, h, &intra, &inter, escape_only)).expect("write");
        let out = Command::new(&ff)
            .args(["-hide_banner", "-nostdin", "-v", "warning", "-f", "m4v", "-i"])
            .arg(&path)
            .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
            .output()
            .expect("run ffmpeg");
        let diag = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success() && diag.trim().is_empty(), "{name}: {diag}");
        out.stdout
    };
    let vlc = decode("vlc.m4v", false);
    let esc = decode("escape.m4v", true);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(vlc.len(), (w * h * 3 / 2 * 2) as usize, "two pictures");
    assert!(vlc.iter().any(|&v| v != vlc[0]), "the pictures carry the coefficients");
    if let Some(i) = vlc.iter().zip(&esc).position(|(a, b)| a != b) {
        let frame = i / (w * h * 3 / 2) as usize;
        panic!("the table-coded stream decodes differently from the escape-coded one (picture {frame}, byte {i})");
    }
}
