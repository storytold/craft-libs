//! Tables Excel files refer to by number.

/// Built-in number format codes (en-US) by id. Ids not listed (23–36 are East Asian
/// locale formats) read as General.
const BUILTIN_FORMATS: [(u32, &str); 36] = [
    (0, "General"),
    (1, "0"),
    (2, "0.00"),
    (3, "#,##0"),
    (4, "#,##0.00"),
    (5, "\"$\"#,##0_);\\(\"$\"#,##0\\)"),
    (6, "\"$\"#,##0_);[Red]\\(\"$\"#,##0\\)"),
    (7, "\"$\"#,##0.00_);\\(\"$\"#,##0.00\\)"),
    (8, "\"$\"#,##0.00_);[Red]\\(\"$\"#,##0.00\\)"),
    (9, "0%"),
    (10, "0.00%"),
    (11, "0.00E+00"),
    (12, "# ?/?"),
    (13, "# ??/??"),
    (14, "m/d/yyyy"),
    (15, "d-mmm-yy"),
    (16, "d-mmm"),
    (17, "mmm-yy"),
    (18, "h:mm AM/PM"),
    (19, "h:mm:ss AM/PM"),
    (20, "h:mm"),
    (21, "h:mm:ss"),
    (22, "m/d/yyyy h:mm"),
    (37, "#,##0 ;(#,##0)"),
    (38, "#,##0 ;[Red](#,##0)"),
    (39, "#,##0.00;(#,##0.00)"),
    (40, "#,##0.00;[Red](#,##0.00)"),
    (41, "_(* #,##0_);_(* \\(#,##0\\);_(* \"-\"_);_(@_)"),
    (42, "_(\"$\"* #,##0_);_(\"$\"* \\(#,##0\\);_(\"$\"* \"-\"_);_(@_)"),
    (43, "_(* #,##0.00_);_(* \\(#,##0.00\\);_(* \"-\"??_);_(@_)"),
    (44, "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)"),
    (45, "mm:ss"),
    (46, "[h]:mm:ss"),
    (47, "mmss.0"),
    (48, "##0.0E+0"),
    (49, "@"),
];

pub fn builtin_format(id: u32) -> Option<&'static str> {
    BUILTIN_FORMATS.iter().find(|(i, _)| *i == id).map(|(_, c)| *c)
}

/// Id to write for a format code when it is a locale-independent built-in. The currency formats
/// 5 to 8 and the accounting formats with a currency, 42 and 44, are left out (`None`, to be
/// written as custom formats) because their meaning depends on the reader's locale.
pub fn builtin_format_id(code: &str) -> Option<u32> {
    BUILTIN_FORMATS.iter().find(|(i, c)| *c == code && !matches!(i, 5..=8 | 42 | 44)).map(|(i, _)| *i)
}

/// The indexed colour table of a workbook that does not replace it, as `0xRRGGBB`: the eight
/// fixed colours, then the 56 a BIFF8 Palette record ([MS-XLS] 2.4.188) or a SpreadsheetML
/// `indexedColors` list (ECMA-376 Part 1, 18.8.27) replaces.
pub const DEFAULT_PALETTE: [u32; 64] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00,
    0xFF00FF, 0x00FFFF, 0x800000, 0x008000, 0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, 0x9999FF, 0x993366, 0xFFFFCC, 0xCCFFFF,
    0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF, 0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF,
    0xCCFFCC, 0xFFFF99, 0x99CCFF, 0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600, 0x666699, 0x969696,
    0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399, 0x333333,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_format_ids() {
        assert_eq!((0..=60).filter(|&id| builtin_format(id).is_some()).count(), 36);
        assert_eq!(builtin_format(0), Some("General"));
        assert_eq!(builtin_format(14), Some("m/d/yyyy"));
        assert_eq!(builtin_format(22), Some("m/d/yyyy h:mm"));
        assert_eq!(builtin_format(49), Some("@"));
        // 23 to 36 are locale formats that are not defined here.
        assert_eq!((23..=36).find_map(builtin_format), None);
        assert_eq!(builtin_format(50), None);
        for id in 0..=60 {
            if let Some(code) = builtin_format(id) {
                let back = builtin_format_id(code);
                assert_eq!(back, (!matches!(id, 5..=8 | 42 | 44)).then_some(id), "{id}");
            }
        }
        assert_eq!(builtin_format_id("yyyy"), None);
    }

    #[test]
    fn the_palette_repeats_its_eight_fixed_colours() {
        assert_eq!(DEFAULT_PALETTE.get(..8), DEFAULT_PALETTE.get(8..16));
        assert_eq!(DEFAULT_PALETTE.get(63), Some(&0x333333));
    }
}
