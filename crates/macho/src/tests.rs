//! Slice table, name and thread-state tests. The thread-state cases are Craft Anything's
//! `macho_thread_state` integration test, called on the command body directly instead of
//! through the app's program model.

use super::*;

const X86_64: u32 = 0x0100_0007;
const ARM64: u32 = 0x0100_000c;

/// A universal binary with `slices` of (cputype, cpusubtype, body), each at a 0x100 boundary.
fn fat(wide: bool, slices: &[(u32, u32, &[u8])]) -> Vec<u8> {
    let entry = if wide { 32 } else { 20 };
    let mut d = vec![0u8; 0x100];
    d[0..4].copy_from_slice(&(if wide { 0xcafe_babfu32 } else { 0xcafe_babe }).to_be_bytes());
    d[4..8].copy_from_slice(&(slices.len() as u32).to_be_bytes());
    for (i, &(cpu, sub, body)) in slices.iter().enumerate() {
        let offset = d.len();
        let o = 8 + i * entry;
        d[o..o + 4].copy_from_slice(&cpu.to_be_bytes());
        d[o + 4..o + 8].copy_from_slice(&sub.to_be_bytes());
        if wide {
            d[o + 8..o + 16].copy_from_slice(&(offset as u64).to_be_bytes());
            d[o + 16..o + 24].copy_from_slice(&(body.len() as u64).to_be_bytes());
            d[o + 24..o + 28].copy_from_slice(&8u32.to_be_bytes());
        } else {
            d[o + 8..o + 12].copy_from_slice(&(offset as u32).to_be_bytes());
            d[o + 12..o + 16].copy_from_slice(&(body.len() as u32).to_be_bytes());
            d[o + 16..o + 20].copy_from_slice(&8u32.to_be_bytes());
        }
        d.extend_from_slice(body);
        d.resize(d.len().next_multiple_of(0x100), 0);
    }
    d
}

#[test]
fn fat_and_fat64_tables_list_their_slices() {
    for wide in [false, true] {
        let d = fat(wide, &[(X86_64, 3, &[1; 16]), (ARM64, 2, &[2; 32])]);
        let s = fat_slices(&d).unwrap();
        assert_eq!(s.len(), 2, "wide={wide}");
        assert_eq!(s[0], FatSlice { arch: "x86_64".into(), cputype: X86_64, cpusubtype: 3, offset: 0x100, size: 16, align: 8 });
        assert_eq!((s[1].arch.as_str(), s[1].offset, s[1].size), ("arm64e", 0x200, 32));
    }
}

#[test]
fn damaged_tables_are_errors() {
    assert_eq!(fat_slices(b"\xca\xfe"), Err(MachoError::Truncated("fat header".into())));
    assert_eq!(fat_slices(&[0xfe, 0xed, 0xfa, 0xcf, 0, 0, 0, 1]), Err(MachoError::Malformed("not a universal binary".into())));
    assert_eq!(fat_slices(&[0xca, 0xfe, 0xba, 0xbe]), Err(MachoError::Truncated("fat header".into())));
    // A Java 21 class file: minor 0, major 65.
    let java = [0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 65];
    assert_eq!(fat_slices(&java).unwrap_err().to_string(), "the file is malformed (65 slices (Java class files share this magic))");
    assert!(matches!(fat_slices(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 0]), Err(MachoError::Malformed(_))));
    let d = fat(false, &[(X86_64, 3, &[1; 16])]);
    assert_eq!(fat_slices(&d[..20]), Err(MachoError::Truncated("fat arch table".into())));
    assert_eq!(fat_slices(&d[..0x108]), Err(MachoError::Truncated("slice 0 runs past the end of the file".into())));
    let mut wrap = fat(true, &[(X86_64, 3, &[1; 16])]);
    wrap[16..24].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(matches!(fat_slices(&wrap), Err(MachoError::Truncated(_))), "offset + size must not wrap");
}

#[test]
fn cpu_names() {
    let names: Vec<String> = [
        (7, 3),
        (X86_64, 3),
        (X86_64, 8),
        (X86_64, 0x8000_0008),
        (X86_64, 0x108),
        (12, 9),
        (ARM64, 0),
        (ARM64, 0x8000_0002),
        (0x0200_000c, 1),
        (18, 0),
        (0x0100_0012, 0),
        (0x99, 0),
    ]
    .into_iter()
    .map(|(cpu, sub)| cpu_name(cpu, sub))
    .collect();
    assert_eq!(names, ["i386", "x86_64", "x86_64h", "x86_64h", "x86_64", "arm", "arm64", "arm64e", "arm64_32", "ppc", "ppc64", "cpu0x99"]);
}

#[test]
fn load_command_names_and_sizes() {
    let known = [
        (0x1, "LC_SEGMENT", 56),
        (0x2, "LC_SYMTAB", 24),
        (0x5, "LC_UNIXTHREAD", 16),
        (0xb, "LC_DYSYMTAB", 80),
        (0xc, "LC_LOAD_DYLIB", 24),
        (0xd, "LC_ID_DYLIB", 24),
        (0xe, "LC_LOAD_DYLINKER", 12),
        (0x19, "LC_SEGMENT_64", 72),
        (0x1b, "LC_UUID", 24),
        (0x1d, "LC_CODE_SIGNATURE", 16),
        (0x1e, "LC_SEGMENT_SPLIT_INFO", 16),
        (0x20, "LC_LAZY_LOAD_DYLIB", 24),
        (0x21, "LC_ENCRYPTION_INFO", 20),
        (0x22, "LC_DYLD_INFO", 48),
        (0x8000_0022, "LC_DYLD_INFO_ONLY", 48),
        (0x24, "LC_VERSION_MIN_MACOSX", 16),
        (0x25, "LC_VERSION_MIN_IPHONEOS", 16),
        (0x26, "LC_FUNCTION_STARTS", 16),
        (0x29, "LC_DATA_IN_CODE", 16),
        (0x2a, "LC_SOURCE_VERSION", 16),
        (0x2c, "LC_ENCRYPTION_INFO_64", 24),
        (0x32, "LC_BUILD_VERSION", 24),
        (0x8000_0018, "LC_LOAD_WEAK_DYLIB", 24),
        (0x8000_001c, "LC_RPATH", 12),
        (0x8000_001f, "LC_REEXPORT_DYLIB", 24),
        (0x8000_0028, "LC_MAIN", 24),
        (0x8000_0033, "LC_DYLD_EXPORTS_TRIE", 16),
        (0x8000_0034, "LC_DYLD_CHAINED_FIXUPS", 16),
    ];
    for (cmd, name, size) in known {
        assert_eq!(load_command_name(cmd), name);
        assert_eq!(load_command_min_size(cmd), size, "{name}");
    }
    assert_eq!(load_command_name(0x8000_0023), "0x80000023");
    assert_eq!(load_command_min_size(0x8000_0023), 24);
    assert_eq!(load_command_min_size(0x27), 12);
    assert_eq!(load_command_min_size(0x2e), 16);
    assert_eq!(load_command_min_size(0x77), 8);
}

#[test]
fn nibble_versions() {
    assert_eq!(nibble_version(0x000e_0500), "14.5.0");
    assert_eq!(nibble_version(0x000a_0f01), "10.15.1");
}

fn state(flavor: u32, count: u32, bytes: usize, pc_offset: usize, pc: u64, wide: bool) -> Vec<u8> {
    let mut b = vec![0u8; 8 + bytes];
    b[0..4].copy_from_slice(&flavor.to_le_bytes());
    b[4..8].copy_from_slice(&count.to_le_bytes());
    if wide {
        b[8 + pc_offset..16 + pc_offset].copy_from_slice(&pc.to_le_bytes());
    } else {
        b[8 + pc_offset..12 + pc_offset].copy_from_slice(&(pc as u32).to_le_bytes());
    }
    b
}

fn decode(cpu: u32, wide: bool, states: &[Vec<u8>]) -> Option<u64> {
    unixthread_pc(&states.concat(), cpu, wide)
}

#[test]
fn a_program_counter_requires_a_complete_declared_state() {
    for count in [0, 1, 34, 41] {
        assert_eq!(decode(X86_64, true, &[state(4, count, 168, 128, 0x1234, true)]), None, "count {count}");
    }
    assert_eq!(decode(X86_64, true, &[state(4, 42, 136, 128, 0x1234, true)]), None);
    assert_eq!(decode(ARM64, true, &[state(6, 68, 264, 256, 0x1234, true)]), None);
}

#[test]
fn flavor_numbers_are_interpreted_for_the_image_cpu() {
    assert_eq!(decode(ARM64, true, &[state(4, 42, 168, 128, 0x1234, true)]), None);
    assert_eq!(decode(X86_64, true, &[state(6, 68, 272, 256, 0x1234, true)]), None);
    assert_eq!(decode(12, false, &[state(1, 17, 68, 40, 0x1234, false)]), None);
}

#[test]
fn a_later_matching_state_is_reached_using_word_counts() {
    let unknown = state(99, 2, 8, 0, 0, true);
    let matching = state(4, 42, 168, 128, 0x1234, true);
    assert_eq!(decode(X86_64, true, &[unknown, matching]), Some(0x1234));
}

#[test]
fn complete_supported_states_keep_their_program_counters() {
    assert_eq!(decode(X86_64, true, &[state(4, 42, 168, 128, 0x1234, true)]), Some(0x1234));
    assert_eq!(decode(ARM64, true, &[state(6, 68, 272, 256, 0x1234, true)]), Some(0x1234));
    assert_eq!(decode(7, false, &[state(1, 16, 64, 40, 0x1234, false)]), Some(0x1234));
}

#[test]
fn endless_empty_states_stop() {
    assert_eq!(unixthread_pc(&vec![0; 8 * (MAX_THREAD_STATES + 4)], X86_64, true), None);
    assert_eq!(unixthread_pc(&[4, 0, 0, 0, 0xff, 0xff, 0xff, 0xff], X86_64, true), None);
}

#[cfg(feature = "serde")]
#[test]
fn slices_serialize() {
    let s = fat_slices(&fat(false, &[(ARM64, 0, &[0; 4])])).unwrap();
    let value = serde_json::to_value(&s[0]).unwrap();
    assert_eq!(value["arch"], "arm64");
    assert_eq!(value["offset"], 0x100);
}

#[test]
fn damaged_input_never_panics() {
    let sample = fat(true, &[(X86_64, 3, &[1; 16]), (ARM64, 2, &[2; 32])]);
    let thread = state(4, 42, 168, 128, 0x1234, true);
    for source in [&sample, &thread] {
        for i in 0..source.len() {
            let _ = fat_slices(&source[..i]);
            let _ = unixthread_pc(&source[..i], X86_64, true);
            for v in [0x00, 0x7f, 0xff] {
                let mut m = source.clone();
                m[i] = v;
                let _ = fat_slices(&m);
                let _ = unixthread_pc(&m, ARM64, true);
                let _ = unixthread_pc(&m, 7, false);
            }
        }
    }
}
