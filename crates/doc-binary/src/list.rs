//! Lists: `PlfLst` (the LSTF definitions with their LVLs appended) and `PlfLfo` (the LFO
//! instances with LFOLVL overrides) resolved into the numbering model
//! ([MS-DOC] §2.8.1 "Determining List Formatting of a Paragraph", §2.9).

use craft_wordmodel::numbering::{AbstractNum, Level, LevelSuffix, Num, Numbering};
use craft_wordmodel::props::{Align, CharProps, ParaProps};
use craft_wordmodel::section::NumFormat;

use crate::fib::{Fib, pair};
use crate::fmt;
use crate::sprm;

/// Lists (and therefore LFOs) we are willing to read.
const MAX_LISTS: usize = 2_000;
/// Override levels per LFO ([MS-DOC] LFOLVL count is at most 9 by construction).
const MAX_LFO_LVL: usize = 9;

/// One list definition: an LSTF plus its levels (one if `fSimpleList`, else nine).
struct ListDef {
    lsid: i32,
    levels: Vec<RawLvl>,
}

/// One LVL, as stored ([MS-DOC] §2.9.182): LVLF fields, the two grpprls and the number text.
struct RawLvl {
    start: u32,
    nfc: u8,
    jc: u8,
    flags: u8,
    ixch_follow: u8,
    rgbxch_nums: [u8; 9],
    papx: Vec<u8>,
    chpx: Vec<u8>,
    /// The number text with each placeholder left as its raw char.
    xst: Vec<u16>,
}

/// A bullet level's character: a symbol-font character (0xF000 to 0xF0FF, the font's own code
/// in the low byte) with the common bullet glyphs mapped to their Unicode equivalents so every
/// font renders them; any other character is itself.
///
/// The same byte is a different glyph in Symbol and in Wingdings, so a level in either font maps
/// it from that font's own table: Symbol from the Unicode Consortium's Adobe Symbol to Unicode
/// table (`MAPPINGS/VENDORS/ADOBE/symbol.txt`, table version 1.0, 2011-07-12), Wingdings from WG2
/// N4384 (L2/12-368, 2012-11-02), whose ID w-1nnn is Wingdings byte nnn in decimal. A level in
/// another font, or with none, reads as before.
fn bullet_char(c: u16, font: Option<&str>) -> char {
    if !(0xF000..=0xF0FF).contains(&c) {
        return char::from_u32(u32::from(c)).unwrap_or('\u{2022}');
    }
    let byte = c & 0x00FF;
    let is = |name: &str| font.is_some_and(|f| f.eq_ignore_ascii_case(name));
    let mapped = if is("Symbol") {
        match byte {
            0xB7 => Some('\u{2022}'), // BULLET
            0xA7 => Some('\u{2663}'), // BLACK CLUB SUIT
            0x6F => Some('\u{03BF}'), // GREEK SMALL LETTER OMICRON
            0x77 => Some('\u{03C9}'), // GREEK SMALL LETTER OMEGA
            0x78 => Some('\u{03BE}'), // GREEK SMALL LETTER XI
            _ => None,
        }
    } else if is("Wingdings") {
        match byte {
            0xA7 => Some('\u{25AA}'),  // w-1167 BLACK SMALL SQUARE
            0x6F => Some('\u{1F78F}'), // w-1111 MEDIUM WHITE SQUARE
            0x77 => Some('\u{1F799}'), // w-1119 BLACK MEDIUM SMALL DIAMOND
            0x78 => Some('\u{2327}'),  // w-1120 X IN A RECTANGLE BOX
            0xB7 => Some('\u{1F550}'), // w-1183 CLOCK FACE ONE OCLOCK
            _ => None,
        }
    } else {
        match byte {
            0xB7 => Some('\u{2022}'),
            0xA7 => Some('\u{25AA}'),
            0x6F => Some('\u{25CB}'),
            0x77 | 0x78 => Some('\u{25CF}'),
            _ => None,
        }
    };
    mapped.unwrap_or_else(|| char::from_u32(u32::from(byte)).unwrap_or('\u{2022}'))
}

/// MSONFC → model number format ([MS-OSHARED] §2.2.1.3; 0x17 = bullets, 0xFF = none). Codes
/// 0x00 to 0x27 run in the order of ST_NumberFormat (decimal, upperRoman … 0x16 decimalZero, 0x17
/// bullet … 0x25 chineseCounting, 0x26 chineseLegalSimplified, 0x27 chineseCountingThousand);
/// later codes are drawn as decimal.
pub(crate) fn num_format(nfc: u8) -> NumFormat {
    match nfc {
        0xFF => NumFormat::None,
        0x00..=0x27 => NumFormat::ALL.get(nfc as usize).copied().unwrap_or(NumFormat::Decimal),
        _ => NumFormat::Decimal,
    }
}

#[cfg(test)]
#[test]
fn number_format_codes() {
    assert_eq!(num_format(0x04), NumFormat::LowerLetter);
    assert_eq!(num_format(0x16), NumFormat::DecimalZero);
    assert_eq!(num_format(0x17), NumFormat::Bullet);
    assert_eq!(num_format(0x25), NumFormat::ChineseCounting);
    assert_eq!(num_format(0x27), NumFormat::ChineseCountingThousand);
    assert_eq!(num_format(0x60), NumFormat::Decimal);
}

/// craft-libs#2764: a symbol-font bullet is read from its level's font, so the same byte in
/// Symbol and in Wingdings gives different characters.
#[cfg(test)]
#[test]
fn a_symbol_font_bullet_is_read_from_its_level_font() {
    let fonts = ["Times New Roman".to_string(), "Symbol".to_string(), "Wingdings".to_string()];
    let bullet = |c: u16, font: Option<u16>| {
        // sprmCRgFtc0 naming the level's font by its index.
        let chpx = font.map(|i| [&0x4A4Fu16.to_le_bytes()[..], &i.to_le_bytes()[..]].concat()).unwrap_or_default();
        let raw = RawLvl { start: 1, nfc: 0x17, jc: 0, flags: 0, ixch_follow: 0, rgbxch_nums: [0; 9], papx: Vec::new(), chpx, xst: vec![c] };
        level_of(&raw, 0, &fonts).text
    };
    for c in [0xF06F, 0xF077, 0xF0A7, 0xF0B7] {
        let (symbol, wingdings) = (bullet(c, Some(1)), bullet(c, Some(2)));
        assert_ne!(symbol, wingdings, "{c:#06x}");
        assert_eq!(symbol.chars().count(), 1);
        assert_eq!(wingdings.chars().count(), 1);
    }
    // Symbol's bullet is the bullet, and a level in another font, or none, reads as before.
    assert_eq!(bullet(0xF0B7, Some(1)), "\u{2022}");
    for c in [0xF06F, 0xF0A7, 0xF0B7] {
        assert_eq!(bullet(c, Some(0)), bullet(c, None), "{c:#06x}");
    }
}

/// Read the lists of a document; empty when the FIB carries none (or they are unreadable).
pub(crate) fn parse(table: &[u8], fib: &Fib, fonts: &[String]) -> Numbering {
    let defs = match fib.pair(pair::PLF_LST) {
        Some((fc, _)) => parse_plf_lst(table, fc),
        None => Vec::new(),
    };
    if defs.is_empty() {
        return Numbering::default();
    }
    let mut numbering = Numbering::default();
    for (i, def) in defs.iter().enumerate() {
        numbering.abstracts.push(AbstractNum {
            id: i as u32,
            name: None,
            levels: def.levels.iter().enumerate().map(|(j, l)| level_of(l, j, fonts)).collect(),
            ..Default::default()
        });
    }
    if let Some((fc, lcb)) = fib.pair(pair::PLF_LFO)
        && lcb > 0
    {
        parse_plf_lfo(table, fc, &defs, &mut numbering, fonts);
    }
    numbering
}

/// `PlfLst`: cLst u16, then cLst × LSTF (28 bytes each); the LVLs follow the whole array,
/// one per simple list and nine per multi-level list, in LSTF order. `lcbPlfLst` covers only
/// the LSTFs, so the LVLs are read past it, bounded by the stream.
fn parse_plf_lst(table: &[u8], fc: u32) -> Vec<ListDef> {
    let at = fc as usize;
    let Some(c_lst) = table.get(at..at.saturating_add(2)).map(|b| u16::from_le_bytes([b[0], b[1]])) else { return Vec::new() };
    let c_lst = c_lst as usize;
    if c_lst == 0 || c_lst > MAX_LISTS {
        return Vec::new();
    }
    let mut defs = Vec::with_capacity(c_lst);
    // LSTF: lsid i32, tplc, rgistdPara[9], flags byte (bit 0 = fSimpleList), grfhic.
    let mut counts = Vec::with_capacity(c_lst);
    for i in 0..c_lst {
        let f = at.saturating_add(2).saturating_add(i.saturating_mul(28));
        let lsid = table.get(f..f.saturating_add(4)).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(-1);
        let simple = table.get(f.saturating_add(26)).copied().unwrap_or(0) & 1 != 0;
        defs.push(ListDef { lsid, levels: Vec::new() });
        counts.push(if simple { 1 } else { 9 });
    }
    let mut off = at.saturating_add(2).saturating_add(c_lst.saturating_mul(28));
    for (def, n) in defs.iter_mut().zip(counts) {
        for _ in 0..n {
            match parse_lvl(table, off) {
                Some((lvl, len)) => {
                    def.levels.push(lvl);
                    off = off.saturating_add(len);
                }
                None => return defs,
            }
        }
    }
    defs
}

/// One LVL ([MS-DOC] §2.9.182): LVLF (28 bytes) + grpprlPapx + grpprlChpx + Xst.
/// Returns the level and its total length in bytes.
fn parse_lvl(table: &[u8], at: usize) -> Option<(RawLvl, usize)> {
    let lvlf = table.get(at..at.saturating_add(28))?;
    let start = u32::from_le_bytes([lvlf[0], lvlf[1], lvlf[2], lvlf[3]]);
    let nfc = lvlf[4];
    // Byte 5 packs jc (bits 0-1), fLegal (bit 2), fNoRestart (bit 3), …
    let jc_flags = lvlf[5];
    let mut rgbxch_nums = [0u8; 9];
    rgbxch_nums.copy_from_slice(&lvlf[6..15]);
    let ixch_follow = lvlf[15];
    let cb_chpx = lvlf[24] as usize;
    let cb_papx = lvlf[25] as usize;
    let mut at = at.saturating_add(28);
    // Field order in an LVL: lvlf, grpprlPapx, grpprlChpx, then the Xst.
    let papx = table.get(at..at.saturating_add(cb_papx)).unwrap_or(&[]).to_vec();
    at = at.saturating_add(cb_papx);
    let chpx = table.get(at..at.saturating_add(cb_chpx)).unwrap_or(&[]).to_vec();
    at = at.saturating_add(cb_chpx);
    let cch = table.get(at..at.saturating_add(2)).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0);
    let cch = cch.min(64) as usize;
    let chars = table.get(at.saturating_add(2)..at.saturating_add(2).saturating_add(cch * 2))?;
    let xst: Vec<u16> = chars.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    let len = 28 + cb_chpx + cb_papx + 2 + cch * 2;
    Some((RawLvl { start: start.min(0x7FFF), nfc, jc: jc_flags & 3, flags: jc_flags, ixch_follow, rgbxch_nums, papx, chpx, xst }, len))
}

/// `PlfLfo`: lfoMac u32, then lfoMac × LFO (16 bytes), then lfoMac × LFOData in parallel.
/// Each LFOData is a CP (skipped) followed by clfolvl × LFOLVL (8 bytes + optional LVL).
fn parse_plf_lfo(table: &[u8], fc: u32, defs: &[ListDef], numbering: &mut Numbering, fonts: &[String]) {
    let at = fc as usize;
    let Some(lfo_mac) = table.get(at..at.saturating_add(4)).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) else { return };
    let lfo_mac = lfo_mac.min(MAX_LISTS as u32) as usize;
    let mut lsids = Vec::with_capacity(lfo_mac);
    let mut clfolvls = Vec::with_capacity(lfo_mac);
    for i in 0..lfo_mac {
        let f = at.saturating_add(4).saturating_add(i.saturating_mul(16));
        let lsid = table.get(f..f.saturating_add(4)).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(-1);
        let clfolvl = table.get(f.saturating_add(12)).copied().unwrap_or(0);
        lsids.push(lsid);
        clfolvls.push(clfolvl);
    }
    let mut off = at.saturating_add(4).saturating_add(lfo_mac.saturating_mul(16));
    for (i, (&lsid, &clfolvl)) in lsids.iter().zip(&clfolvls).enumerate() {
        let Some(def) = defs.iter().position(|d| d.lsid == lsid) else {
            // Keep the LFOData walk in step even for unknown lists.
            off = skip_lfodatas(table, off, clfolvl);
            continue;
        };
        // LFOData: cp u32, then clfolvl × LFOLVL.
        let mut start_overrides = Vec::new();
        let mut overrides = Vec::new();
        off = off.saturating_add(4);
        for _ in 0..(clfolvl as usize).min(MAX_LFO_LVL) {
            let Some(hdr) = table.get(off..off.saturating_add(8)) else { break };
            let i_start = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
            let flags = hdr[4];
            let lvl = (flags & 0x0F) as usize;
            let f_start = flags & 0x10 != 0;
            let f_fmt = flags & 0x20 != 0;
            off = off.saturating_add(8);
            let mut fmt_lvl = None;
            if f_fmt {
                match parse_lvl(table, off) {
                    Some((l, len)) => {
                        fmt_lvl = Some(l);
                        off = off.saturating_add(len);
                    }
                    None => break,
                }
            }
            if f_start {
                start_overrides.push((lvl as u8, i_start.min(0x7FFF)));
            }
            if let Some(l) = fmt_lvl {
                overrides.push((lvl, l));
            }
        }
        // Skip any LFOLVLs we did not consume (clfolvl above the cap).
        if clfolvl as usize > MAX_LFO_LVL {
            off = skip_lfodatas(table, off, clfolvl - MAX_LFO_LVL as u8);
        }
        let abstract_id = if overrides.is_empty() {
            def as u32
        } else {
            // A formatting override replaces whole levels: give the LFO its own abstract
            // built from the base levels with the overrides applied.
            let mut levels = numbering.abstracts.get(def).map(|a| a.levels.clone()).unwrap_or_default();
            for (lvl, raw) in overrides {
                if let Some(slot) = levels.get_mut(lvl) {
                    *slot = level_of(&raw, lvl, fonts);
                }
            }
            let id = numbering.abstracts.len() as u32;
            numbering.abstracts.push(AbstractNum { id, name: None, levels, ..Default::default() });
            id
        };
        numbering.nums.push(Num { id: (i + 1) as u32, abstract_id, start_overrides, ..Default::default() });
    }
}

/// Advance past `n` LFOLVLs whose bodies we do not need; bounded and best-effort.
fn skip_lfodatas(table: &[u8], mut off: usize, n: u8) -> usize {
    for _ in 0..n {
        let Some(hdr) = table.get(off..off.saturating_add(8)) else { return off };
        let f_fmt = hdr[4] & 0x20 != 0;
        off = off.saturating_add(8);
        if f_fmt && let Some((_, len)) = parse_lvl(table, off) {
            off = off.saturating_add(len);
        }
    }
    off
}

/// Turn a stored LVL into a model level.
fn level_of(l: &RawLvl, lvl_index: usize, fonts: &[String]) -> Level {
    // The level's character formatting, whose font says which table a bullet's byte is from.
    let mut chr = CharProps::default();
    let base = chr.clone();
    for prl in sprm::iter(&l.chpx) {
        if prl.sgc() == 2 {
            fmt::apply_char(&mut chr, &prl, &base, fonts);
        }
    }
    // Number text: characters listed in rgbxchNums are level placeholders (their value is
    // the zero-based level); bullet chars carry 0xF000 flag bits to strip. A surrogate pair is
    // one character outside the BMP, so it is decoded as a pair.
    let mut text = String::new();
    let mut i = 0;
    while let Some(&c) = l.xst.get(i) {
        if l.rgbxch_nums.contains(&(i as u8 + 1)) && c <= 8 {
            text.push('%');
            text.push((b'1' + c as u8) as char);
        } else if let Some(pair) = l.xst.get(i + 1).and_then(|&low| char::decode_utf16([c, low]).next()?.ok()).filter(|p| u32::from(*p) > 0xFFFF) {
            text.push(pair);
            i += 1;
        } else {
            text.push(bullet_char(c, chr.font.as_deref()));
        }
        i += 1;
    }
    if text.is_empty() {
        text.push('\u{2022}');
    }
    // Indents from the level's grpprlPapx.
    let mut props = ParaProps::default();
    for prl in sprm::iter(&l.papx) {
        if prl.sgc() == 1 {
            fmt::apply_para(&mut props, &prl);
        }
    }
    let indent = props.indent_left.unwrap_or(18.0 * (lvl_index as f32 + 1.0)).clamp(0.0, 1000.0);
    let hanging = props.indent_first.map(|f| (-f).clamp(0.0, 1000.0)).unwrap_or(18.0);
    Level {
        start: l.start.max(1),
        format: num_format(l.nfc),
        text,
        align: match l.jc {
            1 => Align::Center,
            2 => Align::Right,
            _ => Align::Left,
        },
        indent,
        hanging,
        suffix: match l.ixch_follow {
            1 => LevelSuffix::Space,
            2 => LevelSuffix::Nothing,
            _ => LevelSuffix::Tab,
        },
        chr,
        restart: l.flags & 0x08 == 0,
        legal: l.flags & 0x04 != 0,
        style: None,
        ..Level::default()
    }
}
