//! Program header and dynamic section tests. `an_address_past_the_last_file_offset_maps_to_nothing`
//! is from Craft Anything's `elf.rs`; the image builder is a small stand-in for its testkit's.

use super::*;

/// Field writer in the image's class and byte order.
struct Image {
    d: Vec<u8>,
    is64: bool,
    le: bool,
}

impl Image {
    fn put(&mut self, at: usize, value: u64, width: usize) {
        let bytes = value.to_le_bytes();
        let mut field = bytes[..width].to_vec();
        if !self.le {
            field.reverse();
        }
        self.d[at..at + width].copy_from_slice(&field);
    }

    fn word(&mut self, at: usize, value: u64) {
        self.put(at, value, if self.is64 { 8 } else { 4 });
    }
}

/// An executable with PT_PHDR, PT_LOAD (r-x, whole file at 0x40_0000), PT_DYNAMIC at file 0x200
/// and PT_GNU_STACK (rw-). The dynamic section holds NEEDED, STRTAB and a DT_NULL, then junk.
fn elf(is64: bool, le: bool) -> Vec<u8> {
    let mut img = Image { d: vec![0u8; 0x300], is64, le };
    img.d[..4].copy_from_slice(b"\x7fELF");
    img.d[4] = if is64 { 2 } else { 1 };
    img.d[5] = if le { 1 } else { 2 };
    img.d[6] = 1;
    let (phoff_at, entsize_at, num_at, phoff, entsize) = if is64 { (32, 54, 56, 64, 56) } else { (28, 42, 44, 52, 32) };
    img.word(phoff_at, phoff as u64);
    img.put(entsize_at, entsize as u64, 2);
    img.put(num_at, 4, 2);
    let headers = [
        (6u32, 4u32, phoff as u64, 0x40_0040, 4 * entsize as u64),
        (1, 5, 0, 0x40_0000, 0x300),
        (2, 6, 0x200, 0x40_0200, 0x60),
        (0x6474_e551, 6, 0, 0, 0),
    ];
    for (i, (t, f, off, va, size)) in headers.into_iter().enumerate() {
        let o = phoff + i * entsize;
        img.put(o, u64::from(t), 4);
        if is64 {
            img.put(o + 4, u64::from(f), 4);
            img.word(o + 8, off);
            img.word(o + 16, va);
            img.word(o + 32, size);
            img.word(o + 40, size);
            img.word(o + 48, 0x1000);
        } else {
            img.word(o + 4, off);
            img.word(o + 8, va);
            img.word(o + 16, size);
            img.word(o + 20, size);
            img.put(o + 24, u64::from(f), 4);
            img.word(o + 28, 0x1000);
        }
    }
    let w = if is64 { 8 } else { 4 };
    for (i, (tag, value)) in [(1u64, 1u64), (5, 0x40_0280), (0, 0), (1, 9)].into_iter().enumerate() {
        img.word(0x200 + i * 2 * w, tag);
        img.word(0x200 + i * 2 * w + w, value);
    }
    img.d
}

#[test]
fn program_headers_in_every_class_and_byte_order() {
    for is64 in [false, true] {
        for le in [false, true] {
            let ph = program_headers(&elf(is64, le)).unwrap();
            let kinds: Vec<(&str, &str)> = ph.iter().map(|p| (p.kind.as_str(), p.flags.as_str())).collect();
            assert_eq!(kinds, [("PHDR", "r--"), ("LOAD", "r-x"), ("DYNAMIC", "rw-"), ("GNU_STACK", "rw-")], "is64={is64} le={le}");
            assert_eq!((ph[1].offset, ph[1].vaddr, ph[1].file_size, ph[1].mem_size, ph[1].align), (0, 0x40_0000, 0x300, 0x300, 0x1000));
            assert_eq!((ph[2].p_type, ph[2].p_flags), (2, 6));
            let dynamic = dynamic_entries(&elf(is64, le), &ph).unwrap();
            assert_eq!(dynamic, [(1, 1), (5, 0x40_0280)], "stops at DT_NULL");
            assert_eq!(va_to_off(&ph, 0x40_0280), Some(0x280));
        }
    }
}

#[test]
fn bad_idents_and_headers_are_errors() {
    assert_eq!(program_headers(b"\x7fELX\x02\x01"), Err(ElfError::Malformed("no ELF magic".into())));
    assert_eq!(program_headers(b"\x7fELF"), Err(ElfError::Truncated("ELF ident")));
    assert_eq!(program_headers(b"\x7fELF\x02"), Err(ElfError::Truncated("ELF ident")));
    assert_eq!(program_headers(b"\x7fELF\x03\x01"), Err(ElfError::Malformed("bad ELF class or data encoding".into())));
    assert_eq!(program_headers(b"\x7fELF\x01\x00"), Err(ElfError::Malformed("bad ELF class or data encoding".into())));
    assert_eq!(program_headers(&elf(true, true)[..20]), Err(ElfError::Truncated("ELF header")));
    assert_eq!(ElfError::Truncated("ELF header").to_string(), "the file ends early (ELF header)");
    let mut small = elf(true, true);
    small[54] = 40;
    assert_eq!(program_headers(&small).unwrap_err().to_string(), "the file is malformed (program header entry too small)");
    assert!(dynamic_entries(b"nope", &[]).is_err());
}

#[test]
fn a_table_cut_short_keeps_its_complete_entries() {
    let d = elf(true, true);
    assert_eq!(program_headers(&d[..64 + 56 * 2 + 10]).unwrap().len(), 2);
    let mut none = d.clone();
    none[56] = 0;
    none[54] = 0;
    assert!(program_headers(&none).unwrap().is_empty(), "no headers need no entry size");
    let mut far = d.clone();
    far[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(program_headers(&far).unwrap().is_empty(), "offsets that overflow stop the walk");
}

#[test]
fn dynamic_entries_need_a_dynamic_segment_inside_the_file() {
    let d = elf(false, true);
    let mut ph = program_headers(&d).unwrap();
    assert!(dynamic_entries(&d, &ph[..2]).unwrap().is_empty());
    ph[2].offset = 0x2f8;
    assert!(dynamic_entries(&d, &ph).unwrap().is_empty(), "an entry cut off by the end of the file");
    ph[2].offset = u64::MAX;
    assert!(dynamic_entries(&d, &ph).unwrap().is_empty());
}

fn load(offset: u64, vaddr: u64, file_size: u64) -> ProgramHeader {
    ProgramHeader { kind: "LOAD".into(), flags: String::new(), offset, vaddr, file_size, mem_size: file_size, align: 0, p_type: 1, p_flags: 0 }
}

#[test]
fn an_address_past_the_last_file_offset_maps_to_nothing() {
    assert_eq!(va_to_off(&[load(0x200, 0x1000, 0x100)], 0x1010), Some(0x210));
    assert_eq!(va_to_off(&[load(u64::MAX, 0x1000, 0x100)], 0x1000), Some(u64::MAX));
    assert_eq!(va_to_off(&[load(u64::MAX, 0x1000, 0x100)], 0x1001), None, "the offset would pass u64::MAX");
    assert_eq!(va_to_off(&[load(0x200, 0x1000, 0x100)], 0x1100), None, "the zero-filled tail has no file bytes");
    let mut dynamic = load(0, 0x1000, 0x100);
    dynamic.p_type = 2;
    assert_eq!(va_to_off(&[dynamic], 0x1010), None, "only PT_LOAD maps");
}

#[test]
fn names() {
    let segments: Vec<String> =
        [0, 1, 2, 3, 4, 5, 6, 7, 0x6474_e550, 0x6474_e551, 0x6474_e552, 0x6474_e553, 0x7000_0001, 0x99].into_iter().map(segment_type_name).collect();
    assert_eq!(
        segments,
        [
            "NULL",
            "LOAD",
            "DYNAMIC",
            "INTERP",
            "NOTE",
            "SHLIB",
            "PHDR",
            "TLS",
            "GNU_EH_FRAME",
            "GNU_STACK",
            "GNU_RELRO",
            "GNU_PROPERTY",
            "ARM_EXIDX",
            "0x99"
        ]
    );
    assert_eq!([0, 1, 2, 4, 7].map(permissions), ["---", "--x", "-w-", "r--", "rwx"]);
    let tags: Vec<String> =
        (0..=30).chain([0x6fff_fef5, 0x6fff_fffb, 0x6fff_fff0, 0x6fff_fffe, 0x6fff_ffff, 0x7000_0000]).map(dynamic_tag_name).collect();
    assert_eq!(tags[..3], ["NULL", "NEEDED", "PLTRELSZ"]);
    assert_eq!(tags[29..], ["RUNPATH", "FLAGS", "GNU_HASH", "FLAGS_1", "VERSYM", "VERNEED", "VERNEEDNUM", "0x70000000"]);
    assert!(tags.iter().all(|t| !t.is_empty()));
}

#[cfg(feature = "serde")]
#[test]
fn program_headers_serialize_with_hex_addresses() {
    let ph = program_headers(&elf(true, true)).unwrap();
    let value = serde_json::to_value(&ph[1]).unwrap();
    assert_eq!(value["kind"], "LOAD");
    assert_eq!(value["vaddr"], "0x400000");
    assert!(value.get("p_type").is_none());
}

#[test]
fn damaged_input_never_panics() {
    for sample in [elf(true, true), elf(false, false)] {
        for i in 0..sample.len() {
            if let Ok(ph) = program_headers(&sample[..i]) {
                let _ = dynamic_entries(&sample[..i], &ph);
            }
            for v in [0x00, 0x7f, 0xff] {
                let mut m = sample.clone();
                m[i] = v;
                if let Ok(ph) = program_headers(&m) {
                    let _ = dynamic_entries(&m, &ph);
                    let _ = va_to_off(&ph, 0x40_0280);
                }
            }
        }
    }
}
