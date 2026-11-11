//! The ICC binary parser: header, tag table, curves, lookup tables and text.

use super::*;

// ---------------------------------------------------------------- reader

pub(crate) struct Reader<'a> {
    pub(crate) b: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn slice(&self, off: usize, len: usize) -> Result<&'a [u8], CmsError> {
        off.checked_add(len).and_then(|end| self.b.get(off..end)).ok_or(CmsError::Truncated)
    }
    fn u8(&self, off: usize) -> Result<u8, CmsError> {
        self.b.get(off).copied().ok_or(CmsError::Truncated)
    }
    pub(crate) fn u16(&self, off: usize) -> Result<u16, CmsError> {
        let s = self.slice(off, 2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    pub(crate) fn u32(&self, off: usize) -> Result<u32, CmsError> {
        let s = self.slice(off, 4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn s15f16(&self, off: usize) -> Result<f64, CmsError> {
        Ok(self.u32(off)? as i32 as f64 / 65536.0)
    }
    fn xyz(&self, off: usize) -> Result<[f64; 3], CmsError> {
        let r = Reader { b: self.slice(off, 12)? };
        Ok([r.s15f16(0)?, r.s15f16(4)?, r.s15f16(8)?])
    }
}

/// `base + n * size` for a count or offset read from the file, or `Truncated` when it does not
/// fit a `usize`. Such fields reach `u32::MAX`, which overflows on 32-bit targets (wasm32):
/// a panic with overflow checks on, a wrapped offset that reads the wrong bytes without (#4976).
fn extent(base: usize, n: usize, size: usize) -> Result<usize, CmsError> {
    n.checked_mul(size).and_then(|len| base.checked_add(len)).ok_or(CmsError::Truncated)
}

fn sig(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

/// Upper bound on CLUT nodes × outputs we accept, to reject hostile files.
const MAX_CLUT_VALUES: usize = crate::clut::MAX_VALUES;

pub(super) fn parse_profile(bytes: &[u8]) -> Result<Profile, CmsError> {
    let r = Reader { b: bytes };
    if bytes.len() < 132 {
        return Err(CmsError::Truncated);
    }
    if r.slice(36, 4)? != b"acsp" {
        return Err(CmsError::BadSignature);
    }
    let declared = r.u32(0)? as usize;
    // Some writers pad or truncate; trust the smaller of the two.
    let end = if declared >= 132 && declared <= bytes.len() { declared } else { bytes.len() };
    let r = Reader { b: &bytes[..end] };
    let version = (r.u8(8)?, r.u8(9)?);
    let class = ProfileClass::from_sig(r.u32(12)?);
    let color_space = ColorSpace::from_sig(r.u32(16)?);
    let pcs_sig = r.u32(20)?;
    let pcs = match &pcs_sig.to_be_bytes() {
        b"Lab " => Pcs::Lab,
        _ => Pcs::Xyz,
    };
    let rendering_intent = Intent::from_u32(r.u32(64)? & 0xFFFF).unwrap_or(Intent::Perceptual);
    let count = r.u32(128)? as usize;
    if count > 1024 {
        return Err(CmsError::Invalid(format!("{count} tags")));
    }
    let mut tags: Vec<(u32, usize, usize)> = Vec::with_capacity(count);
    for i in 0..count {
        let o = 132 + i * 12;
        let (s, off, len) = (r.u32(o)?, r.u32(o + 4)? as usize, r.u32(o + 8)? as usize);
        tags.push((s, off, len));
    }
    let find = |s: &[u8; 4]| tags.iter().find(|t| t.0 == sig(s)).map(|t| (t.1, t.2));
    let tag_data = |s: &[u8; 4]| -> Result<Option<&[u8]>, CmsError> {
        match find(s) {
            Some((off, len)) => Ok(Some(r.slice(off, len)?)),
            None => Ok(None),
        }
    };

    let xyz_tag = |s: &[u8; 4]| -> Result<Option<[f64; 3]>, CmsError> {
        match tag_data(s)? {
            Some(d) if d.len() >= 20 && &d[..4] == b"XYZ " => Ok(Some(Reader { b: d }.xyz(8)?)),
            Some(_) => Err(CmsError::Invalid(format!("tag {} is not XYZType", String::from_utf8_lossy(s)))),
            None => Ok(None),
        }
    };
    let curve_tag = |s: &[u8; 4]| -> Result<Option<Curve>, CmsError> {
        match tag_data(s)? {
            Some(d) => Ok(Some(parse_curve(d)?.0)),
            None => Ok(None),
        }
    };

    let white_point = xyz_tag(b"wtpt")?.unwrap_or(math::D50);
    let black_point = xyz_tag(b"bkpt").ok().flatten();
    let chad = match tag_data(b"chad")? {
        Some(d) if d.len() >= 44 && &d[..4] == b"sf32" => {
            let rd = Reader { b: d };
            let mut m = [[0.0; 3]; 3];
            for (i, row) in m.iter_mut().enumerate() {
                for (j, v) in row.iter_mut().enumerate() {
                    *v = rd.s15f16(8 + (i * 3 + j) * 4)?;
                }
            }
            Some(m)
        }
        _ => None,
    };
    let matrix = match (xyz_tag(b"rXYZ")?, xyz_tag(b"gXYZ")?, xyz_tag(b"bXYZ")?) {
        (Some(rr), Some(g), Some(b)) => Some([[rr[0], g[0], b[0]], [rr[1], g[1], b[1]], [rr[2], g[2], b[2]]]),
        _ => None,
    };
    let trc = match (curve_tag(b"rTRC")?, curve_tag(b"gTRC")?, curve_tag(b"bTRC")?) {
        (Some(a), Some(b), Some(c)) => Some([a, b, c]),
        _ => None,
    };
    let gray_trc = curve_tag(b"kTRC")?;
    let lut = |s: &[u8; 4], a2b: bool| -> Result<Option<Lut>, CmsError> {
        match tag_data(s)? {
            Some(d) => parse_lut(d, a2b).map(Some),
            None => Ok(None),
        }
    };
    let a2b = [lut(b"A2B0", true)?, lut(b"A2B1", true)?, lut(b"A2B2", true)?];
    let b2a = [lut(b"B2A0", false)?, lut(b"B2A1", false)?, lut(b"B2A2", false)?];
    let description = tag_data(b"desc")?.map(parse_text).unwrap_or_default();
    let copyright = tag_data(b"cprt")?.map(parse_text).unwrap_or_default();

    let p = Profile {
        version,
        class,
        color_space,
        pcs,
        rendering_intent,
        description,
        copyright,
        white_point,
        black_point,
        chad,
        matrix,
        trc,
        gray_trc,
        a2b,
        b2a,
        bytes: None,
        hash: Default::default(),
    };
    // Validate channel counts of LUTs against the header. A DeviceLink connects two device
    // spaces: its header PCS field holds the output colour space (ICC.1:2010 §7.2.7), so a
    // CMYK→CMYK link has a 4→4 AToB0.
    let dev = p.color_space.channels();
    let conn = if p.class == ProfileClass::DeviceLink { ColorSpace::from_sig(pcs_sig).channels() } else { 3 };
    for l in p.a2b.iter().flatten() {
        if dev != 0 && l.inputs != dev || conn != 0 && l.outputs != conn {
            return Err(CmsError::Invalid(format!("AToB tag has {}→{} channels for a {:?} profile", l.inputs, l.outputs, p.color_space)));
        }
    }
    for l in p.b2a.iter().flatten() {
        if conn != 0 && l.inputs != conn || dev != 0 && l.outputs != dev {
            return Err(CmsError::Invalid(format!("BToA tag has {}→{} channels for a {:?} profile", l.inputs, l.outputs, p.color_space)));
        }
    }
    Ok(p)
}

/// Sampled-curve entries one profile may decode in all (64 MB of `f32`). Tags and curve groups can
/// point at the same encoded bytes, and each reference decodes its own table.
const MAX_CURVE_SAMPLES: usize = 16 << 20;

/// CLUT values one profile may decode in all (128 MB of `f32`): the twelve LUT tags can alias
/// one table, and each reference decodes its own copy.
const MAX_PROFILE_CLUT_VALUES: usize = 32 << 20;

thread_local! {
    /// What is left of [`MAX_CURVE_SAMPLES`] for the profile being parsed on this thread.
    static CURVE_SAMPLES_LEFT: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_CURVE_SAMPLES) };
    /// What is left of [`MAX_PROFILE_CLUT_VALUES`] for the profile being parsed on this thread.
    static CLUT_VALUES_LEFT: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_PROFILE_CLUT_VALUES) };
}

/// Gives the profile about to be parsed on this thread the full decode budgets.
pub(super) fn reset_budgets() {
    CURVE_SAMPLES_LEFT.with(|left| left.set(MAX_CURVE_SAMPLES));
    CLUT_VALUES_LEFT.with(|left| left.set(MAX_PROFILE_CLUT_VALUES));
}

/// Charges `n` decoded curve samples to the profile being parsed.
fn charge_curve_samples(n: usize) -> Result<(), CmsError> {
    CURVE_SAMPLES_LEFT.with(|left| {
        let rest = left.get().checked_sub(n).ok_or_else(|| CmsError::Invalid("the profile's curves hold too many samples".into()))?;
        left.set(rest);
        Ok(())
    })
}

/// Charges `n` decoded CLUT values to the profile being parsed.
fn charge_clut_values(n: usize) -> Result<(), CmsError> {
    CLUT_VALUES_LEFT.with(|left| {
        let rest = left.get().checked_sub(n).ok_or_else(|| CmsError::Invalid("the profile's lookup tables hold too many values".into()))?;
        left.set(rest);
        Ok(())
    })
}

/// Parses a `curv` or `para` element; returns the curve and the element's byte length
/// (unpadded).
fn parse_curve(d: &[u8]) -> Result<(Curve, usize), CmsError> {
    let r = Reader { b: d };
    match r.slice(0, 4)? {
        b"curv" => {
            let n = r.u32(8)? as usize;
            // Bounded before it is used: `n * 2` is computed next, and a count straight from the
            // file overflows a 32-bit `usize` (wasm32) with overflow checks on, panicking before
            // this guard could reject it.
            if n > 1 << 20 {
                return Err(CmsError::Invalid("curve too long".into()));
            }
            let len = 12 + n * 2;
            match n {
                0 => Ok((Curve::Identity, len)),
                1 => Ok((Curve::Gamma(r.u16(12)? as f64 / 256.0), len)),
                _ => {
                    let s = r.slice(12, n * 2)?;
                    charge_curve_samples(n)?;
                    let t = s.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]]) as f32 / 65535.0).collect();
                    Ok((Curve::Table(t), len))
                }
            }
        }
        b"para" => {
            let kind = r.u16(8)?;
            if kind > 4 {
                return Err(CmsError::Unsupported(format!("parametric curve type {kind}")));
            }
            let n = Curve::param_count(kind);
            let mut p = [0.0f64; 7];
            for (i, v) in p.iter_mut().enumerate().take(n) {
                *v = r.s15f16(12 + i * 4)?;
            }
            let len = 12 + n * 4;
            let c = if kind == 0 { Curve::Gamma(p[0]) } else { Curve::Parametric { kind, p } };
            Ok((c, len))
        }
        other => Err(CmsError::Unsupported(format!("curve type {:?}", String::from_utf8_lossy(other)))),
    }
}

fn parse_curves_at(d: &[u8], off: usize, n: usize) -> Result<Vec<Curve>, CmsError> {
    let mut out = Vec::with_capacity(n);
    let mut o = off;
    for _ in 0..n {
        let sub = d.get(o..).ok_or(CmsError::Truncated)?;
        let (c, len) = parse_curve(sub)?;
        out.push(c);
        o += (len + 3) & !3;
    }
    Ok(out)
}

fn parse_lut(d: &[u8], a2b: bool) -> Result<Lut, CmsError> {
    let r = Reader { b: d };
    let ty = r.slice(0, 4)?;
    match ty {
        b"mft1" | b"mft2" => {
            let wide = ty == b"mft2";
            let (i, o, g) = (r.u8(8)? as usize, r.u8(9)? as usize, r.u8(10)? as usize);
            if !(1..=15).contains(&i) || !(1..=15).contains(&o) || g == 0 {
                return Err(CmsError::Invalid("bad lut dimensions".into()));
            }
            let mut m = [[0.0; 3]; 3];
            for (a, row) in m.iter_mut().enumerate() {
                for (b, v) in row.iter_mut().enumerate() {
                    *v = r.s15f16(12 + (a * 3 + b) * 4)?;
                }
            }
            let (n_in, n_out, mut off) = if wide { (r.u16(48)? as usize, r.u16(50)? as usize, 52) } else { (256, 256, 48) };
            if wide && (!(2..=4096).contains(&n_in) || !(2..=4096).contains(&n_out)) {
                return Err(CmsError::Invalid("bad lut16 table size".into()));
            }
            let bps = if wide { 2 } else { 1 };
            let read_tables = |off: &mut usize, count: usize, len: usize| -> Result<Vec<Curve>, CmsError> {
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    let s = r.slice(*off, len * bps)?;
                    let t: Vec<f32> = if wide {
                        s.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]]) as f32 / 65535.0).collect()
                    } else {
                        s.iter().map(|b| *b as f32 / 255.0).collect()
                    };
                    *off += len * bps;
                    v.push(Curve::Table(t));
                }
                Ok(v)
            };
            let in_curves = read_tables(&mut off, i, n_in)?;
            let nodes = (g as u64).checked_pow(i as u32).unwrap_or(u64::MAX);
            if nodes as u128 * o as u128 > MAX_CLUT_VALUES as u128 {
                return Err(CmsError::Invalid("CLUT too large".into()));
            }
            let total = nodes as usize * o;
            charge_clut_values(total)?;
            let s = r.slice(off, total * bps)?;
            let data: Vec<f32> = if wide {
                s.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]]) as f32 / 65535.0).collect()
            } else {
                s.iter().map(|b| *b as f32 / 255.0).collect()
            };
            off += total * bps;
            let out_curves = read_tables(&mut off, o, n_out)?;
            let mut stages = Vec::new();
            // The matrix only applies when the input is XYZ (BToA with an XYZ PCS).
            let ident = m.iter().enumerate().all(|(a, row)| row.iter().enumerate().all(|(b, v)| (v - if a == b { 1.0 } else { 0.0 }).abs() < 1e-6));
            if !a2b && i == 3 && !ident {
                // Only meaningful for an XYZ PCS; the caller decides (we cannot know the PCS here),
                // so keep it: for Lab PCS writers are required to store the identity.
                stages.push(Stage::matrix3(&m));
            }
            stages.push(Stage::Curves(in_curves));
            stages.push(Stage::Clut(Clut::new(vec![g; i], o, data)?));
            stages.push(Stage::Curves(out_curves));
            Ok(Lut { kind: if wide { LutKind::Lut16 } else { LutKind::Lut8 }, inputs: i, outputs: o, stages })
        }
        b"mAB " | b"mBA " => {
            let (i, o) = (r.u8(8)? as usize, r.u8(9)? as usize);
            if !(1..=15).contains(&i) || !(1..=15).contains(&o) {
                return Err(CmsError::Invalid("bad lutAB dimensions".into()));
            }
            let (ob, om, omc, oc, oa) = (r.u32(12)? as usize, r.u32(16)? as usize, r.u32(20)? as usize, r.u32(24)? as usize, r.u32(28)? as usize);
            let matrix = |off: usize| lut_ab_matrix(&r, off);
            let clut = |off: usize, cin: usize, cout: usize| lut_ab_clut(&r, off, cin, cout);
            let mut stages = Vec::new();
            if ty == b"mAB " {
                // A curves → CLUT → M curves → matrix → B curves.
                if oc != 0 {
                    if oa == 0 {
                        return Err(CmsError::Invalid("lutAtoB with CLUT but no A curves".into()));
                    }
                    stages.push(Stage::Curves(parse_curves_at(d, oa, i)?));
                    stages.push(clut(oc, i, o)?);
                } else if i != o {
                    return Err(CmsError::Invalid("lutAtoB without CLUT changes channel count".into()));
                }
                if om != 0 {
                    stages.push(Stage::Curves(parse_curves_at(d, omc, o)?));
                    stages.push(matrix(om)?);
                }
                if ob == 0 {
                    return Err(CmsError::Invalid("lutAtoB without B curves".into()));
                }
                stages.push(Stage::Curves(parse_curves_at(d, ob, o)?));
            } else {
                // B curves → matrix → M curves → CLUT → A curves.
                if ob == 0 {
                    return Err(CmsError::Invalid("lutBtoA without B curves".into()));
                }
                stages.push(Stage::Curves(parse_curves_at(d, ob, i)?));
                if om != 0 {
                    stages.push(matrix(om)?);
                    stages.push(Stage::Curves(parse_curves_at(d, omc, i)?));
                }
                if oc != 0 {
                    if oa == 0 {
                        return Err(CmsError::Invalid("lutBtoA with CLUT but no A curves".into()));
                    }
                    stages.push(clut(oc, i, o)?);
                    stages.push(Stage::Curves(parse_curves_at(d, oa, o)?));
                } else if i != o {
                    return Err(CmsError::Invalid("lutBtoA without CLUT changes channel count".into()));
                }
            }
            Ok(Lut { kind: LutKind::Ab, inputs: i, outputs: o, stages })
        }
        other => Err(CmsError::Unsupported(format!("LUT type {:?}", String::from_utf8_lossy(other)))),
    }
}

/// The 3×3 matrix and offset of a `mAB `/`mBA ` element at `off`, an offset stored in the file.
fn lut_ab_matrix(r: &Reader, off: usize) -> Result<Stage, CmsError> {
    // One bounds-checked slice, then offsets inside it: `off + 44` need not fit a `usize`.
    let e = Reader { b: r.slice(off, 48)? };
    let mut m = vec![0.0; 9];
    for (k, v) in m.iter_mut().enumerate() {
        *v = e.s15f16(k * 4)?;
    }
    let offset = vec![e.s15f16(36)?, e.s15f16(40)?, e.s15f16(44)?];
    Ok(Stage::Matrix { rows: 3, cols: 3, m, offset })
}

/// The CLUT of a `mAB `/`mBA ` element at `off`, an offset stored in the file.
fn lut_ab_clut(r: &Reader, off: usize, cin: usize, cout: usize) -> Result<Stage, CmsError> {
    // Grid points (16 bytes), precision and padding; the samples follow.
    let head = Reader { b: r.slice(off, 20)? };
    let mut grid = Vec::with_capacity(cin);
    for k in 0..cin {
        let g = head.u8(k)? as usize;
        if g < 1 {
            return Err(CmsError::Invalid("zero grid points".into()));
        }
        grid.push(g);
    }
    let prec = head.u8(16)? as usize;
    let nodes: u128 = grid.iter().map(|g| *g as u128).product();
    if nodes * cout as u128 > MAX_CLUT_VALUES as u128 {
        return Err(CmsError::Invalid("CLUT too large".into()));
    }
    let total = nodes as usize * cout;
    charge_clut_values(total)?;
    let samples = extent(off, 20, 1)?;
    let data: Vec<f32> = match prec {
        1 => r.slice(samples, total)?.iter().map(|b| *b as f32 / 255.0).collect(),
        2 => r.slice(samples, total * 2)?.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]]) as f32 / 65535.0).collect(),
        _ => return Err(CmsError::Invalid(format!("CLUT precision {prec}"))),
    };
    Ok(Stage::Clut(Clut::new(grid, cout, data)?))
}

/// Text from `desc` (textDescriptionType), `mluc` (English or first record) or `text`.
fn parse_text(d: &[u8]) -> String {
    let r = Reader { b: d };
    let Ok(ty) = r.slice(0, 4) else { return String::new() };
    let ascii = |s: &[u8]| String::from_utf8_lossy(s.split(|b| *b == 0).next().unwrap_or(&[])).trim().to_string();
    match ty {
        b"desc" => {
            let n = r.u32(8).unwrap_or(0) as usize;
            r.slice(12, n).map(ascii).unwrap_or_default()
        }
        b"text" => ascii(d.get(8..).unwrap_or(&[])),
        b"mluc" => {
            let (Ok(n), Ok(size)) = (r.u32(8), r.u32(12)) else { return String::new() };
            let mut best: Option<(usize, usize)> = None;
            for k in 0..(n as usize).min(256) {
                // Checked: the record size comes from the file, and `16 + k * size` overflows a
                // 32-bit `usize` at k = 1 for a size near u32::MAX (wrapping would read another
                // record's bytes as the language, length and offset).
                let Some(o) = k.checked_mul(size as usize).and_then(|k| 16usize.checked_add(k)) else { break };
                // Complete record read: `slice` does its own checked math, and the fields come
                // from local offsets inside the 12-byte record, so no `o + 4` / `o + 8` is ever
                // computed on a file-derived offset. Reading the fields eagerly at `o`, `o + 4`,
                // `o + 8` evaluates every addition first: with `o` fitting but `o + 8` overflowing
                // a 32-bit `usize` (wasm32, overflow checks on), the addition panics before the
                // failed read could break the loop.
                let Ok(rec) = r.slice(o, 12) else { break };
                let rr = Reader { b: rec };
                let (Ok(lang), Ok(len), Ok(off)) = (rr.u16(0), rr.u32(4), rr.u32(8)) else { break };
                if best.is_none() || lang == u16::from_be_bytes(*b"en") {
                    best = Some((off as usize, len as usize));
                    if lang == u16::from_be_bytes(*b"en") {
                        break;
                    }
                }
            }
            let Some((off, len)) = best else { return String::new() };
            let Ok(s) = r.slice(off, len) else { return String::new() };
            let units: Vec<u16> = s.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units).trim_end_matches('\0').trim().to_string()
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be(v: u32) -> [u8; 4] {
        v.to_be_bytes()
    }

    /// #4976: offsets and counts stored in a profile reach `u32::MAX`, which is `usize::MAX` on
    /// 32-bit targets. The arithmetic on them must report a truncated profile, not overflow.
    #[test]
    fn offsets_at_the_top_of_the_address_space_are_truncated_not_overflowed() {
        let bytes = [0u8; 64];
        let r = Reader { b: &bytes };
        for off in [usize::MAX, usize::MAX - 3, usize::MAX - 11, usize::MAX - 19, usize::MAX - 47] {
            assert_eq!(r.xyz(off), Err(CmsError::Truncated), "xyz at {off}");
            assert!(matches!(lut_ab_matrix(&r, off), Err(CmsError::Truncated)), "matrix at {off}");
            assert!(matches!(lut_ab_clut(&r, off, 3, 3), Err(CmsError::Truncated)), "clut at {off}");
        }
        assert_eq!(extent(16, usize::MAX, 12), Err(CmsError::Truncated));
        assert_eq!(extent(usize::MAX, 1, 1), Err(CmsError::Truncated));
        assert_eq!(extent(16, 3, 12), Ok(52));
    }

    #[test]
    fn a_curve_count_is_checked_before_its_length_is_computed() {
        let mut d = b"curv\0\0\0\0".to_vec();
        d.extend(be(u32::MAX));
        assert_eq!(parse_curve(&d).map(|c| c.1), Err(CmsError::Invalid("curve too long".into())));
        // Controls: the identity and a two-point table still parse with their lengths.
        let mut d = b"curv\0\0\0\0".to_vec();
        d.extend(be(0));
        assert_eq!(parse_curve(&d).map(|c| c.1), Ok(12));
        let mut d = b"curv\0\0\0\0".to_vec();
        d.extend(be(2));
        d.extend([0, 0, 0xFF, 0xFF]);
        assert_eq!(parse_curve(&d).map(|c| c.1), Ok(16));
    }

    #[test]
    fn lut_ab_elements_at_hostile_offsets_are_rejected() {
        // A 3→3 lutAtoB with identity B curves at byte 32; then its B-curve, matrix and CLUT
        // offsets in turn point at the end of a 32-bit file.
        let lut = |b: u32, matrix: u32, m: u32, clut: u32, a: u32| {
            let mut d = b"mAB \0\0\0\0".to_vec();
            d.extend([3, 3, 0, 0]);
            for v in [b, matrix, m, clut, a] {
                d.extend(be(v));
            }
            for _ in 0..3 {
                d.extend(b"curv\0\0\0\0");
                d.extend(be(0));
            }
            d
        };
        assert!(parse_lut(&lut(32, 0, 0, 0, 0), true).is_ok(), "the control parses");
        let top = u32::MAX - 8;
        assert!(parse_lut(&lut(top, 0, 0, 0, 0), true).is_err(), "B curves offset");
        assert!(parse_lut(&lut(32, top, 32, 0, 0), true).is_err(), "matrix offset");
        assert!(parse_lut(&lut(32, 0, 0, top, 32), true).is_err(), "CLUT offset");
    }

    #[test]
    fn multilingual_records_with_a_huge_record_size_give_the_first_record() {
        let record = |lang: &[u8; 2], len: u32, off: u32| {
            let mut r = lang.to_vec();
            r.extend(b"US");
            r.extend(be(len));
            r.extend(be(off));
            r
        };
        // Two records of the standard 12 bytes: the English one wins.
        let mut d = b"mluc\0\0\0\0".to_vec();
        d.extend(be(2));
        d.extend(be(12));
        d.extend(record(b"de", 4, 40));
        d.extend(record(b"en", 4, 44));
        d.extend([0, b'D', 0, b'E', 0, b'E', 0, b'N']);
        assert_eq!(parse_text(&d), "EN");
        // A record size of u32::MAX: the second record lies past any file, so the first is used.
        d[12..16].copy_from_slice(&be(u32::MAX));
        assert_eq!(parse_text(&d), "DE");
        // No records at all.
        let mut d = b"mluc\0\0\0\0".to_vec();
        d.extend(be(u32::MAX));
        d.extend(be(u32::MAX));
        assert_eq!(parse_text(&d), "");
    }

    #[test]
    fn mab_offsets_near_the_end_of_the_address_space_are_errors() {
        // Offsets are file-controlled u32s: element reads must be checked slices, never `off + k`.
        for tag in [b"mAB ", b"mBA "] {
            for off in [u32::MAX, u32::MAX - 3, u32::MAX - 19, u32::MAX - 47] {
                for (ob, om, omc, oc, oa) in [(off, 0, 0, 0, 0), (32, off, 32, 0, 0), (32, 32, 32, off, 32), (32, 0, 0, 32, off)] {
                    let mut e = tag.to_vec();
                    e.extend([0u8; 4]);
                    e.extend([3, 3, 0, 0]);
                    for v in [ob, om, omc, oc, oa] {
                        e.extend(v.to_be_bytes());
                    }
                    e.resize(256, 0);
                    assert!(parse_lut(&e, true).is_err() || parse_lut(&e, false).is_err());
                }
            }
        }
    }

    #[test]
    fn aliased_curves_are_charged_for_each_decoded_table() {
        // One 1M-sample `curv` element referenced seventeen times decodes 17M samples.
        let mut element = b"curv\0\0\0\0".to_vec();
        element.extend((1u32 << 20).to_be_bytes());
        element.extend(std::iter::repeat_n(0u8, 2 << 20));
        reset_budgets();
        let failed_at = (0..17).find(|_| parse_curve(&element).is_err());
        assert_eq!(failed_at, Some(16), "sixteen tables fit the budget, the seventeenth does not");
        // Each profile parse starts with a fresh budget.
        reset_budgets();
        assert!(parse_curve(&element).is_ok());
    }
}
