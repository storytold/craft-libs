//! Tests for detection.

use super::*;
use craft_ui::testing::trial;

/// A program whose VERSIONINFO has a ProductVersion in the U.S. English UTF-16 table but no
/// `\VarFileInfo\Translation` entry still has a version.
#[cfg(windows)]
#[trial]
fn a_version_without_a_translation_entry_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("photocraft.exe");
    std::fs::write(&exe, resource_only_program("1.2.3")).unwrap();
    assert_eq!(windows::executable_version(&exe), version::parse("1.2.3"));
}

/// A VERSIONINFO block node: length, value length, type, the key, then the value and the
/// children, each aligned to 4 bytes.
#[cfg(windows)]
fn version_node(key: &str, value: &[u8], value_length: u16, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let align = |bytes: &mut Vec<u8>| bytes.resize(bytes.len().next_multiple_of(4), 0);
    let mut node = vec![0, 0];
    node.extend(value_length.to_le_bytes());
    node.extend(u16::from(text).to_le_bytes());
    node.extend(key.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
    align(&mut node);
    node.extend(value);
    for child in children {
        align(&mut node);
        node.extend(child);
    }
    let length = u16::try_from(node.len()).unwrap();
    node[..2].copy_from_slice(&length.to_le_bytes());
    node
}

/// A 64-bit program file holding only a version resource (`ProductVersion` = `product`, in
/// the 040904B0 string table) and no `\VarFileInfo\Translation` entry.
#[cfg(windows)]
fn resource_only_program(product: &str) -> Vec<u8> {
    let put = |bytes: &mut Vec<u8>, at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
    let text: Vec<u8> = product.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    let string = version_node("ProductVersion", &text, u16::try_from(product.len() + 1).unwrap(), true, &[]);
    let table = version_node("040904b0", &[], 0, true, &[string]);
    let strings = version_node("StringFileInfo", &[], 0, true, &[table]);
    let mut fixed = vec![0u8; 52];
    put(&mut fixed, 0, &0xFEEF_04BDu32.to_le_bytes());
    put(&mut fixed, 4, &0x0001_0000u32.to_le_bytes());
    let info = version_node("VS_VERSION_INFO", &fixed, 52, false, &[strings]);

    // The resource section: type 16 (RT_VERSION) > id 1 > language 0x0409 > the data.
    const RVA: u32 = 0x1000;
    let mut rsrc = vec![0u8; 0x58];
    for (dir, id, next) in [(0x00, 16u32, 0x8000_0018u32), (0x18, 1, 0x8000_0030), (0x30, 0x0409, 0x48)] {
        put(&mut rsrc, dir + 14, &1u16.to_le_bytes());
        put(&mut rsrc, dir + 16, &id.to_le_bytes());
        put(&mut rsrc, dir + 20, &next.to_le_bytes());
    }
    put(&mut rsrc, 0x48, &(RVA + 0x58).to_le_bytes());
    put(&mut rsrc, 0x4C, &u32::try_from(info.len()).unwrap().to_le_bytes());
    rsrc.extend(&info);
    let raw = u32::try_from(rsrc.len().next_multiple_of(0x200)).unwrap();
    let rsrc_len = u32::try_from(rsrc.len()).unwrap();

    let mut file = vec![0u8; 0x200];
    put(&mut file, 0, b"MZ");
    put(&mut file, 0x3C, &0x40u32.to_le_bytes());
    put(&mut file, 0x40, b"PE\0\0");
    // COFF header: x64, one section, a 240-byte optional header, an executable DLL image.
    put(&mut file, 0x44, &0x8664u16.to_le_bytes());
    put(&mut file, 0x46, &1u16.to_le_bytes());
    put(&mut file, 0x54, &240u16.to_le_bytes());
    put(&mut file, 0x56, &0x2022u16.to_le_bytes());
    // PE32+ optional header.
    let opt = 0x58;
    put(&mut file, opt, &0x20Bu16.to_le_bytes());
    put(&mut file, opt + 8, &raw.to_le_bytes());
    put(&mut file, opt + 24, &0x1_4000_0000u64.to_le_bytes());
    put(&mut file, opt + 32, &0x1000u32.to_le_bytes());
    put(&mut file, opt + 36, &0x200u32.to_le_bytes());
    put(&mut file, opt + 40, &6u16.to_le_bytes());
    put(&mut file, opt + 48, &6u16.to_le_bytes());
    put(&mut file, opt + 56, &(RVA + rsrc_len.next_multiple_of(0x1000)).to_le_bytes());
    put(&mut file, opt + 60, &0x200u32.to_le_bytes());
    put(&mut file, opt + 68, &2u16.to_le_bytes());
    put(&mut file, opt + 72, &0x10_0000u64.to_le_bytes());
    put(&mut file, opt + 80, &0x1000u64.to_le_bytes());
    put(&mut file, opt + 88, &0x10_0000u64.to_le_bytes());
    put(&mut file, opt + 96, &0x1000u64.to_le_bytes());
    put(&mut file, opt + 108, &16u32.to_le_bytes());
    put(&mut file, opt + 112 + 2 * 8, &RVA.to_le_bytes());
    put(&mut file, opt + 112 + 2 * 8 + 4, &rsrc_len.to_le_bytes());
    // The .rsrc section header.
    let section = opt + 240;
    put(&mut file, section, b".rsrc\0\0\0");
    put(&mut file, section + 8, &rsrc_len.to_le_bytes());
    put(&mut file, section + 12, &RVA.to_le_bytes());
    put(&mut file, section + 16, &raw.to_le_bytes());
    put(&mut file, section + 20, &0x200u32.to_le_bytes());
    put(&mut file, section + 36, &0x4000_0040u32.to_le_bytes());
    rsrc.resize(usize::try_from(raw).unwrap(), 0);
    file.extend(rsrc);
    file
}

#[trial]
fn recognises_appimage_names_conservatively() {
    let pdf = ["pdfcraft", "printcraft"];
    let v = |n: &str| appimage_version(n, &pdf);
    assert_eq!(v("pdfcraft-0.3.0-linux-x86_64.AppImage"), Some(Some("0.3.0".into())));
    assert_eq!(v("printcraft-0.2.1-linux-aarch64.appimage"), Some(Some("0.2.1".into())));
    assert_eq!(v("pdfcraft-0.3.0-rc.1-linux-x86_64.AppImage"), Some(Some("0.3.0-rc.1".into())));
    // Gear Lever's rename, and a plain rename by hand.
    assert_eq!(v("pdfcraft.appimage"), Some(None));
    assert_eq!(v("PdfCraft.AppImage"), Some(None));
    for other in [
        "pdfcraft-0.3.0-linux-x86_64.AppImage.zsync",
        "pdfcraft-0.3.0-linux-x86_64.deb",
        "pdfcraft-0.3.0-windows-x64.AppImage",
        "pdfcraft-cli-0.3.0-linux-x86_64.AppImage",
        "photocraft-0.6.0-linux-x86_64.AppImage",
        "mypdfcraft.AppImage",
        "pdfcraft-latest.AppImage",
        ".appimage",
        "x",
    ] {
        assert_eq!(v(other), None, "{other}");
    }
}
