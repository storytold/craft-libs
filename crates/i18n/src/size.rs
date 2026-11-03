//! File sizes and other measures with a unit, in the interface language.
//!
//! Unit names are catalog rows in context `unit`, keyed by the English template with the
//! number as `{n}` (`{n} MB`, `{n} px`, `{n}%`), so a translation can change the symbol and its
//! spacing (`{n} Mo`, `{n} %`). They are looked up with [`I18n::get`], never the plain row: a
//! plain `B` row means bold. A byte count in words is the `@plural` row `{n} byte|{n} bytes`.

use crate::{I18n, Lang, fmt, ltr};

/// The context unit rows are in.
const CONTEXT: &str = "unit";

/// A file-size unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SizeUnit {
    Byte,
    Kilo,
    Mega,
    Giga,
    Tera,
}

impl SizeUnit {
    const ALL: [SizeUnit; 5] = [SizeUnit::Byte, SizeUnit::Kilo, SizeUnit::Mega, SizeUnit::Giga, SizeUnit::Tera];
}

/// How [`I18n::file_size`] writes a byte count: the step between units, their names, and the
/// digits after the decimal point. Start from [`FileSize::BINARY`], [`FileSize::IEC`] or
/// [`FileSize::DECIMAL`] and adjust:
///
/// ```
/// use craft_i18n::{FileSize, SizeUnit};
/// // `12 MB`, `3.4 MB`, `640 KB`: decimal steps, whole kilobytes, no decimals from 10 MB.
/// let style = FileSize::DECIMAL.smallest(SizeUnit::Kilo).largest(SizeUnit::Mega).unit_decimals(SizeUnit::Kilo, 0).whole_from(10);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileSize {
    step: u64,
    iec: bool,
    /// Per unit, in [`SizeUnit::ALL`] order; bytes are always whole.
    decimals: [u8; 5],
    whole_from: u32,
    smallest: SizeUnit,
    largest: SizeUnit,
    byte_symbol: bool,
}

impl FileSize {
    /// Steps of 1024 named KB, MB, GB and TB, as Windows Explorer shows sizes; one decimal, bytes
    /// in words.
    pub const BINARY: FileSize = FileSize {
        step: 1024,
        iec: false,
        decimals: [0, 1, 1, 1, 1],
        whole_from: 0,
        smallest: SizeUnit::Byte,
        largest: SizeUnit::Tera,
        byte_symbol: false,
    };
    /// Steps of 1024 named KiB, MiB, GiB and TiB.
    pub const IEC: FileSize = FileSize { iec: true, ..FileSize::BINARY };
    /// Steps of 1000 named KB, MB, GB and TB, as macOS shows sizes and as download and disk sizes
    /// are sold.
    pub const DECIMAL: FileSize = FileSize { step: 1000, ..FileSize::BINARY };

    /// `n` digits after the decimal point in every unit above bytes.
    pub const fn decimals(mut self, n: u8) -> FileSize {
        self.decimals = [0, n, n, n, n];
        self
    }

    /// `n` digits after the decimal point in `unit` (`0` for whole kilobytes, `2` for gigabytes).
    pub const fn unit_decimals(mut self, unit: SizeUnit, n: u8) -> FileSize {
        if !matches!(unit, SizeUnit::Byte) {
            self.decimals[unit as usize] = n;
        }
        self
    }

    /// No decimals from `n` of a unit up: `whole_from(10)` writes `9.5 MB` and `12 MB`. `0`, the
    /// default, keeps them.
    pub const fn whole_from(mut self, n: u32) -> FileSize {
        self.whole_from = n;
        self
    }

    /// The smallest unit shown. Sizes below it are written in it, rounded up, so a file that
    /// isn't empty never reads `0 KB`.
    pub const fn smallest(mut self, unit: SizeUnit) -> FileSize {
        self.smallest = unit;
        self
    }

    /// The largest unit shown; larger sizes are counted in it (`2048 GB`).
    pub const fn largest(mut self, unit: SizeUnit) -> FileSize {
        self.largest = unit;
        self
    }

    /// Bytes as the symbol `B` (`512 B`) instead of words (`512 bytes`).
    pub const fn byte_symbol(mut self) -> FileSize {
        self.byte_symbol = true;
        self
    }

    /// The unit `bytes` is written in, the number in it and its digits after the point.
    fn parts(&self, bytes: u64) -> (SizeUnit, f64, usize) {
        let (smallest, largest) = (self.smallest.min(self.largest), self.largest);
        let mut unit = smallest;
        for &u in SizeUnit::ALL.iter().filter(|u| (smallest..=largest).contains(u)) {
            if bytes as f64 >= self.scale(u) {
                unit = u;
            }
        }
        loop {
            let value = bytes as f64 / self.scale(unit);
            let round = |decimals: u8| {
                let factor = 10f64.powi(i32::from(decimals));
                (if unit == smallest && bytes > 0 && value < 1.0 { (value * factor).ceil() } else { (value * factor).round() }) / factor
            };
            let mut decimals = if unit == SizeUnit::Byte { 0 } else { self.decimals.get(unit as usize).copied().unwrap_or(0) };
            let mut rounded = round(decimals);
            // After rounding, so 9.96 MB, which rounds to 10.0, is written 10 MB.
            if self.whole_from > 0 && rounded >= f64::from(self.whole_from) {
                decimals = 0;
                rounded = round(0);
            }
            // 1023.96 KB rounds to 1024.0 KB: the next unit writes it as 1.0 MB.
            if rounded >= self.step as f64
                && let Some(&next) = SizeUnit::ALL.get(unit as usize + 1)
                && next <= largest
            {
                unit = next;
                continue;
            }
            return (unit, rounded, usize::from(decimals));
        }
    }

    fn scale(&self, unit: SizeUnit) -> f64 {
        (self.step as f64).powi(unit as i32)
    }

    /// The English template for `unit`, which is also its catalog key.
    fn template(&self, unit: SizeUnit) -> &'static str {
        match (unit, self.iec) {
            (SizeUnit::Byte, _) => "{n} B",
            (SizeUnit::Kilo, false) => "{n} KB",
            (SizeUnit::Mega, false) => "{n} MB",
            (SizeUnit::Giga, false) => "{n} GB",
            (SizeUnit::Tera, false) => "{n} TB",
            (SizeUnit::Kilo, true) => "{n} KiB",
            (SizeUnit::Mega, true) => "{n} MiB",
            (SizeUnit::Giga, true) => "{n} GiB",
            (SizeUnit::Tera, true) => "{n} TiB",
        }
    }
}

impl I18n {
    /// `bytes` as a file size in `lang`, written the way `style` says (`1.5 MB`, `512 bytes`), the
    /// number as [`I18n::format`] writes it (`1,5 Mo`).
    pub fn file_size(&self, lang: Lang, bytes: u64, style: FileSize) -> String {
        let (unit, n, decimals) = style.parts(bytes);
        if unit == SizeUnit::Byte && !style.byte_symbol {
            return self.trn(lang, bytes, "{n} byte", "{n} bytes");
        }
        self.unit_text(lang, style.template(unit), &self.format().number(n, decimals))
    }

    /// `value` with `decimals` digits after the decimal separator in the unit template `unit`, an
    /// English catalog key with the number as `{n}` (`{n} px`, `{n} dB`), the number as
    /// [`I18n::format`] writes it. Percentages are [`crate::Format::percent`].
    pub fn measure(&self, lang: Lang, value: f64, decimals: usize, unit: &str) -> String {
        self.unit_text(lang, unit, &self.format().number(value, decimals))
    }

    fn unit_text(&self, lang: Lang, template: &str, n: &str) -> String {
        fmt(self.get(lang, CONTEXT, template).unwrap_or(template), &[("n", &ltr(n))])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Embedded, Format};

    static E: &Embedded = &[(
        "fr",
        "unit\t{n} KB\t{n} Ko\nunit\t{n} MB\t{n} Mo\nunit\t{n} B\t{n} o\nunit\t{n}%\t{n} %\n\tB\tG\n@plural\t{n} byte|{n} bytes\t{n} octet|{n} octets\n",
        "",
    )];

    fn en(bytes: u64, style: FileSize) -> String {
        let (i18n, _) = I18n::new(&[], &[]);
        i18n.set_format(Format::from_tag("en-US"));
        i18n.file_size(Lang::EN, bytes, style)
    }

    #[test]
    fn binary_decimal_and_iec_steps() {
        assert_eq!(en(512, FileSize::BINARY), "512 bytes");
        assert_eq!(en(1023, FileSize::BINARY), "1,023 bytes");
        assert_eq!(en(1, FileSize::BINARY), "1 byte");
        assert_eq!(en(1536, FileSize::BINARY), "1.5 KB");
        assert_eq!(en(1536, FileSize::IEC), "1.5 KiB");
        assert_eq!(en(1500, FileSize::DECIMAL), "1.5 KB");
        assert_eq!(en(3 << 30, FileSize::BINARY), "3.0 GB");
        assert_eq!(en(5 << 40, FileSize::IEC.decimals(0)), "5 TiB");
        assert_eq!(en(512, FileSize::BINARY.byte_symbol()), "512 B");
    }

    #[test]
    fn precision_and_unit_range() {
        // Rounding up to the next unit's step moves to that unit.
        assert_eq!(en(1_048_575, FileSize::BINARY), "1.0 MB");
        assert_eq!(en(999_999, FileSize::DECIMAL.decimals(0)), "1 MB");
        assert_eq!(en(1_610_612_736, FileSize::BINARY.unit_decimals(SizeUnit::Giga, 2)), "1.50 GB");
        // Lightcraft's denoise sizes: whole kilobytes, at least 1, no decimals from 10 MB.
        let style = FileSize::DECIMAL.smallest(SizeUnit::Kilo).largest(SizeUnit::Mega).unit_decimals(SizeUnit::Kilo, 0).whole_from(10);
        assert_eq!(en(1, style), "1 KB");
        assert_eq!(en(0, style), "0 KB");
        assert_eq!(en(640_400, style), "640 KB");
        assert_eq!(en(3_400_000, style), "3.4 MB");
        assert_eq!(en(12_600_000, style), "13 MB");
        assert_eq!(en(9_960_000, style), "10 MB", "whole once rounded to 10");
        assert_eq!(en(9_940_000, style), "9.9 MB");
        assert_eq!(en(5_000_000_000, style), "5,000 MB");
        // Only sizes below the smallest unit round up.
        assert_eq!(en(1025, FileSize::BINARY.smallest(SizeUnit::Kilo).unit_decimals(SizeUnit::Kilo, 0)), "1 KB");
        assert_eq!(en(100, FileSize::BINARY.smallest(SizeUnit::Kilo).unit_decimals(SizeUnit::Kilo, 0)), "1 KB");
    }

    #[test]
    fn numbers_are_left_to_right_isolates_in_rtl_units() {
        let (i18n, problems) = I18n::new(&[("ar", "unit\t{n} KB\t{n} ك.ب\n", "")], &["ar"]);
        assert!(problems.is_empty(), "{problems:?}");
        i18n.set_format(Format::from_tag("en-US"));
        let ar = i18n.from_code("ar").unwrap();
        assert_eq!(i18n.file_size(ar, 1536, FileSize::BINARY), "\u{2066}1.5\u{2069} ك.ب");
        assert_eq!(en(1536, FileSize::BINARY), "1.5 KB", "no marks in left-to-right text");
    }

    #[test]
    fn unit_names_are_catalog_rows() {
        let (i18n, problems) = I18n::new(E, &["fr"]);
        assert!(problems.is_empty(), "{problems:?}");
        let fr = i18n.from_code("fr").unwrap();
        i18n.set_format(Format::from_tag("fr-FR"));
        assert_eq!(i18n.file_size(fr, 1536, FileSize::BINARY), "1,5 Ko");
        assert_eq!(i18n.file_size(fr, 512, FileSize::BINARY), "512 octets");
        assert_eq!(i18n.file_size(fr, 512, FileSize::BINARY.byte_symbol()), "512 o", "not the plain row B (bold)");
        assert_eq!(i18n.file_size(fr, 3 << 30, FileSize::BINARY), "3,0 GB", "a missing row is English");
        assert_eq!(i18n.measure(fr, 50.0, 0, "{n}%"), "50 %");
        assert_eq!(i18n.measure(Lang::EN, 12.34, 1, "{n} px"), "12,3 px", "numbers follow the region, not the language");
    }

    #[test]
    fn locale_digits() {
        let (i18n, _) = I18n::new(&[], &[]);
        i18n.set_format(Format::from_tag("ar-EG"));
        assert_eq!(i18n.file_size(Lang::EN, 1536, FileSize::BINARY), "١٫٥ KB");
        i18n.set_format(Format::from_tag("hi"));
        assert_eq!(i18n.file_size(Lang::EN, 1_234_567, FileSize::DECIMAL.largest(SizeUnit::Byte).byte_symbol()), "12,34,567 B");
    }
}
