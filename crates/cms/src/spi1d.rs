//! Sony Imageworks SPI1D tables, preserving EffectCraft's permissive header syntax.
//!
//! Unknown header lines and trailing text after the closing brace are ignored. Table
//! samples are not clamped or sanitized: callers own their numerical rendering policy.
//! The declared length is validated before storing samples; surplus samples are refused.

/// Largest accepted table (the existing EffectCraft interpolation limit).
pub const MAX_ENTRIES: usize = 1 << 24;

/// A one-dimensional RGB lookup table and its common input domain.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub values: Vec<[f32; 3]>,
    pub domain: [f32; 2],
}

/// Parse a SPI1D table with one or three components, expanding monochrome samples to RGB.
/// No allocation is based only on the claimed length. At most `MAX_ENTRIES` RGB entries
/// are retained. Missing final braces remain accepted, as in the source reader.
pub fn parse(text: &str) -> Option<Table> {
    let (mut from, mut len, mut comps) = ([0.0f32, 1.0], 0usize, 1usize);
    let mut values = Vec::new();
    let mut body = false;
    let mut sample = [0.0; 3];
    let mut channel = 0;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if body {
            if line.starts_with('}') {
                break;
            }
            for t in line.split_whitespace() {
                if values.len() == len {
                    return None;
                }
                sample[channel] = t.parse().ok()?;
                channel += 1;
                if channel == comps {
                    values.push(if comps == 1 { [sample[0]; 3] } else { sample });
                    channel = 0;
                }
            }
            continue;
        }
        let mut t = line.split_whitespace();
        match t.next()? {
            "{" => {
                if !(2..=MAX_ENTRIES).contains(&len) || !matches!(comps, 1 | 3) || from[1] <= from[0] || from.iter().any(|v| !v.is_finite()) {
                    return None;
                }
                body = true;
            }
            "From" => from = [t.next()?.parse().ok()?, t.next()?.parse().ok()?],
            "Length" => len = t.next()?.parse().ok()?,
            "Components" => comps = t.next()?.parse().ok()?,
            _ => {}
        }
    }
    (body && values.len() == len && channel == 0).then_some(Table { values, domain: from })
}
