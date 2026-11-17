// From Craft Anything crates/formats/tests/pe_optional_header_framing.rs.
use craft_pe as pe;

fn image(plus: bool, optional_size: usize, declared_dirs: u32) -> Vec<u8> {
    let opt = 0x98;
    let fixed = if plus { 112 } else { 96 };
    let mut data = vec![0; opt + optional_size.max(fixed) + 64];
    data[..2].copy_from_slice(b"MZ");
    data[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    data[0x80..0x84].copy_from_slice(b"PE\0\0");
    data[0x94..0x96].copy_from_slice(&(optional_size as u16).to_le_bytes());
    data[opt..opt + 2].copy_from_slice(&(if plus { 0x20bu16 } else { 0x10b }).to_le_bytes());
    data[opt + 68..opt + 70].copy_from_slice(&3u16.to_le_bytes());
    data[opt + fixed - 4..opt + fixed].copy_from_slice(&declared_dirs.to_le_bytes());
    data
}

#[test]
fn required_optional_header_fields_cannot_be_borrowed_from_the_section_table() {
    for plus in [false, true] {
        for size in [0, 2, 70, if plus { 111 } else { 95 }] {
            assert!(pe::headers(&image(plus, size, 16)).is_err(), "plus={plus}, size={size}");
        }
    }
}

#[test]
fn directory_count_is_bounded_by_complete_entries_in_the_optional_header() {
    for plus in [false, true] {
        let fixed = if plus { 112 } else { 96 };
        for available in [0, 7, 8, 15] {
            let mut data = image(plus, fixed + available, 16);
            data[0x98 + fixed..0x98 + fixed + 16].fill(0x55);
            let parsed = pe::headers(&data).unwrap();
            assert_eq!(parsed.data_dirs.len(), available / 8, "plus={plus}, available={available}");
            if available >= 8 {
                assert_eq!(parsed.data_dirs[0], (0x5555_5555, 0x5555_5555), "plus={plus}, available={available}");
            }
        }
    }
}

#[test]
fn zero_or_one_directory_and_unused_optional_bytes_are_valid() {
    for plus in [false, true] {
        let fixed = if plus { 112 } else { 96 };
        for dirs in [0u32, 1] {
            for padding in [0, 12] {
                let size = fixed + dirs as usize * 8 + padding;
                let mut data = image(plus, size, dirs);
                data[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
                let section = 0x98 + size;
                data[section..section + 5].copy_from_slice(b".test");
                let parsed = pe::headers(&data).unwrap();
                assert_eq!(parsed.pe32_plus, plus);
                assert_eq!(parsed.subsystem, 3);
                assert_eq!(parsed.data_dirs.len(), dirs as usize);
                assert_eq!(parsed.sections[0].name, ".test");
            }
        }
    }
}
