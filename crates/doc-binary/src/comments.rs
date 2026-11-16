//! Comments ("annotations"): `PlcfandRef` (each comment's reference CP and its ATRD: the author's
//! initials, the author's index in `GrpXstAtnOwners`, the tag of the range it covers),
//! `PlcfandTxt` (each comment's story in the comment subdocument), the ranges (`SttbfAtnBkmk`
//! tags, `PlcfAtnBkf` starts, `PlcfAtnBkl` ends) and the dates (`AtrdExtra`) ([MS-DOC] §2.9.11,
//! §2.9.172, §2.9.12).
//!
//! A Word 6.0/95 file has the same pairs, with a smaller ATRD and 8-bit text: a 20-byte ATRD
//! (the initials a length byte and up to 9 characters, the author's index at 10, the tag at 16)
//! and the authors as length-prefixed 8-bit names. Both were seen in files Word 95 wrote, which
//! Word 2016 reads with these authors and initials and no date. Its ranges are not read: every
//! comment seen had none (tag -1), so it covers its reference only, as Word shows it.

use crate::fib::Fib;

/// Comments read per document.
const MAX: usize = 10_000;
/// UTF-16 units (or bytes) kept of an author's name: each comment holds a copy of its author's,
/// so a long one named by every comment can't multiply into a large allocation.
const MAX_AUTHOR: usize = 255;
/// Pair indexes ([MS-DOC] `FibRgFcLcb97`, `FibRgFcLcb2002`).
const PLCF_AND_REF: usize = 4;
const PLCF_AND_TXT: usize = 5;
const GRP_XST_ATN_OWNERS: usize = 36;
const STTBF_ATN_BKMK: usize = 37;
const PLCF_ATN_BKF: usize = 42;
const PLCF_ATN_BKL: usize = 43;
const ATRD_EXTRA: usize = 112;

/// One comment as stored.
pub(crate) struct RawComment {
    /// The reference character's CP.
    pub(crate) cp: u32,
    /// Its story within the comment subdocument.
    pub(crate) story: (u32, u32),
    pub(crate) author: String,
    pub(crate) initials: String,
    /// ISO 8601, as the `.docx` reader keeps `w:date`; empty when the file has none.
    pub(crate) date: String,
    /// The CPs it covers, when the file gives a range.
    pub(crate) range: Option<(u32, u32)>,
}

/// The layout of one format's comments.
#[derive(Clone, Copy)]
pub(crate) enum Layout {
    Word97,
    /// Word 6.0/95, with the code page of its 8-bit names.
    Word6(u16),
}

/// The comments of a document, in reference order. `table` holds the structures (the Table
/// stream; for Word 6.0/95, the WordDocument stream).
pub(crate) fn parse(table: &[u8], fib: &Fib, layout: Layout) -> Vec<RawComment> {
    let atrd = match layout {
        Layout::Word97 => 30,
        Layout::Word6(_) => 20,
    };
    let bytes =
        |pair: usize| fib.pair(pair).and_then(|(fc, lcb)| table.get(fc as usize..(fc as usize).saturating_add(lcb as usize))).unwrap_or_default();
    let refs = bytes(PLCF_AND_REF);
    // The ATRDs follow all the stored CPs, however many are read.
    let stored = refs.len().saturating_sub(4) / (4 + atrd);
    let n = stored.min(MAX);
    let cps = plc_cps(refs, n + 1);
    let txts = plc_cps(bytes(PLCF_AND_TXT), n + 1);
    let owners = owners(bytes(GRP_XST_ATN_OWNERS), layout);
    let (ranges, dates) = match layout {
        Layout::Word97 => (ranges(bytes(STTBF_ATN_BKMK), bytes(PLCF_ATN_BKF), bytes(PLCF_ATN_BKL)), dates(bytes(ATRD_EXTRA), n)),
        Layout::Word6(_) => (Vec::new(), Vec::new()),
    };
    // The first range of each tag, as a scan would find it.
    let mut by_tag: std::collections::HashMap<i32, (u32, u32)> = std::collections::HashMap::new();
    for (t, r) in ranges {
        by_tag.entry(t).or_insert(r);
    }
    let mut out = Vec::new();
    let mut kept_to = 0;
    for i in 0..n {
        let (Some(&cp), Some(&a), Some(&b)) = (cps.get(i), txts.get(i), txts.get(i + 1)) else { break };
        let Some(d) = refs.get(4 * (stored + 1) + atrd * i..4 * (stored + 1) + atrd * (i + 1)) else { break };
        // As for notes: a story starting before the end of the last one kept is a hostile table
        // reading the same text again.
        if b <= a || a < kept_to {
            continue;
        }
        kept_to = b;
        let u16_at = |at: usize| d.get(at..at + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
        let i32_at = |at: usize| d.get(at..at + 4).map_or(-1, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let (initials, ibst, tag) = match layout {
            Layout::Word97 => {
                let len = usize::from(u16_at(0)).min(9);
                let units: Vec<u16> = (0..len).map(|k| u16_at(2 + 2 * k)).collect();
                (String::from_utf16_lossy(&units), u16_at(20), i32_at(26))
            }
            Layout::Word6(cp) => {
                let len = usize::from(d.first().copied().unwrap_or(0)).min(9);
                let initials = d.get(1..1 + len).unwrap_or_default().iter().map(|&b| craft_codepage::code_page_byte(cp, b)).collect();
                (initials, u16_at(10), i32_at(16))
            }
        };
        out.push(RawComment {
            cp,
            story: (a, b),
            author: owners.get(usize::from(ibst)).cloned().unwrap_or_default(),
            initials,
            date: dates.get(i).cloned().unwrap_or_default(),
            range: (tag != -1).then(|| by_tag.get(&tag).copied()).flatten(),
        });
    }
    out
}

/// The first `count` CPs of a PLC.
fn plc_cps(plc: &[u8], count: usize) -> Vec<u32> {
    plc.as_chunks::<4>().0.iter().take(count).map(|b| u32::from_le_bytes(*b)).collect()
}

/// The authors' names: Xsts (u16 count, UTF-16), or for Word 6.0/95 a length byte and 8-bit text.
fn owners(grp: &[u8], layout: Layout) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < grp.len() && out.len() < MAX {
        match layout {
            Layout::Word97 => {
                let Some(b) = grp.get(at..at + 2) else { break };
                let len = usize::from(u16::from_le_bytes([b[0], b[1]]));
                let Some(text) = grp.get(at + 2..at + 2 + 2 * len) else { break };
                let units: Vec<u16> = text.as_chunks::<2>().0.iter().take(MAX_AUTHOR).map(|u| u16::from_le_bytes(*u)).collect();
                out.push(String::from_utf16_lossy(&units));
                at += 2 + 2 * len;
            }
            Layout::Word6(cp) => {
                let Some(&len) = grp.get(at) else { break };
                let len = usize::from(len);
                let Some(text) = grp.get(at + 1..at + 1 + len) else { break };
                out.push(text.iter().take(MAX_AUTHOR).map(|&b| craft_codepage::code_page_byte(cp, b)).collect());
                at += 1 + len;
            }
        }
    }
    out
}

/// Each comment range as (tag, (start, end)). `SttbfAtnBkmk` is an extended STTB (0xFFFF, a
/// count, 10 extra bytes per entry) whose entries carry the tags at extra byte 2; `PlcfAtnBkf`'s
/// i-th start belongs to the i-th tag and names its end's index in `PlcfAtnBkl`.
fn ranges(sttb: &[u8], bkf: &[u8], bkl: &[u8]) -> Vec<(i32, (u32, u32))> {
    let u16_at = |b: &[u8], at: usize| b.get(at..at + 2).map(|v| u16::from_le_bytes([v[0], v[1]]));
    if u16_at(sttb, 0) != Some(0xFFFF) {
        return Vec::new();
    }
    let (Some(count), Some(extra)) = (u16_at(sttb, 2), u16_at(sttb, 4)) else { return Vec::new() };
    let (count, extra) = (usize::from(count).min(MAX), usize::from(extra));
    let mut tags = Vec::new();
    let mut at = 6;
    for _ in 0..count {
        let Some(cch) = u16_at(sttb, at) else { break };
        at += 2 + 2 * usize::from(cch);
        let Some(e) = sttb.get(at..at + extra) else { break };
        tags.push(e.get(2..6).map_or(-1, |t| i32::from_le_bytes([t[0], t[1], t[2], t[3]])));
        at += extra;
    }
    let starts_n = bkf.len().saturating_sub(4) / 8;
    let starts = plc_cps(bkf, starts_n);
    let ends = plc_cps(bkl, bkl.len() / 4);
    let mut out = Vec::new();
    for (i, &tag) in tags.iter().enumerate().take(starts_n) {
        let ibkl = u16_at(bkf, 4 * (starts_n + 1) + 4 * i).map(usize::from);
        if let (Some(&start), Some(&end)) = (starts.get(i), ibkl.and_then(|k| ends.get(k)))
            && start <= end
        {
            out.push((tag, (start, end)));
        }
    }
    out
}

/// Each comment's date from `AtrdExtra`: an 18-byte ATRDPost10 per comment, a DTTM first.
fn dates(extra: &[u8], n: usize) -> Vec<String> {
    extra.as_chunks::<18>().0.iter().take(n).map(|e| dttm(u32::from_le_bytes([e[0], e[1], e[2], e[3]]))).collect()
}

/// A DTTM ([MS-DOC] §2.9.58: minutes, hours, day, month and years since 1900 in 6, 5, 5, 4 and 9
/// bits) as `w:date` gives it (Word writes the time it stores, marked UTC); empty when it isn't a
/// date (a day or month of 0 means none).
fn dttm(v: u32) -> String {
    let (min, hour, day, month, year) = (v & 0x3F, (v >> 6) & 0x1F, (v >> 11) & 0x1F, (v >> 16) & 0xF, 1900 + ((v >> 20) & 0x1FF) as i32);
    chrono::NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|d| d.and_hms_opt(hour, min, 0))
        .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{Layout, parse};
    use crate::fib::{Fib, SubDocs};

    /// A FIB with `blobs` (pair index, bytes) laid out one after another in the returned table.
    fn with(blobs: &[(usize, Vec<u8>)]) -> (Fib, Vec<u8>) {
        let mut pairs = vec![0u32; 2 * 113];
        let mut table = Vec::new();
        for (i, b) in blobs {
            pairs[2 * i] = table.len() as u32;
            pairs[2 * i + 1] = b.len() as u32;
            table.extend_from_slice(b);
        }
        let ccp = SubDocs { text: 0, ftn: 0, hdd: 0, atn: 0, edn: 0, txbx: 0, hdr_txbx: 0 };
        (Fib { which_tbl_stm: false, ccp, pairs }, table)
    }

    fn u32s(v: &[u32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// A Word 97 ATRD: UTF-16 initials (u16 count + 9 units), author index, two unused words, tag.
    fn atrd97(initials: &str, ibst: u16, tag: i32) -> Vec<u8> {
        let mut v = vec![0u8; 30];
        let units: Vec<u16> = initials.encode_utf16().collect();
        v[0..2].copy_from_slice(&(units.len() as u16).to_le_bytes());
        for (k, u) in units.iter().enumerate() {
            v[2 + 2 * k..4 + 2 * k].copy_from_slice(&u.to_le_bytes());
        }
        v[20..22].copy_from_slice(&ibst.to_le_bytes());
        v[26..30].copy_from_slice(&tag.to_le_bytes());
        v
    }

    #[test]
    fn word_97_comments_take_their_authors_ranges_and_dates() {
        // Word 2016's "Alpha beta gamma delta": Ann on "beta" (tag 0x060AE0FA), Bob on "gamma"
        // (tag 0x72A0A4AD), Ann on "gamma delta" (tag 0x4CFD23D7), as its comments.doc stores
        // them. Word reads them with these ranges, authors, initials and 2026-11-14T13:37:00Z.
        let (t1, t2, t3) = (0x060A_E0FA, 0x72A0_A4AD, 0x4CFD_23D7);
        let refs = [u32s(&[10, 17, 24, 26]), atrd97("AA", 0, t1), atrd97("BB", 1, t2), atrd97("AA", 0, t3)].concat();
        let txts = u32s(&[0, 9, 20, 29, 30]);
        let mut owners = Vec::new();
        for name in ["Ann Able", "Bob Brown"] {
            owners.extend_from_slice(&(name.len() as u16).to_le_bytes());
            owners.extend(name.encode_utf16().flat_map(u16::to_le_bytes));
        }
        // SttbfAtnBkmk: extended, 3 entries of no name and 10 extra bytes (bmc, tag, old tag).
        let mut sttb = [0xFFFFu16, 3, 10].iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        for tag in [t1, t3, t2] {
            sttb.extend_from_slice(&[0, 0, 0, 1]);
            sttb.extend_from_slice(&tag.to_le_bytes());
            sttb.extend_from_slice(&(-1i32).to_le_bytes());
        }
        // Starts 6, 12, 12 with their ends' indexes 0, 2, 1; ends 10, 17, 24.
        let bkf = [u32s(&[6, 12, 12, 28]), u32s(&[0, 2, 1])].concat();
        let bkl = u32s(&[10, 17, 24, 28]);
        let extra: Vec<u8> = (0..3).flat_map(|_| [0x65, 0x73, 0xEB, 0xC7].into_iter().chain([0; 14])).collect();
        let (fib, table) = with(&[(4, refs), (5, txts), (36, owners), (37, sttb), (42, bkf), (43, bkl), (112, extra)]);
        let got = parse(&table, &fib, Layout::Word97);
        let summary: Vec<_> = got.iter().map(|c| (c.cp, c.story, c.author.as_str(), c.initials.as_str(), c.range, c.date.as_str())).collect();
        let date = "2026-11-14T13:37:00Z";
        assert_eq!(
            summary,
            [
                (10, (0, 9), "Ann Able", "AA", Some((6, 10)), date),
                (17, (9, 20), "Bob Brown", "BB", Some((12, 17)), date),
                (24, (20, 29), "Ann Able", "AA", Some((12, 24)), date),
            ]
        );
    }

    #[test]
    fn more_comments_than_are_read_keep_their_atrds_and_short_authors() {
        // 10,001 comments, one more than are read: the ATRDs still start after all the stored
        // CPs, so the first comment keeps its initials and author. Its author's name, 300
        // characters, keeps its first 255.
        let n = super::MAX + 1;
        let cps: Vec<u32> = (0..=n as u32).collect();
        let atrds: Vec<u8> = (0..n).flat_map(|i| atrd97(if i == 0 { "ZZ" } else { "AA" }, u16::from(i == 0), -1)).collect();
        let refs = [u32s(&cps), atrds].concat();
        let txts = u32s(&(0..=n as u32).map(|i| 2 * i).collect::<Vec<_>>());
        let long = "x".repeat(300);
        let mut owners = Vec::new();
        for name in ["Ann", long.as_str()] {
            owners.extend_from_slice(&(name.len() as u16).to_le_bytes());
            owners.extend(name.encode_utf16().flat_map(u16::to_le_bytes));
        }
        let (fib, table) = with(&[(4, refs), (5, txts), (36, owners)]);
        let got = parse(&table, &fib, Layout::Word97);
        assert_eq!(got.len(), super::MAX);
        assert_eq!((got[0].initials.as_str(), got[0].author.len()), ("ZZ", super::MAX_AUTHOR));
        assert_eq!((got[1].initials.as_str(), got[1].author.as_str()), ("AA", "Ann"));
    }

    #[test]
    fn word_6_comments_have_8_bit_initials_and_authors() {
        // As Word 95 writes them (`atn2`): a 20-byte ATRD (initials as a length byte and up to 9
        // characters, the author's index at 10, tag -1 at 16), authors as length-prefixed 8-bit
        // names. Word reads "BB"/Bob Brown and "AA"/Ann Able, no date and no range.
        let atrd = |initials: &[u8], ibst: u16| {
            let mut v = vec![0u8; 20];
            v[0] = initials.len() as u8;
            v[1..1 + initials.len()].copy_from_slice(initials);
            v[10..12].copy_from_slice(&ibst.to_le_bytes());
            v[16..20].copy_from_slice(&(-1i32).to_le_bytes());
            v
        };
        let refs = [u32s(&[5, 11, 20]), atrd(b"BB", 0), atrd(b"AA", 1)].concat();
        let (fib, table) = with(&[(4, refs), (5, u32s(&[0, 9, 18, 19])), (36, b"\x09Bob Brown\x08Ann Able".to_vec())]);
        let got = parse(&table, &fib, Layout::Word6(1252));
        let summary: Vec<_> = got.iter().map(|c| (c.author.as_str(), c.initials.as_str(), c.range, c.date.as_str())).collect();
        assert_eq!(summary, [("Bob Brown", "BB", None, ""), ("Ann Able", "AA", None, "")]);
    }

    #[test]
    fn a_dttm_reads_as_word_writes_it() {
        // Word 2016's comment: 65 73 eb c7, which it saves as w:date="2026-11-14T13:37:00Z".
        assert_eq!(super::dttm(0xC7EB_7365), "2026-11-14T13:37:00Z");
        assert_eq!(super::dttm(0), "");
    }
}
