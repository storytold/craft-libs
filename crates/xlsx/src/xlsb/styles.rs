//! The styles part of a binary workbook (`styles.bin`, [MS-XLSB] 2.1.7.50), read into the same
//! [`Styles`] the XML part gives: what Excel writes in `styles.xml` for the same workbook. Table
//! styles, recent colours and future records are not read.

use craft_sheet::{BorderStyle, HAlign, Pattern, Script, Underline, VAlign};
use craft_xml::El;

use super::{Budget, Bytes, Records, invalid};
use crate::styles::{
    Alignment, Border, BorderEdge, CellStyle, Color, ColorSource, Dxf, Fill, Font, FontScheme, GradientFill, NumFmt, PatternFill, Protection, Styles,
    Xf,
};
use crate::{Error, Limits, Warnings};

/// Characters of a font name or number format kept, as the XML side keeps of a font name.
const MAX_NAME_CHARS: usize = 255;
/// Longest string read from the part (Excel's own limits are 31 and 255 characters).
const MAX_STRING_UNITS: u32 = 32_767;
/// Most stops in one gradient fill (Excel writes a handful).
const MAX_STOPS: u32 = 1024;
/// Most entries in one table of the part (Excel allows about 64,000 cell formats). Each entry is
/// an allocation, and the record allowance alone would let a small part hold millions.
const MAX_ENTRIES: usize = 65_536;

/// `BrtColor` and `XFPropColor`: the same eight bytes. `None` for "not set" (`XFPropColor` type 4).
fn color(b: &mut Bytes<'_>) -> Result<Option<Color>, Error> {
    let kind = b.u8()? >> 1;
    let index = b.u8()?;
    let tint = i16::from_le_bytes(<[u8; 2]>::try_from(b.take(2)?).map_err(|_| invalid("truncated colour"))?);
    let [r, g, bl, a] = <[u8; 4]>::try_from(b.take(4)?).map_err(|_| invalid("truncated colour"))?;
    let source = match kind {
        0 => ColorSource::Auto,
        1 => ColorSource::Indexed(u32::from(index)),
        2 => ColorSource::Rgb(u32::from_be_bytes([a, r, g, bl])),
        3 => ColorSource::Theme(u32::from(index)),
        _ => return Ok(None),
    };
    Ok(Some(Color { source, tint: (f64::from(tint) / 32767.0).clamp(-1.0, 1.0) }))
}

fn pattern(fls: u32) -> Result<Pattern, Error> {
    const PATTERNS: [Pattern; 19] = [
        Pattern::None,
        Pattern::Solid,
        Pattern::Gray50,
        Pattern::Gray75,
        Pattern::Gray25,
        Pattern::DarkHorizontal,
        Pattern::DarkVertical,
        Pattern::DarkDown,
        Pattern::DarkUp,
        Pattern::DarkGrid,
        Pattern::DarkTrellis,
        Pattern::LightHorizontal,
        Pattern::LightVertical,
        Pattern::LightDown,
        Pattern::LightUp,
        Pattern::LightGrid,
        Pattern::LightTrellis,
        Pattern::Gray125,
        Pattern::Gray0625,
    ];
    PATTERNS.get(fls as usize).copied().ok_or_else(|| invalid("unknown fill pattern"))
}

fn border_style(dg: u16) -> Result<BorderStyle, Error> {
    const STYLES: [BorderStyle; 14] = [
        BorderStyle::None,
        BorderStyle::Thin,
        BorderStyle::Medium,
        BorderStyle::Dashed,
        BorderStyle::Dotted,
        BorderStyle::Thick,
        BorderStyle::Double,
        BorderStyle::Hair,
        BorderStyle::MediumDashed,
        BorderStyle::DashDot,
        BorderStyle::MediumDashDot,
        BorderStyle::DashDotDot,
        BorderStyle::MediumDashDotDot,
        BorderStyle::SlantDashDot,
    ];
    STYLES.get(usize::from(dg)).copied().ok_or_else(|| invalid("unknown border style"))
}

fn halign(alc: u32) -> Option<HAlign> {
    [HAlign::General, HAlign::Left, HAlign::Center, HAlign::Right, HAlign::Fill, HAlign::Justify, HAlign::CenterAcross, HAlign::Distributed]
        .get(alc as usize)
        .copied()
}

fn valign(alcv: u32) -> Option<VAlign> {
    [VAlign::Top, VAlign::Center, VAlign::Bottom, VAlign::Justify, VAlign::Distributed].get(alcv as usize).copied()
}

fn underline(uls: u32) -> Underline {
    match uls {
        0 => Underline::None,
        2 => Underline::Double,
        0x21 => Underline::SingleAccounting,
        0x22 => Underline::DoubleAccounting,
        _ => Underline::Single,
    }
}

fn script(sss: u32) -> Script {
    match sss {
        1 => Script::Superscript,
        2 => Script::Subscript,
        _ => Script::Baseline,
    }
}

fn scheme(s: u32) -> Option<FontScheme> {
    match s {
        0 => Some(FontScheme::None),
        1 => Some(FontScheme::Major),
        2 => Some(FontScheme::Minor),
        _ => None,
    }
}

fn name(text: String) -> String {
    text.chars().take(MAX_NAME_CHARS).collect()
}

/// `BrtFont`: only what Excel's `.xlsx` states for the same font (no `<b val="0"/>`, no scheme
/// `none`, no family or character set 0).
fn font(b: &mut Bytes<'_>) -> Result<Font, Error> {
    let height = b.u16()?;
    let flags = b.u16()?;
    let weight = b.u16()?;
    let sss = b.u16()?;
    let uls = b.u8()?;
    let family = b.u8()?;
    let charset = b.u8()?;
    b.u8()?;
    let color = color(b)?;
    let font_scheme = b.u8()?;
    let font_name = name(b.text(MAX_STRING_UNITS)?);
    let on = |bit: u16| (flags & bit != 0).then_some(true);
    Ok(Font {
        name: Some(font_name),
        size: Some(f64::from(height) / 20.0),
        bold: (weight >= 0x2BC).then_some(true),
        italic: on(0x02),
        strike: on(0x08),
        outline: on(0x10),
        shadow: on(0x20),
        condense: on(0x40),
        extend: on(0x80),
        underline: (uls != 0).then(|| underline(u32::from(uls))),
        vert_align: (sss != 0).then(|| script(u32::from(sss))),
        color,
        family: (family != 0).then_some(u32::from(family)),
        charset: (charset != 0).then_some(u32::from(charset)),
        scheme: scheme(u32::from(font_scheme)).filter(|s| *s != FontScheme::None),
    })
}

/// A gradient's angle and inner rectangle, as `BrtFill` and `XFPropGradient` hold them.
fn gradient(b: &mut Bytes<'_>, path: bool) -> Result<GradientFill, Error> {
    Ok(GradientFill { path, degree: b.f64()?, left: b.f64()?, right: b.f64()?, top: b.f64()?, bottom: b.f64()?, stops: vec![] })
}

/// `BrtFill`. Excel's `.xlsx` leaves out the default colours: system foreground (indexed 64) as
/// the pattern colour and system background (indexed 65) as the background.
fn fill(b: &mut Bytes<'_>) -> Result<Fill, Error> {
    let fls = b.u32()?;
    let fg = color(b)?;
    let bg = color(b)?;
    let kind = b.u32()?;
    let mut g = gradient(b, kind == 1)?;
    let count = b.u32()?;
    if fls == 0x28 {
        if count > MAX_STOPS {
            return Err(invalid("too many gradient stops"));
        }
        for _ in 0..count {
            let c = color(b)?.unwrap_or_default();
            g.stops.push((b.f64()?, c));
        }
        return Ok(Fill::Gradient(g));
    }
    let system = |c: Color, i: u32| c.source == ColorSource::Indexed(i) && c.tint == 0.0;
    Ok(Fill::Pattern(PatternFill { pattern: Some(pattern(fls)?), fg: fg.filter(|c| !system(*c, 64)), bg: bg.filter(|c| !system(*c, 65)) }))
}

/// `BrtBorder`: the five edges a cell border has, each stated (`<left/>` when it has no line).
fn border(b: &mut Bytes<'_>) -> Result<Border, Error> {
    let flags = b.u8()?;
    let mut edge = || -> Result<BorderEdge, Error> {
        let style = border_style(u16::from(b.u8()?))?;
        b.u8()?;
        let c = color(b)?;
        Ok(BorderEdge { style, color: c.filter(|_| style != BorderStyle::None) })
    };
    let (top, bottom, left, right, diagonal) = (edge()?, edge()?, edge()?, edge()?, edge()?);
    Ok(Border {
        left: Some(left),
        right: Some(right),
        top: Some(top),
        bottom: Some(bottom),
        diagonal: Some(diagonal),
        vertical: None,
        horizontal: None,
        diagonal_up: flags & 2 != 0,
        diagonal_down: flags & 1 != 0,
        outline: true,
    })
}

/// `BrtXF`. A cell style format (`style`) marks what it leaves out (`apply*="0"`); a cell format
/// marks what it sets apart from its style (`apply*="1"`).
fn xf(b: &mut Bytes<'_>, style: bool) -> Result<Xf, Error> {
    const APPLY: [(u8, &str); 6] = [
        (0x01, "applyNumberFormat"),
        (0x02, "applyFont"),
        (0x10, "applyFill"),
        (0x08, "applyBorder"),
        (0x04, "applyAlignment"),
        (0x20, "applyProtection"),
    ];
    let parent = b.u16()?;
    let num_fmt_id = u32::from(b.u16()?);
    let font_id = u32::from(b.u16()?);
    let fill_id = u32::from(b.u16()?);
    let border_id = u32::from(b.u16()?);
    let rotation = u32::from(b.u8()?);
    let indent = u32::from(b.u8()?);
    let flags = u32::from(b.u16()?);
    let apply = b.u8()?;
    b.u8()?;
    let alignment = Alignment {
        horizontal: halign(flags & 7).filter(|h| *h != HAlign::General),
        vertical: valign((flags >> 3) & 7).filter(|v| *v != VAlign::Bottom),
        text_rotation: (rotation != 0).then_some(rotation),
        wrap_text: flags & 0x40 != 0,
        justify_last_line: flags & 0x80 != 0,
        shrink_to_fit: flags & 0x100 != 0,
        indent: (indent != 0).then_some(indent),
        reading_order: Some((flags >> 10) & 3).filter(|r| *r != 0),
    };
    let protection = Protection { locked: (flags & 0x1000 == 0).then_some(false), hidden: (flags & 0x2000 != 0).then_some(true) };
    let mut el = El::new("xf");
    for (bit, attr) in APPLY {
        if apply & bit != 0 {
            el = el.with_attr(attr, if style { "0" } else { "1" });
        }
    }
    Ok(Xf {
        num_fmt_id,
        font_id,
        fill_id,
        border_id,
        xf_id: (!style).then_some(u32::from(parent)),
        alignment: (alignment != Alignment::default()).then_some(alignment),
        protection: (protection != Protection::default()).then_some(protection),
        quote_prefix: flags & 0x8000 != 0,
        el,
    })
}

/// `BrtStyle`.
fn cell_style(b: &mut Bytes<'_>) -> Result<CellStyle, Error> {
    let xf_id = b.u32()?;
    let flags = b.u16()?;
    let builtin = b.u8()?;
    let level = b.u8()?;
    let style_name = b.string(MAX_STRING_UNITS, true)?.map(name).unwrap_or_default();
    let is_builtin = flags & 1 != 0;
    Ok(CellStyle {
        name: style_name,
        xf_id,
        builtin_id: is_builtin.then_some(u32::from(builtin)),
        level: (is_builtin && matches!(builtin, 1 | 2)).then_some(u32::from(level)),
        hidden: flags & 2 != 0,
        custom_builtin: flags & 4 != 0,
    })
}

/// An unsigned integer of the size a property has (1, 2 or 4 bytes).
fn uint(data: &[u8]) -> Result<u32, Error> {
    match *data {
        [a] => Ok(u32::from(a)),
        [a, b] => Ok(u32::from(u16::from_le_bytes([a, b]))),
        [a, b, c, d] => Ok(u32::from_le_bytes([a, b, c, d])),
        _ => Err(invalid("unexpected formatting property size")),
    }
}

/// A string with a 2-byte length (`LPWideString`, and the number format of `XFProp` 0x26).
fn short_string(b: &mut Bytes<'_>) -> Result<String, Error> {
    let count = usize::from(b.u16()?);
    let mut text = String::new();
    for ch in char::decode_utf16(b.take(count * 2)?.as_chunks::<2>().0.iter().copied().map(u16::from_le_bytes)) {
        text.push(ch.map_err(|_| invalid("invalid UTF-16 string"))?);
    }
    Ok(name(text))
}

fn dxf_font(d: &mut Dxf) -> &mut Font {
    d.font.get_or_insert_with(Font::default)
}

/// `BrtDXF`: its `XFProps`, each stating one property.
fn dxf(b: &mut Bytes<'_>) -> Result<Dxf, Error> {
    b.u16()?;
    b.u16()?;
    let count = b.u16()?;
    let mut d = Dxf::default();
    let mut gradient_fill: Option<GradientFill> = None;
    let mut pattern_fill: Option<PatternFill> = None;
    for _ in 0..count {
        let kind = b.u16()?;
        let size = usize::from(b.u16()?).checked_sub(4).ok_or_else(|| invalid("formatting property too short"))?;
        let data = b.take(size)?;
        let mut p = Bytes::new(data);
        match kind {
            0x00 => pattern_fill.get_or_insert_with(PatternFill::default).pattern = Some(pattern(uint(data)?)?),
            0x01 => pattern_fill.get_or_insert_with(PatternFill::default).fg = color(&mut p)?,
            0x02 => pattern_fill.get_or_insert_with(PatternFill::default).bg = color(&mut p)?,
            0x03 => {
                let path = p.u32()? == 1;
                gradient_fill = Some(gradient(&mut p, path)?);
            }
            0x04 => {
                let g = gradient_fill.as_mut().ok_or_else(|| invalid("gradient stop without a gradient"))?;
                if g.stops.len() >= MAX_STOPS as usize {
                    return Err(invalid("too many gradient stops"));
                }
                p.u16()?;
                let position = p.f64()?;
                g.stops.push((position, color(&mut p)?.unwrap_or_default()));
            }
            0x05 => dxf_font(&mut d).color = color(&mut p)?,
            0x06..=0x0C => {
                let c = color(&mut p)?;
                let edge = Some(BorderEdge { style: border_style(p.u16()?)?, color: c });
                let border = d.border.get_or_insert_with(|| Border { outline: true, ..Border::default() });
                match kind {
                    0x06 => border.top = edge,
                    0x07 => border.bottom = edge,
                    0x08 => border.left = edge,
                    0x09 => border.right = edge,
                    0x0A => border.diagonal = edge,
                    0x0B => border.vertical = edge,
                    _ => border.horizontal = edge,
                }
            }
            0x0D => d.border.get_or_insert_with(|| Border { outline: true, ..Border::default() }).diagonal_up = uint(data)? != 0,
            0x0E => d.border.get_or_insert_with(|| Border { outline: true, ..Border::default() }).diagonal_down = uint(data)? != 0,
            0x0F..=0x16 => {
                let v = uint(data)?;
                let a = d.alignment.get_or_insert_with(Alignment::default);
                match kind {
                    0x0F => a.horizontal = halign(v),
                    0x10 => a.vertical = valign(v),
                    0x11 => a.text_rotation = Some(v),
                    0x12 => a.indent = Some(v),
                    0x13 => a.reading_order = Some(v),
                    0x14 => a.wrap_text = v != 0,
                    0x15 => a.justify_last_line = v != 0,
                    _ => a.shrink_to_fit = v != 0,
                }
            }
            0x18 => dxf_font(&mut d).name = Some(short_string(&mut p)?),
            0x19 => dxf_font(&mut d).bold = Some(uint(data)? >= 0x2BC),
            0x1A => dxf_font(&mut d).underline = Some(underline(uint(data)?)),
            0x1B => dxf_font(&mut d).vert_align = Some(script(uint(data)?)),
            0x1C => dxf_font(&mut d).italic = Some(uint(data)? != 0),
            0x1D => dxf_font(&mut d).strike = Some(uint(data)? != 0),
            0x1E => dxf_font(&mut d).outline = Some(uint(data)? != 0),
            0x1F => dxf_font(&mut d).shadow = Some(uint(data)? != 0),
            0x20 => dxf_font(&mut d).condense = Some(uint(data)? != 0),
            0x21 => dxf_font(&mut d).extend = Some(uint(data)? != 0),
            0x22 => dxf_font(&mut d).charset = Some(uint(data)?),
            0x23 => dxf_font(&mut d).family = Some(uint(data)?),
            0x24 => dxf_font(&mut d).size = Some(f64::from(uint(data)?) / 20.0),
            0x25 => dxf_font(&mut d).scheme = scheme(uint(data)?),
            0x26 => d.num_fmt.get_or_insert_with(NumFmt::default).code = Some(short_string(&mut p)?),
            0x29 => d.num_fmt.get_or_insert_with(NumFmt::default).id = Some(uint(data)?),
            0x2B => d.protection.get_or_insert_with(Protection::default).locked = Some(uint(data)? != 0),
            0x2C => d.protection.get_or_insert_with(Protection::default).hidden = Some(uint(data)? != 0),
            // Merged cells (0x17) and relative indents (0x2A) have no place in `<dxf>`.
            _ => {}
        }
    }
    d.fill = gradient_fill.map(Fill::Gradient).or(pattern_fill.map(Fill::Pattern));
    Ok(d)
}

/// The styles part: `BrtBeginStyleSheet` to `BrtEndStyleSheet`.
pub(crate) fn styles(bytes: &[u8], limits: &Limits, budget: &Budget, warnings: &mut Warnings) -> Result<Styles, Error> {
    let mut records = Records::new(bytes, limits, budget);
    records.first(278, warnings)?;
    let mut s = Styles::default();
    // Inside BrtBeginCellStyleXFs (true) or BrtBeginCellXFs (false).
    let mut xfs: Option<bool> = None;
    let mut palette = false;
    while let Some((id, payload)) = records.next(warnings)? {
        let mut b = Bytes::new(payload);
        let entries = match id {
            44 => s.num_fmts.len(),
            43 => s.fonts.len(),
            45 => s.fills.len(),
            46 => s.borders.len(),
            47 => s.cell_style_xfs.len().max(s.cell_xfs.len()),
            48 => s.cell_styles.len(),
            507 => s.dxfs.len(),
            475 => s.indexed_colors.len(),
            _ => 0,
        };
        if entries >= MAX_ENTRIES {
            return Err(invalid(format!("more than {MAX_ENTRIES} entries in a styles table")));
        }
        match id {
            279 => {
                records.end(payload, warnings)?;
                return Ok(s);
            }
            44 => {
                let id = u32::from(b.u16()?);
                let code = name(b.text(MAX_STRING_UNITS)?);
                budget.text(code.len(), limits)?;
                s.num_fmts.push((id, code));
            }
            43 => {
                let f = font(&mut b)?;
                budget.text(f.name.as_ref().map_or(0, String::len), limits)?;
                s.fonts.push(f);
            }
            45 => s.fills.push(fill(&mut b)?),
            46 => s.borders.push(border(&mut b)?),
            626 => xfs = Some(true),
            617 => xfs = Some(false),
            627 | 618 => xfs = None,
            47 => match xfs {
                Some(true) => s.cell_style_xfs.push(xf(&mut b, true)?),
                Some(false) => s.cell_xfs.push(xf(&mut b, false)?),
                None => return Err(invalid("cell format outside its list")),
            },
            48 => {
                let c = cell_style(&mut b)?;
                budget.text(c.name.len(), limits)?;
                s.cell_styles.push(c);
            }
            507 => s.dxfs.push(dxf(&mut b)?),
            // BrtBeginIndexedColors, BrtEndIndexedColors.
            565 => palette = true,
            566 => palette = false,
            475 if palette => {
                let [r, g, bl, _] = <[u8; 4]>::try_from(b.take(4)?).map_err(|_| invalid("truncated colour"))?;
                s.indexed_colors.push(u32::from_be_bytes([0, r, g, bl]));
            }
            // BrtBeginTableStyles: the count of custom table styles, which are not read.
            508 => {
                if b.u32()? > 0 {
                    warnings.warn("custom table styles in a binary workbook were not read");
                }
                continue;
            }
            _ => continue,
        }
        // The records read above are read whole; the rest are skipped.
        if matches!(id, 44 | 43 | 45 | 46 | 47 | 48 | 507 | 475) {
            b.finish()?;
        }
    }
    Err(invalid("missing styles end record"))
}
