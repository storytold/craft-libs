//! Photoshop `.aco` swatches and `.act` colour tables.

use super::reader::*;
use super::*;

// ---------------------------------------------------------------- .aco

/// A colour record as stored, at 16 bits (CMYK as ink amounts).
#[derive(Clone, Copy)]
pub(super) enum AcoColor {
    Rgb([u16; 3]),
    Hsb([u16; 3]),
    Cmyk([u16; 4]),
    Lab { l: u16, a: i16, b: i16 },
    Gray(u16),
    Other { space: u16, values: [u16; 4] },
}

const ACO_RGB: u16 = 0;
const ACO_HSB: u16 = 1;
const ACO_CMYK: u16 = 2;
const ACO_LAB: u16 = 7;
const ACO_GRAY: u16 = 8;

impl AcoColor {
    fn from_record(space: u16, v: [u16; 4]) -> AcoColor {
        match space {
            ACO_RGB => AcoColor::Rgb([v[0], v[1], v[2]]),
            ACO_HSB => AcoColor::Hsb([v[0], v[1], v[2]]),
            ACO_CMYK => AcoColor::Cmyk(v.map(|x| u16::MAX - x)),
            ACO_LAB => AcoColor::Lab { l: v[0], a: v[1] as i16, b: v[2] as i16 },
            ACO_GRAY => AcoColor::Gray(v[0]),
            _ => AcoColor::Other { space, values: v },
        }
    }

    fn to_record(self) -> (u16, [u16; 4]) {
        match self {
            AcoColor::Rgb([r, g, b]) => (ACO_RGB, [r, g, b, 0]),
            AcoColor::Hsb([h, s, b]) => (ACO_HSB, [h, s, b, 0]),
            AcoColor::Cmyk(c) => (ACO_CMYK, c.map(|x| u16::MAX - x)),
            AcoColor::Lab { l, a, b } => (ACO_LAB, [l, a as u16, b as u16, 0]),
            AcoColor::Gray(g) => (ACO_GRAY, [g, 0, 0, 0]),
            AcoColor::Other { space, values } => (space, values),
        }
    }
}

fn q16(v: f32) -> u16 {
    (unit(v) * 65535.0).round() as u16
}

pub(super) fn from_aco(c: AcoColor) -> SwatchColor {
    let f = |v: u16| f32::from(v) / 65535.0;
    match c {
        AcoColor::Rgb(v) => SwatchColor::Rgb(v.map(f)),
        AcoColor::Hsb([h, sa, b]) => SwatchColor::Hsb([f(h) * 360.0, f(sa), f(b)]),
        AcoColor::Cmyk(v) => SwatchColor::Cmyk(v.map(f)),
        AcoColor::Lab { l, a, b } => SwatchColor::Lab([f32::from(l.min(10000)) / 100.0, f32::from(a) / 100.0, f32::from(b) / 100.0]),
        AcoColor::Gray(g) => SwatchColor::Gray(1.0 - f32::from(g.min(10000)) / 10000.0),
        AcoColor::Other { space, values } => SwatchColor::Book { space, values },
    }
}

pub(super) fn to_aco(c: &SwatchColor) -> AcoColor {
    match *c {
        SwatchColor::Rgb(v) => AcoColor::Rgb(v.map(q16)),
        SwatchColor::Hsb([h, sa, b]) => {
            let h = finite(h).rem_euclid(360.0);
            AcoColor::Hsb([((h / 360.0) * 65535.0).round().min(65535.0) as u16, q16(sa), q16(b)])
        }
        SwatchColor::Cmyk(v) => AcoColor::Cmyk(v.map(q16)),
        SwatchColor::Lab([l, a, b]) => {
            let ab = |v: f32| (finite(v) * 100.0).round().clamp(-12800.0, 12700.0) as i16;
            AcoColor::Lab { l: (finite(l) * 100.0).round().clamp(0.0, 10000.0) as u16, a: ab(a), b: ab(b) }
        }
        SwatchColor::Gray(k) => AcoColor::Gray(((1.0 - unit(k)) * 10000.0).round() as u16),
        SwatchColor::Book { space, values } => AcoColor::Other { space, values },
    }
}

fn aco_record(r: &mut Reader<'_>) -> Result<AcoColor, String> {
    let space = r.u16()?;
    let v = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
    Ok(AcoColor::from_record(space, v))
}

fn aco_section(r: &mut Reader<'_>, named: bool) -> Result<Vec<(String, AcoColor)>, String> {
    let count = usize::from(r.u16()?);
    r.check_count(count as u64, if named { 14 } else { 10 })?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let color = aco_record(r)?;
        let name = if named {
            let n = r.u32()?;
            r.units(usize::try_from(n).map_err(|_| "a name is too long".to_string())?)?
        } else {
            String::new()
        };
        out.push((name, color));
    }
    Ok(out)
}

/// The swatches of an `.aco` file (version 2 names when the file has them) and how many bytes its
/// sections span.
pub(super) fn parse_aco(data: &[u8]) -> Result<(Vec<(String, AcoColor)>, usize), String> {
    let mut r = Reader::new(data);
    match r.u16()? {
        1 => {
            let v1 = aco_section(&mut r, false)?;
            // Files from Photoshop 6 on follow with a version 2 section; other trailing bytes are
            // padding.
            if r.remaining() >= 4 && data[r.pos..].get(..2) == Some(&[0, 2]) {
                r.u16()?;
                let v2 = aco_section(&mut r, true)?;
                return Ok((v2, r.pos));
            }
            Ok((v1, r.pos))
        }
        2 => {
            let v2 = aco_section(&mut r, true)?;
            Ok((v2, r.pos))
        }
        v => Err(format!("unsupported version {v}")),
    }
}

pub(super) fn write_aco(entries: &[(String, AcoColor)]) -> Vec<u8> {
    let count = entries.len() as u16;
    let mut out = Vec::with_capacity(8 + entries.len() * 40);
    for version in [1u16, 2] {
        out.extend(version.to_be_bytes());
        out.extend(count.to_be_bytes());
        for (name, color) in entries {
            let (space, v) = color.to_record();
            out.extend(space.to_be_bytes());
            for x in v {
                out.extend(x.to_be_bytes());
            }
            if version == 2 {
                let units = utf16_units(name, MAX_NAME_UNITS);
                // The length counts the terminating null.
                out.extend((units.len() as u32 + 1).to_be_bytes());
                for u in units {
                    out.extend(u.to_be_bytes());
                }
                out.extend(0u16.to_be_bytes());
            }
        }
    }
    out
}

// ---------------------------------------------------------------- .act

pub(super) fn act_used(data: &[u8]) -> Result<Vec<[u8; 3]>, String> {
    if data.len() != ACT_PALETTE_LEN && data.len() != ACT_PALETTE_LEN + ACT_TRAILER_LEN {
        return Err("an .act file is 768 bytes, or 772 with the count trailer".into());
    }
    let mut colors: Vec<[u8; 3]> = data[..ACT_PALETTE_LEN].as_chunks::<3>().0.to_vec();
    if data.len() > ACT_PALETTE_LEN {
        let count = usize::from(u16::from_be_bytes([data[ACT_PALETTE_LEN], data[ACT_PALETTE_LEN + 1]]));
        colors.truncate(count.min(ACT_COLORS));
    }
    Ok(colors)
}
