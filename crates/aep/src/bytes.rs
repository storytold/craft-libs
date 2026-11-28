//! Bounds-checked big-endian readers shared by the typed views. Every read returns `None` when the
//! bytes are not there; nothing here indexes a slice directly.

pub(crate) fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// The bytes up to the first NUL (all of them when there is none).
pub(crate) fn until_nul(b: &[u8]) -> &[u8] {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    b.get(..end).unwrap_or(&[])
}

/// A NUL-terminated name at `at`, at most `max` bytes long ([`legacy_text`]).
pub(crate) fn cstr(b: &[u8], at: usize, max: usize) -> Option<String> {
    let end = at.checked_add(max)?.min(b.len());
    let field = b.get(at..end)?;
    let name = until_nul(field);
    // Only a name with no NUL after it runs to the end of the field and can end in a cut character.
    Some(legacy_text(name, name.len() == field.len()))
}

/// The text of a fixed-size name field (solid and footage names in `opti`, parameter names in
/// `pard`). After Effects writes these in the encoding of the system that saved the file, not
/// always UTF-8: a Japanese-language After Effects on Windows writes solid names as UTF-8 but
/// footage names (and, in a 2025 save, parameter names) as Shift_JIS, so reading every byte as
/// Latin-1 garbled both. Valid UTF-8 is read as UTF-8, else valid Shift_JIS as Shift_JIS, else
/// Latin-1 (the encoding of the Western files the corpus was made with). When `filled` (the text
/// runs to the end of its field, with no NUL after it), a character cut by the end of the field is
/// dropped; a NUL-terminated text was not cut, so it keeps every byte.
pub(crate) fn legacy_text(b: &[u8], filled: bool) -> String {
    let latin1 = |b: &[u8]| b.iter().map(|c| char::from(*c)).collect::<String>();
    if b.is_ascii() {
        return latin1(b);
    }
    match std::str::from_utf8(b) {
        Ok(s) => return s.to_string(),
        // Only the last character is incomplete: the field ended inside it.
        Err(e) if filled && e.error_len().is_none() && e.valid_up_to() > 0 => {
            if let Some(Ok(s)) = b.get(..e.valid_up_to()).map(std::str::from_utf8)
                && !s.is_ascii()
            {
                return s.to_string();
            }
        }
        Err(_) => {}
    }
    let sjis = |b: &[u8]| encoding_rs::SHIFT_JIS.decode_without_bom_handling_and_without_replacement(b).map(|s| s.into_owned());
    // A Shift_JIS lead byte (0x81-0x9F, 0xE0-0xFC) at the very end is half a cut character.
    let cut = || {
        b.split_last()
            .filter(|(last, rest)| filled && matches!(**last, 0x81..=0x9f | 0xe0..=0xfc) && !rest.is_empty())
            .and_then(|(_, rest)| sjis(rest))
    };
    sjis(b).or_else(cut).unwrap_or_else(|| latin1(b))
}

pub(crate) fn array<const N: usize>(b: &[u8], at: usize) -> Option<[u8; N]> {
    let s = b.get(at..at.checked_add(N)?)?;
    let mut a = [0u8; N];
    a.copy_from_slice(s);
    Some(a)
}

pub(crate) fn be_u16(b: &[u8], at: usize) -> Option<u16> {
    array::<2>(b, at).map(u16::from_be_bytes)
}

pub(crate) fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    array::<4>(b, at).map(u32::from_be_bytes)
}

pub(crate) fn be_i32(b: &[u8], at: usize) -> Option<i32> {
    array::<4>(b, at).map(i32::from_be_bytes)
}

pub(crate) fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    array::<4>(b, at).map(u32::from_le_bytes)
}

pub(crate) fn be_f32(b: &[u8], at: usize) -> Option<f32> {
    array::<4>(b, at).map(f32::from_be_bytes)
}

pub(crate) fn be_f64(b: &[u8], at: usize) -> Option<f64> {
    array::<8>(b, at).map(f64::from_be_bytes)
}

#[cfg(test)]
mod tests {
    use super::{cstr, legacy_text};

    fn shift_jis(s: &str) -> Vec<u8> {
        let (bytes, _, unmappable) = encoding_rs::SHIFT_JIS.encode(s);
        assert!(!unmappable, "{s}");
        bytes.into_owned()
    }

    #[test]
    fn names_in_utf8_shift_jis_and_latin1() {
        // Solid names of a Japanese-language save: UTF-8.
        assert_eq!(legacy_text("赤い平面 2".as_bytes(), false), "赤い平面 2");
        // Footage names (and the parameter names of a 2025 save): Shift_JIS, which is not UTF-8.
        for name in ["背景の素材.png", "テスト用の名前", "円の中心", "ﾊﾝｶｸ"] {
            let bytes = shift_jis(name);
            assert!(std::str::from_utf8(&bytes).is_err(), "{name}");
            assert_eq!(legacy_text(&bytes, false), name);
        }
        // Western names stay Latin-1.
        assert_eq!(legacy_text(b"Caf\xe9 n\xb0 1", false), "Café n° 1");
        assert_eq!(legacy_text(b"Ambient Occlusion", false), "Ambient Occlusion");
    }

    #[test]
    fn a_shift_jis_name_field_reads_through_cstr() {
        let mut field = shift_jis("背景の素材.png");
        field.resize(40, 0);
        assert_eq!(cstr(&field, 0, 40).as_deref(), Some("背景の素材.png"));
    }

    #[test]
    fn a_character_cut_by_the_field_end_is_dropped() {
        // A 32-byte field: the last character does not fit (11 characters of 3 bytes).
        let utf8 = "あいうえおかきくけこさ".as_bytes();
        assert_eq!(cstr(utf8, 0, 32).as_deref(), Some("あいうえおかきくけこ"));
        let sjis = shift_jis("あいうえおかきくけこさしすせそた");
        assert_eq!(cstr(&sjis, 0, 31).as_deref(), Some("あいうえおかきくけこさしすせそ"));
        // Damaged bytes never panic and fall back to Latin-1.
        assert_eq!(legacy_text(&[0xff, 0xfe, 0x80], true).chars().count(), 3);
        assert_eq!(cstr(&[0x83], 0, 8).as_deref(), Some("\u{83}"));
    }

    #[test]
    fn only_a_name_that_fills_its_field_can_end_in_a_cut_character() {
        // Ended by a NUL, nothing was cut: an undecodable name keeps every byte as Latin-1
        // (0xe9 is also a Shift_JIS lead byte and 0xe3 a UTF-8 lead byte; 0xff is neither).
        assert_eq!(cstr(b"Ha\xff\0", 0, 32).as_deref(), Some("Ha\u{ff}"));
        assert_eq!(cstr(b"Caf\xe9\0", 0, 32).as_deref(), Some("Caf\u{e9}"));
        assert_eq!(cstr(b"\xc3\xa9\xe3\0", 0, 32).as_deref(), Some("\u{c3}\u{a9}\u{e3}"));
        // Filling the field, a last byte that cannot start a Shift_JIS character is not dropped.
        assert_eq!(cstr(b"Ha\xff", 0, 3).as_deref(), Some("Ha\u{ff}"));
    }
}
