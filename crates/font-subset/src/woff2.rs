//! WOFF2 encoding (<https://www.w3.org/TR/WOFF2/>) with the null transform:
//! every table is stored as-is and the concatenated table data is one Brotli
//! stream. Single sfnt input only; font collections are rejected.

use brotli::enc::BrotliEncoderParams;
use brotli::enc::backward_references::BrotliEncoderMode;

/// Brotli effort for the WOFF2 output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// Quality 11, for files precomputed at sync.
    Static,
    /// Quality 5, for on-demand responses: about 5% larger, 20 times faster.
    OnDemand,
}

/// Why WOFF2 encoding failed.
#[derive(Debug, thiserror::Error)]
pub enum Woff2Error {
    /// The input is too short or a font collection.
    #[error("input is not an sfnt font (too short or a font collection)")]
    NotSfnt,
    /// The table directory or table data is out of bounds.
    #[error("sfnt table directory or table data is out of bounds")]
    Truncated,
    /// Two tables share a tag.
    #[error("sfnt has duplicate {0} tables")]
    DuplicateTable(String),
    /// A length does not fit WOFF2's fields.
    #[error("sfnt too large for WOFF2")]
    TooLarge,
    /// Brotli failed.
    #[error("Brotli compression failed: {0}")]
    Brotli(#[from] std::io::Error),
}

/// Known table tags in WOFF2 directory-flag order (index = flag value).
const KNOWN_TAGS: [&[u8; 4]; 63] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm", b"glyf", b"loca", b"prep", b"CFF ", b"VORG", b"EBDT",
    b"EBLC", b"gasp", b"hdmx", b"kern", b"LTSH", b"PCLT", b"VDMX", b"vhea", b"vmtx", b"BASE", b"GDEF", b"GPOS", b"GSUB", b"EBSC", b"JSTF", b"MATH",
    b"CBDT", b"CBLC", b"COLR", b"CPAL", b"SVG ", b"sbix", b"acnt", b"avar", b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc", b"feat", b"fmtx", b"fvar",
    b"gvar", b"hsty", b"just", b"lcar", b"mort", b"morx", b"opbd", b"prop", b"trak", b"Zapf", b"Silf", b"Glat", b"Gloc", b"Feat", b"Sill",
];

const HEADER_LEN: usize = 48;
/// Null-transform version for glyf and loca (version 0 is the real transform).
const NULL_TRANSFORM_GLYF_LOCA: u8 = 3;

struct Table<'a> {
    tag: [u8; 4],
    data: &'a [u8],
}

fn be16(b: &[u8], at: usize) -> Result<u16, Woff2Error> {
    let s = b.get(at..at + 2).ok_or(Woff2Error::Truncated)?;
    Ok(u16::from_be_bytes([s[0], s[1]]))
}

fn be32(b: &[u8], at: usize) -> Result<u32, Woff2Error> {
    let s = b.get(at..at + 4).ok_or(Woff2Error::Truncated)?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn parse_tables(sfnt: &[u8]) -> Result<Vec<Table<'_>>, Woff2Error> {
    if sfnt.len() < 12 || &sfnt[..4] == b"ttcf" {
        return Err(Woff2Error::NotSfnt);
    }
    let num = usize::from(be16(sfnt, 4)?);
    let mut tables: Vec<Table> = Vec::with_capacity(num);
    for i in 0..num {
        let rec = 12 + 16 * i;
        let tag: [u8; 4] = sfnt.get(rec..rec + 4).ok_or(Woff2Error::Truncated)?.try_into().map_err(|_| Woff2Error::Truncated)?;
        let offset = be32(sfnt, rec + 8)? as usize;
        let length = be32(sfnt, rec + 12)? as usize;
        let end = offset.checked_add(length).ok_or(Woff2Error::Truncated)?;
        let data = sfnt.get(offset..end).ok_or(Woff2Error::Truncated)?;
        if tables.iter().any(|t| t.tag == tag) {
            return Err(Woff2Error::DuplicateTable(String::from_utf8_lossy(&tag).into_owned()));
        }
        tables.push(Table { tag, data });
    }
    // Directory order is by tag; glyf must be immediately followed by loca.
    tables.sort_by_key(|t| t.tag);
    if let Some(l) = tables.iter().position(|t| &t.tag == b"loca")
        && let Some(g) = tables.iter().position(|t| &t.tag == b"glyf")
    {
        let loca = tables.remove(l);
        let g = if l < g { g - 1 } else { g };
        tables.insert(g + 1, loca);
    }
    Ok(tables)
}

fn push_base128(out: &mut Vec<u8>, mut v: u32) {
    let mut buf = [0u8; 5];
    let mut n = 0;
    loop {
        buf[n] = (v & 0x7f) as u8;
        n += 1;
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        out.push(buf[i] | if i == 0 { 0 } else { 0x80 });
    }
}

fn padded(n: usize) -> usize {
    (n + 3) & !3
}

pub(crate) fn encode(sfnt: &[u8], quality: Quality) -> Result<Vec<u8>, Woff2Error> {
    let tables = parse_tables(sfnt)?;

    let mut directory = Vec::new();
    let mut raw = Vec::new();
    let mut sfnt_size = 12 + 16 * tables.len();
    for t in &tables {
        let len = u32::try_from(t.data.len()).map_err(|_| Woff2Error::TooLarge)?;
        let null_version = if matches!(&t.tag, b"glyf" | b"loca") { NULL_TRANSFORM_GLYF_LOCA } else { 0 };
        match KNOWN_TAGS.iter().position(|k| **k == t.tag) {
            Some(i) => directory.push(i as u8 | null_version << 6),
            None => {
                directory.push(63 | null_version << 6);
                directory.extend_from_slice(&t.tag);
            }
        }
        push_base128(&mut directory, len);
        raw.extend_from_slice(t.data);
        sfnt_size += padded(t.data.len());
    }
    let total_sfnt_size = u32::try_from(sfnt_size).map_err(|_| Woff2Error::TooLarge)?;

    let params = BrotliEncoderParams {
        quality: match quality {
            Quality::Static => 11,
            Quality::OnDemand => 5,
        },
        lgwin: 22,
        mode: BrotliEncoderMode::BROTLI_MODE_FONT,
        size_hint: raw.len(),
        ..Default::default()
    };
    let mut compressed = Vec::new();
    brotli::BrotliCompress(&mut raw.as_slice(), &mut compressed, &params)?;
    let compressed_len = u32::try_from(compressed.len()).map_err(|_| Woff2Error::TooLarge)?;

    let length = padded(HEADER_LEN + directory.len() + compressed.len());
    let mut out = Vec::with_capacity(length);
    out.extend_from_slice(b"wOF2");
    out.extend_from_slice(&sfnt[..4]); // flavor
    out.extend_from_slice(&u32::try_from(length).map_err(|_| Woff2Error::TooLarge)?.to_be_bytes());
    out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // reserved
    out.extend_from_slice(&total_sfnt_size.to_be_bytes());
    out.extend_from_slice(&compressed_len.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes()); // major version
    out.extend_from_slice(&0u16.to_be_bytes()); // minor version
    out.extend_from_slice(&[0; 20]); // meta and private block offsets and sizes
    out.extend_from_slice(&directory);
    out.extend_from_slice(&compressed);
    out.resize(length, 0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: [&str; 7] = [
        "abeezee/ABeeZee-Regular.ttf",
        "bbhbartle/BBHBartle-Regular.ttf",
        "sarabun/Sarabun-Regular.ttf",
        "ubuntu/Ubuntu-Regular.ttf",
        "orbitron/Orbitron[wght].ttf",
        "librefranklin/LibreFranklin[wght].ttf",
        "notosanscjk/NotoSansCJKsc-test.otf",
    ];

    fn fixture(path: &str) -> Vec<u8> {
        std::fs::read(format!("{}/tests/data/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn round_trips_every_table_and_header_matches_spec() {
        for path in FIXTURES {
            let sfnt = fixture(path);
            let tables = parse_tables(&sfnt).unwrap();
            for quality in [Quality::Static, Quality::OnDemand] {
                let woff2 = encode(&sfnt, quality).unwrap();
                assert_eq!(&woff2[..4], b"wOF2", "{path}");
                assert_eq!(&woff2[4..8], &sfnt[..4], "{path}");
                assert_eq!(be32(&woff2, 8).unwrap() as usize, woff2.len(), "{path}");
                assert_eq!(woff2.len() % 4, 0, "{path}");
                assert_eq!(usize::from(be16(&woff2, 12).unwrap()), tables.len());
                let expected: usize = 12 + 16 * tables.len() + tables.iter().map(|t| padded(t.data.len())).sum::<usize>();
                assert_eq!(be32(&woff2, 16).unwrap() as usize, expected, "{path}");

                let decoded = wuff::decompress_woff2(&woff2).unwrap();
                let back = parse_tables(&decoded).unwrap();
                assert_eq!(back.len(), tables.len(), "{path}");
                for t in &tables {
                    let b = back.iter().find(|b| b.tag == t.tag).unwrap();
                    let (mut want, mut got) = (t.data.to_vec(), b.data.to_vec());
                    if &t.tag == b"head" {
                        // Decoders recompute checkSumAdjustment.
                        want[8..12].fill(0);
                        got[8..12].fill(0);
                    }
                    assert_eq!(got, want, "{path} {:?}", t.tag);
                }
            }
        }
    }

    #[test]
    fn loca_follows_glyf() {
        let sfnt = fixture("ubuntu/Ubuntu-Regular.ttf");
        let tables = parse_tables(&sfnt).unwrap();
        let g = tables.iter().position(|t| &t.tag == b"glyf").unwrap();
        assert_eq!(&tables[g + 1].tag, b"loca");
    }

    #[test]
    fn base128_encodes_per_spec() {
        for (v, bytes) in [(0u32, &[0u8][..]), (63, &[63]), (128, &[0x81, 0]), (0x3fff, &[0xff, 0x7f])] {
            let mut out = Vec::new();
            push_base128(&mut out, v);
            assert_eq!(out, bytes);
        }
    }

    #[test]
    fn truncated_or_collection_input_is_an_error() {
        let sfnt = fixture("abeezee/ABeeZee-Regular.ttf");
        for cut in [0, 5, 12, 40, sfnt.len() / 2, sfnt.len() - 8] {
            assert!(encode(&sfnt[..cut], Quality::OnDemand).is_err(), "cut {cut}");
        }
        assert!(matches!(encode(b"ttcf\0\x01\0\0\0\0\0\0", Quality::OnDemand), Err(Woff2Error::NotSfnt)));
    }
}
