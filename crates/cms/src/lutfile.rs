//! Creative 3D LUT files for Image › Adjustments › Color Lookup: `.cube` (Adobe/Resolve), `.3dl`
//! (Autodesk Lustre / Flame) and `.look` (SpeedGrade XML), plus a few generated (CC0) looks.
//!
//! Every table is normalised to [`LutFile`]: `size`³ RGB triplets in 0..=1, **red varying
//! fastest** (`((b·size + g)·size + r)·3`). Formats follow their public descriptions: the Adobe
//! "Cube LUT Specification 1.0", the Lustre 3DL layout as documented by OpenColorIO, and the
//! SpeedGrade `.look` XML (hex-encoded little-endian float32 RGB triplets).

/// A parsed 3D LUT.
#[derive(Clone, Debug, PartialEq)]
pub struct LutFile {
    pub title: String,
    pub size: usize,
    /// `size`³ × 3 values, red fastest.
    pub data: Vec<f32>,
    /// The input domain the table expects (`.cube` `DOMAIN_MIN`/`DOMAIN_MAX`,
    /// `LUT_3D_INPUT_RANGE`): an input `c` samples the table at `(c - min) / (max - min)`,
    /// saturating outside. `[0, 1]` is the identity default; applying a table with any other
    /// domain is rejected upstream until Color Lookup carries it through.
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
}

/// Largest edge length accepted (64³ is Photoshop's practical maximum; 129 leaves headroom while
/// keeping the GPU table within texture limits).
pub const MAX_SIZE: usize = 129;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LutError(pub String);

impl std::fmt::Display for LutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LutError {}

fn err<T>(msg: impl Into<String>) -> Result<T, LutError> {
    Err(LutError(msg.into()))
}

fn checked_data_len(size: usize) -> Result<(usize, usize), LutError> {
    if !(2..=MAX_SIZE).contains(&size) {
        return err(format!("unsupported LUT size {size}"));
    }
    let entries = size.checked_pow(3).ok_or_else(|| LutError(format!("unsupported LUT size {size}")))?;
    let data_len = entries.checked_mul(3).ok_or_else(|| LutError(format!("unsupported LUT size {size}")))?;
    Ok((entries, data_len))
}

impl LutFile {
    /// The identity table of edge `size` (2 to [`MAX_SIZE`]).
    pub fn identity(size: usize) -> Result<Self, LutError> {
        Self::from_fn("Identity", size, |c| c)
    }

    /// Samples `f` on an `size`³ grid, `size` from 2 to [`MAX_SIZE`].
    pub fn from_fn(title: &str, size: usize, f: impl Fn([f32; 3]) -> [f32; 3]) -> Result<Self, LutError> {
        let (_, data_len) = checked_data_len(size)?;
        let m = (size - 1) as f32;
        let mut data = Vec::with_capacity(data_len);
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    let o = f([r as f32 / m, g as f32 / m, b as f32 / m]);
                    data.extend(o.map(|v| v.clamp(0.0, 1.0)));
                }
            }
        }
        Ok(LutFile { title: title.to_string(), size, data, domain_min: [0.0; 3], domain_max: [1.0; 3] })
    }

    /// Whether the table expects plain 0..1 inputs (the common case; Photoshop's default).
    pub fn domain_is_default(&self) -> bool {
        self.domain_min == [0.0, 0.0, 0.0] && self.domain_max == [1.0, 1.0, 1.0]
    }

    fn check(self) -> Result<Self, LutError> {
        let (entries, data_len) = checked_data_len(self.size)?;
        if self.data.len() != data_len {
            return err(format!("expected {entries} entries, found {}", self.data.len() / 3));
        }
        if self.data.iter().any(|v| !v.is_finite()) {
            return err("non-finite LUT value");
        }
        Ok(self)
    }
}

/// A LUT file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Cube,
    ThreeDl,
    Look,
}

impl Format {
    /// The format [`parse`] reads `bytes` as: from `file_name`'s extension, else sniffed
    /// (XML is `.look`, anything else `.cube`).
    pub fn detect(file_name: &str, bytes: &[u8]) -> Self {
        let lower = file_name.to_ascii_lowercase();
        if lower.ends_with(".3dl") {
            Format::ThreeDl
        } else if lower.ends_with(".look") || decode_text(bytes).trim_start().starts_with('<') {
            Format::Look
        } else {
            Format::Cube
        }
    }

    /// The file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Cube => "cube",
            Format::ThreeDl => "3dl",
            Format::Look => "look",
        }
    }
}

/// Parses a LUT, picking the format from `file_name`'s extension (falls back to sniffing).
pub fn parse(file_name: &str, bytes: &[u8]) -> Result<LutFile, LutError> {
    parse_as(Format::detect(file_name, bytes), bytes)
}

/// Parses a LUT in a known format.
pub fn parse_as(format: Format, bytes: &[u8]) -> Result<LutFile, LutError> {
    let text = decode_text(bytes);
    match format {
        Format::Cube => parse_cube(&text),
        Format::ThreeDl => parse_3dl(&text),
        Format::Look => parse_look(&text),
    }
}

/// UTF-16 with a byte-order mark (Photoshop loads such `.look` files, #9118), else UTF-8
/// (a UTF-8 BOM is stripped).
fn decode_text(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    let utf16 = |rest: &[u8], from: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|&c| from(c)).collect();
        String::from_utf16_lossy(&units).into()
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest),
        _ => String::from_utf8_lossy(bytes),
    }
}

/// Adobe / Resolve `.cube`. 1D LUTs (`LUT_1D_SIZE`) are expanded to a 33³ table.
pub fn parse_cube(text: &str) -> Result<LutFile, LutError> {
    let mut title = String::new();
    let (mut size3, mut size1) = (0usize, 0usize);
    let (mut dmin, mut dmax) = ([0.0f32; 3], [1.0f32; 3]);
    let mut rows: Vec<f32> = Vec::new();
    for line in text.lines() {
        // Only a quote at the beginning of a TITLE value protects '#'. Quotes inside an
        // unquoted title's comment must not turn that comment into part of the title.
        if let Some(quoted) = line
            .trim_start()
            .strip_prefix("TITLE")
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .and_then(|rest| rest.trim_start().strip_prefix('"'))
        {
            let Some((value, _)) = quoted.split_once('"') else { return err("unterminated quoted TITLE") };
            title = value.to_string();
            continue;
        }
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let key = it.next().unwrap_or("");
        let nums = |it: std::str::SplitWhitespace<'_>| -> Result<[f32; 3], LutError> {
            let v: Vec<f32> = it.filter_map(|s| s.parse().ok()).collect();
            if v.len() == 3 { Ok([v[0], v[1], v[2]]) } else { err(format!("bad line `{line}`")) }
        };
        match key {
            "TITLE" => title = line[5..].trim().trim_matches('"').to_string(),
            "LUT_3D_SIZE" => size3 = it.next().and_then(|s| s.parse().ok()).ok_or_else(|| LutError("bad LUT_3D_SIZE".into()))?,
            "LUT_1D_SIZE" => size1 = it.next().and_then(|s| s.parse().ok()).ok_or_else(|| LutError("bad LUT_1D_SIZE".into()))?,
            "DOMAIN_MIN" => dmin = nums(it)?,
            "DOMAIN_MAX" => dmax = nums(it)?,
            "LUT_3D_INPUT_RANGE" | "LUT_1D_INPUT_RANGE" => {
                let v: Vec<f32> = it.filter_map(|s| s.parse().ok()).collect();
                if v.len() == 2 {
                    dmin = [v[0]; 3];
                    dmax = [v[1]; 3];
                }
            }
            k if k.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+' || c == '.') => {
                let mut v = vec![k.parse::<f32>().map_err(|_| LutError(format!("bad number in `{line}`")))?];
                for value in it {
                    v.push(value.parse::<f32>().map_err(|_| LutError(format!("bad number in `{line}`")))?);
                }
                if v.len() != 3 {
                    return err(format!("bad data line `{line}`"));
                }
                rows.extend(v);
            }
            // Unknown keywords (e.g. Resolve's LUT_IN_VIDEO_RANGE) are ignored, as the spec allows.
            _ => {}
        }
    }
    let domain_ok = (0..3).all(|k| dmin[k] < dmax[k] && dmin[k].is_finite() && dmax[k].is_finite());
    if (size3 > 0 || size1 >= 2) && !domain_ok {
        return err(format!("DOMAIN_MIN {:?} must lie below DOMAIN_MAX {:?}", dmin, dmax));
    }
    if size3 > 0 {
        return LutFile { title, size: size3, data: rows, domain_min: dmin, domain_max: dmax }.check();
    }
    if size1 >= 2 {
        let expected_rows = size1.checked_mul(3).ok_or_else(|| LutError(format!("unsupported 1D LUT size {size1}")))?;
        if rows.len() != expected_rows {
            return err("no LUT_3D_SIZE in .cube file");
        }
        // #8913: the resample below bakes the curve onto a fixed 33-point grid, so a steep
        // curve over a narrow domain cannot survive the resample — the baked table silently
        // shifts colours (measured 42 RGB8 levels for a two-node ramp over 0.24..0.26, where
        // Photoshop renders the ramp exactly). Rather than proxy the risk by domain width (a
        // 0.1..0.17 ramp spans two grid cells yet still loses ~28 levels), measure the
        // resample's actual error against the exact curve and refuse only tables whose baked
        // output shifts by more than a couple of RGB8 levels. Default-domain tables keep the
        // existing bake untouched; gentle curves over any domain bake within tolerance and
        // stay accepted.
        // The 1D table is resampled onto a 0..1 cube anyway, so its input domain
        // (`DOMAIN_MIN`/`MAX`, `LUT_1D_INPUT_RANGE`) is baked into the resample: an input `v`
        // reads the table at `(v - min) / (max - min)`, saturating outside.
        let curve = |ch: usize, v: f32| {
            let t = (v - dmin[ch]) / (dmax[ch] - dmin[ch]);
            let x = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 } * (size1 - 1) as f32;
            let i = (x.floor() as usize).min(size1 - 2);
            let f = x - i as f32;
            rows[i * 3 + ch] * (1.0 - f) + rows[(i + 1) * 3 + ch] * f
        };
        let mut l = LutFile::from_fn(&title, 33, |c| [curve(0, c[0]), curve(1, c[1]), curve(2, c[2])])?;
        l.title = title;
        let non_default = dmin.iter().any(|v| *v != 0.0) || dmax.iter().any(|v| *v != 1.0);
        if non_default {
            // Worst baked-vs-exact error per channel, in RGB8 levels: the baked cube is read
            // by linear interpolation between grid points (red fastest, so channel `ch` sits
            // at stride 33^ch), while `curve` is what the file actually says.
            const MAX_BAKE_ERROR: f32 = 2.0 / 255.0;
            for ch in 0..3 {
                let stride = [1usize, 33, 33 * 33][ch];
                let mut worst = 0.0f32;
                for k in 0..=256 {
                    let v = k as f32 / 256.0;
                    let x = v * 32.0;
                    let i = (x as usize).min(31);
                    let f = x - i as f32;
                    let baked = l.data[i * stride * 3 + ch] * (1.0 - f) + l.data[(i + 1).min(32) * stride * 3 + ch] * f;
                    worst = worst.max((baked - curve(ch, v)).abs());
                }
                if worst > MAX_BAKE_ERROR {
                    return err(format!(
                        "1D LUT domain on {} is {min}..{max}: the curve changes faster than the 33-point grid can \
                         hold (baking shifts colours by up to {levels:.0} RGB8 levels); export the look as a 3D LUT \
                         or widen DOMAIN_MIN/DOMAIN_MAX",
                        ["red", "green", "blue"][ch],
                        min = dmin[ch],
                        max = dmax[ch],
                        levels = worst * 255.0,
                    ));
                }
            }
        }
        return l.check();
    }
    err("no LUT_3D_SIZE in .cube file")
}

/// Lustre `.3dl`: an optional `3DMESH` magic and `Mesh <inBits> <outBits>` line, an optional
/// shaper line of input values, then integer RGB rows with **blue** varying fastest. The
/// `Mesh` line declares the cube edge (2^inBits + 1) and the output code range (up to
/// 2^outBits − 1), and Photoshop honours both (#9073, #9078); without it the edge is the cube
/// root of the row count and the range is inferred from the largest value (#9121). A shaper that
/// isn't the even ramp is applied before the table, as Photoshop does (#8982): see [`apply_shaper`].
pub fn parse_3dl(text: &str) -> Result<LutFile, LutError> {
    let mut mesh: Option<(u32, u32)> = None;
    let mut rows: Vec<[f64; 3]> = Vec::new();
    // How many numbers the first numeric line carried: a shaper of exactly `n` values (n ≤ 3)
    // is otherwise indistinguishable from a node row.
    let mut first_len: Option<usize> = None;
    // Any text line before the rows is a header: a headerless table peaking at 255 or 1023 is
    // on the 12-bit scale, as Photoshop 27.11 reads it (#9121).
    let mut header = false;
    let mut shaper: Option<Vec<f64>> = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        // A header is `3DMESH` or starts with a word (`Mesh 4 12`): a letter, and not a number.
        // `NaN` and `inf` are numbers, and `0oops` starts like one: those rows are read strictly.
        let Some(first) = line.split_whitespace().next() else { continue };
        if first.eq_ignore_ascii_case("3DMESH") || first.starts_with(char::is_alphabetic) && first.parse::<f64>().is_err() {
            header = true;
            // The only header keyword that carries data. A malformed or out-of-range `Mesh`
            // line is ignored, as every text line always was.
            let mut it = line.split_whitespace();
            if it.next().is_some_and(|k| k.eq_ignore_ascii_case("Mesh")) {
                // A declaration is exactly two integer fields after the keyword. Anything else
                // (a non-numeric field, too few or too many) is not a declaration: `mesh` stays
                // unset so headerless inference applies, rather than recovering numbers from
                // the wrong positions (#9078).
                let fields: Vec<&str> = it.collect();
                if let [a, b] = fields.as_slice()
                    && let (Ok(n), Ok(bits)) = (a.parse::<u32>(), b.parse::<u32>())
                    && (1..=7).contains(&n)
                    && (1..=16).contains(&bits)
                {
                    mesh = Some((n, bits));
                }
            }
            continue;
        }
        // Every token is a number: a malformed one is refused rather than dropped, which could
        // turn a shaper into an even ramp and lose it silently.
        let v: Vec<f64> = line.split_whitespace().map(str::parse).collect::<Result<_, _>>().map_err(|_| LutError(format!("bad line `{line}`")))?;
        first_len.get_or_insert(v.len());
        if v.len() == 3 {
            rows.push([v[0], v[1], v[2]]);
        } else if v.len() >= 2 {
            // Two values are a 2-point shaper (a 2³ table's), not something to drop.
            // The input shaper: one row of values ahead of the table, and only one.
            if shaper.is_some() || !rows.is_empty() {
                return err(format!("a second shaper row `{line}`"));
            }
            shaper = Some(v);
        }
    }
    let (n, nodes, scale) = match mesh {
        Some((in_bits, out_bits)) => {
            let n = 2usize.pow(in_bits) + 1;
            let expected = n.checked_pow(3).ok_or_else(|| LutError(format!("unsupported 3dl cube size {n}")))?;
            // One row more than the declared cube, and the first row has exactly `n` values:
            // that row is the shaper, not a 28th node of a 3³ cube (#9078).
            let nodes = if shaper.is_none() && rows.len() == expected + 1 && first_len == Some(n) {
                shaper = rows.first().map(|row| row.to_vec());
                &rows[1..]
            } else {
                &rows[..]
            };
            if nodes.len() != expected {
                return err(format!("expected {expected} rows for the declared {n}³ cube, found {}", nodes.len()));
            }
            // Under a valid declaration the node codes are integers in the declared range, so
            // the declaration wins for every in-range value — including a small max, where 1 is
            // 1/declared of full output, not full output itself (#9073). Only a node beyond the
            // declared range is a wrong declaration, and the old inference keeps such a file
            // loading as before.
            let max = nodes.iter().flatten().fold(0.0f64, |a, &b| a.max(b));
            let declared = 2f64.powi(i32::try_from(out_bits).unwrap_or(16)) - 1.0;
            (n, nodes, if max > declared { inferred_3dl_scale(max, header) } else { declared })
        }
        None => {
            let n = (rows.len() as f64).cbrt().round() as usize;
            if n < 2 || n * n * n != rows.len() {
                return err(format!("{} rows is not a cube", rows.len()));
            }
            let max = rows.iter().flatten().fold(0.0f64, |a, &b| a.max(b));
            (n, &rows[..], inferred_3dl_scale(max, header))
        }
    };
    let mut data = vec![0.0f32; n * n * n * 3];
    for (i, row) in nodes.iter().enumerate() {
        let (r, g, b) = (i / (n * n), (i / n) % n, i % n);
        let at = ((b * n + g) * n + r) * 3;
        for k in 0..3 {
            data[at + k] = (row[k] / scale) as f32;
        }
    }
    let lut = LutFile { title: String::new(), size: n, data, domain_min: [0.0; 3], domain_max: [1.0; 3] }.check()?;
    Ok(match shaper {
        Some(shaper) => apply_shaper(lut, &shaper)?,
        None => lut,
    })
}

/// Bakes a `.3dl` shaper into the table. Photoshop applies the shaper row as a 1-D curve on each
/// input channel ahead of the 3-D table (input `x` reads the shaper, sampled evenly over 0..1,
/// at `x`, interpolated linearly), so each node of the table becomes the table's own trilinear
/// value at the shaped input. An even ramp (the usual shaper) changes nothing and is skipped.
/// The table is baked on a grid holding both its own nodes and the shaper's knots: exact for an
/// identity table, an approximation at that resolution otherwise. A shaper that would need more
/// than [`MAX_SIZE`] nodes for that, or that holds a value that isn't finite, is refused.
fn apply_shaper(lut: LutFile, shaper: &[f64]) -> Result<LutFile, LutError> {
    let m = shaper.len();
    if m < 2 {
        return Ok(lut);
    }
    // Apply or refuse: a shaper that can't be applied never leaves the table to render alone.
    if shaper.iter().any(|v| !v.is_finite()) {
        return err("the .3dl shaper holds a value that isn't finite");
    }
    // The shaper's own depth, from 10 bits up: a 0..1023 ramp is the even ramp with or without a header.
    let scale = inferred_3dl_scale(shaper.iter().fold(0.0f64, |a, &b| a.max(b)), true);
    let even = |k: usize| scale * k as f64 / (m - 1) as f64;
    // Shapers round their ramp: within one code is still the even ramp. An integer shaper's code
    // is 1; a float shaper (0..1) is held to one 10-bit step.
    let code = if scale > 1.0 { 1.0 } else { 1.0 / 1023.0 };
    if shaper.iter().enumerate().all(|(k, &v)| (v - even(k)).abs() <= code) {
        return Ok(lut);
    }
    // Scaled to 0..1 first, so interpolating between two extreme values can't overflow.
    let shaped: Vec<f64> = shaper.iter().map(|v| v / scale).collect();
    let curve = |x: f64| -> f32 {
        let p = x.clamp(0.0, 1.0) * (m - 1) as f64;
        let i = (p.floor() as usize).min(m - 2);
        let f = p - i as f64;
        let (a, b) = (shaped.get(i).copied().unwrap_or(0.0), shaped.get(i + 1).copied().unwrap_or(0.0));
        (a + (b - a) * f).clamp(0.0, 1.0) as f32
    };
    // The bake grid holds both the table's nodes and the shaper's knots (lcm of their intervals),
    // so the curve keeps its breakpoints whatever the table's size: exact on an identity table. A
    // grid that would pass MAX_SIZE is refused: sampled more coarsely, a bent shaper could read as
    // even and be lost without a word.
    let gcd = |mut a: usize, mut b: usize| {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    };
    let (a, k) = (lut.size - 1, m - 1);
    let aligned = (a / gcd(a, k)).checked_mul(k).map(|l| l + 1).filter(|&l| l <= MAX_SIZE);
    let Some(n) = aligned else {
        return err(format!("the .3dl shaper's {m} points can't be kept on a table of at most {MAX_SIZE} nodes"));
    };
    let step = (n - 1) as f64;
    let mut out = LutFile { size: n, data: vec![0.0; n * n * n * 3], ..lut.clone() };
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                let at = ((b * n + g) * n + r) * 3;
                let v = sample(&lut, [curve(r as f64 / step), curve(g as f64 / step), curve(b as f64 / step)]);
                if let Some(dst) = out.data.get_mut(at..at + 3) {
                    dst.copy_from_slice(&v);
                }
            }
        }
    }
    out.check()
}

/// Trilinear lookup of `rgb` (0..1) in a [`LutFile`] (red fastest).
fn sample(l: &LutFile, rgb: [f32; 3]) -> [f32; 3] {
    let n = l.size;
    let loc = |v: f32| {
        let p = v.clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (p as usize).min(n.saturating_sub(2));
        (i, p - i as f32)
    };
    let (r0, fr) = loc(rgb[0]);
    let (g0, fg) = loc(rgb[1]);
    let (b0, fb) = loc(rgb[2]);
    let at = |r: usize, g: usize, b: usize, k: usize| l.data.get(((b * n + g) * n + r) * 3 + k).copied().unwrap_or(0.0);
    let mut o = [0.0f32; 3];
    for (k, ov) in o.iter_mut().enumerate() {
        let c = |r, g, b| at(r, g, b, k);
        let x00 = c(r0, g0, b0) + (c(r0 + 1, g0, b0) - c(r0, g0, b0)) * fr;
        let x10 = c(r0, g0 + 1, b0) + (c(r0 + 1, g0 + 1, b0) - c(r0, g0 + 1, b0)) * fr;
        let x01 = c(r0, g0, b0 + 1) + (c(r0 + 1, g0, b0 + 1) - c(r0, g0, b0 + 1)) * fr;
        let x11 = c(r0, g0 + 1, b0 + 1) + (c(r0 + 1, g0 + 1, b0 + 1) - c(r0, g0 + 1, b0 + 1)) * fr;
        let y0 = x00 + (x10 - x00) * fg;
        let y1 = x01 + (x11 - x01) * fg;
        *ov = y0 + (y1 - y0) * fb;
    }
    o
}

/// The code range a `.3dl` node table without a declared output depth was written with: the
/// smallest common integer depth that covers the largest value; a table of 0..1 floats passes
/// through. A headerless table has no 10-bit reading — 255 and 1023 are on the 12-bit scale, as
/// Photoshop 27.11 reads them (#9121).
fn inferred_3dl_scale(max: f64, header: bool) -> f64 {
    if max <= 1.0 {
        return 1.0;
    }
    let depths: &[f64] = if header { &[1023.0, 4095.0, 16383.0, 65535.0] } else { &[4095.0, 16383.0, 65535.0] };
    depths.iter().copied().find(|&s| max <= s).unwrap_or(max)
}

/// SpeedGrade `.look`: `<size>` and a hex `<data>` string of little-endian float32 RGB triplets
/// (red fastest).
pub fn parse_look(text: &str) -> Result<LutFile, LutError> {
    // Comments may hold tag-like text, so drop them before looking for tags (#9109).
    let mut clean = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("<!--") {
        clean.push_str(&rest[..i]);
        rest = rest[i + 4..].find("-->").map_or("", |e| &rest[i + 4 + e + 3..]);
    }
    clean.push_str(rest);
    let text = clean.as_str();
    let tag = |name: &str| -> Option<String> {
        let open = format!("<{name}>");
        let start = text.find(&open)? + open.len();
        let end = text[start..].find(&format!("</{name}>"))? + start;
        Some(text[start..end].trim().trim_matches('"').trim().to_string())
    };
    let size: usize = tag("size").and_then(|s| s.parse().ok()).ok_or_else(|| LutError("no <size> in .look".into()))?;
    let (entries, _) = checked_data_len(size)?;
    let hex: Vec<u8> = tag("data").ok_or_else(|| LutError("no <data> in .look".into()))?.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if hex.iter().any(|b| !b.is_ascii_hexdigit()) {
        return err("invalid hexadecimal <data> in .look");
    }
    // Each float needs eight hex digits; as_chunks would silently discard a partial value.
    if !hex.len().is_multiple_of(8) {
        return err("incomplete float in .look <data>");
    }
    let nib = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
    let bytes: Vec<u8> = hex.as_chunks::<2>().0.iter().map(|p| nib(p[0]) << 4 | nib(p[1])).collect();
    let floats: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let rgba_entries = entries.checked_mul(4).ok_or_else(|| LutError(format!("unsupported LUT size {size}")))?;
    let data = if floats.len() == rgba_entries { floats.as_chunks::<4>().0.iter().flat_map(|c| [c[0], c[1], c[2]]).collect() } else { floats };
    LutFile { title: tag("title").unwrap_or_default(), size, data, domain_min: [0.0; 3], domain_max: [1.0; 3] }.check()
}

/// Writes a `.cube` file (what Photoshop embeds in a Color Lookup layer).
pub fn write_cube(l: &LutFile) -> String {
    let mut s = String::new();
    if !l.title.is_empty() {
        s.push_str(&format!("TITLE \"{}\"\n", l.title.replace('"', "'")));
    }
    // The domain is what every row means: a table read with a non-default one must not be
    // written back as if it expected inputs in 0..1. `parse_cube` accepts these anywhere.
    if !l.domain_is_default() {
        s.push_str(&format!("DOMAIN_MIN {} {} {}\n", l.domain_min[0], l.domain_min[1], l.domain_min[2]));
        s.push_str(&format!("DOMAIN_MAX {} {} {}\n", l.domain_max[0], l.domain_max[1], l.domain_max[2]));
    }
    s.push_str(&format!("LUT_3D_SIZE {}\n", l.size));
    for c in l.data.as_chunks::<3>().0 {
        s.push_str(&format!("{:.6} {:.6} {:.6}\n", c[0], c[1], c[2]));
    }
    s
}

fn luma(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Generated looks shipped with this crate (procedural, CC0): `(id, label)`.
pub const BUILTIN: [(&str, &str); 8] = [
    ("warm", "Warm Filter"),
    ("cool", "Cool Filter"),
    ("tealOrange", "Teal & Orange"),
    ("bleachBypass", "Bleach Bypass"),
    ("fadedFilm", "Faded Film"),
    ("dayForNight", "Day for Night"),
    ("monoContrast", "Mono High Contrast"),
    ("crossProcess", "Cross Process"),
];

/// A built-in look by id (33³), or None.
pub fn builtin(id: &str) -> Option<LutFile> {
    let label = BUILTIN.iter().find(|b| b.0 == id)?.1;
    let f: Box<dyn Fn([f32; 3]) -> [f32; 3]> = match id {
        "warm" => Box::new(|c: [f32; 3]| [c[0] * 0.92 + 0.08, c[1] * 0.97 + 0.02, c[2] * 0.85]),
        "cool" => Box::new(|c: [f32; 3]| [c[0] * 0.86, c[1] * 0.97 + 0.01, c[2] * 0.9 + 0.1]),
        "tealOrange" => Box::new(|c: [f32; 3]| {
            // Shadows toward teal, highlights toward orange, skin-ish midtones kept.
            let l = luma(c);
            let t = smooth(l);
            let teal = [0.0, 0.5, 0.55];
            let orange = [1.0, 0.62, 0.3];
            std::array::from_fn(|k| {
                let tint = teal[k] * (1.0 - t) + orange[k] * t;
                c[k] * 0.75 + (tint * l * 1.1).min(1.0) * 0.25 + (smooth(c[k]) - c[k]) * 0.3
            })
        }),
        "bleachBypass" => Box::new(|c: [f32; 3]| {
            let l = luma(c);
            let s = smooth(l);
            std::array::from_fn(|k| {
                let desat = c[k] * 0.45 + l * 0.55;
                desat * 0.5 + s * 0.5
            })
        }),
        "fadedFilm" => Box::new(|c: [f32; 3]| {
            let l = luma(c);
            let tint = [1.0, 0.96, 0.88];
            std::array::from_fn(|k| (0.08 + (c[k] * 0.8 + l * 0.2) * 0.84) * tint[k] + 0.02)
        }),
        "dayForNight" => Box::new(|c: [f32; 3]| {
            let l = luma(c);
            [l * 0.35, l * 0.45 + c[1] * 0.05, l * 0.65 + c[2] * 0.15]
        }),
        "monoContrast" => Box::new(|c: [f32; 3]| [smooth(smooth(luma(c))); 3]),
        _ /* crossProcess */ => Box::new(|c: [f32; 3]| [smooth(c[0]) * 1.05, c[1] * 1.1 - 0.03, c[2] * 0.7 + 0.15]),
    };
    LutFile::from_fn(label, 33, f).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_title_hashes_and_comment_quotes_are_distinct() {
        let rows = "LUT_1D_SIZE 2\n0 0 0 # low\n1 1 1 # high\n";
        for (line, title) in [
            (r#"TITLE "Look #3" # comment "ignored""#, "Look #3"),
            (r#"TITLE Look # "draft""#, "Look"),
            ("  TITLE\t\"Été #2\"", "Été #2"),
            (r#"TITLE "" # empty"#, ""),
        ] {
            assert_eq!(parse_cube(&format!("{line}\n{rows}")).unwrap().title, title);
        }
        let lut = LutFile::from_fn("Look #3", 2, |c| c).unwrap();
        assert_eq!(parse_cube(&write_cube(&lut)).unwrap().title, "Look #3");
        assert!(parse_cube(&format!("TITLE \"unterminated # title\n{rows}")).is_err());
    }

    #[test]
    fn cube_rejects_invalid_numeric_fields() {
        let text = "LUT_3D_SIZE 2\n0.25 nope 0.5 0.75\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n";
        assert!(parse_cube(text).is_err(), "invalid numeric field must not be ignored");
    }

    #[test]
    fn utf8_bom_is_stripped() {
        let l = LutFile::identity(3).unwrap();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(write_cube(&l).as_bytes());
        let p = parse("a.cube", &bytes).unwrap();
        assert_eq!(p.size, l.size);
        assert_eq!(p.title, l.title);
        let titled = LutFile::from_fn("Look", 3, |c| c).unwrap();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(write_cube(&titled).as_bytes());
        assert_eq!(parse("a.cube", &bytes).unwrap().title, "Look");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"LUT_3D_SIZE 2\n");
        for i in 0..8 {
            bytes.extend_from_slice(format!("{} {} {}\n", i & 1, (i >> 1) & 1, i >> 2).as_bytes());
        }
        assert_eq!(parse("a.cube", &bytes).unwrap().size, 2);
    }

    /// An identity `.3dl` of edge `n` (12-bit nodes, blue fastest) under `shaper`.
    fn identity_3dl(shaper: &[u32], n: usize) -> String {
        let mut t = shaper.iter().map(u32::to_string).collect::<Vec<_>>().join(" ") + "\n";
        let node = |i: usize| (i * 4095 / (n - 1)).to_string();
        for r in 0..n {
            for g in 0..n {
                for b in 0..n {
                    t += &format!("{} {} {}\n", node(r), node(g), node(b));
                }
            }
        }
        t
    }

    #[test]
    fn three_dl_shaper_applies_before_the_table() {
        // #8982: Photoshop applies a .3dl's shaper row as a curve ahead of the 3-D table. On grey
        // 128 an identity table with the shaper 0 114 455 1023 gives 71 there, and a 17-point
        // squared shaper 64 (reported measurements); even ramps leave grey 128 as it is.
        let grey = |l: &LutFile| (sample(l, [128.0 / 255.0; 3])[0] * 255.0).round();
        let bent = parse_3dl(&identity_3dl(&[0, 114, 455, 1023], 4)).unwrap();
        assert_eq!(grey(&bent), 71.0);
        let squared: Vec<u32> = (0..17).map(|i| (1023.0 * (i as f64 / 16.0).powi(2)).round() as u32).collect();
        assert_eq!(grey(&parse_3dl(&identity_3dl(&squared, 17)).unwrap()), 64.0);
        let even4 = parse_3dl(&identity_3dl(&[0, 341, 682, 1023], 4)).unwrap();
        assert_eq!(grey(&even4), 128.0);
        assert_eq!(even4, parse_3dl(&identity_3dl(&[0, 341, 682, 1023], 4).lines().skip(1).collect::<Vec<_>>().join("\n")).unwrap());
        let even17: Vec<u32> = (0..17).map(|i| (1023.0 * i as f64 / 16.0).round() as u32).collect();
        assert_eq!(grey(&parse_3dl(&identity_3dl(&even17, 17)).unwrap()), 128.0);
        // The ends stay put, and the shaper bends each channel on its own.
        assert_eq!(sample(&bent, [0.0, 1.0, 0.0]), [0.0, 1.0, 0.0]);
        let c = sample(&bent, [1.0 / 3.0, 0.0, 2.0 / 3.0]);
        assert!((c[0] - 114.0 / 1023.0).abs() < 1e-4 && (c[2] - 455.0 / 1023.0).abs() < 1e-4, "{c:?}");
        // A shaper that isn't finite is refused rather than dropped (see
        // `three_dl_non_finite_shaper_is_refused`).
        assert!(parse_3dl(&identity_3dl(&[0, 341, 682, 1023], 4).replacen("341", "NaN", 1)).is_err());
    }

    /// #9170 review: a float shaper (values in 0..1) is held to a float ramp, not to one integer
    /// code, so a bent float shaper is applied.
    #[test]
    fn three_dl_float_shaper_is_applied() {
        let grey = |l: &LutFile| (sample(l, [128.0 / 255.0; 3])[0] * 255.0).round();
        let with = |shaper: &str| {
            let table = identity_3dl(&[0, 1, 2, 3], 4);
            parse_3dl(&format!("{shaper}\n{}", table.split_once('\n').unwrap().1)).unwrap()
        };
        // 0.502 lands between 0.1 and 0.3 at 0.506 of the way: 0.2012, so grey 51.
        assert_eq!(grey(&with("0 0.1 0.3 1.0")), 51.0);
        // An even float ramp, written to three decimals, still changes nothing.
        assert_eq!(grey(&with("0 0.333 0.667 1.0")), 128.0);
    }

    /// #9170 review: a shaper with more points than the table keeps its curve: the table is
    /// resampled at the shaper's resolution rather than reading the shaper at its own two nodes.
    #[test]
    fn three_dl_shaper_finer_than_the_table_keeps_its_curve() {
        let grey = |l: &LutFile| (sample(l, [128.0 / 255.0; 3])[0] * 255.0).round();
        let small = parse_3dl(&identity_3dl(&[0, 0, 512, 1023], 2)).unwrap();
        // 0.502 lands between 0 and 512 at 0.506 of the way: 259 / 1023, so grey 65.
        assert_eq!(grey(&small), 65.0);
        assert_eq!(sample(&small, [1.0, 0.0, 1.0]), [1.0, 0.0, 1.0], "the ends stay put");
    }

    /// #9170 review: a finite but extreme shaper can't overflow into NaN when it is baked.
    #[test]
    fn three_dl_extreme_shaper_bakes_only_finite_values() {
        let table = identity_3dl(&[0, 1, 2, 3], 2);
        let text = format!("0 1e308 -1e308 0\n{}", table.split_once('\n').unwrap().1);
        // Refused or baked, but never a table holding NaN.
        if let Ok(l) = parse_3dl(&text) {
            assert!(l.data.iter().all(|v| v.is_finite()), "baked a non-finite value");
        }
    }

    /// #9170 review (bommerts): the shaper's breakpoints survive a table whose grid doesn't line
    /// up with them. The bake grid holds both the table's nodes and the shaper's knots, so on an
    /// identity table the curve is exact: `0 0 512 1023` is 0 at ⅓ over 5³ and 17³ tables.
    #[test]
    fn three_dl_shaper_knots_survive_a_nonaligned_table() {
        for n in [5, 17] {
            let l = parse_3dl(&identity_3dl(&[0, 0, 512, 1023], n)).unwrap();
            let at = |x: f32| sample(&l, [x, 0.0, 0.0])[0];
            assert!(at(1.0 / 3.0).abs() < 1e-6, "{n}³: {}", at(1.0 / 3.0));
            // The helper's 12-bit nodes are truncated, so the table itself is within a 12-bit step.
            assert!((at(2.0 / 3.0) - 512.0 / 1023.0).abs() < 1.0 / 4095.0, "{n}³: {}", at(2.0 / 3.0));
            assert!((at(0.5) - 256.0 / 1023.0).abs() < 1.0 / 4095.0, "{n}³: halfway between the knots");
        }
    }

    /// A two-value shaper row (a 2³ table's) is applied like a longer one, not dropped; the even
    /// two-point ramp changes nothing.
    #[test]
    fn three_dl_two_point_shaper_is_applied() {
        let bent = parse_3dl(&identity_3dl(&[0, 512], 2)).unwrap();
        let top = sample(&bent, [1.0, 1.0, 1.0]);
        assert!(top.iter().all(|v| (v - 512.0 / 1023.0).abs() < 1e-4), "{top:?}");
        assert!((sample(&bent, [0.5, 0.0, 0.0])[0] - 256.0 / 1023.0).abs() < 1e-4);
        let even = parse_3dl(&identity_3dl(&[0, 1023], 2)).unwrap();
        assert_eq!(
            even,
            parse_3dl(&identity_3dl(&[0, 1023], 2).lines().skip(1).collect::<Vec<_>>().join(
                "
"
            ))
            .unwrap()
        );
        // A second two-value row is a second shaper, refused like a longer one.
        let body = identity_3dl(&[0, 512], 2);
        assert!(
            parse_3dl(&format!(
                "{body}0 512
"
            ))
            .is_err()
        );
    }

    /// #9170 review (bommerts): a shaper row with a token that isn't a number, or a second
    /// shaper row, is refused rather than silently dropped.
    #[test]
    fn three_dl_malformed_shaper_rows_are_refused() {
        let table = identity_3dl(&[0, 1, 2, 3], 4);
        let body = table.split_once('\n').unwrap().1;
        assert!(parse_3dl(&format!("0 341 junk 682 1023\n{body}")).is_err(), "a junk token");
        assert!(parse_3dl(&format!("0 114 455 1023\n0 114 455 1023\n{body}")).is_err(), "two shaper rows");
        assert!(parse_3dl(&format!("0 114 455 1023\n{body}")).is_ok());
    }

    /// #9170 review (bommerts): the standard `3DMESH` / `Title lut 12` preamble still reads; only
    /// the rows after it are parsed strictly.
    #[test]
    fn three_dl_mesh_preamble_is_accepted() {
        let grey = |l: &LutFile| (sample(l, [128.0 / 255.0; 3])[0] * 255.0).round();
        let plain = identity_3dl(&[0, 114, 455, 1023], 4);
        let headed = parse_3dl(&format!("3DMESH\nTitle lut 12\n{plain}")).unwrap();
        assert_eq!(headed, parse_3dl(&plain).unwrap(), "the preamble changes nothing");
        assert_eq!(grey(&headed), 71.0, "the bent shaper still applies");
        let even = identity_3dl(&[0, 341, 682, 1023], 4);
        assert_eq!(grey(&parse_3dl(&format!("3dmesh\nTitle lut 12\n{even}")).unwrap()), 128.0);
        assert!(
            parse_3dl(&format!("3DMESH\nTitle lut 12\n0 341 junk 682 1023\n{}", even.split_once('\n').unwrap().1)).is_err(),
            "still strict after it"
        );
    }

    /// #9170 review: a shaper that isn't finite is refused, not dropped, so a Color Lookup never
    /// renders the table alone for a file whose shaper it can't apply.
    #[test]
    fn three_dl_non_finite_shaper_is_refused() {
        let body = identity_3dl(&[0, 1, 2, 3], 4);
        let body = body.split_once('\n').unwrap().1;
        for shaper in ["0 NaN 682 1023", "0 341 inf 1023", "-inf 341 682 1023"] {
            assert!(parse_3dl(&format!("{shaper}\n{body}")).is_err(), "{shaper}");
        }
    }

    /// #9170 review: a shaper row that starts with `NaN` or `inf` is a shaper, not a header, so
    /// it reaches the non-finite refusal instead of being skipped.
    #[test]
    fn three_dl_shaper_starting_non_finite_is_refused() {
        let body = identity_3dl(&[0, 1, 2, 3], 4);
        let body = body.split_once('\n').unwrap().1;
        for shaper in ["NaN 341 682 1023", "inf 341 682 1023", "-inf 341 682 1023"] {
            assert!(parse_3dl(&format!("{shaper}\n{body}")).is_err(), "{shaper}");
        }
    }

    /// #9170 review: only a line starting with a word is a header. One whose first token looks
    /// like a number but isn't (`0oops`) is read strictly and refused, not skipped with its shaper.
    #[test]
    fn three_dl_malformed_first_token_is_refused() {
        let body = identity_3dl(&[0, 1, 2, 3], 4);
        let body = body.split_once('\n').unwrap().1;
        for shaper in ["0oops 114 455 1023", "-x 114 455 1023", ".5x 114 455 1023", "+ 114 455 1023"] {
            assert!(parse_3dl(&format!("{shaper}\n{body}")).is_err(), "{shaper}");
        }
        let (first, rest) = body.split_once('\n').unwrap();
        assert!(parse_3dl(&format!("0 114 455 1023\n{first}\n1x 0 0\n{rest}")).is_err(), "a table row too");
        assert!(parse_3dl(&format!("3DMESH\nTitle lut 12\n0 114 455 1023\n{body}")).is_ok(), "headers still skip");
    }

    /// #9170 review: a bent shaper whose knots can't all be kept within MAX_SIZE is refused rather
    /// than sampled where it may look even (here a 257-point shaper bent only at odd knots, whose
    /// 129-node fallback would read the identity).
    #[test]
    fn three_dl_shaper_too_fine_to_keep_is_refused() {
        let shaper: Vec<u32> = (0..257u32).map(|i| (i * 1023 / 256 + if i % 2 == 1 { 40 } else { 0 }).min(1023)).collect();
        assert!(parse_3dl(&identity_3dl(&shaper, 2)).is_err());
        // As many knots as nodes stays within reach.
        let same: Vec<u32> = (0..17u32).map(|i| (1023.0 * (f64::from(i) / 16.0).powi(2)).round() as u32).collect();
        assert!(parse_3dl(&identity_3dl(&same, 17)).is_ok());
    }

    #[test]
    fn cube_title_with_hash_round_trips() {
        let l = LutFile::from_fn("Look #3", 2, |c| c).unwrap();
        assert_eq!(parse_cube(&write_cube(&l)).unwrap().title, "Look #3");
        let t = "TITLE \"A #b\" # note\nLUT_3D_SIZE 2 # c\n# comment\n";
        let rows = "0 0 0 # x\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let p = parse_cube(&format!("{t}{rows}")).unwrap();
        assert_eq!(p.title, "A #b");
    }

    #[test]
    fn write_cube_keeps_non_default_domain() {
        let mut l = LutFile::identity(2).unwrap();
        l.domain_min = [0.1; 3];
        l.domain_max = [0.9; 3];
        let back = parse_cube(&write_cube(&l)).unwrap();
        assert_eq!(back.domain_min, [0.1; 3]);
        assert_eq!(back.domain_max, [0.9; 3]);
        l.domain_min = [0.0, 0.0, 0.5];
        l.domain_max = [1.0; 3];
        let back = parse_cube(&write_cube(&l)).unwrap();
        assert_eq!(back.domain_min, [0.0, 0.0, 0.5]);
        assert_eq!(back.domain_max, [1.0; 3]);
        assert!(!write_cube(&LutFile::identity(2).unwrap()).contains("DOMAIN"));
    }

    #[test]
    fn cube_domain_is_kept_and_validated() {
        let base = "TITLE \"t\"
LUT_3D_SIZE 2
";
        let rows = "0 0 0
0 0 1
0 1 0
0 1 1
1 0 0
1 0 1
1 1 0
1 1 1
";
        // The default domain parses as default.
        let d = parse_cube(&format!("{base}{rows}")).expect("default domain");
        assert!(d.domain_is_default());
        // DOMAIN_MIN/MAX are kept for the caller to act on.
        let d = parse_cube(&format!(
            "DOMAIN_MIN 0.1 0.2 0.3
DOMAIN_MAX 0.9 0.8 0.7
{base}{rows}"
        ))
        .expect("custom domain");
        assert_eq!(d.domain_min, [0.1, 0.2, 0.3]);
        assert_eq!(d.domain_max, [0.9, 0.8, 0.7]);
        assert!(!d.domain_is_default());
        // A degenerate domain is a parse error, not a division by zero later.
        let e = parse_cube(&format!(
            "DOMAIN_MIN 0.5 0.5 0.5
DOMAIN_MAX 0.5 1 1
{base}{rows}"
        ))
        .unwrap_err();
        assert!(e.0.contains("below DOMAIN_MAX"), "{e}");
    }

    #[test]
    fn cube_1d_narrow_domain_is_refused() {
        // #8913: the issue's fixture — a two-node ramp over 0.24..0.26. Baking it onto the
        // 33-point grid rendered grey-65 as 148 where Photoshop renders 190 (42-level error).
        // It must now be an actionable parse error rather than a silently wrong table.
        let e = parse_cube("LUT_1D_SIZE 2\nDOMAIN_MIN 0.24 0.24 0.24\nDOMAIN_MAX 0.26 0.26 0.26\n0 0 0\n1 1 1\n").unwrap_err();
        assert!(e.0.contains("changes faster than the 33-point grid"), "{e}");
        // The same domain via LUT_1D_INPUT_RANGE is refused identically.
        let e = parse_cube("LUT_1D_SIZE 2\nLUT_1D_INPUT_RANGE 0.24 0.26\n0 0 0\n1 1 1\n").unwrap_err();
        assert!(e.0.contains("changes faster"), "{e}");
        // A per-channel narrow domain names the offending channel.
        let e = parse_cube("LUT_1D_SIZE 2\nDOMAIN_MIN 0 0 0.24\nDOMAIN_MAX 1 1 0.26\n0 0 0\n1 1 1\n").unwrap_err();
        assert!(e.0.contains("blue"), "{e}");
    }

    #[test]
    fn cube_1d_wide_domain_still_bakes() {
        // A steep ramp over 0.1..0.17 spans two grid cells but still loses ~28 RGB8 levels
        // to the resample, so the measured-error check refuses it even though the old width
        // check let it through.
        let e = parse_cube("LUT_1D_SIZE 2\nLUT_1D_INPUT_RANGE 0.1 0.17\n0 0 0\n1 1 1\n").unwrap_err();
        assert!(e.0.contains("changes faster"), "steep ramp over 0.1..0.17 refused: {e}");
        // A gentle (here linear) curve over a grid-aligned domain bakes exactly and stays
        // accepted — the check measures the bake's error, not the domain's width. Value
        // assertions, not just a successful parse: the baked table must reproduce the curve.
        // Table row k sits at t = k/32 of the domain 0.25..0.75; the curve y = 0.4 + 0.4t
        // is gentle (a half-level per grid step), so the bake must reproduce it exactly.
        let mut table = String::from("LUT_1D_SIZE 33\nLUT_1D_INPUT_RANGE 0.25 0.75\n");
        for k in 0..33 {
            let y = 0.4 + 0.4 * (k as f32 / 32.0);
            table.push_str(&format!("{y} {y} {y}\n"));
        }
        let l = parse_cube(&table).expect("gentle curve over a non-default domain still bakes");
        let at = |r: usize| l.data[r * 3];
        // Grid entry r is input r/32: 0.5 is the middle of the domain (t=0.5) -> 0.6.
        assert!((at(16) - 0.6).abs() < 1.0 / 255.0, "baked value at 0.5 tracks the curve: {}", at(16));
        // Domain top (input 0.75 = grid 24, t=1) -> 0.8.
        assert!((at(24) - 0.8).abs() < 1.0 / 255.0, "baked value at the domain top tracks the curve: {}", at(24));
        // Below the domain saturates to the first row.
        assert!((at(8) - 0.4).abs() < 1.0 / 255.0, "below the domain saturates: {}", at(8));
        parse_cube("LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").expect("default domain");
    }

    #[test]
    fn cube_1d_domain_is_baked_into_the_resample() {
        // An identity ramp over the domain 0.25..0.75: 0.25 maps to 0, 0.5 to 0.5, 0.75 to 1.
        let table = "LUT_1D_SIZE 2\nLUT_1D_INPUT_RANGE 0.25 0.75\n0 0 0\n1 1 1\n";
        let l = parse_cube(table).expect("1D with input range");
        assert!(l.domain_is_default(), "the domain is baked in, so the cube is plain 0..1");
        let at = |r: usize| l.data[r * 3];
        // 33-point grid, red fastest: entry r on the red axis is input r / 32.
        assert!(at(0).abs() < 1e-6, "below the domain saturates to 0");
        assert!((at(16) - 0.5).abs() < 1e-5, "0.5 is the middle of 0.25..0.75: {}", at(16));
        assert!((at(8) - 0.0).abs() < 1e-5, "0.25 maps to 0: {}", at(8));
        assert!((at(24) - 1.0).abs() < 1e-5, "0.75 maps to 1: {}", at(24));
        assert!((at(32) - 1.0).abs() < 1e-6, "above the domain saturates to 1");
        // Per-channel DOMAIN_MIN/MAX work the same way.
        let l = parse_cube("LUT_1D_SIZE 2\nDOMAIN_MIN 0 0 0.5\nDOMAIN_MAX 1 1 1\n0 0 0\n1 1 1\n").expect("1D domain");
        let blue_mid = l.data[(16 * 33 * 33) * 3 + 2];
        assert!(blue_mid.abs() < 1e-5, "blue 0.5 is the bottom of its domain: {blue_mid}");
        // A degenerate 1D domain is rejected like the 3D one.
        let e = parse_cube("LUT_1D_SIZE 2\nLUT_1D_INPUT_RANGE 1 1\n0 0 0\n1 1 1\n").unwrap_err();
        assert!(e.0.contains("below DOMAIN_MAX"), "{e}");
    }

    #[test]
    fn cube_roundtrip_and_identity() {
        let id = LutFile::identity(5).unwrap();
        let text = write_cube(&LutFile { title: "Id".into(), ..id.clone() });
        let back = parse_cube(&text).unwrap();
        assert_eq!(back.size, 5);
        assert_eq!(back.title, "Id");
        for (a, b) in back.data.iter().zip(&id.data) {
            assert!((a - b).abs() < 1e-5);
        }
        // Red varies fastest: entry 1 is (0.25, 0, 0).
        assert_eq!(&back.data[3..6], &[0.25, 0.0, 0.0]);
    }

    #[test]
    fn cube_1d_expands() {
        let text = "LUT_1D_SIZE 2\n0 0 0\n0.5 1 1\n";
        let l = parse_cube(text).unwrap();
        assert_eq!(l.size, 33);
        let last = &l.data[l.data.len() - 3..];
        assert!((last[0] - 0.5).abs() < 1e-6 && (last[1] - 1.0).abs() < 1e-6);

        let mut larger = String::from("LUT_1D_SIZE 256\n");
        for i in 0..256 {
            let value = i as f32 / 255.0;
            larger.push_str(&format!("{value} {value} {value}\n"));
        }
        assert_eq!(parse_cube(&larger).unwrap().size, 33);
    }

    #[test]
    fn cube_1d_size_multiplication_overflow_is_rejected() {
        let text = format!("LUT_1D_SIZE {}\n", usize::MAX);
        assert!(parse_cube(&text).is_err());
    }

    #[test]
    fn three_dl_without_a_header_reads_low_nodes_as_12_bit() {
        let table = |peak: u32| -> String {
            let mut text = String::from("0 1023\n");
            for i in 0..8 {
                let on = |bit: u32| if i & bit != 0 { peak } else { 0 };
                text.push_str(&format!("{} {} {}\n", on(4), on(2), on(1)));
            }
            text
        };
        for peak in [255, 1023, 4095] {
            let white = parse_3dl(&table(peak)).unwrap().data[21..24].to_vec();
            assert_eq!(white, [peak as f32 / 4095.0; 3], "peak {peak}");
        }
        // A header keeps the inferred 10-bit depth. The bare `3DMESH` magic is a header without a
        // cube declaration; `Mesh 1 10` would declare a 3³ cube and refuse this 2³ table (#9078).
        let white = parse_3dl(&format!("3DMESH\n{}", table(1023))).unwrap().data[21..24].to_vec();
        assert_eq!(white, [1.0; 3]);
    }

    #[test]
    fn three_dl_blue_fastest_and_bit_depth() {
        let mut text = String::from("0 1023\n");
        for r in 0..2 {
            for g in 0..2 {
                for b in 0..2 {
                    text.push_str(&format!("{} {} {}\n", r * 4095, g * 4095, b * 4095));
                }
            }
        }
        let l = parse_3dl(&text).unwrap();
        assert_eq!(l.size, 2);
        assert_eq!(l.data, LutFile::identity(2).unwrap().data);
    }

    /// A `size`³ inverse-identity `.3dl` (blue fastest): node (r,g,b) outputs `max - max·v/(size-1)`
    /// per channel, with a uniform shaper of `size` codes up to 1023.
    fn mesh_3dl(n_bits: u32, out_bits: u32, size: usize, max: f64, shaper: bool) -> String {
        let mut text = format!("3DMESH\nMesh {n_bits} {out_bits}\n");
        if shaper {
            for i in 0..size {
                if i > 0 {
                    text.push(' ');
                }
                text.push_str(&format!("{}", (1023.0 * i as f64 / (size - 1) as f64).round()));
            }
            text.push('\n');
        }
        for r in 0..size {
            for g in 0..size {
                for b in 0..size {
                    let v = [r, g, b].map(|c| (max - max * c as f64 / (size - 1) as f64).round());
                    text.push_str(&format!("{} {} {}\n", v[0], v[1], v[2]));
                }
            }
        }
        text
    }

    #[test]
    fn three_dl_mesh_three_point_shaper_is_not_a_node() {
        // #9078: Photoshop loads a 3³ cube whose shaper line has three values; counting it as a
        // 28th node rejected the whole file.
        let l = parse_3dl(&mesh_3dl(1, 12, 3, 4095.0, true)).unwrap();
        assert_eq!(l.size, 3);
        assert_eq!(l.data.len(), 3 * 3 * 3 * 3);
        // The inverse identity: node (0,0,0) is full, node (2,2,2) black.
        assert!((l.data[0] - 1.0).abs() < 1e-6 && (l.data[1] - 1.0).abs() < 1e-6 && (l.data[2] - 1.0).abs() < 1e-6);
        let last = &l.data[l.data.len() - 3..];
        assert_eq!(last, [0.0, 0.0, 0.0]);
        // The same 27 nodes without the shaper row still load (the node-only control).
        assert_eq!(parse_3dl(&mesh_3dl(1, 12, 3, 4095.0, false)).unwrap().data, l.data);
    }

    #[test]
    fn three_dl_mesh_declares_the_output_depth() {
        // #9073: identical node codes divide by the declared range, not by the largest value.
        let mut text_10 = String::new();
        let mut text_12 = String::new();
        for (n_bits, out_bits, dst) in [(2, 10, &mut text_10), (2, 12, &mut text_12)] {
            // A 5³ identity scaled to 10-bit codes (max 1023).
            *dst = format!("3DMESH\nMesh {n_bits} {out_bits}\n0 256 512 768 1023\n");
            for r in 0..5 {
                for g in 0..5 {
                    for b in 0..5 {
                        dst.push_str(&format!("{} {} {}\n", r * 255, g * 255, b * 255));
                    }
                }
            }
        }
        let white_10 = parse_3dl(&text_10).unwrap();
        let white_12 = parse_3dl(&text_12).unwrap();
        assert_eq!(white_10.size, 5);
        let last_10 = &white_10.data[white_10.data.len() - 3..];
        let last_12 = &white_12.data[white_12.data.len() - 3..];
        assert!((last_10[0] - 1020.0 / 1023.0).abs() < 1e-6, "{last_10:?}");
        // 1020/4095 ≈ 0.249: white renders grey, as Photoshop measures it.
        assert!((last_12[0] - 1020.0 / 4095.0).abs() < 1e-6, "{last_12:?}");
    }

    #[test]
    fn three_dl_mesh_header_mismatch_is_rejected() {
        // A Mesh header that disagrees with the row count is refused, not silently re-inferred.
        assert!(parse_3dl("3DMESH\nMesh 1 12\n0 0 0\n0 0 1\n").is_err());
        // A node beyond the declared range is a wrong declaration: the old inference from the
        // largest value keeps the file loading as it did before the header was honoured.
        let mut text = String::from("3DMESH\nMesh 1 10\n");
        for r in 0..3 {
            for g in 0..3 {
                for b in 0..3 {
                    text.push_str(&format!("{} {} {}\n", r * 2047, g * 2047, b * 2047));
                }
            }
        }
        let l = parse_3dl(&text).unwrap();
        let last = &l.data[l.data.len() - 3..];
        assert!((last[0] - 4094.0 / 4095.0).abs() < 1e-6, "{last:?}");
    }

    #[test]
    fn three_dl_mesh_extra_node_cannot_replace_a_captured_shaper() {
        for (bits, size) in [(2, 5), (3, 9)] {
            let mut text = mesh_3dl(bits, 12, size, 4095.0, true);
            let valid = parse_3dl(&text).unwrap();
            assert_eq!(valid.size, size);
            assert_eq!(valid.data, parse_3dl(&mesh_3dl(bits, 12, size, 4095.0, false)).unwrap().data);
            text.push_str("4095 4095 4095\n");
            let error = parse_3dl(&text).unwrap_err().to_string();
            assert!(error.contains(&format!("expected {} rows", size * size * size)), "{error}");
        }
    }

    #[test]
    fn three_dl_malformed_mesh_header_falls_back_to_inference() {
        // A `Mesh` line that is not exactly two integer fields is not a declaration: it is
        // ignored and the table is inferred as if headerless, rather than recovering numbers
        // from the wrong positions (#9078). A 2³ cube would otherwise be read as a 3³ cube.
        for header in ["Mesh invalid 1 12", "Mesh 1 12 13", "Mesh 1 x", "Mesh 1"] {
            let mut text = format!("3DMESH\n{header}\n");
            for r in 0..2 {
                for g in 0..2 {
                    for b in 0..2 {
                        text.push_str(&format!("{} {} {}\n", r * 1023, g * 1023, b * 1023));
                    }
                }
            }
            let l = parse_3dl(&text).unwrap();
            assert_eq!(l.size, 2, "{header}");
            assert_eq!(l.data, LutFile::identity(2).unwrap().data, "{header}");
        }
    }

    #[test]
    fn three_dl_mesh_uses_declared_range_for_small_integer_codes() {
        // #9073: under a valid `Mesh` declaration the node codes are integers in the declared
        // range, so a max value of 1 is 1/4095 of full output — not full output itself.
        let mut text = String::from("3DMESH\nMesh 1 12\n");
        for r in 0..3 {
            for g in 0..3 {
                for b in 0..3 {
                    let v = u8::from(r == 0 && g == 0 && b == 0);
                    text.push_str(&format!("{v} {v} {v}\n"));
                }
            }
        }
        let l = parse_3dl(&text).unwrap();
        assert_eq!(l.size, 3);
        // blue fastest: node (0,0,0) is the first entry, code 1 → 1/4095.
        assert!((l.data[0] - 1.0 / 4095.0).abs() < 1e-6, "{:?}", &l.data[0..3]);
        assert!(l.data[3..].iter().all(|&v| v == 0.0), "{:?}", l.data);
    }

    #[test]
    fn look_data_is_little_endian_float32() {
        // Bytes written out by hand rather than by the encoder the parser mirrors, so a wrong byte
        // order fails: 0.25, 0.5, 1.0 (3E800000, 3F000000, 3F800000 stored low byte first), then zeros.
        let hex = format!("0000803E 0000003F 0000803F {}", "00000000 ".repeat(21));
        let xml = format!("<?xml version=\"1.0\"?><look><LUT><size>\"2\"</size><data>\"{hex}\"</data></LUT></look>");
        let data = parse("a.look", xml.as_bytes()).unwrap().data;
        assert_eq!(data[..3], [0.25, 0.5, 1.0]);
        assert!(data[3..].iter().all(|&v| v == 0.0));
    }

    #[test]
    fn look_reads_utf16_with_either_byte_order() {
        let id = LutFile::identity(2).unwrap();
        let hex: String = id.data.iter().flat_map(|v| v.to_le_bytes()).map(|b| format!("{b:02X}")).collect();
        let xml = format!("<?xml version=\"1.0\" encoding=\"UTF-16\"?><look><LUT><size>\"2\"</size><data>\"{hex}\"</data></LUT></look>");
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain(xml.encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain(xml.encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(parse("a.look", &le).unwrap().data, id.data);
        assert_eq!(parse("a.look", &be).unwrap().data, id.data);
        // Sniffed as XML through the byte-order mark too, so the recorded format is the one parsed.
        assert_eq!(Format::detect("a.lut", &le), Format::Look);
        assert_eq!(parse_as(Format::Look, &be).unwrap().data, id.data);
    }

    #[test]
    fn look_ignores_tags_inside_xml_comments() {
        let hex = |l: &LutFile| -> String { l.data.iter().flat_map(|v| v.to_le_bytes()).map(|b| format!("{b:02X}")).collect() };
        let inverse = LutFile::from_fn("", 2, |[r, g, b]| [1.0 - r, 1.0 - g, 1.0 - b]).unwrap();
        let xml = format!(
            "<look><!-- <size>\"3\"</size><data>\"{}\"</data> --><LUT><size>\"2\"</size><data>\"{}\"</data></LUT><!-- unterminated <data>00",
            hex(&LutFile::identity(2).unwrap()),
            hex(&inverse)
        );
        assert_eq!(parse("commented.look", xml.as_bytes()).unwrap().data, inverse.data);
    }

    #[test]
    fn look_rgb_and_rgba_allow_whitespace_in_hex_data() {
        let id = LutFile::identity(2).unwrap();
        for channels in [3, 4] {
            let mut hex = String::new();
            for rgb in id.data.as_chunks::<3>().0 {
                for value in rgb.iter().copied().chain((channels == 4).then_some(1.0)) {
                    for byte in value.to_le_bytes() {
                        hex.push_str(&format!("{byte:02x} \t\r\n"));
                    }
                }
            }
            let xml = format!("<look><size>2</size><data>\"{hex}\"</data></look>");
            assert_eq!(parse("valid.look", xml.as_bytes()).unwrap().data, id.data);
        }
    }

    #[test]
    fn look_rejects_partial_floats_and_invalid_hex() {
        let id = LutFile::identity(2).unwrap();
        let hex: String = id.data.iter().flat_map(|v| v.to_le_bytes()).map(|b| format!("{b:02X}")).collect();
        for suffix in ["0", "00", "0000", "000000", "G", "é"] {
            let xml = format!("<look><size>2</size><data>{hex}{suffix}</data></look>");
            assert!(parse("bad.look", xml.as_bytes()).is_err(), "accepted malformed suffix {suffix:?}");
        }
        let invalid = hex.replacen('0', "G", 1);
        let xml = format!("<look><size>2</size><data>{invalid}</data></look>");
        assert!(parse("bad.look", xml.as_bytes()).unwrap_err().0.contains("invalid hexadecimal"));
        let xml = format!("<look><size>2</size><data>{hex}00</data></look>");
        assert!(parse("bad.look", xml.as_bytes()).unwrap_err().0.contains("incomplete float"));
    }

    #[test]
    fn look_3d_size_exponent_overflow_is_rejected() {
        let text = format!("<look><size>{}</size><data>00</data></look>", usize::MAX);
        assert!(parse_look(&text).is_err());
    }

    #[test]
    fn format_detection_matches_parse() {
        assert_eq!(Format::detect("a.3DL", b"<look>"), Format::ThreeDl);
        assert_eq!(Format::detect("a.Look", b""), Format::Look);
        assert_eq!(Format::detect("a.cube", b"  <look>"), Format::Look, "XML is sniffed as .look");
        assert_eq!(Format::detect("a.txt", b"LUT_3D_SIZE 2"), Format::Cube);
        assert_eq!(Format::ThreeDl.extension(), "3dl");
        let text = write_cube(&LutFile::identity(2).unwrap());
        assert_eq!(parse_as(Format::Cube, text.as_bytes()), parse("a.cube", text.as_bytes()));
    }

    #[test]
    fn malformed_rejected() {
        assert!(parse_cube("LUT_3D_SIZE 2\n0 0 0\n").is_err());
        assert!(parse_cube("garbage").is_err());
        assert!(parse_cube("LUT_3D_SIZE 99999\n").is_err());
        assert!(parse_3dl("1 2 3\n4 5 6\n").is_err());
        assert!(parse_look("<look></look>").is_err());
    }

    #[test]
    fn builtins_exist_and_are_in_range() {
        for (id, _) in BUILTIN {
            let l = builtin(id).unwrap();
            assert_eq!(l.data.len(), 33 * 33 * 33 * 3);
            assert!(l.data.iter().all(|v| (0.0..=1.0).contains(v)), "{id}");
        }
        assert!(builtin("nope").is_none());
    }

    #[test]
    fn cube_domain_survives_a_write() {
        let cube = "DOMAIN_MIN 0.25 0.25 0.25
DOMAIN_MAX 0.75 0.75 0.75
LUT_3D_SIZE 2
0 0 0
0 0 1
0 1 0
0 1 1
1 0 0
1 0 1
1 1 0
1 1 1
";
        let l = parse_cube(cube).expect("a cube with a non-default domain");
        assert!(!l.domain_is_default());
        let back = parse_cube(&write_cube(&l)).expect("roundtrip");
        assert_eq!(back.domain_min, l.domain_min, "the written table must mean the same");
        assert_eq!(back.domain_max, l.domain_max);
    }
}
