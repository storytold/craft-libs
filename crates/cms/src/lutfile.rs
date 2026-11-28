//! Creative 3D LUT files for Image › Adjustments › Color Lookup: `.cube` (Adobe/Resolve), `.3dl`
//! (Autodesk Lustre / Flame) and `.look` (SpeedGrade XML), plus a few generated (CC0) looks.
//!
//! Every table is normalised to [`LutFile`]: `size`³ RGB triplets in 0..=1, **red varying
//! fastest** (`((b·size + g)·size + r)·3`). A `.3dl` also carries its input shaper beside the
//! table, as its own 1-D step: [`sample`] shapes the input and then reads the table, which is the
//! order Photoshop renders in (#11452). Formats follow their public descriptions: the Adobe
//! "Cube LUT Specification 1.0", the Lustre 3DL layout as documented by OpenColorIO, and the
//! SpeedGrade `.look` XML (hex-encoded little-endian float32 RGB triplets).

/// A parsed 3D LUT.
#[derive(Clone, Debug, PartialEq)]
pub struct LutFile {
    /// Title from the file, or empty when it has none.
    pub title: String,
    /// Edge length of the cube: the table has `size` points per axis.
    pub size: usize,
    /// `size`³ × 3 values, red fastest.
    pub data: Vec<f32>,
    /// The `.3dl` input shaper as its 0..=1 knots: a 1-D curve on every input channel *ahead of*
    /// the table, which [`sample`] applies as a pre-step (#11452, #8982). `None` for every other
    /// format, for a file without a shaper row, and for the even ramp, which changes nothing.
    /// Baking it into `data` instead is only exact for an identity table: the composed function
    /// has breakpoints where the shaper reaches a table node value, and no uniform cube grid holds
    /// those in general. [`write_3dl`] is the writer that carries it.
    pub shaper: Option<std::sync::Arc<Vec<f32>>>,
    /// The input domain the table expects (`.cube` `DOMAIN_MIN`/`DOMAIN_MAX`,
    /// `LUT_3D_INPUT_RANGE`): an input `c` samples the table at `(c - min) / (max - min)`,
    /// saturating outside. `[0, 1]` is the identity default; applying a table with any other
    /// domain is rejected upstream until Color Lookup carries it through.
    pub domain_min: [f32; 3],
    /// Upper end of the input domain per channel; see [`domain_min`](Self::domain_min).
    pub domain_max: [f32; 3],
}

/// Largest edge length accepted (64³ is Photoshop's practical maximum; 129 leaves headroom while
/// keeping the GPU table within texture limits).
pub const MAX_SIZE: usize = 129;

/// Why a LUT file could not be read; the string is a message for the user.
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
        Ok(LutFile { title: title.to_string(), size, data, shaper: None, domain_min: [0.0; 3], domain_max: [1.0; 3] })
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

/// Most files [`list_folder`] lists.
const MAX_FILES: usize = 500;
/// Largest file [`list_folder`] lists: a 129³ `.cube` is about 60 MB.
const MAX_BYTES: u64 = 64 << 20;

/// The `.cube`, `.3dl` and `.look` files directly in `dir` as (file name, path), sorted by name
/// without regard to case. An unreadable folder lists nothing; hidden files, subfolders and files
/// too large to be a LUT are skipped, and a symlink to a LUT file is listed.
pub fn list_folder(dir: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(String, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
            if name.starts_with('.') || !matches!(extension.as_str(), "cube" | "3dl" | "look") {
                return None;
            }
            let path = entry.path();
            let meta = std::fs::metadata(&path).ok()?;
            (meta.is_file() && meta.len() <= MAX_BYTES).then_some((name, path))
        })
        .collect();
    out.sort_by_key(|(name, _)| name.to_lowercase());
    out.truncate(MAX_FILES);
    out
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

/// A bounded, three-dimensional Cube profile with RGB rows in file order.
/// Unlike [`parse_cube`], this preserves the optional title and does not resample 1D curves.
#[derive(Clone, Debug, PartialEq)]
pub struct Cube3d {
    pub title: Option<String>,
    pub size: usize,
    pub data: Vec<[f32; 3]>,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
}

/// Read the simple 3D profile dialect used by camera-profile consumers.
///
/// `max_size` must be in 2..=256; the row budget is checked before each allocation.
/// Whole-line comments and literal titles are preserved. Unknown alphabetic directives
/// are ignored; 1D tables are refused. Finite domains are returned unchanged so the
/// consumer retains its existing policy for equal or reversed endpoints.
/// Numeric fields must consist of exactly three finite numbers (never silently skip tokens).
pub fn parse_cube_3d(text: &str, max_size: usize) -> Result<Cube3d, LutError> {
    let p = read_cube(text, Some(max_size))?;
    if p.size3 == 0 {
        return err("not a 3D .cube LUT (no LUT_3D_SIZE)");
    }
    let expected = p.size3.checked_pow(3).ok_or_else(|| LutError("LUT size overflow".into()))?;
    if p.rows.len() != expected {
        return err(format!("expected {expected} entries, found {}", p.rows.len()));
    }
    Ok(Cube3d { title: p.title, size: p.size3, data: p.rows, domain_min: p.min, domain_max: p.max })
}

struct CubeRows {
    title: Option<String>,
    size3: usize,
    size1: usize,
    min: [f32; 3],
    max: [f32; 3],
    rows: Vec<[f32; 3]>,
}

fn cube_triplet(text: &str) -> Result<[f32; 3], LutError> {
    let mut it = text.split_whitespace();
    let mut row = [0.0; 3];
    for value in &mut row {
        *value = it.next().and_then(|s| s.parse::<f32>().ok()).filter(|v| v.is_finite()).ok_or_else(|| LutError(format!("bad line `{text}`")))?;
    }
    if it.next().is_some() {
        return err(format!("bad line `{text}`"));
    }
    Ok(row)
}

fn read_cube(text: &str, profile_max: Option<usize>) -> Result<CubeRows, LutError> {
    if profile_max.is_some_and(|n| !(2..=256).contains(&n)) {
        return err("3D profile size limit must be in 2..=256");
    }
    let mut p = CubeRows { title: None, size3: 0, size1: 0, min: [0.0; 3], max: [1.0; 3], rows: Vec::new() };
    let row_limit = profile_max.unwrap_or(256).checked_pow(3).ok_or_else(|| LutError("LUT size overflow".into()))?;
    for raw in text.lines() {
        let profile = profile_max.is_some();
        let line = if profile { raw.trim() } else { raw.split('#').next().unwrap_or("").trim() };
        // The standard dialect protects '#' inside quoted titles; profile titles are literal.
        if !profile
            && let Some(quoted) =
                raw.trim_start().strip_prefix("TITLE").filter(|r| r.starts_with(char::is_whitespace)).and_then(|r| r.trim_start().strip_prefix('"'))
        {
            let Some((title, _)) = quoted.split_once('"') else { return err("unterminated quoted TITLE") };
            p.title = Some(title.to_string());
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, rest) = if profile {
            ["TITLE", "LUT_3D_SIZE", "LUT_1D_SIZE", "DOMAIN_MIN", "DOMAIN_MAX"]
                .into_iter()
                .find_map(|key| line.strip_prefix(key).map(|rest| (key, rest.trim())))
                .unwrap_or_else(|| line.split_once(char::is_whitespace).unwrap_or((line, "")))
        } else {
            line.split_once(char::is_whitespace).map_or((line, ""), |(key, rest)| (key, rest.trim()))
        };
        match key {
            "TITLE" => p.title = Some(rest.trim_matches('"').to_string()),
            "LUT_3D_SIZE" => {
                let value = if profile { rest } else { rest.split_whitespace().next().unwrap_or("") };
                p.size3 = value.parse().map_err(|_| LutError(format!("bad LUT_3D_SIZE `{value}`")))?;
                let limit = profile_max.unwrap_or(MAX_SIZE);
                if !(2..=limit).contains(&p.size3) {
                    return err(format!("LUT_3D_SIZE {} out of range", p.size3));
                }
            }
            "LUT_1D_SIZE" => {
                if profile {
                    return err("1D LUTs are not supported (only 3D .cube files)");
                }
                p.size1 = rest.split_whitespace().next().and_then(|s| s.parse().ok()).ok_or_else(|| LutError("bad LUT_1D_SIZE".into()))?;
                if p.size1 > row_limit {
                    return err("1D LUT exceeds row budget");
                }
            }
            "DOMAIN_MIN" => p.min = cube_triplet(rest).map_err(|_| LutError("bad DOMAIN_MIN".into()))?,
            "DOMAIN_MAX" => p.max = cube_triplet(rest).map_err(|_| LutError("bad DOMAIN_MAX".into()))?,
            "LUT_3D_INPUT_RANGE" | "LUT_1D_INPUT_RANGE" if !profile => {
                let mut it = rest.split_whitespace();
                let min = it.next().and_then(|s| s.parse::<f32>().ok());
                let max = it.next().and_then(|s| s.parse::<f32>().ok());
                if let (Some(min), Some(max), None) = (min, max, it.next()) {
                    p.min = [min; 3];
                    p.max = [max; 3];
                }
            }
            _ => {
                let numeric = key.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+' || c == '.');
                let nonfinite = key.parse::<f32>().is_ok();
                if numeric || nonfinite || (profile && !key.starts_with(|c: char| c.is_ascii_alphabetic())) {
                    let row = cube_triplet(line)?;
                    if p.rows.len() >= row_limit {
                        return err("LUT exceeds row budget");
                    }
                    // Explicit fallible growth prevents capacity growth past the configured budget.
                    if p.rows.len() == p.rows.capacity() {
                        let grow = (p.rows.capacity().max(1)).min(row_limit - p.rows.len());
                        p.rows.try_reserve_exact(grow).map_err(|_| LutError("LUT allocation failed".into()))?;
                    }
                    p.rows.push(row);
                }
            }
        }
    }
    Ok(p)
}

/// Adobe / Resolve `.cube`. 1D LUTs (`LUT_1D_SIZE`) are expanded to a 33³ table.
pub fn parse_cube(text: &str) -> Result<LutFile, LutError> {
    let p = read_cube(text, None)?;
    let (title, size3, size1, dmin, dmax) = (p.title.unwrap_or_default(), p.size3, p.size1, p.min, p.max);
    let rows: Vec<f32> = p.rows.into_iter().flatten().collect();
    let domain_ok = (0..3).all(|k| dmin[k] < dmax[k] && dmin[k].is_finite() && dmax[k].is_finite());
    if (size3 > 0 || size1 >= 2) && !domain_ok {
        return err(format!("DOMAIN_MIN {:?} must lie below DOMAIN_MAX {:?}", dmin, dmax));
    }
    if size3 > 0 {
        return LutFile { title, size: size3, data: rows, shaper: None, domain_min: dmin, domain_max: dmax }.check();
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
/// isn't the even ramp is kept as a 1-D curve on [`LutFile::shaper`], applied to the input ahead of
/// the table as Photoshop does (#8982, #11452): see [`shaper_knots`] and [`sample`].
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
    let shaper = shaper_knots(shaper.as_deref().unwrap_or(&[]))?;
    LutFile { title: String::new(), size: n, data, shaper, domain_min: [0.0; 3], domain_max: [1.0; 3] }.check()
}

/// The shaper row of a `.3dl` as the knots of its 1-D curve, scaled to 0..=1: `None` when there is
/// no row or the row is the even ramp (the usual shaper, and a no-op), an error when it holds a
/// value that isn't finite. Photoshop reads the row as a curve on each input channel ahead of the
/// table (#8982); keeping it as a curve instead of baking it into the table is what makes that
/// order exact for a table that isn't the identity (#11452).
///
/// The row's own depth is inferred from its largest value by [`inferred_3dl_scale`] (10-bit and
/// up, or 0..1 floats), independently of the `Mesh` output depth, which governs only the table.
/// [`write_3dl`] picks the codes it writes through the same function, so a row written there is
/// read back at the scale it was written at. A `0..1023` ramp is the even ramp with or without a
/// header. Shapers round their ramp, so a knot within one code of even is still even: one code
/// for an integer shaper, one 10-bit step for a float one (0..1).
fn shaper_knots(shaper: &[f64]) -> Result<Option<std::sync::Arc<Vec<f32>>>, LutError> {
    let m = shaper.len();
    if m < 2 {
        return Ok(None);
    }
    // Apply or refuse: a shaper that can't be applied never leaves the table to render alone.
    if shaper.iter().any(|v| !v.is_finite()) {
        return err("the .3dl shaper holds a value that isn't finite");
    }
    let scale = inferred_3dl_scale(shaper.iter().fold(0.0f64, |a, &b| a.max(b)), true);
    let even = |k: usize| scale * k as f64 / (m - 1) as f64;
    let code = if scale > 1.0 { 1.0 } else { 1.0 / 1023.0 };
    if shaper.iter().enumerate().all(|(k, &v)| (v - even(k)).abs() <= code) {
        return Ok(None);
    }
    // Scaled to 0..1 before any interpolation, so interpolating between two extreme values
    // can't overflow.
    Ok(Some(std::sync::Arc::new(shaper.iter().map(|&v| (v / scale) as f32).collect())))
}

/// The `.3dl` shaper's curve at `v`: knots evenly spaced over 0..=1, linear between them, input
/// and result clamped to 0..=1. Fewer than two knots is the identity, so a caller can pass
/// [`LutFile::shaper`]'s value without a branch. One curve bends every input channel.
///
/// The segment is interpolated in f64 as `a*(1-f) + b*f`, which holds both ends, rather than
/// `a + (b-a)*f`: a shaper row is hostile input, and with a knot far outside 0..=1 `b - a` is as
/// wide as the knot is deep, so rounding swallows the range and white comes out black; between
/// opposite-signed extremes it overflows, and `inf * 0` is NaN at the segment's own endpoints. A
/// convex combination can neither cancel nor overflow. A NaN `v` reads as 0.
pub fn shape(knots: &[f32], v: f32) -> f32 {
    let m = knots.len();
    if m < 2 {
        return v;
    }
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    let p = f64::from(v) * (m - 1) as f64;
    let i = (p.floor() as usize).min(m - 2);
    let f = p - i as f64;
    let at = |k: usize| f64::from(knots.get(k).copied().unwrap_or(0.0));
    let (a, b) = (at(i), at(i + 1));
    (a * (1.0 - f) + b * f).clamp(0.0, 1.0) as f32
}

/// Trilinear lookup of `rgb` (0..1) in a [`LutFile`] (red fastest), after its shaper's 1-D
/// pre-step: the order Photoshop renders a `.3dl` in (#8982, #11452).
pub fn sample(l: &LutFile, rgb: [f32; 3]) -> [f32; 3] {
    let rgb = match l.shaper.as_deref() {
        Some(k) => [shape(k, rgb[0]), shape(k, rgb[1]), shape(k, rgb[2])],
        None => rgb,
    };
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
    LutFile { title: tag("title").unwrap_or_default(), size, data, shaper: None, domain_min: [0.0; 3], domain_max: [1.0; 3] }.check()
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
    cube_rows(s, l.data.as_chunks::<3>().0.iter().copied())
}

/// Writes a `.cube` file with a creator comment, section comments and an explicit input domain.
///
/// This layout is useful for exported LUTs; [`write_cube`] keeps its compact layout for embedded
/// tables. Values retain six decimal places, and their order and quantisation belong to the
/// caller. A default domain is written as `0.0` and `1.0` for compatibility with export consumers.
pub fn write_cube_commented(l: &LutFile, creator: &str) -> String {
    let mut s = format!("#Created by: {creator}\nTITLE \"{}\"\n\n#LUT size\nLUT_3D_SIZE {}\n\n#data domain\n", l.title.replace('"', "'"), l.size);
    if l.domain_is_default() {
        s.push_str("DOMAIN_MIN 0.0 0.0 0.0\nDOMAIN_MAX 1.0 1.0 1.0\n");
    } else {
        s.push_str(&format!("DOMAIN_MIN {} {} {}\n", l.domain_min[0], l.domain_min[1], l.domain_min[2]));
        s.push_str(&format!("DOMAIN_MAX {} {} {}\n", l.domain_max[0], l.domain_max[1], l.domain_max[2]));
    }
    s.push_str("\n#LUT data points\n");
    cube_rows(s, l.data.as_chunks::<3>().0.iter().copied())
}

fn cube_rows(mut s: String, rows: impl Iterator<Item = [f32; 3]>) -> String {
    for c in rows {
        s.push_str(&format!("{:.6} {:.6} {:.6}\n", c[0], c[1], c[2]));
    }
    s
}

/// A `.3dl` carrying `l`, for [`parse_3dl`] to read back: a `Mesh` line declaring the cube edge and
/// a 16-bit output, then the shaper row and the nodes as integer codes with **blue** varying
/// fastest. This is the only format here that carries a shaper, so it is what a lookup with a
/// curve is saved as (#11452).
///
/// A 2-node table, which `.3dl` cannot declare (the edge is `2^bits + 1` with `bits` in 1..=7, and
/// an undeclared 3-value shaper row would read as a node, #9078), is written as the 3-node table
/// that renders the same: the look is kept, curve included, rather than refused into a `.cube`
/// that would drop the curve (photocraft#11581 review). It reopens with `size` 3.
///
/// What `.3dl` cannot hold is refused with an error the caller should report, never written as
/// something else:
/// - an edge that isn't `2^bits + 1` (other than 2);
/// - a non-default input domain (the format has no such keyword);
/// - a shaper with fewer than two finite knots, or a knot above 1 (the reader never produces one,
///   and no row can state a scale for it).
///
/// The table's codes are integers on the declared 16-bit range, which the parser honours exactly
/// (#9073), so the table comes back within half a step (1.5e-5, a hundredth of an 8-bit code).
///
/// The shaper row's scale is not the table's: the reader infers it from the row's largest code
/// ([`shaper_knots`]), so the writer must pick codes that infer to the scale it wrote at. A row
/// whose largest knot rounds above 16383 at 65535 reads back as 16-bit and is written as 16-bit
/// integers. A lower-range row (every knot at most about 0.25) would be read at a smaller depth
/// than it was written at, so it is written as plain 0..1 decimals instead, which read back
/// exactly as floats (`[0, 1/1023, 2/1023]` stays `[0, 1/1023, 2/1023]`, not `[0, 64/1023,
/// 128/1023]`). A 3-knot row on a cube wider than 3 would read as a node row, so its midpoints
/// are added first: the same piecewise-linear curve with 5 knots. Knots below black are kept.
pub fn write_3dl(l: &LutFile) -> Result<String, LutError> {
    if !l.domain_is_default() {
        return err("a .3dl has no input domain keyword: the table's non-default domain can't be saved");
    }
    checked_data_len(l.size).and_then(|(_, len)| if l.data.len() == len { Ok(()) } else { err("the table doesn't match its size") })?;
    // A 2-node table has no `Mesh` edge (`2^bits + 1`, bits 1 to 7), so it is written as the 3-node
    // table holding the same function: trilinear interpolation of 2 nodes is multilinear, and its
    // samples at the halfway points interpolate back to it exactly. The shaper stays as it is.
    let refined;
    let l = if l.size == 2 {
        let flat = LutFile { shaper: None, ..l.clone() };
        refined = LutFile { shaper: l.shaper.clone(), ..LutFile::from_fn(&l.title, 3, |c| sample(&flat, c))? };
        &refined
    } else {
        l
    };
    let edge = l.size - 1;
    if !edge.is_power_of_two() || edge > 128 {
        return err(format!("a {}-node table can't be saved as .3dl: the edge must be 2^bits + 1 with bits 1 to 7", l.size));
    }
    let in_bits = edge.trailing_zeros();
    let mut s = format!("3DMESH\nMesh {in_bits} 16\n");
    if let Some(knots) = l.shaper.as_deref() {
        s.push_str(&shaper_row(knots, l.size)?);
        s.push('\n');
    }
    let scale = f64::from(u16::MAX);
    let node = |v: f32| (f64::from(v.clamp(0.0, 1.0)) * scale).round() as u32;
    for r in 0..l.size {
        for g in 0..l.size {
            for b in 0..l.size {
                let at = ((b * l.size + g) * l.size + r) * 3;
                let Some(c) = l.data.get(at..at + 3) else { return err("the table doesn't match its size") };
                s.push_str(&format!("{} {} {}\n", node(c[0]), node(c[1]), node(c[2])));
            }
        }
    }
    Ok(s)
}

/// The shaper row [`write_3dl`] writes for `knots` on a cube of `size` nodes, in a form
/// [`shaper_knots`] reads back at the scale it was written at.
fn shaper_row(knots: &[f32], size: usize) -> Result<String, LutError> {
    if knots.len() < 2 || knots.iter().any(|k| !k.is_finite()) {
        return err("a .3dl shaper needs at least two finite knots");
    }
    // A 3-value row is a node row to the reader unless the cube is 3 wide.
    let knots: Vec<f32> = if knots.len() == 3 && size != 3 {
        let mid = |a: f32, b: f32| ((f64::from(a) + f64::from(b)) / 2.0) as f32;
        vec![knots[0], mid(knots[0], knots[1]), knots[1], mid(knots[1], knots[2]), knots[2]]
    } else {
        knots.to_vec()
    };
    let max = knots.iter().fold(f64::NEG_INFINITY, |a, &k| a.max(f64::from(k)));
    if max > 1.0 {
        return err("a .3dl shaper knot above 1 can't be saved: the row has no way to state its scale");
    }
    let top = f64::from(u16::MAX);
    // `+ 0.0` turns a rounded -0 into 0.
    let codes = |k: f32| (f64::from(k) * top).round() + 0.0;
    let join = |it: Vec<String>| it.join(" ");
    if inferred_3dl_scale(codes(max as f32), true) == top {
        Ok(join(knots.iter().map(|&k| format!("{}", codes(k))).collect()))
    } else if inferred_3dl_scale(max, true) == 1.0 {
        // Decimals in 0..1: the reader takes a row that peaks at 1 or below as floats, exactly.
        Ok(join(knots.iter().map(|k| format!("{k}")).collect()))
    } else {
        err("the .3dl shaper has no unambiguous scale")
    }
}
/// Annotated identity-domain CUBE export, streamed in red-fastest order without a table copy.
/// Callers supply exactly `size³` rows for a valid size (2..=[`MAX_SIZE`]), as with [`write_cube`].
/// Integer-depth exports clamp/round to 1/32768 steps; floating exports clamp without rounding.
pub fn write_cube_export(creator: &str, title: &str, size: usize, integer_depth: bool, rows: impl Iterator<Item = [f32; 3]>) -> String {
    let s = format!(
        "#Created by: {creator}\nTITLE \"{}\"\n\n#LUT size\nLUT_3D_SIZE {size}\n\n#data domain\nDOMAIN_MIN 0.0 0.0 0.0\nDOMAIN_MAX 1.0 1.0 1.0\n\n#LUT data points\n",
        title.replace('"', "'")
    );
    cube_rows(
        s,
        rows.map(|c| {
            c.map(|v| {
                let v = v.clamp(0.0, 1.0);
                if integer_depth { (v * 32768.0).round() / 32768.0 } else { v }
            })
        }),
    )
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
    fn commented_cube_preserves_export_header_and_rows() {
        let mut lut = super::LutFile::identity(2).unwrap();
        lut.title = "A \"look\"".into();
        let text = super::write_cube_commented(&lut, "ExampleCraft");
        assert!(text.starts_with("#Created by: ExampleCraft\nTITLE \"A 'look'\"\n\n#LUT size\nLUT_3D_SIZE 2\n\n#data domain\nDOMAIN_MIN 0.0 0.0 0.0\nDOMAIN_MAX 1.0 1.0 1.0\n\n#LUT data points\n"));
        let parsed = super::parse_cube(&text).unwrap();
        assert_eq!(parsed.data, lut.data);
        assert_eq!(parsed.title, "A 'look'");
        assert_eq!(text.lines().count(), 11 + 8);
        let rows = |s: &str| s.lines().filter(|line| line.starts_with(|c: char| c.is_ascii_digit())).map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(rows(&text), rows(&super::write_cube(&lut)));
    }

    #[test]
    fn commented_cube_keeps_nondefault_domain_and_sample_precision() {
        let mut lut = super::LutFile::identity(2).unwrap();
        lut.domain_min = [0.125, 0.25, 0.375];
        lut.domain_max = [0.625, 0.75, 0.875];
        lut.data[0] = 0.833344;
        let parsed = super::parse_cube(&super::write_cube_commented(&lut, "ExampleCraft")).unwrap();
        assert_eq!(parsed, lut);
    }

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

    /// #9170 review: a finite but extreme shaper can't overflow into NaN, in the table or the curve.
    #[test]
    fn three_dl_extreme_shaper_keeps_only_finite_values() {
        let table = identity_3dl(&[0, 1, 2, 3], 2);
        let text = format!("0 1e308 -1e308 0\n{}", table.split_once('\n').unwrap().1);
        // Refused or kept, but never a table or a curve holding NaN.
        if let Ok(l) = parse_3dl(&text) {
            assert!(l.data.iter().all(|v| v.is_finite()), "non-finite table value");
            assert!(l.shaper.iter().all(|k| k.iter().all(|v| v.is_finite())), "non-finite knot");
            assert!(sample(&l, [0.5; 3]).iter().all(|v| v.is_finite()), "non-finite sample");
        }
    }
    /// #9170 review (bommerts): the shaper's breakpoints survive a table whose grid doesn't line
    /// up with them. The curve is its own step, so on an identity table it is exact: `0 0 512 1023` is 0 at ⅓ over 5³ and 17³ tables.
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

    /// #11452: the curve is applied as its own step, so a bent shaper finer than any table is kept
    /// at full resolution. #9170 had to hold its knots by baking the table on a grid of at most
    /// [`MAX_SIZE`] nodes and refused a file whose grid didn't fit there; there is no grid now.
    #[test]
    fn three_dl_shaper_finer_than_the_table_is_kept() {
        let codes: Vec<u32> = (0..257u32).map(|i| (i * 1023 / 256 + if i % 2 == 1 { 40 } else { 0 }).min(1023)).collect();
        let l = parse_3dl(&identity_3dl(&codes, 2)).unwrap();
        assert_eq!(l.shaper.as_deref().map(|k| k.len()), Some(257), "every knot survives");
        assert_eq!(l.size, 2, "the table keeps its own size");
        // Grey 128 lands between knots 128 and 129, which the odd knots lift by 40 codes: read as
        // even it would be 128, bent it is 133.
        let grey = sample(&l, [128.0 / 255.0; 3])[0] * 255.0;
        assert!((grey - 132.9).abs() < 0.5, "{grey}");
        // As many knots as nodes still loads.
        let same: Vec<u32> = (0..17u32).map(|i| (1023.0 * (f64::from(i) / 16.0).powi(2)).round() as u32).collect();
        assert!(parse_3dl(&identity_3dl(&same, 17)).is_ok());
    }

    /// A `Mesh 1 12` 3^3 `.3dl` under a 10-bit `shaper`, whose red output is `reds` (12-bit codes)
    /// and whose green and blue outputs are black: a table that isn't the identity.
    fn red_only_3dl(shaper: &[u32; 3], reds: &[u32; 3]) -> String {
        let mut t = format!("3DMESH\nMesh 1 12\n{} {} {}\n", shaper[0], shaper[1], shaper[2]);
        for red in reds {
            for _ in 0..9 {
                t += &format!("{red} 0 0\n");
            }
        }
        t
    }

    /// The piecewise-linear curve through `codes / scale`, at knots evenly spaced over 0..=1.
    fn ramp(codes: &[u32], scale: f64, x: f64) -> f64 {
        let p = x.clamp(0.0, 1.0) * (codes.len() - 1) as f64;
        let i = (p.floor() as usize).min(codes.len() - 2);
        let (a, b) = (f64::from(codes[i]) / scale, f64::from(codes[i + 1]) / scale);
        a + (b - a) * (p - i as f64)
    }

    /// #11452's worked example: the shaper is a 1-D pre-step, so a bent shaper ahead of a table that
    /// isn't the identity renders the composition, not an interpolation of a baked table.
    #[test]
    fn three_dl_shaper_before_a_non_identity_table_is_exact() {
        // Table red output [0, 0, 1] over nodes 0, 1/2, 1; shaper [0, 256, 1023] (10-bit). At red
        // 0.75 the curve gives 639.5 codes, which the table reads 1/4 of the way from its middle to
        // its top node: 256/1023 = 0.2502, the issue's "about 0.2502". Both pieces are linear
        // there, so the value is exact up to the f32 rounding of the knots.
        let l = parse_3dl(&red_only_3dl(&[0, 256, 1023], &[0, 0, 4095])).unwrap();
        assert!(l.shaper.is_some(), "the row is a shaper, not a 28th node");
        assert_eq!(l.size, 3, "the table keeps its own size");
        // The curve travels beside the table: the table's nodes are the file's, not a bake.
        let red = |r: usize| l.data[r * 3];
        assert_eq!((red(0), red(1), red(2)), (0.0, 0.0, 1.0));
        let at = |x: f32| sample(&l, [x, 0.0, 0.0])[0];
        assert!((at(0.75) - 256.0 / 1023.0).abs() < 1e-4, "shaper, then table: {}", at(0.75));
        // Baking the shaper into the 3-node table gave 0.5 here.
        assert!(at(0.75) < 0.3, "{}", at(0.75));
        assert_eq!(at(0.0), 0.0);
        assert!((at(1.0) - 1.0).abs() < 1e-6, "{}", at(1.0));
        assert!(at(0.5).abs() < 1e-6, "S(0.5) = 256/1023 is below the node at 0.5");
        assert_eq!(sample(&l, [0.75, 0.75, 0.75])[1], 0.0, "a red-only table stays red-only");
    }

    /// A linear shaper ahead of a linear table composes analytically, on every channel.
    #[test]
    fn three_dl_shaper_and_linear_table_compose() {
        // The table halves every channel (nodes 0, 1024, 2047 of 4095), the shaper is a 0.8 ramp
        // (codes 409 and 818 of 1023).
        let (table, shaper) = ([0u32, 1024, 2047], [0u32, 409, 818]);
        let mut text = format!("3DMESH\nMesh 1 12\n{} {} {}\n", shaper[0], shaper[1], shaper[2]);
        for red in &table {
            for green in &table {
                for blue in &table {
                    text += &format!("{red} {green} {blue}\n");
                }
            }
        }
        let l = parse_3dl(&text).unwrap();
        let want = |x: f64| ramp(&table, 4095.0, ramp(&shaper, 1023.0, x));
        for x in [0.0, 0.25, 0.4, 0.5, 0.62, 0.75, 1.0] {
            let got = sample(&l, [x as f32, (1.0 - x) as f32, (x * 0.5) as f32]);
            for (k, v) in [x, 1.0 - x, x * 0.5].into_iter().enumerate() {
                assert!((f64::from(got[k]) - want(v)).abs() < 1e-4, "channel {k} at {v}: {} vs {}", got[k], want(v));
            }
        }
        let shaped = sample(&l, [0.75, 0.0, 0.0])[0];
        assert!((f64::from(shaped) - ramp(&table, 4095.0, 0.75)).abs() > 0.05, "{shaped} is the unshaped table");
    }

    /// A hostile row must not lose the curve's endpoints to the segment arithmetic.
    #[test]
    fn shape_keeps_its_endpoints_under_extreme_knots() {
        // `a + (b - a) * f` with a knot a trillion codes under black cancels white to black.
        let deep = [-1e12_f32 / 1023.0, 1.0];
        assert_eq!(shape(&deep, 0.0), 0.0, "black stays black");
        assert_eq!(shape(&deep, 1.0), 1.0, "white stays white");
        assert_eq!(shape(&deep, 0.5), 0.0, "the lower half is still below black");
        // Opposite extremes: `b - a` overflows f32 to inf and inf * 0 is NaN at the endpoints.
        let wild = [3.4e38, -3.4e38];
        assert_eq!(shape(&wild, 0.0), 1.0);
        assert_eq!(shape(&wild, 1.0), 0.0);
        for v in [0.0, 0.25, 0.5, 0.75, 1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(shape(&wild, v).is_finite(), "{v} gave a non-finite curve");
        }
        assert_eq!(shape(&[], 0.3), 0.3, "no knots is the identity");
        let bent = [0.0, 256.0 / 1023.0, 1.0];
        assert!((f64::from(shape(&bent, 0.75)) - (256.0 / 1023.0 + 1.0) / 2.0).abs() < 1e-6);
        // End to end over an identity table: white still reaches white.
        let mut text = String::from("-1e12 1023\n");
        for r in 0..2 {
            for g in 0..2 {
                for b in 0..2 {
                    text += &format!("{} {} {}\n", r * 4095, g * 4095, b * 4095);
                }
            }
        }
        let l = parse_3dl(&text).unwrap();
        assert_eq!(sample(&l, [1.0; 3]), [1.0; 3]);
        assert_eq!(sample(&l, [0.0; 3]), [0.0; 3]);
    }

    /// The curves of `a` and `b` agree to `tol` on a spread of inputs, and so do the renders.
    fn assert_same_look(a: &LutFile, b: &LutFile, tol: f32) {
        for i in 0..=20 {
            let x = i as f32 / 20.0;
            for c in [[x, x, x], [x, 1.0 - x, 0.5 * x], [1.0 - x, 0.3, x]] {
                let (pa, pb) = (sample(a, c), sample(b, c));
                for k in 0..3 {
                    assert!((pa[k] - pb[k]).abs() <= tol, "{c:?}: {pa:?} vs {pb:?}");
                }
            }
        }
    }

    /// A table that is not the identity, so a shaper that moved is visible in the render.
    fn warped(size: usize) -> LutFile {
        LutFile::from_fn("warped", size, |c| [c[0] * c[0], 0.5 * c[1] + 0.25 * c[0], 1.0 - c[2]]).unwrap()
    }

    fn with_shaper(mut l: LutFile, knots: &[f32]) -> LutFile {
        l.shaper = Some(std::sync::Arc::new(knots.to_vec()));
        l
    }

    /// Review of photocraft#11581: `write_3dl` scaled every knot by 65535 while the reader infers the
    /// shaper's scale from its largest code, so `[0, 1/1023, 2/1023]` wrote `0 64 128` and read back
    /// as `[0, 64/1023, 128/1023]`: white changed from 0.001955 to 0.125122.
    #[test]
    fn write_3dl_keeps_a_low_range_shaper_across_a_save_and_reopen() {
        let knots = [0.0, 1.0 / 1023.0, 2.0 / 1023.0];
        let l = with_shaper(LutFile::identity(3).unwrap(), &knots);
        let back = parse_3dl(&write_3dl(&l).unwrap()).unwrap();
        let kept = back.shaper.clone().expect("the curve survives the save");
        assert_eq!(kept.len(), 3);
        for (a, b) in knots.iter().zip(kept.iter()) {
            assert!((a - b).abs() <= 1e-6, "{a} vs {b}");
        }
        let white = sample(&back, [1.0; 3]);
        assert!((white[0] - 2.0 / 1023.0).abs() < 1e-5, "white reloaded as {white:?}, was 0.001955");
        assert!(white[0] < 0.01, "the reviewer's wrong answer was 0.125122");
        assert_same_look(&l, &back, 2e-5);
    }

    /// Every depth the shaper can sit at round-trips, on every declarable table edge: small and
    /// large ranges, the threshold between the two forms, a knot below black, 2, 3 and many knots.
    #[test]
    fn write_3dl_round_trips_shapers_of_every_range() {
        let cases: [&[f32]; 9] = [
            &[0.0, 1.0 / 1023.0, 2.0 / 1023.0],
            &[0.0, 0.1, 0.2],
            &[0.0, 0.2, 0.25],
            &[0.0, 0.2501, 0.26],
            &[0.0, 0.5],
            &[0.0, 0.3, 1.0],
            &[0.0, 0.1, 0.9, 1.0],
            &[-0.1, 0.4, 1.0, 1.0],
            &[0.0, 0.01, 0.02, 0.1, 0.2, 0.21, 0.22, 0.23, 0.24],
        ];
        for knots in cases {
            for size in [3, 5, 9, 17] {
                let l = with_shaper(warped(size), knots);
                let text = write_3dl(&l).unwrap_or_else(|e| panic!("{knots:?} on {size}: {e}"));
                let back = parse_3dl(&text).unwrap_or_else(|e| panic!("{knots:?} on {size}: {e}\n{text}"));
                assert_eq!(back.size, size);
                assert!(back.shaper.is_some(), "{knots:?} on {size} lost its curve");
                assert_same_look(&l, &back, 3e-5);
            }
        }
    }

    /// A 3-value shaper row is a node row to the reader unless the cube is 3 wide, so a 3-knot curve
    /// on a wider cube is written with its midpoints added: the same piecewise-linear curve.
    #[test]
    fn write_3dl_writes_a_three_knot_shaper_on_a_wider_cube() {
        let l = with_shaper(warped(5), &[0.0, 0.3, 1.0]);
        let text = write_3dl(&l).unwrap();
        let back = parse_3dl(&text).unwrap();
        assert_eq!(back.size, 5);
        assert_eq!(back.shaper.as_deref().map(|k| k.len()), Some(5), "{text}");
        assert_same_look(&l, &back, 3e-5);
    }

    /// The table of a shaped lookup comes back within half a 16-bit step, with the curve.
    #[test]
    fn write_3dl_round_trips_a_shaped_table() {
        let l = parse_3dl(&red_only_3dl(&[0, 256, 1023], &[0, 0, 4095])).unwrap();
        let text = write_3dl(&l).unwrap();
        assert!(text.starts_with("3DMESH\nMesh 1 16\n"), "{text}");
        let back = parse_3dl(&text).unwrap();
        assert_eq!((back.size, back.data.len()), (l.size, l.data.len()));
        for (a, b) in l.data.iter().zip(&back.data) {
            assert!((a - b).abs() <= 1e-5, "{a} vs {b}");
        }
        assert!((sample(&back, [0.75, 0.0, 0.0])[0] - 256.0 / 1023.0).abs() < 1e-4, "the same look");
    }

    /// A shaped 2-node table has no `.3dl` edge to declare (`2^bits + 1`, bits 1 to 7), so it is
    /// written as the 3-node table with the same render, curve kept (photocraft#11581 review):
    /// saving it as `.cube` instead would drop the curve.
    #[test]
    fn write_3dl_keeps_the_look_of_a_shaped_two_node_table() {
        for knots in [&[0.0, 0.25, 1.0][..], &[0.0, 0.5][..]] {
            let l = with_shaper(warped(2), knots);
            let text = write_3dl(&l).unwrap();
            assert!(text.starts_with("3DMESH\nMesh 1 16\n"), "{text}");
            let back = parse_3dl(&text).unwrap();
            assert_eq!(back.size, 3);
            assert_eq!(back.shaper.as_deref().map(Vec::len), Some(knots.len()), "{text}");
            assert_same_look(&l, &back, 3e-5);
        }
        assert_same_look(&warped(2), &parse_3dl(&write_3dl(&warped(2)).unwrap()).unwrap(), 3e-5);
    }

    /// What `.3dl` cannot declare is refused, never written as something else.
    #[test]
    fn write_3dl_refuses_what_it_cannot_declare() {
        let bent = [0.0, 0.25, 0.5, 1.0];
        assert!(write_3dl(&with_shaper(LutFile::identity(32).unwrap(), &bent)).is_err(), "32 is not 2^bits + 1");
        assert!(write_3dl(&with_shaper(LutFile::identity(65).unwrap(), &bent)).is_ok(), "65 is declarable");
        let mut d = with_shaper(LutFile::identity(3).unwrap(), &bent);
        d.domain_min = [0.25; 3];
        assert!(write_3dl(&d).is_err(), "the format has no input domain");
        for knots in [&[0.0, 1.5][..], &[0.0][..], &[][..], &[0.0, f32::NAN, 1.0][..], &[0.0, f32::INFINITY][..]] {
            assert!(write_3dl(&with_shaper(LutFile::identity(3).unwrap(), knots)).is_err(), "{knots:?}");
        }
        let mut short = with_shaper(LutFile::identity(3).unwrap(), &bent);
        short.data.pop();
        assert!(write_3dl(&short).is_err(), "a table that doesn't match its size");
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
    fn annotated_cube_export_preserves_header_and_integer_precision() {
        let mut lut = LutFile::identity(2).unwrap();
        lut.title = "My \"Look\"".into();
        lut.data[..3].copy_from_slice(&[1.0 / 6.0, 5.0 / 6.0, 160.0 / 255.0]);
        let output = write_cube_export("PhotoCraft", &lut.title, lut.size, true, lut.data.as_chunks::<3>().0.iter().copied());
        assert!(output.starts_with("#Created by: PhotoCraft\nTITLE \"My 'Look'\"\n\n#LUT size\nLUT_3D_SIZE 2\n\n#data domain\nDOMAIN_MIN 0.0 0.0 0.0\nDOMAIN_MAX 1.0 1.0 1.0\n\n#LUT data points\n0.166656 0.833344 0.627441\n"));
        assert_eq!(output.lines().count(), 11 + 8);
        let parsed = parse_cube(&output).unwrap();
        assert_eq!(parsed.size, 2);
        assert_eq!(&parsed.data[3..], &lut.data[3..]);
        let float = write_cube_export("PhotoCraft", &lut.title, lut.size, false, lut.data.as_chunks::<3>().0.iter().copied());
        assert!(float.contains("0.166667 0.833333 0.627451\n"));
    }

    #[test]
    fn annotated_empty_title_and_float_clamping() {
        let text = write_cube_export("Exporter", "", 2, false, std::iter::repeat_n([-0.5, 0.25, 1.5], 8));
        assert!(text.contains("TITLE \"\"\n"));
        let back = parse_cube(&text).unwrap();
        assert_eq!(back.data, [0.0, 0.25, 1.0].repeat(8));
    }
}

#[cfg(test)]
mod profile_cube_tests {
    use super::*;

    fn cube(prefix: &str) -> String {
        format!("{prefix}\nLUT_3D_SIZE 2\n{}", "0.25 0.5 0.75\n".repeat(8))
    }

    #[test]
    fn profile_preserves_optional_literal_title_and_domains() {
        let no_title = parse_cube_3d(&cube(""), 256).unwrap();
        assert_eq!(no_title.title, None);
        let value = parse_cube_3d(&cube("TITLE \"look # literal\"\nDOMAIN_MIN 1 0 2\nDOMAIN_MAX 0 0 1"), 256).unwrap();
        assert_eq!(value.title.as_deref(), Some("look # literal"));
        assert_eq!(value.domain_min, [1.0, 0.0, 2.0]);
        assert_eq!(value.domain_max, [0.0, 0.0, 1.0]);
        assert_eq!(value.data, vec![[0.25, 0.5, 0.75]; 8]);
        assert_eq!(parse_cube_3d(&cube("TITLE \"\""), 256).unwrap().title.as_deref(), Some(""));
        assert!(parse_cube(&cube("DOMAIN_MIN 1 0 2\nDOMAIN_MAX 0 0 1")).is_err());
    }

    #[test]
    fn profile_limits_are_checked_without_large_allocations() {
        assert!(parse_cube_3d("LUT_3D_SIZE 256", 256).unwrap_err().0.contains("16777216"));
        assert!(parse_cube_3d("LUT_3D_SIZE 257", 256).unwrap_err().0.contains("out of range"));
        assert!(parse_cube_3d("LUT_3D_SIZE 256", 129).unwrap_err().0.contains("out of range"));
        assert!(parse_cube_3d(&cube(""), usize::MAX).is_err());
        assert!(parse_cube_3d(&cube(""), 1).is_err());
        assert!(parse_cube_3d(&format!("{}0 0 0\n", cube("")), 2).unwrap_err().0.contains("budget"));
        assert!(parse_cube_3d("LUT_1D_SIZE 2\n0 0 0\n1 1 1", 256).is_err());
    }

    #[test]
    fn every_numeric_token_must_be_valid_and_finite() {
        for row in ["0 ignored 0 0", "0 0 0 ignored", "NaN 0 0", "inf 0 0", "0 0 -inf", "1e39 0 0", "0 0", "0 0 0 0"] {
            assert!(parse_cube_3d(&format!("LUT_3D_SIZE 2\n{row}\n{}", "0 0 0\n".repeat(7)), 256).is_err(), "{row}");
            assert!(parse_cube_3d(&cube(&format!("DOMAIN_MIN {row}")), 256).is_err(), "{row}");
        }
    }

    #[test]
    fn standard_reader_keeps_quoted_comments_and_1d_expansion() {
        assert_eq!(parse_cube(&cube("TITLE \"look # preserved\" # comment")).unwrap().title, "look # preserved");
        let value = parse_cube("LUT_1D_SIZE 2\n0 0 0\n1 1 1").unwrap();
        assert_eq!(value.size, 33);
        assert!(parse_cube_3d(&cube("DOMAIN_MIN 0 0 0 # not a whole-line comment"), 256).is_err());
    }
}
