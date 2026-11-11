//! File sizes and other measures with a unit, in the interface language.
//!
//! Unit names are catalog rows in context `unit`, keyed by the English template with the
//! number as `{n}` (`{n} MB`, `{n} px`, `{n}%`), so a translation can change the symbol and its
//! spacing (`{n} Mo`, `{n} %`). They are looked up with [`I18n::get`], never the plain row: a
//! plain `B` row means bold. A byte count in words is the `@plural` row `{n} byte|{n} bytes`.

use crate::{Counted, Format, I18n, Lang, Word, Words, fmt, ltr};

/// The context unit rows are in.
const CONTEXT: &str = "unit";

/// A byte count in words.
const BYTES: Counted = Counted::new("", "{n} byte", "{n} bytes");

/// A file-size unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SizeUnit {
    /// Bytes.
    Byte,
    /// Kilobytes, or kibibytes in an IEC style.
    Kilo,
    /// Megabytes, or mebibytes in an IEC style.
    Mega,
    /// Gigabytes, or gibibytes in an IEC style.
    Giga,
    /// Terabytes, or tebibytes in an IEC style.
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

impl FileSize {
    /// `bytes` written by `words`: the unit rows of context `unit` and the count row of bytes, the
    /// number as `format` writes it. [`I18n::file_size`] is this with the app's catalogs; a UI scope
    /// that holds `Words` and a `Format` and no `I18n` calls it.
    pub fn text(&self, bytes: u64, format: &Format, words: &Words) -> String {
        self.render(
            bytes,
            format,
            |template| words.get(Word::in_context(CONTEXT, template)).into_owned(),
            |n| words.counted(BYTES, n, &ltr(&format.integer(n))),
        )
    }

    /// The text of `bytes`: `unit_row` gives a unit template's translation, `count` the byte count in words.
    fn render(&self, bytes: u64, format: &Format, unit_row: impl Fn(&'static str) -> String, count: impl Fn(u64) -> String) -> String {
        let (unit, n, decimals) = self.parts(bytes);
        if unit == SizeUnit::Byte && !self.byte_symbol {
            return count(bytes);
        }
        fmt(&unit_row(self.template(unit)), &[("n", &ltr(&format.number(n, decimals)))])
    }
}

impl I18n {
    /// `bytes` as a file size in `lang`, written the way `style` says (`1.5 MB`, `512 bytes`), the
    /// number as [`I18n::format`] writes it (`1,5 Mo`).
    pub fn file_size(&self, lang: Lang, bytes: u64, style: FileSize) -> String {
        style.render(
            bytes,
            &self.format(),
            |template| self.get(lang, CONTEXT, template).unwrap_or(template).to_owned(),
            |n| self.trn(lang, n, "{n} byte", "{n} bytes"),
        )
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
