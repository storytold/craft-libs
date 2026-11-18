//! Header, mapping and Rich header tests. `header_rvas_stop_before_file_padding` is from Craft
//! Anything's `pe.rs`; the image builder here is a small stand-in for its testkit's `pe64`.

use super::*;

const PE_OFFSET: usize = 0xc0;
const OPT: usize = PE_OFFSET + 24;
const OPT_SIZE: usize = 112 + 16 * 8;
const SECTIONS: usize = OPT + OPT_SIZE;
const BASE: u64 = 0x1_4000_0000;

fn put(data: &mut [u8], at: usize, bytes: &[u8]) {
    data[at..at + bytes.len()].copy_from_slice(bytes);
}

/// A PE32+ console image with `.text` at RVA 0x1000 (file 0x400) and `.rdata` at RVA 0x2000
/// (file 0x600), an import directory, and a Rich header with two entries.
fn pe64() -> Vec<u8> {
    let mut d = vec![0u8; 0x800];
    put(&mut d, 0, b"MZ");
    put(&mut d, 0x3c, &(PE_OFFSET as u32).to_le_bytes());
    rich(&mut d, 0x5a5a_1234, &[(0x0104, 30_729, 7), (0x00ff, 40_000, 1)]);
    put(&mut d, PE_OFFSET, b"PE\0\0");
    let coff = PE_OFFSET + 4;
    put(&mut d, coff, &0x8664u16.to_le_bytes());
    put(&mut d, coff + 2, &2u16.to_le_bytes());
    put(&mut d, coff + 4, &0x6000_0000u32.to_le_bytes());
    put(&mut d, coff + 16, &(OPT_SIZE as u16).to_le_bytes());
    put(&mut d, coff + 18, &0x22u16.to_le_bytes());
    put(&mut d, OPT, &0x20bu16.to_le_bytes());
    d[OPT + 2] = 14;
    d[OPT + 3] = 36;
    put(&mut d, OPT + 16, &0x1010u32.to_le_bytes());
    put(&mut d, OPT + 24, &BASE.to_le_bytes());
    put(&mut d, OPT + 60, &0x400u32.to_le_bytes());
    put(&mut d, OPT + 68, &3u16.to_le_bytes());
    put(&mut d, OPT + 70, &0x8160u16.to_le_bytes());
    put(&mut d, OPT + 108, &16u32.to_le_bytes());
    put(&mut d, OPT + 112 + 8, &0x2000u32.to_le_bytes());
    put(&mut d, OPT + 112 + 12, &0x28u32.to_le_bytes());
    for (i, (name, rva, raw)) in [(&b".text"[..], 0x1000u32, 0x400u32), (&b".rdata"[..], 0x2000, 0x600)].into_iter().enumerate() {
        let o = SECTIONS + i * 40;
        put(&mut d, o, name);
        put(&mut d, o + 8, &0x180u32.to_le_bytes());
        put(&mut d, o + 12, &rva.to_le_bytes());
        put(&mut d, o + 16, &0x200u32.to_le_bytes());
        put(&mut d, o + 20, &raw.to_le_bytes());
        put(&mut d, o + 36, &0x6000_0020u32.to_le_bytes());
    }
    d
}

/// Write a Rich header (`DanS`, three zero pads, entries, `Rich`, key) at 0x80, XOR-masked.
fn rich(d: &mut [u8], key: u32, entries: &[(u16, u16, u32)]) {
    let mut o = 0x80;
    for word in [0x536e_6144u32, 0, 0, 0] {
        put(d, o, &(word ^ key).to_le_bytes());
        o += 4;
    }
    for &(product, build, count) in entries {
        put(d, o, &(((u32::from(product)) << 16 | u32::from(build)) ^ key).to_le_bytes());
        put(d, o + 4, &(count ^ key).to_le_bytes());
        o += 8;
    }
    put(d, o, b"Rich");
    put(d, o + 4, &key.to_le_bytes());
}

/// Every truncation and a spread of single-byte corruptions.
fn mutations(sample: &[u8]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = (0..sample.len()).step_by(7).map(|len| sample[..len].to_vec()).collect();
    for i in (0..sample.len()).step_by(3) {
        for v in [0x00, 0x7f, 0xff] {
            let mut m = sample.to_vec();
            m[i] = v;
            out.push(m);
        }
    }
    out
}

#[test]
fn reads_coff_optional_header_and_sections() {
    let d = pe64();
    assert!(has_pe_header(&d));
    let h = headers(&d).unwrap();
    assert_eq!(h.pe_offset, PE_OFFSET);
    assert_eq!(h.machine, 0x8664);
    assert_eq!(h.timestamp, 0x6000_0000);
    assert_eq!(h.characteristics, 0x22);
    assert!(h.pe32_plus);
    assert_eq!(h.image_base, BASE);
    assert_eq!(h.entry_rva, 0x1010);
    assert_eq!(h.size_of_headers, 0x400);
    assert_eq!(subsystem_name(h.subsystem), "windows-console");
    assert_eq!(h.dll_characteristics, 0x8160);
    assert_eq!(h.linker, (14, 36));
    assert_eq!(h.data_dirs.len(), 16);
    assert_eq!(h.dir(1), Some((0x2000, 0x28)));
    assert_eq!(DATA_DIRECTORY_NAMES[1], "import");
    assert_eq!(h.dir(0), None);
    assert_eq!(h.dir(16), None);
    let names: Vec<&str> = h.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, [".text", ".rdata"]);
    assert_eq!(h.sections[1].characteristics, 0x6000_0020);
}

#[test]
fn pe32_reads_a_32_bit_image_base() {
    let mut d = pe64();
    put(&mut d, OPT, &0x10bu16.to_le_bytes());
    put(&mut d, OPT + 28, &0x0040_0000u32.to_le_bytes());
    put(&mut d, OPT + 92, &2u32.to_le_bytes());
    let h = headers(&d).unwrap();
    assert!(!h.pe32_plus);
    assert_eq!(h.image_base, 0x0040_0000);
    assert_eq!(h.data_dirs.len(), 2);
    assert_eq!(h.image_va(0x1000), Some(0x0040_1000));
    let mut high = h.clone();
    high.image_base = 0xffff_f000;
    assert_eq!(high.image_va(0x1000), None, "a PE32 address cannot leave 32 bits");
}

#[test]
fn bad_signatures_and_short_headers_are_errors() {
    let d = pe64();
    assert_eq!(headers(b"ZM"), Err(PeError::Malformed("no MZ signature".into())));
    assert_eq!(headers(b"MZ"), Err(PeError::Truncated("DOS header")));
    let mut no_pe = d.clone();
    no_pe[PE_OFFSET] = b'X';
    assert!(!has_pe_header(&no_pe));
    assert_eq!(headers(&no_pe), Err(PeError::Malformed("no PE signature".into())));
    assert_eq!(headers(&d[..PE_OFFSET + 6]), Err(PeError::Truncated("COFF header")));
    assert_eq!(headers(&d[..OPT + 10]), Err(PeError::Truncated("optional header")));
    let mut magic = d.clone();
    put(&mut magic, OPT, &0x107u16.to_le_bytes());
    assert_eq!(headers(&magic), Err(PeError::Malformed("optional header magic 0x107".into())));
    assert_eq!(headers(&magic).unwrap_err().to_string(), "the file is malformed (optional header magic 0x107)");
    assert_eq!(PeError::Truncated("COFF header").to_string(), "the file ends early (COFF header)");
    let mut empty_optional = d.clone();
    put(&mut empty_optional, PE_OFFSET + 20, &0u16.to_le_bytes());
    assert_eq!(headers(&empty_optional), Err(PeError::Truncated("optional header")));
}

#[test]
fn a_section_table_cut_short_keeps_the_complete_entries() {
    let d = pe64();
    let h = headers(&d[..SECTIONS + 40 + 7]).unwrap();
    assert_eq!(h.sections.len(), 1);
    let mut many = d.clone();
    put(&mut many, PE_OFFSET + 6, &u16::MAX.to_le_bytes());
    many.resize(SECTIONS + 40 * 200, 0);
    assert_eq!(headers(&many).unwrap().sections.len(), MAX_SECTIONS);
}

#[test]
fn header_rvas_stop_before_file_padding() {
    let mut pe = pe64();
    let first = headers(&pe).unwrap().sections[0].raw_offset;
    let opt = Reader::new(&pe).u32_le(0x3c).unwrap() as usize + 24;
    let size_headers = 0x200_u32;
    pe[opt + 60..opt + 64].copy_from_slice(&size_headers.to_le_bytes());
    let coff = opt - 20;
    let section = opt + usize::from(Reader::new(&pe).u16_le(coff + 16).unwrap());
    pe[section + 20..section + 24].copy_from_slice(&(first + 0x400).to_le_bytes());
    let h = headers(&pe).unwrap();
    assert_eq!(h.rva_to_offset(size_headers - 1), Some(0x1ff));
    assert_eq!(h.rva_to_offset(size_headers), None);
    assert_eq!(h.rva_to_offset(h.sections[0].rva), Some((first + 0x400) as usize));
}

#[test]
fn rvas_map_through_sections_and_stop_at_raw_data() {
    let h = headers(&pe64()).unwrap();
    assert_eq!(h.rva_to_offset(0x10), Some(0x10));
    assert_eq!(h.rva_to_offset(0x1010), Some(0x410));
    assert_eq!(h.rva_to_offset(0x2000), Some(0x600));
    assert_eq!(h.rva_to_offset(0x1000 + 0x200), None, "past the raw data in the virtual tail");
    assert_eq!(h.rva_to_offset(0x9000), None);
    assert_eq!(h.rva_file_len(0x1010), Some(0x1f0));
    assert_eq!(h.rva_file_len(0x2100), Some(0x100));
    assert_eq!(h.rva_file_len(0x10), Some(0x3f0));
    assert_eq!(h.rva_file_len(0x9000), None);
    assert_eq!(h.image_va(0x1010), Some(BASE + 0x1010));
    assert_eq!(h.image_va(u64::from(u32::MAX) + 1), None);
}

#[test]
fn contiguous_file_length_stops_at_the_next_section() {
    let mut h = headers(&pe64()).unwrap();
    // `.text` claims raw bytes that run into `.rdata`'s RVA range.
    h.sections[0].raw_size = 0x2000;
    h.sections[0].virtual_size = 0x2000;
    assert_eq!(h.rva_file_len(0x1800), Some(0x1800));
    // Headers end where the first section begins.
    h.size_of_headers = 0x1800;
    assert_eq!(h.rva_file_len(0x0f00), Some(0x100));
}

#[test]
fn rich_header_entries_are_unmasked() {
    let d = pe64();
    assert_eq!(rich_header(&d, PE_OFFSET), Some(vec![(0x0104, 30_729, 7), (0x00ff, 40_000, 1)]));
    assert_eq!(rich_header(&d, 0x80), None, "Rich lies past the stub");
    let mut no_dans = d.clone();
    no_dans[0x80] ^= 1;
    assert_eq!(rich_header(&no_dans, PE_OFFSET), None);
    assert_eq!(rich_header(&d[..0x30], PE_OFFSET), None);
}

#[test]
fn rich_header_is_capped_at_256_entries() {
    let mut d = vec![0u8; 0x80 + 16 + 300 * 8 + 8 + 16];
    let entries: Vec<(u16, u16, u32)> = (0..300).map(|i| (1, i, 1)).collect();
    rich(&mut d, 7, &entries);
    assert_eq!(rich_header(&d, d.len()).map(|e| e.len()), Some(256));
}

#[test]
fn subsystem_names() {
    let names: Vec<&str> = [1, 2, 3, 7, 9, 10, 11, 12, 14, 16, 99].into_iter().map(subsystem_name).collect();
    assert_eq!(
        names,
        [
            "native",
            "windows-gui",
            "windows-console",
            "posix-console",
            "windows-ce",
            "efi-application",
            "efi-boot-driver",
            "efi-runtime-driver",
            "xbox",
            "windows-boot",
            "unknown"
        ]
    );
}

#[cfg(feature = "serde")]
#[test]
fn sections_serialize() {
    let h = headers(&pe64()).unwrap();
    let value = serde_json::to_value(&h.sections[0]).unwrap();
    assert_eq!(value["name"], ".text");
    assert_eq!(value["rva"], 0x1000);
}

#[test]
fn damaged_headers_never_panic() {
    for m in mutations(&pe64()) {
        let _ = has_pe_header(&m);
        if let Ok(h) = headers(&m) {
            for rva in [0, 0x10, 0x1010, 0x2000, 0x2100, u32::MAX] {
                let _ = h.rva_to_offset(rva);
                let _ = h.rva_file_len(rva);
                let _ = h.image_va(u64::from(rva));
            }
            let _ = rich_header(&m, h.pe_offset);
        }
        let _ = rich_header(&m, usize::MAX);
    }
}
