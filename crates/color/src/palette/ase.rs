//! Adobe swatch exchange (`.ase`) files.

use super::reader::*;
use super::*;

// ---------------------------------------------------------------- .ase

pub(super) fn from_ase(c: &AseColor) -> Option<SwatchColor> {
    Some(match *c {
        AseColor::Rgb(v) => SwatchColor::Rgb(v),
        AseColor::Cmyk(v) => SwatchColor::Cmyk(v),
        // L is stored as a fraction (L*/100); a few writers store L* itself, which can only be told
        // apart above 1.
        AseColor::Lab([l, a, b]) => SwatchColor::Lab([if l > 1.0 { l } else { l * 100.0 }, a, b]),
        AseColor::Gray(g) => SwatchColor::Gray(1.0 - g),
        AseColor::Other => return None,
    })
}

pub(super) fn to_ase(c: &SwatchColor) -> Option<AseColor> {
    Some(match *c {
        SwatchColor::Rgb(v) => AseColor::Rgb(v),
        SwatchColor::Hsb(v) => AseColor::Rgb(hsb_to_rgb(v)),
        SwatchColor::Cmyk(v) => AseColor::Cmyk(v),
        SwatchColor::Lab([l, a, b]) => AseColor::Lab([l / 100.0, a, b]),
        SwatchColor::Gray(k) => AseColor::Gray(1.0 - k),
        SwatchColor::Book { .. } => return None,
    })
}

pub(super) fn hsb_to_rgb([h, s, b]: [f32; 3]) -> [f32; 3] {
    Srgba::from_hsb(Hsb { h: finite(h), s: unit(s), b: unit(b) }).to_rgb_array().map(unit)
}

pub(super) enum AseColor {
    Rgb([f32; 3]),
    Cmyk([f32; 4]),
    Lab([f32; 3]),
    Gray(f32),
    /// A colour model this crate does not read.
    Other,
}

pub(super) struct AseSwatch {
    pub(super) name: String,
    pub(super) color: AseColor,
    pub(super) kind: SwatchKind,
}

pub(super) enum AseItem {
    Swatch(AseSwatch),
    Group(String, Vec<AseSwatch>),
}

fn ase_name(r: &mut Reader<'_>) -> Result<String, String> {
    if r.remaining() == 0 {
        return Ok(String::new());
    }
    let n = usize::from(r.u16()?);
    r.units(n)
}

fn ase_color(body: &mut Reader<'_>) -> Result<AseSwatch, String> {
    let name = ase_name(body)?;
    let model: [u8; 4] = body.array()?;
    let color = match &model {
        b"RGB " => AseColor::Rgb([body.f32()?, body.f32()?, body.f32()?]),
        b"CMYK" => AseColor::Cmyk([body.f32()?, body.f32()?, body.f32()?, body.f32()?]),
        b"LAB " => AseColor::Lab([body.f32()?, body.f32()?, body.f32()?]),
        b"Gray" => AseColor::Gray(body.f32()?),
        _ => {
            // Everything but the trailing colour type belongs to the unknown model.
            let n = body.remaining().saturating_sub(2);
            body.bytes(n)?;
            AseColor::Other
        }
    };
    // Some writers omit the colour type; it defaults to process.
    let kind = if body.remaining() >= 2 {
        match body.u16()? {
            0 => SwatchKind::Global,
            1 => SwatchKind::Spot,
            _ => SwatchKind::Process,
        }
    } else {
        SwatchKind::Process
    };
    Ok(AseSwatch { name, color, kind })
}

pub(super) fn parse_ase(data: &[u8]) -> Result<Vec<AseItem>, String> {
    let mut r = Reader::new(data);
    let sig: [u8; 4] = r.array()?;
    if &sig != ASE_SIGNATURE {
        return Err("the ASEF signature is missing".into());
    }
    let major = r.u16()?;
    let _minor = r.u16()?;
    if major != 1 {
        return Err(format!("unsupported version {major}"));
    }
    let count = u64::from(r.u32()?);
    // Every block has at least a type and a length.
    r.check_count(count, 6)?;
    let mut items = Vec::new();
    let mut open: Option<(String, Vec<AseSwatch>)> = None;
    let (mut total, mut groups) = (0usize, 0usize);
    for _ in 0..count {
        let kind = r.u16()?;
        let len = usize::try_from(r.u32()?).map_err(|_| "a block is too long".to_string())?;
        let mut body = Reader::new(r.bytes(len)?);
        match kind {
            ASE_GROUP_START => {
                groups += 1;
                if groups > ASE_MAX_GROUPS {
                    return Err("too many groups".into());
                }
                if let Some((name, swatches)) = open.take() {
                    items.push(AseItem::Group(name, swatches));
                }
                open = Some((ase_name(&mut body)?, Vec::new()));
            }
            ASE_GROUP_END => {
                if let Some((name, swatches)) = open.take() {
                    items.push(AseItem::Group(name, swatches));
                }
            }
            ASE_COLOR => {
                total += 1;
                if total > ASE_MAX_SWATCHES {
                    return Err("too many swatches".into());
                }
                let s = ase_color(&mut body)?;
                match open.as_mut() {
                    Some((_, v)) => v.push(s),
                    None => items.push(AseItem::Swatch(s)),
                }
            }
            _ => {}
        }
    }
    if let Some((name, swatches)) = open.take() {
        items.push(AseItem::Group(name, swatches));
    }
    Ok(items)
}

pub(super) fn put_ase_name(out: &mut Vec<u8>, name: &str) {
    let units = utf16_units(name, MAX_NAME_UNITS);
    out.extend((units.len() as u16 + 1).to_be_bytes());
    for u in units {
        out.extend(u.to_be_bytes());
    }
    out.extend(0u16.to_be_bytes());
}

pub(super) fn put_ase_block(out: &mut Vec<u8>, kind: u16, body: &[u8]) {
    out.extend(kind.to_be_bytes());
    out.extend((body.len() as u32).to_be_bytes());
    out.extend(body);
}

pub(super) fn put_ase_color(out: &mut Vec<u8>, name: &str, color: &AseColor, kind: SwatchKind) {
    let mut b = Vec::with_capacity(64);
    put_ase_name(&mut b, name);
    let floats = |b: &mut Vec<u8>, v: &[f32]| {
        for x in v {
            b.extend(x.to_bits().to_be_bytes());
        }
    };
    match color {
        AseColor::Rgb(c) => {
            b.extend(b"RGB ");
            floats(&mut b, c);
        }
        AseColor::Cmyk(c) => {
            b.extend(b"CMYK");
            floats(&mut b, c);
        }
        AseColor::Lab(c) => {
            b.extend(b"LAB ");
            floats(&mut b, c);
        }
        AseColor::Gray(g) => {
            b.extend(b"Gray");
            floats(&mut b, &[*g]);
        }
        AseColor::Other => {}
    }
    let code: u16 = match kind {
        SwatchKind::Global => 0,
        SwatchKind::Spot => 1,
        SwatchKind::Process => 2,
    };
    b.extend(code.to_be_bytes());
    put_ase_block(out, ASE_COLOR, &b);
}
