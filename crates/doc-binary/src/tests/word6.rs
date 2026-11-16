//! Word 6.0/95 fixtures, built in code. No public specification describes this version; these
//! build the layout observed in real files and described in `crate::word6`.

use proptest::prelude::*;

use super::{body_texts, cfb_file};
use crate::{DocbinError, read};

/// Where the text starts: right after the FIB, as in every observed file.
const TEXT_AT: u32 = 0x300;

/// One stylesheet entry as Word 6/95 stores it.
#[derive(Clone, Copy, Default)]
struct Std<'a> {
    /// The built-in style id, or 0x0FFE for a user-defined style.
    sti: u16,
    /// 1 paragraph, 2 character.
    sgc: u16,
    /// The base style's index, or 0x0FFF.
    base: u16,
    name: &'a str,
    /// Paragraph sprms (paragraph styles only).
    papx: &'a [u8],
    /// Character sprms.
    chpx: &'a [u8],
}

/// The stylesheet bytes for `styles`: the 14-byte header (count, fixed-part size 8, …), then per
/// style a size and the entry, or 0 for an empty slot.
fn stylesheet(styles: &[Option<Std>]) -> Vec<u8> {
    let mut v = vec![14u8, 0];
    for w in [styles.len() as u16, 8, 1, 0x4B, 0x0F, 0, 0] {
        v.extend_from_slice(&w.to_le_bytes());
    }
    for (istd, st) in styles.iter().enumerate() {
        let Some(st) = st else {
            v.extend_from_slice(&[0, 0]);
            continue;
        };
        let cupx: u16 = if st.sgc == 1 { 2 } else { 1 };
        let mut e = Vec::new();
        for w in [st.sti, st.sgc | st.base << 4, cupx, 0] {
            e.extend_from_slice(&w.to_le_bytes());
        }
        e.push(st.name.len() as u8);
        e.extend_from_slice(st.name.as_bytes());
        e.push(0);
        let upx = |e: &mut Vec<u8>, bytes: &[u8]| {
            if e.len() % 2 == 1 {
                e.push(0);
            }
            e.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
            e.extend_from_slice(bytes);
        };
        if st.sgc == 1 {
            let mut papx = (istd as u16).to_le_bytes().to_vec();
            papx.extend_from_slice(st.papx);
            upx(&mut e, &papx);
        }
        upx(&mut e, st.chpx);
        v.extend_from_slice(&(e.len() as u16).to_le_bytes());
        v.extend_from_slice(&e);
    }
    v
}

/// What a test Word 6.0/95 file holds. Offsets in `runs` and `pieces` are into `text`.
#[derive(Clone, Default)]
struct Doc6<'a> {
    nfib: u16,
    flags: u16,
    /// The stored 8-bit text.
    text: &'a [u8],
    /// (charset, name) per font index.
    fonts: &'a [(u8, &'a str)],
    /// The font table's entries as raw bytes instead of `fonts` (the size word is added).
    raw_fonts: Option<&'a [u8]>,
    /// Contiguous character runs (lo, hi, grpprl), all in one FKP page.
    runs: &'a [(u32, u32, &'a [u8])],
    /// Contiguous paragraph runs (lo, hi, grpprl), style 0, all in one FKP page. Each covers the
    /// paragraph whose mark lies in it.
    paras: &'a [(u32, u32, &'a [u8])],
    /// For a fast-saved file: (length, offset) per piece, in CP order.
    pieces: Option<&'a [(u32, u32)]>,
    /// Each piece's Prm (0 when absent or shorter than `pieces`).
    prms: &'a [u16],
    /// The Clx's Prc grpprls, which a Prm1 indexes.
    prcs: &'a [&'a [u8]],
    /// The FIB's character set word at 0x14.
    chse: u16,
    /// The style of each paragraph run (0, Normal, when absent or shorter than `paras`).
    para_styles: &'a [u16],
    /// The stylesheet: one entry per style index, `None` for an empty slot.
    styles: &'a [Option<Std<'a>>],
    /// The main text's length in characters, when it isn't the stored bytes' (16-bit text) less
    /// the other stories'.
    ccp: Option<u32>,
    /// The lengths of the footnote, header, macro, comment and endnote stories, whose text follows
    /// the main text's in `text`.
    stories: [u32; 5],
    /// Sections: (end CP, SEPX grpprl of Word 6 section sprms).
    sections: &'a [(u32, &'a [u8])],
    /// The header table's CPs (pair 11).
    hdd: &'a [u32],
    /// The Dop's first bytes (pair 31).
    dop: &'a [u8],
    /// Footnotes and endnotes: (reference CPs, story CPs within their subdocument).
    footnotes: (&'a [u32], &'a [u32]),
    endnotes: (&'a [u32], &'a [u32]),
    /// The footnotes' FRDs as stored, when not 1, 2, ... (0: a custom mark).
    footnote_frds: Option<&'a [u8]>,
    /// Comments: (reference CPs, story CPs, the 20-byte ATRDs), and the authors' list (pair 36).
    comments: (&'a [u32], &'a [u32], &'a [u8]),
    owners: &'a [u8],
}

fn put16(v: &mut [u8], at: usize, x: u16) {
    v[at..at + 2].copy_from_slice(&x.to_le_bytes());
}

fn put32(v: &mut [u8], at: usize, x: u32) {
    v[at..at + 4].copy_from_slice(&x.to_le_bytes());
}

/// Append one FKP page for `runs` and its bin table, and return the bin table's (fc, lcb). A
/// character page has a 1-byte entry per run and CHPXs (byte count + grpprl); a paragraph page
/// has 7-byte entries and PAPXs (word count + style 0 + grpprl padded to whole words).
fn fkp(w: &mut Vec<u8>, runs: &[(u32, u32, &[u8])], papx: bool, istds: &[u16]) -> (u32, u32) {
    let (Some(first), Some(last)) = (runs.first(), runs.last()) else { return (0, 0) };
    w.resize(w.len().div_ceil(512) * 512, 0);
    let pn = (w.len() / 512) as u16;
    let crun = runs.len();
    let entry = if papx { 7 } else { 1 };
    let mut page = [0u8; 512];
    for (k, (lo, _, _)) in runs.iter().enumerate() {
        put32(&mut page, 4 * k, TEXT_AT + lo);
    }
    put32(&mut page, 4 * crun, TEXT_AT + last.1);
    // Records from the end of the page down, each at an even offset.
    let mut top = 511usize;
    for (k, (_, _, g)) in runs.iter().enumerate() {
        let body: Vec<u8> = if papx {
            let mut b = istds.get(k).copied().unwrap_or(0).to_le_bytes().to_vec();
            b.extend_from_slice(g);
            if b.len() % 2 == 1 {
                b.push(0);
            }
            b
        } else if g.is_empty() {
            continue;
        } else {
            g.to_vec()
        };
        let at = (top - 1 - body.len()) & !1;
        page[at] = if papx { (body.len() / 2) as u8 } else { body.len() as u8 };
        page[at + 1..at + 1 + body.len()].copy_from_slice(&body);
        page[4 * (crun + 1) + entry * k] = (at / 2) as u8;
        top = at;
    }
    page[511] = crun as u8;
    w.extend_from_slice(&page);
    let mut bte = Vec::new();
    bte.extend_from_slice(&(TEXT_AT + first.0).to_le_bytes());
    bte.extend_from_slice(&(TEXT_AT + last.1).to_le_bytes());
    bte.extend_from_slice(&pn.to_le_bytes());
    let at = w.len() as u32;
    w.extend_from_slice(&bte);
    (at, bte.len() as u32)
}

/// The WordDocument stream: FIB, text, then the FKP pages, font table and piece table.
fn word_stream(d: &Doc6) -> Vec<u8> {
    let mut w = vec![0u8; TEXT_AT as usize];
    w.extend_from_slice(d.text);
    let others: u32 = d.stories.iter().sum();
    let ccp = match (d.ccp, d.pieces) {
        (Some(ccp), _) => ccp,
        (None, pieces) => match pieces {
            Some(p) => p.iter().map(|(len, _)| len).sum::<u32>() - others,
            None => d.text.len() as u32 - others,
        },
    };
    let mut pairs: Vec<(usize, u32, u32)> = Vec::new();
    if !d.sections.is_empty() {
        // Each SEPX (u16 size + grpprl), then the PlcfSed: CPs, then 12-byte SEDs.
        let mut fcs = Vec::new();
        for (_, sepx) in d.sections {
            fcs.push(w.len() as u32);
            w.extend_from_slice(&(sepx.len() as u16).to_le_bytes());
            w.extend_from_slice(sepx);
        }
        let mut plc: Vec<u8> = 0u32.to_le_bytes().to_vec();
        for (end, _) in d.sections {
            plc.extend_from_slice(&end.to_le_bytes());
        }
        for fc in fcs {
            plc.extend_from_slice(&[4, 0]);
            plc.extend_from_slice(&fc.to_le_bytes());
            plc.extend_from_slice(&[0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
        }
        pairs.push((6, w.len() as u32, plc.len() as u32));
        w.extend_from_slice(&plc);
    }
    let mut plc = |w: &mut Vec<u8>, pair: usize, cps: &[u32], data: &[u8]| {
        if cps.is_empty() {
            return;
        }
        let at = w.len() as u32;
        for cp in cps {
            w.extend_from_slice(&cp.to_le_bytes());
        }
        w.extend_from_slice(data);
        pairs.push((pair, at, w.len() as u32 - at));
    };
    plc(&mut w, 11, d.hdd, &[]);
    plc(&mut w, 31, &[], &[]);
    // References: CPs, then a 2-byte FRD (1, 2, ...: auto-numbered) each but the limit's.
    let frds = |n: usize| (1..n as u16).flat_map(|i| i.to_le_bytes()).collect::<Vec<u8>>();
    plc(&mut w, 2, d.footnotes.0, &d.footnote_frds.map_or_else(|| frds(d.footnotes.0.len()), <[u8]>::to_vec));
    plc(&mut w, 4, d.comments.0, d.comments.2);
    plc(&mut w, 5, d.comments.1, &[]);
    plc(&mut w, 3, d.footnotes.1, &[]);
    plc(&mut w, 46, d.endnotes.0, &frds(d.endnotes.0.len()));
    plc(&mut w, 47, d.endnotes.1, &[]);
    if !d.owners.is_empty() {
        pairs.push((36, w.len() as u32, d.owners.len() as u32));
        w.extend_from_slice(d.owners);
    }
    if !d.dop.is_empty() {
        pairs.push((31, w.len() as u32, d.dop.len() as u32));
        w.extend_from_slice(d.dop);
    }
    for (pair, runs, papx) in [(12, d.runs, false), (13, d.paras, true)] {
        let (fc, lcb) = fkp(&mut w, runs, papx, d.para_styles);
        if lcb > 0 {
            pairs.push((pair, fc, lcb));
        }
    }
    if !d.styles.is_empty() {
        let st = stylesheet(d.styles);
        pairs.push((1, w.len() as u32, st.len() as u32));
        w.extend_from_slice(&st);
    }
    if !d.fonts.is_empty() || d.raw_fonts.is_some() {
        let mut ffn = vec![0u8, 0];
        ffn.extend_from_slice(d.raw_fonts.unwrap_or_default());
        for (charset, name) in d.fonts {
            let mut e = vec![0u8, 0x16, 0x90, 0x01, *charset, 0x00];
            e.extend_from_slice(name.as_bytes());
            e.push(0);
            e[0] = (e.len() - 1) as u8;
            ffn.extend_from_slice(&e);
        }
        let size = ffn.len() as u16;
        put16(&mut ffn, 0, size);
        pairs.push((15, w.len() as u32, ffn.len() as u32));
        w.extend_from_slice(&ffn);
    }
    if let Some(pieces) = d.pieces {
        let mut plc = Vec::new();
        let mut cp = 0u32;
        plc.extend_from_slice(&cp.to_le_bytes());
        for (len, _) in pieces {
            cp += len;
            plc.extend_from_slice(&cp.to_le_bytes());
        }
        for (i, (_, off)) in pieces.iter().enumerate() {
            plc.extend_from_slice(&[0, 0]);
            plc.extend_from_slice(&(TEXT_AT + off).to_le_bytes());
            plc.extend_from_slice(&d.prms.get(i).copied().unwrap_or(0).to_le_bytes());
        }
        let mut clx = Vec::new();
        for g in d.prcs {
            clx.push(0x01);
            clx.extend_from_slice(&(g.len() as u16).to_le_bytes());
            clx.extend_from_slice(g);
        }
        clx.push(0x02);
        clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
        clx.extend_from_slice(&plc);
        pairs.push((33, w.len() as u32, clx.len() as u32));
        w.extend_from_slice(&clx);
    }
    put16(&mut w, 0x00, 0xA5DC);
    put16(&mut w, 0x02, if d.nfib == 0 { 0x0065 } else { d.nfib });
    put16(&mut w, 0x06, 0x0409);
    put16(&mut w, 0x0A, d.flags | if d.pieces.is_some() { 0x0004 } else { 0 });
    put16(&mut w, 0x0C, 0x0065);
    put16(&mut w, 0x14, d.chse);
    put32(&mut w, 0x18, TEXT_AT);
    put32(&mut w, 0x1C, TEXT_AT + d.text.len() as u32);
    let len = w.len() as u32;
    put32(&mut w, 0x20, len);
    put32(&mut w, 0x34, ccp);
    for (k, n) in d.stories.iter().enumerate() {
        put32(&mut w, 0x38 + 4 * k, *n);
    }
    for (i, fc, lcb) in pairs {
        // From index 38 on, the pairs sit 10 bytes later.
        let at = 0x58 + i * 8 + if i >= 38 { 10 } else { 0 };
        put32(&mut w, at, fc);
        put32(&mut w, at + 4, lcb);
    }
    w
}

/// A Word 6.0/95 file holding `d`: the WordDocument stream only, as the real ones have no Table
/// stream.
fn word6_file(d: &Doc6) -> Vec<u8> {
    cfb_file(&[("WordDocument", &word_stream(d))])
}

fn texts(d: &Doc6) -> Vec<String> {
    body_texts(&read(&word6_file(d)).expect("a Word 6.0/95 file opens"))
}

/// Fonts 0 (Western) and 1 (Cyrillic, charset 204), as the Russian files have.
const FONTS: &[(u8, &str)] = &[(0, "Times New Roman"), (204, "Times New Roman Cyr")];

#[test]
fn word_6_and_95_files_open_as_paragraphs() {
    for nfib in [0x0065, 0x0068] {
        let d = Doc6 { nfib, text: b"Hello\rWorld\r", ..Default::default() };
        assert_eq!(texts(&d), ["Hello", "World"], "nFib 0x{nfib:04X}");
    }
}

#[test]
fn fast_saved_text_follows_the_piece_table() {
    // Stored out of order; the pieces' fcs are plain byte offsets (Word 97 would halve them).
    let d = Doc6 { text: b"World\rHello\r", pieces: Some(&[(6, 6), (6, 0)]), ..Default::default() };
    assert_eq!(texts(&d), ["Hello", "World"]);
}

#[test]
fn fast_saved_file_without_a_piece_table_is_malformed() {
    let d = Doc6 { flags: 0x0004, text: b"Hello\r", ..Default::default() };
    assert!(matches!(read(&word6_file(&d)), Err(DocbinError::Malformed(_))));
}

#[test]
fn a_runs_font_charset_chooses_its_code_page() {
    // "Da " in font 0, then 0xC4 0xE0 ("Да" in Windows-1251) in the Cyrillic font 1.
    let text = b"Da \xC4\xE0\r";
    let runs: &[(u32, u32, &[u8])] = &[(0, 3, &[]), (3, 5, &[0x5D, 0x01, 0x00]), (5, 6, &[])];
    let d = Doc6 { text, fonts: FONTS, runs, ..Default::default() };
    assert_eq!(texts(&d), ["Da Да"]);
}

/// Text in an East Asian code page (a Japanese font, charset 128: Shift-JIS) is refused, not
/// read a byte at a time into replacement characters (review of wordcraft#6345).
#[test]
fn east_asian_text_is_refused_not_lost() {
    let fonts: &[(u8, &str)] = &[(0, "Times New Roman"), (128, "MS Mincho")];
    let runs: &[(u32, u32, &[u8])] = &[(0, 2, &[0x5D, 0x01, 0x00]), (2, 3, &[])];
    let d = Doc6 { text: b"\x93\xfa\r", fonts, runs, ..Default::default() };
    assert!(matches!(read(&word6_file(&d)), Err(DocbinError::NotWord(e)) if e.contains("East Asian")));
    // The same file in a Western font still opens.
    let d = Doc6 { text: b"\x93\xfa\r", fonts, runs: &[], ..Default::default() };
    assert_eq!(texts(&d).len(), 1);
}

#[test]
fn the_font_sprm_is_found_after_other_sprms() {
    // As in real files: bold-like toggles, a size, a picture anchor's length-prefixed sprm.
    let text = b"\xC4\xE0\r";
    for g in [&[0x55, 0x81, 0x63, 0x18, 0x00, 0x5D, 0x01, 0x00][..], &[0x75, 0x01, 0x44, 0x04, 0, 0, 0, 0, 0x5D, 0x01, 0x00][..]] {
        let runs: &[(u32, u32, &[u8])] = &[(0, 2, g), (2, 3, &[])];
        let d = Doc6 { text, fonts: FONTS, runs, ..Default::default() };
        assert_eq!(texts(&d), ["Да"], "grpprl {g:02X?}");
    }
}

#[test]
fn the_last_font_sprm_of_a_run_chooses_its_code_page() {
    // Font 0, then font 1 (Cyrillic): the later sprm is the one the formatting applies.
    let runs: &[(u32, u32, &[u8])] = &[(0, 2, &[0x5D, 0x00, 0x00, 0x5D, 0x01, 0x00]), (2, 3, &[])];
    let d = Doc6 { text: b"\xC4\xE0\r", fonts: FONTS, runs, ..Default::default() };
    assert_eq!(texts(&d), ["Да"]);
}

#[test]
fn an_unknown_sprm_leaves_font_0() {
    // 0x99 has no known operand length, so the rest can't be framed: font 0 (Windows-1252).
    let text = b"\xC4\xE0\r";
    let runs: &[(u32, u32, &[u8])] = &[(0, 2, &[0x99, 0x5D, 0x01, 0x00]), (2, 3, &[])];
    let d = Doc6 { text, fonts: FONTS, runs, ..Default::default() };
    assert_eq!(texts(&d), ["Äà"]);
}

#[test]
fn the_toggle_sprms_before_the_font_are_framed() {
    // 0x57 (strike, a one-byte toggle in [MS-DOC]'s Prm0 isprm table) before the font sprm: the
    // font must still be found, or the Cyrillic text reads as Windows-1252.
    let text = b"\xC4\xE0\r";
    for op in [0x57, 0x58, 0x59, 0x5C] {
        let g = [op, 0x01, 0x5D, 0x01, 0x00];
        let runs: &[(u32, u32, &[u8])] = &[(0, 2, &g), (2, 3, &[])];
        let d = Doc6 { text, fonts: FONTS, runs, ..Default::default() };
        assert_eq!(texts(&d), ["Да"], "opcode 0x{op:02X}");
    }
}

#[test]
fn a_font_entry_too_short_for_its_charset_ends_the_font_table() {
    // Entry 0 says it is 3 bytes long, which can't hold the charset at offset 4. The bytes after
    // it (the next entry, whose flags byte is 0xCC) must not be read as font 0's charset.
    let raw: &[u8] = &[0x02, 0x16, 0x90, 0x05, 0xCC, 0x90, 0x01, 0xCC, 0x00, b'X', 0];
    let d = Doc6 { text: b"\xC4\xE0\r", raw_fonts: Some(raw), runs: &[(0, 3, &[])], ..Default::default() };
    assert_eq!(texts(&d), ["Äà"], "font 0 has no charset: Windows-1252");
}

#[test]
fn a_bin_table_naming_one_page_many_times_reads_it_once() {
    // A hostile bin table: 1,000,000 entries, all naming one page of 50 Cyrillic runs. The text still
    // reads right; that the page is read once is pinned in `word6::tests`.
    let text: Vec<u8> = std::iter::repeat_n(0xC4u8, 50).chain(*b"\r").collect();
    let g: &[u8] = &[0x5D, 0x01, 0x00];
    let runs: Vec<(u32, u32, &[u8])> = (0..50).map(|i| (i, i + 1, g)).chain([(50, 51, &[][..])]).collect();
    let mut w = word_stream(&Doc6 { text: &text, fonts: FONTS, runs: &runs, ..Default::default() });
    let u32_at = |w: &[u8], at: usize| u32::from_le_bytes([w[at], w[at + 1], w[at + 2], w[at + 3]]);
    let bte_fc = u32_at(&w, 0x58 + 12 * 8) as usize;
    let pn = u16::from_le_bytes([w[bte_fc + 8], w[bte_fc + 9]]);
    let n = 1_000_000usize;
    let mut bte = Vec::with_capacity(4 * (n + 1) + 2 * n);
    bte.extend_from_slice(&TEXT_AT.to_le_bytes());
    for _ in 0..n {
        bte.extend_from_slice(&(TEXT_AT + 51).to_le_bytes());
    }
    for _ in 0..n {
        bte.extend_from_slice(&pn.to_le_bytes());
    }
    let at = w.len() as u32;
    w.extend_from_slice(&bte);
    put32(&mut w, 0x58 + 12 * 8, at);
    put32(&mut w, 0x58 + 12 * 8 + 4, bte.len() as u32);
    let doc = read(&cfb_file(&[("WordDocument", &w)])).expect("opens");
    assert_eq!(body_texts(&doc), ["Д".repeat(50)]);
}

/// `s` as UTF-16LE bytes.
fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

#[test]
fn a_file_with_extended_characters_holds_utf16_text() {
    // fExtChar (0x1000): Word reads the text as 16-bit units.
    let text = utf16("Да ok\r");
    let d = Doc6 { flags: 0x1000, text: &text, ccp: Some(6), ..Default::default() };
    assert_eq!(texts(&d), ["Да ok"]);
}

#[test]
fn utf16_text_stops_at_the_end_of_its_bytes() {
    // Word reads no more 16-bit characters than the text's byte range (0x18..0x1C) holds, even
    // when the character count says more.
    let text = utf16("Hi\r");
    let d = Doc6 { flags: 0x1000, text: &text, ccp: Some(50), ..Default::default() };
    assert_eq!(texts(&d), ["Hi"]);
}

#[test]
fn text_stops_at_the_end_of_its_bytes() {
    // The same for 8-bit text: Word read 19 characters of a file whose count said 40. The font
    // table after the text must not be read as body text.
    let d = Doc6 { text: b"Hi\r", ccp: Some(50), fonts: FONTS, ..Default::default() };
    assert_eq!(texts(&d), ["Hi"]);
}

#[test]
fn fast_saved_utf16_pieces_are_at_plain_byte_offsets() {
    let text = utf16("World\rHello\r");
    let d = Doc6 { flags: 0x1000, text: &text, pieces: Some(&[(6, 12), (6, 0)]), ..Default::default() };
    assert_eq!(texts(&d), ["Hello", "World"]);
}

#[test]
fn a_macintosh_document_reads_as_mac_roman() {
    // FIB 0x14 = 0x0100: byte 0x8E is "é" (Mac Roman), not "Ž" (Windows-1252), even in a run whose
    // font is Cyrillic.
    let runs: &[(u32, u32, &[u8])] = &[(0, 2, &[0x5D, 0x01, 0x00]), (2, 3, &[])];
    let d = Doc6 { chse: 0x0100, text: b"H\x8E\r", fonts: FONTS, runs, ..Default::default() };
    assert_eq!(texts(&d), ["Hé"]);
}

#[test]
fn a_symbol_document_maps_characters_to_private_use() {
    // FIB 0x14 = 0x0002: every character from the space up is U+F000 + byte; marks stay marks.
    let d = Doc6 { chse: 0x0002, text: b"Hi\x8E\rA\r", ..Default::default() };
    assert_eq!(texts(&d), ["\u{F048}\u{F069}\u{F08E}", "\u{F041}"]);
}

#[test]
fn another_character_set_leaves_only_ascii_readable() {
    let d = Doc6 { chse: 0x0001, text: b"H\x8Ei\r", ..Default::default() };
    assert_eq!(texts(&d), ["H\u{FFFD}i"]);
}

#[test]
fn a_prm_on_the_marks_piece_formats_its_paragraph() {
    // As Word 95 fast-saves a 1" left indent on the first paragraph: the piece holding its mark
    // has Prm1 -> Prc `11 a0 05` (left indent 1440), and its PAPX says nothing. Word shows 72 pt;
    // the second paragraph keeps none.
    let text = b"First\rSecond\r";
    let pieces: &[(u32, u32)] = &[(5, 0), (1, 5), (7, 6)];
    let d = Doc6 { text, pieces: Some(pieces), prms: &[0, 0x0001], prcs: &[&[0x11, 0xA0, 0x05]], ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    let paras: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).collect();
    assert_eq!(paras.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["First", "Second"]);
    assert_eq!((paras[0].props.indent_left, paras[1].props.indent_left), (Some(72.0), None));
}

#[test]
fn a_pieces_prm_formats_its_text() {
    // As Word 95 fast-saves formatting: piece 0 has Prm1 -> a Prc setting font 1 and bold, which
    // its character run doesn't. Piece 0 shows in font 1, bold (and decodes in its code page);
    // piece 1 keeps the run's plain formatting.
    let text = b"\xC4\xE0\xC4\xE0\r";
    let d =
        Doc6 { text, fonts: FONTS, pieces: Some(&[(2, 0), (3, 2)]), prms: &[0x0001], prcs: &[&[0x5D, 0x01, 0x00, 0x55, 0x01]], ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    let p = first_para(&doc);
    assert_eq!(p.text, "ДаÄà");
    let first = &p.runs.first().expect("a run").props;
    assert_eq!((first.font.as_deref(), first.bold), (Some("Times New Roman Cyr"), Some(true)));
    let last = &p.runs.last().expect("a run").props;
    assert_eq!((last.font.as_deref(), last.bold), (None, None));
}

#[test]
fn a_font_in_a_pieces_prm_chooses_its_code_page() {
    // As Word 95 fast-saves a font change: piece 0 ("\xC4\xE0") has Prm1 -> Prc 0 = sprm 0x5D font 1
    // (Cyrillic), while its character run names no font. Piece 1 keeps the run's font 0.
    let text = b"\xC4\xE0\xC4\xE0\r";
    let d = Doc6 { text, fonts: FONTS, pieces: Some(&[(2, 0), (3, 2)]), prms: &[0x0001], prcs: &[&[0x5D, 0x01, 0x00]], ..Default::default() };
    assert_eq!(texts(&d), ["ДаÄà"]);
}

#[test]
fn a_prm_index_past_the_prcs_is_ignored() {
    let d = Doc6 { text: b"\xC4\xE0\r", fonts: FONTS, pieces: Some(&[(3, 0)]), prms: &[0x0011], prcs: &[&[0x5D, 0x01, 0x00]], ..Default::default() };
    assert_eq!(texts(&d), ["Äà"]);
}

#[test]
fn password_protected_files_are_refused() {
    let d = Doc6 { flags: 0x0100, text: b"secret\r", ..Default::default() };
    assert!(matches!(read(&word6_file(&d)), Err(DocbinError::Encryption(_))));
}

/// The first paragraph of `doc`'s body.
fn first_para(doc: &craft_wordmodel::Document) -> &craft_wordmodel::para::Paragraph {
    doc.body.iter().find_map(|b| b.as_para()).expect("a paragraph")
}

#[test]
fn character_formatting_is_read() {
    // Bold on, italic as the opposite of the (unread, so plain) style, 14 pt, red, single
    // underline, font 2: the Word 6 opcodes as observed.
    let g: &[u8] = &[0x55, 0x01, 0x56, 0x81, 0x63, 0x1C, 0x00, 0x62, 0x06, 0x5E, 0x01, 0x5D, 0x02, 0x00];
    let runs: &[(u32, u32, &[u8])] = &[(0, 2, g), (2, 3, &[])];
    let fonts = &[(0, "Times New Roman"), (2, "Symbol"), (0, "Arial")];
    let doc = read(&word6_file(&Doc6 { text: b"Hi\r", fonts, runs, ..Default::default() })).expect("opens");
    let p = first_para(&doc);
    let c = &p.runs.first().expect("a run").props;
    assert_eq!((c.bold, c.italic, c.size), (Some(true), Some(true), Some(14.0)));
    assert_eq!(c.color, Some(craft_wordmodel::props::TextColor::Rgb(craft_wordmodel::props::Rgb(0xFF, 0, 0))));
    assert_eq!(c.underline, Some(craft_wordmodel::props::Underline::Single));
    assert_eq!(c.font.as_deref(), Some("Arial"));
}

#[test]
fn unseen_operand_values_are_left_unread() {
    // Underline 6 (Word 97's thick) and "multiple" line spacing were never seen in a file Word 95
    // wrote, so they aren't guessed at.
    let runs: &[(u32, u32, &[u8])] = &[(0, 3, &[0x5E, 0x06])];
    let paras: &[(u32, u32, &[u8])] = &[(0, 3, &[0x14, 0xF0, 0x00, 0x01, 0x00])];
    let doc = read(&word6_file(&Doc6 { text: b"Hi\r", runs, paras, ..Default::default() })).expect("opens");
    let p = first_para(&doc);
    assert_eq!(p.runs.first().and_then(|r| r.props.underline), None);
    assert_eq!(p.props.line_spacing, None);
}

#[test]
fn an_unseen_alignment_value_is_left_unread() {
    // Only 0-3 were seen; 4 would otherwise become "distribute" and 0xFF "left".
    for jc in [0x04u8, 0xFF] {
        let g = [0x05, jc];
        let paras: &[(u32, u32, &[u8])] = &[(0, 3, &g)];
        let doc = read(&word6_file(&Doc6 { text: b"Hi\r", paras, ..Default::default() })).expect("opens");
        assert_eq!(first_para(&doc).props.align, None, "jc {jc:#04x}");
    }
}

#[test]
fn paragraph_formatting_is_read() {
    // Justified, 0.5" left and 0.25" right indents, a 0.5" hanging first line, at least 27 pt.
    let g: &[u8] = &[0x05, 0x03, 0x11, 0xD0, 0x02, 0x10, 0x68, 0x01, 0x13, 0x30, 0xFD, 0x14, 0x1C, 0x02, 0x00, 0x00];
    let paras: &[(u32, u32, &[u8])] = &[(0, 3, g), (3, 6, &[0x05, 0x01])];
    let doc = read(&word6_file(&Doc6 { text: b"Hi\rHo\r", paras, ..Default::default() })).expect("opens");
    let ps: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).collect();
    let p = &ps[0].props;
    assert_eq!(p.align, Some(craft_wordmodel::props::Align::Justify));
    assert_eq!((p.indent_left, p.indent_right, p.indent_first), (Some(36.0), Some(18.0), Some(-36.0)));
    assert_eq!(p.line_spacing, Some(craft_wordmodel::props::LineSpacing::AtLeast(27.0)));
    assert_eq!(ps[1].props.align, Some(craft_wordmodel::props::Align::Center));
}

/// The paragraph sprms Word 95 wrote for one property each (`para.doc`), as Word 2016 reads them
/// (its `.docx`): widow control off; 12 pt before and 18 pt after; keep together, keep with next
/// and page break before; no line numbers; no hyphenation; and a paragraph of a style that keeps
/// its lines together and has 6 pt before, turning both off (with the PAPX's padding byte). A
/// paragraph with none of them keeps widow control on, Word 95's default.
#[test]
fn paragraph_spacing_keep_and_widow_control_are_read() {
    let paras: &[(u32, u32, &[u8])] = &[
        (0, 2, &[0x33, 0x00]),
        (2, 4, &[0x15, 0xF0, 0x00, 0x16, 0x68, 0x01]),
        (4, 6, &[0x07, 0x01, 0x08, 0x01, 0x09, 0x01]),
        (6, 8, &[0x0E, 0x01]),
        (8, 10, &[0x2C, 0x01]),
        (10, 12, &[0x07, 0x00, 0x15, 0x00, 0x00, 0x00]),
        (12, 14, &[]),
    ];
    let styles = &[
        Some(Std { sgc: 1, base: 0x0FFF, name: "Normal", ..Default::default() }),
        Some(Std {
            sti: 0x0FFE,
            sgc: 1,
            base: 0,
            name: "Spaced",
            papx: &[0x07, 0x01, 0x08, 0x01, 0x15, 0x78, 0x00, 0x16, 0x3C, 0x00],
            ..Default::default()
        }),
    ];
    let d = Doc6 { text: b"a\rb\rc\rd\re\rf\rg\r", paras, para_styles: &[0, 0, 0, 0, 0, 1, 0], styles, ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    let ps: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.clone()).collect();
    assert_eq!(ps[0].widow_control, Some(false));
    assert_eq!((ps[1].space_before, ps[1].space_after), (Some(12.0), Some(18.0)));
    assert_eq!((ps[2].keep_lines, ps[2].keep_next, ps[2].page_break_before), (Some(true), Some(true), Some(true)));
    assert_eq!(ps[3].suppress_line_numbers, Some(true));
    assert_eq!(ps[4].suppress_hyphens, Some(true));
    assert_eq!((ps[5].keep_lines, ps[5].space_before), (Some(false), Some(0.0)));
    assert_eq!(ps[6].widow_control, None);
    let spaced = doc.styles.styles.iter().find(|s| s.name == "Spaced").expect("style");
    assert_eq!(
        (spaced.para.keep_lines, spaced.para.keep_next, spaced.para.space_before, spaced.para.space_after),
        (Some(true), Some(true), Some(6.0), Some(3.0))
    );
}

/// Tab stops as Word 95 wrote them (`tabs.doc`): 25 pt left, 150 pt centred and 300 pt right with
/// a dot leader in one paragraph; and in a paragraph of a style with stops at 50 pt and 250 pt
/// (right), the 50 pt one removed and one at 100 pt added. Word 2016 reads the same stops.
#[test]
fn tab_stops_are_read() {
    use craft_wordmodel::props::{TabAlign, TabLeader};
    let multi: &[u8] = &[0x0F, 0x0B, 0x00, 0x03, 0xF4, 0x01, 0xB8, 0x0B, 0x70, 0x17, 0x00, 0x01, 0x0A, 0x00];
    let changed: &[u8] = &[0x0F, 0x07, 0x01, 0xE8, 0x03, 0x01, 0xD0, 0x07, 0x00, 0x00];
    let paras: &[(u32, u32, &[u8])] = &[(0, 2, multi), (2, 4, changed)];
    let styles = &[
        Some(Std { sgc: 1, base: 0x0FFF, name: "Normal", ..Default::default() }),
        Some(Std {
            sti: 0x0FFE,
            sgc: 1,
            base: 0,
            name: "Tabbed",
            papx: &[0x0F, 0x08, 0x00, 0x02, 0xE8, 0x03, 0x88, 0x13, 0x00, 0x02],
            ..Default::default()
        }),
    ];
    let d = Doc6 { text: b"a\rb\r", paras, para_styles: &[0, 1], styles, ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    let tabs = |i: usize| -> Vec<(f32, TabAlign, TabLeader)> {
        let p = doc.body.iter().filter_map(|b| b.as_para()).nth(i).expect("para");
        p.props.tabs.iter().flatten().map(|t| (t.pos, t.align, t.leader)).collect()
    };
    assert_eq!(
        tabs(0),
        [(25.0, TabAlign::Left, TabLeader::None), (150.0, TabAlign::Center, TabLeader::None), (300.0, TabAlign::Right, TabLeader::Dot)]
    );
    assert_eq!(tabs(1), [(50.0, TabAlign::Clear, TabLeader::None), (100.0, TabAlign::Left, TabLeader::None)]);
    let tabbed = doc.styles.styles.iter().find(|s| s.name == "Tabbed").expect("style");
    let style_tabs: Vec<_> = tabbed.para.tabs.iter().flatten().map(|t| (t.pos, t.align)).collect();
    assert_eq!(style_tabs, [(50.0, TabAlign::Left), (250.0, TabAlign::Right)]);
}

/// The character sprms Word 95 wrote for one property each (`chr.doc`), as Word 2016 reads them:
/// superscript, subscript, raised 3 pt and lowered 2 pt (a position, not superscript), hidden,
/// expanded 1 pt and condensed 0.5 pt, kerning from 12 pt, strikethrough, outline, shadow, and
/// double underline. The toggles are 0x81, as Word 95 writes them over a plain style.
#[test]
fn character_effects_position_spacing_and_kerning_are_read() {
    use craft_wordmodel::props::{Underline, VertAlign};
    let runs: &[(u32, u32, &[u8])] = &[
        (0, 1, &[0x68, 0x01]),
        (1, 2, &[0x68, 0x02]),
        (2, 3, &[0x65, 0x06, 0x00]),
        (3, 4, &[0x65, 0xFC, 0xFF]),
        (4, 5, &[0x5C, 0x81]),
        (5, 6, &[0x60, 0x14, 0x00]),
        (6, 7, &[0x60, 0xF6, 0xFF]),
        (7, 8, &[0x6B, 0x18, 0x00]),
        (8, 9, &[0x57, 0x81]),
        (9, 10, &[0x58, 0x81]),
        (10, 11, &[0x59, 0x81]),
        (11, 12, &[0x5E, 0x03]),
        (12, 13, &[]),
    ];
    let doc = read(&word6_file(&Doc6 { text: b"abcdefghijkl\r", runs, ..Default::default() })).expect("opens");
    let r: Vec<_> = first_para(&doc).runs.iter().map(|r| r.props.clone()).collect();
    assert_eq!((r[0].vert_align, r[1].vert_align), (Some(VertAlign::Superscript), Some(VertAlign::Subscript)));
    assert_eq!((r[2].position, r[3].position, r[2].vert_align), (Some(3.0), Some(-2.0), None));
    assert_eq!(r[4].hidden, Some(true));
    assert_eq!((r[5].spacing, r[6].spacing), (Some(1.0), Some(-0.5)));
    assert_eq!(r[7].kern, Some(12.0));
    assert_eq!((r[8].strike, r[9].outline, r[10].shadow), (Some(true), Some(true), Some(true)));
    assert_eq!(r[11].underline, Some(Underline::Double));
}

/// Character styles as Word 95 writes them (`cs.doc`) and Word 2016 shows them: 0x50 names the
/// run's style; in a bold paragraph style, a bold character style is not bold, and `55 81` over
/// it is.
#[test]
fn character_styles_apply_to_their_runs() {
    let runs: &[(u32, u32, &[u8])] =
        &[(0, 1, &[0x50, 0x02, 0x00]), (1, 2, &[0x50, 0x02, 0x00, 0x55, 0x81]), (2, 4, &[]), (4, 5, &[0x50, 0x02, 0x00]), (5, 6, &[])];
    let paras: &[(u32, u32, &[u8])] = &[(0, 4, &[]), (4, 6, &[])];
    let styles = &[
        Some(Std { sgc: 1, base: 0x0FFF, name: "Normal", ..Default::default() }),
        Some(Std { sti: 0x0FFE, sgc: 1, base: 0, name: "BoldPara", chpx: &[0x55, 0x81], ..Default::default() }),
        Some(Std { sti: 0x0FFE, sgc: 2, base: 0x0FFF, name: "MyStrong", chpx: &[0x55, 0x81, 0x62, 0x06], ..Default::default() }),
    ];
    let d = Doc6 { text: b"abc\rd\r", runs, paras, para_styles: &[1, 0], styles, ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    let ps: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).collect();
    let runs = |i: usize| -> Vec<(Option<String>, bool)> {
        let p = ps[i];
        p.runs.iter().map(|r| (r.props.style.clone(), doc.styles.resolve_char(p.props.style.as_deref(), &r.props).bold)).collect()
    };
    let strong = || Some("MyStrong".to_owned());
    assert_eq!(runs(0), [(strong(), false), (strong(), true), (None, true)]);
    assert_eq!(runs(1), [(strong(), true)]);
    let style = doc.styles.get("MyStrong").expect("MyStrong");
    assert_eq!(style.kind, craft_wordmodel::styles::StyleKind::Character);
}

/// A Word 6 table definition sprm (0xBE): the cell edges, then a 10-byte descriptor per cell
/// whose first word holds the merge flags.
fn def_table(edges: &[i16], rgfs: &[u16]) -> Vec<u8> {
    let mut rest = vec![rgfs.len() as u8];
    for e in edges {
        rest.extend_from_slice(&e.to_le_bytes());
    }
    for r in rgfs {
        rest.extend_from_slice(&r.to_le_bytes());
        rest.extend_from_slice(&[0; 8]);
    }
    let mut v = vec![0xBE];
    v.extend_from_slice(&((rest.len() + 1) as u16).to_le_bytes());
    v.extend_from_slice(&rest);
    v
}

/// The rows of the first table in `doc`'s body, each as its cells' texts.
fn table_rows(doc: &craft_wordmodel::Document) -> Vec<Vec<String>> {
    let t = doc.body.iter().find_map(|b| match b.as_ref() {
        craft_wordmodel::Block::Table(t) => Some(t),
        _ => None,
    });
    let t = t.expect("a table");
    t.rows
        .iter()
        .map(|r| r.cells.iter().map(|c| c.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect::<Vec<_>>().join("|")).collect())
        .collect()
}

#[test]
fn table_rows_and_merged_cells_are_read() {
    // Row 1: "A", "B". Row 2: "C" merged across both cells. In-table marks carry 0x18, the row
    // ends 0x18, 0x19 and the table definition.
    let text = b"A\x07B\x07\x07C\x07\x07\x07after\r";
    let mut row1 = vec![0x18, 1, 0x19, 1];
    row1.extend(def_table(&[0, 1440, 2880], &[0, 0]));
    let mut row2 = vec![0x18, 1, 0x19, 1];
    row2.extend(def_table(&[0, 1440, 2880], &[0x0001, 0x0002]));
    let cell: &[u8] = &[0x18, 1];
    let paras: &[(u32, u32, &[u8])] = &[(0, 2, cell), (2, 4, cell), (4, 5, &row1), (5, 7, cell), (7, 8, cell), (8, 9, &row2), (9, 15, &[])];
    let doc = read(&word6_file(&Doc6 { text, paras, ..Default::default() })).expect("opens");
    assert_eq!(table_rows(&doc), [vec!["A", "B"], vec!["C"]]);
    assert_eq!(body_texts(&doc), ["after"], "the paragraph after the table stays outside it");
}

#[test]
fn several_merges_in_one_row_each_keep_their_cells() {
    // Five cells merged as [A B] [C] [D E]: the second merge must not be shifted by the first.
    let text = b"A\x07B\x07C\x07D\x07E\x07\x07";
    let mut row = vec![0x18, 1, 0x19, 1];
    row.extend(def_table(&[0, 100, 200, 300, 400, 500], &[0x0001, 0x0002, 0x0000, 0x0001, 0x0002]));
    let cell: &[u8] = &[0x18, 1];
    let paras: &[(u32, u32, &[u8])] = &[(0, 2, cell), (2, 4, cell), (4, 6, cell), (6, 8, cell), (8, 10, cell), (10, 11, &row)];
    let doc = read(&word6_file(&Doc6 { text, paras, ..Default::default() })).expect("opens");
    assert_eq!(table_rows(&doc), [vec!["A|B", "C", "D|E"]]);
}

#[test]
fn a_field_shows_its_cached_result() {
    let d = Doc6 { text: b"Date \x13DATE \x1412/12/97\x15\r", ..Default::default() };
    let text = read(&word6_file(&d)).expect("opens").plain_text(craft_wordmodel::StoryRef::Body);
    assert!(text.contains("Date 12/12/97"), "{text:?}");
    assert!(!text.contains("DATE"), "{text:?}");
}

/// A fixture's paragraph styles: Normal, an empty slot, then "Mine" (user-defined, centred, bold, on
/// Normal) and "Heading" stored under a local name with sti 1.
const STYLES: &[Option<Std>] = &[
    Some(Std { sti: 0, sgc: 1, base: 0x0FFF, name: "Standard", papx: &[], chpx: &[0x61, 0x09, 0x04] }),
    None,
    Some(Std { sti: 0x0FFE, sgc: 1, base: 0, name: "Mine", papx: &[0x05, 0x01], chpx: &[0x55, 0x01] }),
    Some(Std { sti: 1, sgc: 1, base: 0, name: "Überschrift 1", papx: &[0x08, 0x01], chpx: &[0x55, 0x81, 0x63, 0x20, 0x00] }),
];

/// Each paragraph's style name, through the document's style sheet.
fn style_names(doc: &craft_wordmodel::Document) -> Vec<String> {
    doc.body
        .iter()
        .filter_map(|b| b.as_para())
        .map(|p| p.props.style.as_deref().and_then(|id| doc.styles.get(id)).map_or(String::new(), |s| s.name.clone()))
        .collect()
}

#[test]
fn paragraphs_take_their_styles() {
    let paras: &[(u32, u32, &[u8])] = &[(0, 2, &[]), (2, 4, &[]), (4, 6, &[])];
    let d = Doc6 { text: b"A\rB\rC\r", paras, para_styles: &[0, 2, 3], styles: STYLES, ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    // A built-in style goes by its built-in name, whatever the file stores (as in Word).
    assert_eq!(style_names(&doc), ["Normal", "Mine", "Heading 1"]);
}

#[test]
fn styles_carry_their_formatting_and_base() {
    let d = Doc6 { text: b"A\r", styles: STYLES, ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    let mine = doc.styles.styles.iter().find(|s| s.name == "Mine").expect("Mine");
    assert_eq!(mine.para.align, Some(craft_wordmodel::props::Align::Center));
    assert_eq!(mine.chr.bold, Some(true));
    assert_eq!(mine.based_on.as_deref().and_then(|id| doc.styles.get(id)).map(|s| s.name.as_str()), Some("Normal"));
    let heading = doc.styles.styles.iter().find(|s| s.name == "Heading 1").expect("Heading 1");
    assert_eq!((heading.para.keep_next, heading.chr.size), (Some(true), Some(16.0)));
}

#[test]
fn bulleted_styles_frame_past_their_numbering() {
    // 0x0D (1 byte) and 0x0C (a count byte and its bytes) before the indents, as in the corpus.
    let mut papx = vec![0x05, 0x03, 0x0D, 0x0B, 0x11, 0x1C, 0x01, 0x13, 0xE4, 0xFE, 0x0C, 4, 0xFF, 0x01, 0x00, 0x08];
    papx.extend_from_slice(&[0x0E, 0x01, 0x2C, 0x00, 0x1D, 0x60, 0x1A, 0xF8, 0xFF, 0x25, 0x02, 0x10, 0x68, 0x01]);
    let styles = &[Some(Std { sgc: 1, base: 0x0FFF, name: "Normal", papx: &papx, ..Default::default() })];
    let doc = read(&word6_file(&Doc6 { text: b"A\r", styles, ..Default::default() })).expect("opens");
    let normal = doc.styles.get("Normal").expect("Normal");
    assert_eq!((normal.para.indent_left, normal.para.indent_first, normal.para.indent_right), (Some(14.2), Some(-14.2), Some(18.0)));
}

#[test]
fn a_runs_language_or_its_styles_chooses_the_code_page_without_a_font() {
    // No font named anywhere: the style's language decides (Russian -> Windows-1251), as in Word.
    let styles = &[Some(Std { sgc: 1, base: 0x0FFF, name: "Normal", chpx: &[0x61, 0x19, 0x04], ..Default::default() })];
    let d = Doc6 { text: b"\xC4\xE0\r", fonts: FONTS, styles, ..Default::default() };
    assert_eq!(texts(&d), ["Да"]);
    // The run's own language outranks the style's.
    let runs: &[(u32, u32, &[u8])] = &[(0, 3, &[0x61, 0x09, 0x04])];
    let d = Doc6 { text: b"\xC4\xE0\r", fonts: FONTS, styles, runs, ..Default::default() };
    assert_eq!(texts(&d), ["Äà"]);
}

#[test]
fn font_0_is_not_a_fallback() {
    // Font 0 is Cyrillic, but nothing names it and the style's language is English: Word reads
    // Windows-1252.
    let fonts = &[(204, "Times New Roman Cyr"), (0, "Arial")];
    let styles = &[Some(Std { sgc: 1, base: 0x0FFF, name: "Normal", chpx: &[0x61, 0x09, 0x04], ..Default::default() })];
    // A character run that names no font (where the reader used to take font 0).
    let runs: &[(u32, u32, &[u8])] = &[(0, 3, &[])];
    let d = Doc6 { text: b"\xC4\xE0\r", fonts, styles, runs, ..Default::default() };
    assert_eq!(texts(&d), ["Äà"]);
}

#[test]
fn a_font_named_in_the_style_chooses_the_code_page() {
    // The base style names the Cyrillic font 1; the run and its own style name none.
    let styles = &[
        Some(Std { sgc: 1, base: 0x0FFF, name: "Normal", chpx: &[0x5D, 0x01, 0x00], ..Default::default() }),
        Some(Std { sti: 0x0FFE, sgc: 1, base: 0, name: "Child", ..Default::default() }),
    ];
    let paras: &[(u32, u32, &[u8])] = &[(0, 3, &[])];
    let d = Doc6 { text: b"\xC4\xE0\r", fonts: FONTS, styles, paras, para_styles: &[1], ..Default::default() };
    assert_eq!(texts(&d), ["Да"]);
}

#[test]
fn an_ansi_font_leaves_the_code_page_to_the_language() {
    // Arial (ANSI) named on the run, the style's language Russian: Word reads Cyrillic.
    let fonts = &[(0, "Times New Roman"), (0, "Arial")];
    let styles = &[Some(Std { sgc: 1, base: 0x0FFF, name: "Normal", chpx: &[0x61, 0x19, 0x04], ..Default::default() })];
    let runs: &[(u32, u32, &[u8])] = &[(0, 2, &[0x5D, 0x01, 0x00]), (2, 3, &[])];
    let d = Doc6 { text: b"\xC4\xE0\r", fonts, styles, runs, ..Default::default() };
    assert_eq!(texts(&d), ["Да"]);
}

/// A fixture with every structure read: runs, fonts and a piece table.
fn full_stream() -> Vec<u8> {
    let runs: &[(u32, u32, &[u8])] = &[(0, 6, &[0x55, 0x81]), (6, 9, &[0x5D, 0x01, 0x00]), (9, 12, &[])];
    let styles = &[
        Some(Std { sgc: 1, base: 0x0FFF, name: "Normal", chpx: &[0x61, 0x09, 0x04], ..Default::default() }),
        None,
        Some(Std { sti: 0x0FFE, sgc: 1, base: 0, name: "Mine", papx: &[0x05, 0x01], chpx: &[0x55, 0x01] }),
    ];
    let paras: &[(u32, u32, &[u8])] = &[(0, 6, &[]), (6, 12, &[])];
    word_stream(&Doc6 {
        // The main text, then a footnote's story, a header's, and the final mark.
        text: b"Hello\r\xC4\xE0\rx\x02\r\x02F\rH\r\r\r",
        fonts: FONTS,
        runs,
        paras,
        para_styles: &[2, 0],
        styles,
        pieces: Some(&[(6, 0), (13, 6)]),
        ccp: Some(12),
        stories: [3, 3, 0, 0, 0],
        sections: &[(19, &[0x8E, 0x00, 0x99, 0x02])],
        hdd: &[0, 3, 4],
        footnotes: (&[10, 12], &[0, 3, 4]),
        // A custom-mark FRD (its reference is 0x02, so it stays a numbered one) and a Dop.
        footnote_frds: Some(&[0, 0]),
        dop: &[0x43, 0x00, 0x0E, 0x00],
        ..Default::default()
    })
}

#[test]
fn the_full_fixture_reads() {
    let f = cfb_file(&[("WordDocument", &full_stream())]);
    let doc = read(&f).expect("opens");
    assert_eq!(body_texts(&doc), ["Hello", "Да", "x\u{FFFC}"]);
    assert_eq!(style_names(&doc)[0], "Mine");
    assert_eq!(part_text(&doc, doc.last_section.headers.default), Some("H".into()));
    assert_eq!(notes_of(&doc).len(), 1);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn corrupt_word6_streams_never_panic(edits in proptest::collection::vec((0..0x700usize, any::<u8>()), 1..16)) {
        let mut w = full_stream();
        for (at, b) in edits {
            if let Some(x) = w.get_mut(at) {
                *x = b;
            }
        }
        let _ = read(&cfb_file(&[("WordDocument", &w)]));
    }

    #[test]
    fn truncated_word6_streams_never_panic(n in 0..0x700usize) {
        let w = full_stream();
        let _ = read(&cfb_file(&[("WordDocument", &w[..n.min(w.len())])]));
    }
}

/// The paragraphs' text and the section each ends, if any.
fn sections_of(doc: &craft_wordmodel::Document) -> Vec<craft_wordmodel::section::SectionProps> {
    let mut out: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()?.section.as_deref().cloned()).collect();
    out.push(doc.last_section.clone());
    out
}

/// A part's paragraphs' text.
fn part_text(doc: &craft_wordmodel::Document, id: Option<u32>) -> Option<String> {
    let part = doc.parts.get(&id?)?;
    Some(part.blocks.iter().filter_map(|b| b.as_para().map(|p| p.text.clone())).collect::<Vec<_>>().join("|"))
}

#[test]
fn sections_take_their_properties() {
    // As Word 95 writes them (`secprops`, `sectypes`): section 2 starts on the same page and is
    // landscape (0x8E 0, 0xA2 2, page 15840 x 12240); section 3 has 3 columns 360 apart and its
    // own margins (0x90, 0x91, 0xA6-0xA9) and header/footer distances (0x9C, 0x9D), and starts
    // on an odd page (0x8E 4). Word shows each so.
    let s2: &[u8] = &[0x8E, 0x00, 0xA2, 0x02, 0xA4, 0xE0, 0x3D, 0xA5, 0xD0, 0x2F];
    let s3: &[u8] = &[
        0x8E, 0x04, 0x90, 0x02, 0x00, 0x91, 0x68, 0x01, 0x9C, 0x38, 0x04, 0x9D, 0x84, 0x03, 0xA6, 0x40, 0x0B, 0xA7, 0xD0, 0x02, 0xA8, 0x70, 0x08,
        0xA9, 0x38, 0x04,
    ];
    let d = Doc6 { text: b"One\r\x0cTwo\r\x0cThree\r", sections: &[(5, &[]), (10, s2), (16, s3)], ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    assert_eq!(body_texts(&doc), ["One", "", "Two", "", "Three"]);
    let s = sections_of(&doc);
    assert_eq!(s.len(), 3);
    use craft_wordmodel::section::SectionStart;
    assert_eq!((s[0].start, s[0].landscape, s[0].page_w), (SectionStart::NextPage, false, 612.0));
    assert_eq!((s[1].start, s[1].landscape, s[1].page_w, s[1].page_h), (SectionStart::Continuous, true, 792.0, 612.0));
    assert_eq!((s[2].start, s[2].columns.count), (SectionStart::OddPage, 3));
    assert_eq!((s[2].margin_left, s[2].margin_right, s[2].margin_top, s[2].margin_bottom), (144.0, 36.0, 108.0, 54.0));
    assert_eq!((s[2].header, s[2].footer), (54.0, 45.0));
}

#[test]
fn the_header_table_holds_only_the_stories_a_section_names() {
    // As `hf3`: one section naming all six stories (0x99 = 0x3F), stored in the order even header,
    // odd header, even footer, odd footer, first header, first footer, each with its guard mark.
    let text = b"Body\rHE\r\rHO\r\rFE\r\rFO\r\rHF\r\rFF\r\r\r";
    let hdd: &[u32] = &[0, 4, 8, 12, 16, 20, 24, 25];
    let d = Doc6 { text, stories: [0, 25, 0, 0, 0], sections: &[(30, &[0x8F, 0x01, 0x99, 0x3F])], hdd, ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    let s = &doc.last_section;
    assert!(s.title_page);
    let got = [s.headers.even, s.headers.default, s.footers.even, s.footers.default, s.headers.first, s.footers.first].map(|id| part_text(&doc, id));
    assert_eq!(got, ["HE", "HO", "FE", "FO", "HF", "FF"].map(|t| Some(t.to_string())));
}

#[test]
fn a_story_a_section_doesnt_name_is_the_previous_sections() {
    // As `hfsec`: section 1 names its odd header and footer (0x0A), section 2 only its header
    // (0x02). The table holds three stories; Word shows section 2's footer linked to section 1's.
    let text = b"One\r\x0cTwo\rH1\r\rF1\r\rH2\r\r\r";
    let hdd: &[u32] = &[0, 4, 8, 12, 13];
    let d = Doc6 { text, stories: [0, 13, 0, 0, 0], sections: &[(5, &[0x99, 0x0A]), (22, &[0x99, 0x02])], hdd, ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    let s = sections_of(&doc);
    assert_eq!((part_text(&doc, s[0].headers.default), part_text(&doc, s[0].footers.default)), (Some("H1".into()), Some("F1".into())));
    assert_eq!((part_text(&doc, s[1].headers.default), s[1].footers.default), (Some("H2".into()), None));
}

#[test]
fn separator_stories_come_first() {
    // As `seps`: the Dop's byte 1 names a footnote separator (bit 0); it comes before the
    // section's header in the table, and isn't a header.
    let text = b"Body\rSEP\r\rHDR\r\r\r";
    let hdd: &[u32] = &[0, 5, 10, 11];
    let d = Doc6 { text, stories: [0, 11, 0, 0, 0], sections: &[(16, &[0x99, 0x02])], hdd, dop: &[0x02, 0x01, 0x04, 0x00], ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    assert_eq!(part_text(&doc, doc.last_section.headers.default), Some("HDR".into()));
    // ... and it is the document's footnote separator.
    let sep = doc.footnote_separators.separator.as_ref().and_then(|b| b.first()).and_then(|b| b.as_para()).map(|p| p.text.clone());
    assert_eq!(sep.as_deref(), Some("SEP"));
}

#[test]
fn a_fast_saved_section_mark_adds_its_pieces_sprms() {
    // As `cn.orthodox…DOC`: the SEPX doesn't name the title page or the header; the Prc of the
    // piece holding the last section's mark (its last CP, after every story) does (8F 01, 99 02).
    // Word shows both.
    let text = b"Body\rHDR\r\r\r";
    let pieces: &[(u32, u32)] = &[(5, 0), (5, 5), (1, 10)];
    let d = Doc6 {
        text,
        pieces: Some(pieces),
        prms: &[0, 0, 0x0001],
        prcs: &[&[0x8F, 0x01, 0x99, 0x02]],
        ccp: Some(5),
        stories: [0, 5, 0, 0, 0],
        sections: &[(11, &[0xA1, 0x01, 0x00])],
        hdd: &[0, 5, 6],
        ..Default::default()
    };
    let doc = read(&word6_file(&d)).expect("opens");
    assert!(doc.last_section.title_page);
    assert_eq!(part_text(&doc, doc.last_section.headers.default), Some("HDR".into()));
}

/// Each note reference's kind and text, in body order.
fn notes_of(doc: &craft_wordmodel::Document) -> Vec<(craft_wordmodel::para::NoteKind, String)> {
    let mut out = Vec::new();
    for p in doc.body.iter().filter_map(|b| b.as_para()) {
        for o in &p.objects {
            if let craft_wordmodel::para::InlineObject::NoteRef { kind, id, .. } = o {
                out.push((*kind, part_text(doc, Some(*id)).unwrap_or_default()));
            }
        }
    }
    out
}

#[test]
fn footnotes_and_endnotes_are_read() {
    // As `fnen`: two footnotes (pairs 2 and 3) and an endnote (pairs 46 and 47, after the gap in
    // the pairs), each story after the main text in its own subdocument.
    use craft_wordmodel::para::NoteKind;
    let text = b"A\x02 B\x02 C\x02\r\x02 F1\r\x02 F3\r\r\x02 E2\r\r\r";
    let d =
        Doc6 { text, stories: [11, 0, 0, 0, 7], footnotes: (&[1, 7, 12], &[0, 5, 10, 11]), endnotes: (&[4, 12], &[0, 5, 7]), ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    // A note story's own reference mark links to nothing, so only the space after it is kept.
    let want = [(NoteKind::Footnote, " F1"), (NoteKind::Endnote, " E2"), (NoteKind::Footnote, " F3")].map(|(k, t)| (k, t.to_string()));
    assert_eq!(notes_of(&doc), want);
}

#[test]
fn comments_are_read_and_anchored_at_their_references() {
    // As Word 95 writes two comments (`atn2`): 0x05 at each reference, the stories after the main
    // text in the comment subdocument (0x44), 20-byte ATRDs and 8-bit authors. Word reads Bob
    // Brown/BB and Ann Able/AA, no date, each covering its reference's place.
    use craft_wordmodel::para::InlineObject;
    let atrd = |initials: &[u8], ibst: u8| {
        let mut v = vec![0u8; 20];
        v[0] = initials.len() as u8;
        v[1..1 + initials.len()].copy_from_slice(initials);
        v[10] = ibst;
        v[16..20].copy_from_slice(&[0xFF; 4]);
        v
    };
    let atrds = [atrd(b"BB", 0), atrd(b"AA", 1)].concat();
    let text = b"Alpha\x05 beta\x05\r\x05 C-BOB\r\x05 C-ANN\r\r";
    let d = Doc6 {
        text,
        stories: [0, 0, 0, 17, 0],
        comments: (&[5, 11, 13], &[0, 8, 16, 17], &atrds),
        owners: b"\x09Bob Brown\x08Ann Able",
        ..Default::default()
    };
    let doc = read(&word6_file(&d)).expect("opens");
    let p = first_para(&doc);
    assert_eq!(p.text, "Alpha\u{FFFC}\u{FFFC} beta\u{FFFC}\u{FFFC}");
    let anchors: Vec<_> = p
        .objects
        .iter()
        .map(|o| match o {
            InlineObject::CommentStart { id } => format!("<{id}"),
            InlineObject::CommentEnd { id } => format!("{id}>"),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(anchors, ["<0", "0>", "<1", "1>"]);
    let comments: Vec<_> =
        doc.comments.values().map(|c| (c.author.as_str(), c.initials.as_str(), c.date.as_str(), part_text(&doc, Some(c.part)))).collect();
    assert_eq!(comments, [("Bob Brown", "BB", "", Some(" C-BOB".into())), ("Ann Able", "AA", "", Some(" C-ANN".into()))]);
}

#[test]
fn a_custom_note_mark_is_read_as_the_mark() {
    // As Word 95 writes a footnote marked "*" (`fn`): FRD 0, the "*" at the reference in its own
    // run, and again at the start of the note's text. Word reads a custom mark "*".
    let text = b"Alpha* end\r* FN\r\r";
    let runs: &[(u32, u32, &[u8])] = &[(0, 5, &[]), (5, 6, &[0x55, 0x01]), (6, 17, &[])];
    let d = Doc6 { text, runs, stories: [6, 0, 0, 0, 0], footnotes: (&[5, 12], &[0, 5, 6]), footnote_frds: Some(&[0, 0]), ..Default::default() };
    let doc = read(&word6_file(&d)).expect("opens");
    let p = first_para(&doc);
    assert_eq!(p.text, "Alpha\u{FFFC} end");
    let craft_wordmodel::para::InlineObject::NoteRef { custom, id, .. } = p.objects.first().expect("a note") else { panic!("not a note") };
    assert_eq!((custom.as_str(), part_text(&doc, Some(*id))), ("*", Some("* FN".into())));
}

#[test]
fn note_options_and_facing_pages_come_from_the_dop() {
    // As Word 95 writes them (`noteopts`, `hf3`): Dop byte 0 0x43 (facing pages, fpc 2), rncFtn 2
    // and nFtn 3 at 2, nEdn 2 at 0x34, and at 0x36 epc 0 with nfcFtnRef 2 and nfcEdnRef 3 (0x00C8).
    // Word reads footnotes beneath the text, i ii iii from iii restarting each page, endnotes at
    // the end of each section, A B C from B, and different odd and even pages.
    use craft_wordmodel::section::{NotePos, NoteRestart, NumFormat};
    let mut dop = vec![0u8; 0x58];
    dop[0] = 0x43;
    dop[2..4].copy_from_slice(&0x000Eu16.to_le_bytes());
    dop[0x34..0x36].copy_from_slice(&0x0008u16.to_le_bytes());
    dop[0x36..0x38].copy_from_slice(&0x00C8u16.to_le_bytes());
    let d = Doc6 { text: b"Text\r", dop: &dop, ..Default::default() };
    let st = read(&word6_file(&d)).expect("opens").settings;
    assert!(st.even_odd_headers);
    let ftn = (st.footnote_pr.pos, st.footnote_pr.num_start, st.footnote_pr.num_restart, st.footnote_format);
    assert_eq!(ftn, (Some(NotePos::BeneathText), Some(3), Some(NoteRestart::EachPage), NumFormat::LowerRoman));
    assert_eq!((st.endnote_pr.pos, st.endnote_pr.num_start, st.endnote_format), (Some(NotePos::SectEnd), Some(2), NumFormat::UpperLetter));
}
