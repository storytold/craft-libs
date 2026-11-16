//! Conditional formatting: a CondFmt record names the ranges ([MS-XLS] 2.4.56), and each CF
//! record after it is one rule (2.4.42): "cell value is" with an operator and one or two
//! formulas, or a formula that is true. Its look is a DXFN (2.5.81), where every attribute it
//! leaves unset ("ninch") is left as the cell has it.

use craft_sheet::{CellRef, Range, builtin_format};

use crate::Error;
use crate::model::{BorderLine, DifferentialFormat, DxfFill, Operator, Underline};
use crate::records::Cursor;
use crate::styles::{border_style, color, pattern};

/// Ranges one CondFmt record lists at most.
pub(crate) const MAX_CF_RANGES: usize = 4096;
/// Conditional formats kept per sheet; more are skipped with a warning.
pub(crate) const MAX_RULES: usize = 65_536;

/// A CondFmt record: the top left cell of its bounding range (what the rules' relative
/// references are offsets from) and its ranges.
pub(crate) fn condfmt(c: &mut Cursor<'_, '_>) -> Result<(CellRef, Vec<Range>), Error> {
    // Rule count, then flags and an id.
    c.skip(4)?;
    // refBound: rows (first, last), columns (first, last).
    let (r0, _, c0, _) = (c.u16()?, c.u16()?, c.u16()?, c.u16()?);
    let n = usize::from(c.u16()?).min(MAX_CF_RANGES).min(c.remaining() / 8);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let (r0, r1, c0, c1) = (c.u16()?, c.u16()?, c.u16()?, c.u16()?);
        out.push(Range::new(CellRef::new(u32::from(r0), u32::from(c0)), CellRef::new(u32::from(r1), u32::from(c1))));
    }
    Ok((CellRef::new(u32::from(r0), u32::from(c0)), out))
}

/// One CF record, before its formulas are decoded.
pub(crate) struct Cf {
    /// 1: the cell value compared; 2: a formula.
    pub kind: u8,
    pub operator: Option<Operator>,
    pub format: DifferentialFormat,
    pub rgce1: Vec<u8>,
    pub rgce2: Vec<u8>,
}

/// Reads a CF record: its kind, operator, look and the two formulas' tokens. `num_fmt` gives the
/// code of a number format by id.
pub(crate) fn cf(c: &mut Cursor<'_, '_>, palette: &[u32; 64], num_fmt: impl Fn(u16) -> Option<String>) -> Result<Cf, Error> {
    let kind = c.u8()?;
    let operator = match c.u8()? {
        1 => Some(Operator::Between),
        2 => Some(Operator::NotBetween),
        3 => Some(Operator::Equal),
        4 => Some(Operator::NotEqual),
        5 => Some(Operator::Greater),
        6 => Some(Operator::Less),
        7 => Some(Operator::GreaterOrEqual),
        8 => Some(Operator::LessOrEqual),
        _ => None,
    };
    let (cce1, cce2) = (usize::from(c.u16()?), usize::from(c.u16()?));
    let format = dxfn(c, palette, num_fmt)?;
    let rest = c.rest();
    let rgce1 = rest.get(..cce1).unwrap_or(&[]).to_vec();
    let rgce2 = rest.get(cce1..cce1.saturating_add(cce2)).unwrap_or(&[]).to_vec();
    Ok(Cf { kind, operator, format, rgce1, rgce2 })
}

fn bit(flags: u32, n: u32) -> bool {
    flags & (1 << n) != 0
}

/// A DXFN: which blocks follow (number format, font, alignment, border, pattern, protection)
/// and, per attribute, whether it is left unset.
fn dxfn(c: &mut Cursor<'_, '_>, palette: &[u32; 64], num_fmt: impl Fn(u16) -> Option<String>) -> Result<DifferentialFormat, Error> {
    let flags = c.u32()?;
    let flags2 = c.u16()?;
    let mut d = DifferentialFormat::default();
    if bit(flags, 25) {
        if flags2 & 1 != 0 {
            // DXFNumUsr: its size, then the format code.
            c.u16()?;
            d.number_format = Some(c.string()?);
        } else {
            c.u8()?;
            let id = u16::from(c.u8()?);
            d.number_format = num_fmt(id).or_else(|| builtin_format(u32::from(id)).map(str::to_string));
        }
    }
    if bit(flags, 26) {
        // DXFFntD, 118 bytes: name (64), height, options (italic 0x02, strikeout 0x80), weight,
        // script, underline, colour, then which of those are unset. As in a SpreadsheetML
        // differential font, the face and size stay the cell's.
        let mut b = [0u8; 118];
        for slot in b.iter_mut() {
            *slot = c.u8()?;
        }
        let u32_at = |at: usize| b.get(at..at + 4).and_then(|x| <[u8; 4]>::try_from(x).ok()).map_or(u32::MAX, u32::from_le_bytes);
        let u16_at = |at: usize| b.get(at..at + 2).and_then(|x| <[u8; 2]>::try_from(x).ok()).map_or(0, u16::from_le_bytes);
        let (options, weight, underline, icv) = (u32_at(68), u16_at(72), b.get(76).copied().unwrap_or(0), u32_at(80));
        let (style_unset, underline_unset, weight_unset) = (u32_at(88), u32_at(96) != 0, u32_at(100) != 0);
        if !weight_unset {
            d.bold = Some(weight >= 600);
        }
        if style_unset & 0x02 == 0 {
            d.italic = Some(options & 0x02 != 0);
        }
        if style_unset & 0x80 == 0 {
            d.strike = Some(options & 0x80 != 0);
        }
        if !underline_unset {
            d.underline = Some(match underline {
                1 => Underline::Single,
                2 => Underline::Double,
                0x21 => Underline::SingleAccounting,
                0x22 => Underline::DoubleAccounting,
                _ => Underline::None,
            });
        }
        // 0xFFFFFFFF: the colour is unset.
        if let Ok(icv) = u16::try_from(icv) {
            d.font_color = Some(color(icv, palette));
        }
    }
    if bit(flags, 27) {
        c.skip(8)?;
    }
    if bit(flags, 28) {
        let (a, b) = (c.u32()?, c.u32()?);
        let line = |unset: bool, dg: u32, icv: u32| {
            (!unset).then(|| BorderLine { style: border_style(dg & 0x0F), color: color((icv & 0x7F) as u16, palette) })
        };
        d.left = line(bit(flags, 10), a, a >> 16);
        d.right = line(bit(flags, 11), a >> 4, a >> 23);
        d.top = line(bit(flags, 12), a >> 8, b);
        d.bottom = line(bit(flags, 13), a >> 12, b >> 7);
    }
    if bit(flags, 29) {
        let (p1, p2) = (c.u16()?, c.u16()?);
        d.fill = Some(DxfFill {
            pattern: (!bit(flags, 16)).then(|| pattern((p1 >> 10) & 0x3F)),
            fg: (!bit(flags, 17)).then(|| color(p2 & 0x7F, palette)),
            bg: (!bit(flags, 18)).then(|| color((p2 >> 7) & 0x7F, palette)),
        });
    }
    if bit(flags, 30) {
        c.skip(2)?;
    }
    Ok(d)
}
