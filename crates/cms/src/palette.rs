//! Palette I/O for `.gpl` (GIMP Palette), `.css` (CSS custom properties, write-only),
//! `.ase` (Adobe Swatch Exchange, binary), and `.acb` (Adobe Color Book, read-only).

use std::collections::{HashMap, HashSet};

/// A colour in a palette swatch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaletteColor {
    Rgb { r: f32, g: f32, b: f32 },
    Cmyk { c: f32, m: f32, y: f32, k: f32 },
    Lab { l: f32, a: f32, b: f32 },
    Gray { k: f32 },
}

impl PaletteColor {
    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::Rgb { r, g, b }
    }

    pub fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Self::Rgb { r: f32::from(r) / 255.0, g: f32::from(g) / 255.0, b: f32::from(b) / 255.0 }
    }

    pub fn cmyk(c: f32, m: f32, y: f32, k: f32) -> Self {
        Self::Cmyk { c, m, y, k }
    }

    pub fn lab(l: f32, a: f32, b: f32) -> Self {
        Self::Lab { l, a, b }
    }

    pub fn gray(k: f32) -> Self {
        Self::Gray { k }
    }

    /// Approximate 8-bit sRGB representation for formats that only store RGB (`.gpl`, `.css`).
    pub fn to_rgb8(self) -> [u8; 3] {
        let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        match self {
            Self::Rgb { r, g, b } => [to_u8(r), to_u8(g), to_u8(b)],
            Self::Gray { k } => {
                let v = to_u8(1.0 - k.clamp(0.0, 1.0));
                [v, v, v]
            }
            Self::Cmyk { c, m, y, k } => {
                let (c, m, y, k) = (c.clamp(0.0, 1.0), m.clamp(0.0, 1.0), y.clamp(0.0, 1.0), k.clamp(0.0, 1.0));
                [to_u8((1.0 - c) * (1.0 - k)), to_u8((1.0 - m) * (1.0 - k)), to_u8((1.0 - y) * (1.0 - k))]
            }
            Self::Lab { l, a, b } => lab_to_srgb8(l, a, b),
        }
    }
}

fn lab_to_srgb8(l: f32, a: f32, b: f32) -> [u8; 3] {
    let fy = (l + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b / 200.0;
    let inv = |t: f32| {
        if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0 / 29.0) * (6.0 / 29.0) * (t - 4.0 / 29.0) }
    };
    // D65 white point
    let x = 0.95047 * inv(fx);
    let y = 1.00000 * inv(fy);
    let z = 1.08883 * inv(fz);
    let rl = 3.2404542 * x - 1.5371385 * y - 0.4985314 * z;
    let gl = -0.969_266 * x + 1.8760108 * y + 0.0415560 * z;
    let bl = 0.0556434 * x - 0.2040259 * y + 1.0572252 * z;
    let gamma = |u: f32| {
        let u = u.clamp(0.0, 1.0);
        let s = if u <= 0.0031308 { 12.92 * u } else { 1.055 * u.powf(1.0 / 2.4) - 0.055 };
        (s.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    [gamma(rl), gamma(gl), gamma(bl)]
}

/// A single named colour swatch in a [`Palette`].
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteSwatch {
    pub name: String,
    pub color: PaletteColor,
    pub global: bool,
    pub spot: bool,
}

/// A named group of swatches in a [`Palette`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaletteGroup {
    pub name: String,
    pub swatches: Vec<PaletteSwatch>,
}

/// A colour palette / swatch library.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Palette {
    pub name: String,
    pub swatches: Vec<PaletteSwatch>,
    pub groups: Vec<PaletteGroup>,
}

/// Supported palette file formats for writing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteFormat {
    /// `.gpl`: plain-text 8-bit RGB palette.
    Gpl,
    /// `.ase`: binary Adobe Swatch Exchange.
    Ase,
    /// `.css`: CSS custom properties on `:root` (write-only).
    Css,
}

impl PaletteFormat {
    pub const ALL: [PaletteFormat; 3] = [PaletteFormat::Gpl, PaletteFormat::Ase, PaletteFormat::Css];

    /// The format's id / file extension.
    pub fn id(self) -> &'static str {
        match self {
            PaletteFormat::Gpl => "gpl",
            PaletteFormat::Ase => "ase",
            PaletteFormat::Css => "css",
        }
    }

    /// Parse a format id or extension (case-insensitive, leading dot allowed).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim_start_matches('.').to_ascii_lowercase();
        Self::ALL.into_iter().find(|f| f.id() == s)
    }

    pub fn readable(self) -> bool {
        self != PaletteFormat::Css
    }

    pub fn binary(self) -> bool {
        self == PaletteFormat::Ase
    }
}

const GPL_HEADER: &str = "GIMP Palette";
const GPL_GROUP: &str = "# Group:";
const ASE_SIGNATURE: &[u8] = b"ASEF";
const ASE_GROUP_START: u16 = 0xC001;
const ASE_GROUP_END: u16 = 0xC002;
const ASE_COLOR: u16 = 0x0001;
const ASE_MAX_ENTRIES: usize = 100_000;
const ASE_MAX_NAME: usize = 1024;
const ASE_CUT_SHORT: &str = "the swatch exchange file is cut short";
const ACB_SIGNATURE: &[u8] = b"8BCB";
const ACB_CUT_SHORT: &str = "the color book is cut short";
const ACB_MAX_NAME_BYTES: usize = 64 << 20;

/// Write `palette` in `format`.
pub fn write_palette(palette: &Palette, format: PaletteFormat) -> Result<Vec<u8>, String> {
    match format {
        PaletteFormat::Gpl => Ok(write_gpl(palette).into_bytes()),
        PaletteFormat::Ase => write_ase(palette),
        PaletteFormat::Css => Ok(write_css(palette).into_bytes()),
    }
}

/// Read a palette from `bytes` (`.ase`, `.acb`, or `.gpl`, detected from content).
/// `fallback_name` is used when the file does not specify a library name.
pub fn read_palette(bytes: &[u8], fallback_name: &str) -> Result<Palette, String> {
    if bytes.starts_with(ASE_SIGNATURE) {
        return read_ase(bytes, fallback_name);
    }
    if bytes.starts_with(ACB_SIGNATURE) {
        return read_acb(bytes, fallback_name).map_err(|e| if e == ASE_CUT_SHORT { ACB_CUT_SHORT.into() } else { e });
    }
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if text.starts_with(GPL_HEADER) {
        return Ok(read_gpl(text, fallback_name));
    }
    Err("not a supported palette (.gpl, .ase, or .acb)".into())
}

/// Returns `true` if `bytes` looks like a `.gpl`, `.ase`, or `.acb` palette.
pub fn sniff_palette(bytes: &[u8]) -> bool {
    if bytes.starts_with(ASE_SIGNATURE) || bytes.starts_with(ACB_SIGNATURE) {
        return true;
    }
    let prefix = bytes.get(..1024).unwrap_or(bytes);
    let text = String::from_utf8_lossy(prefix);
    text.trim_start_matches('\u{feff}').trim_start().starts_with(GPL_HEADER)
}

fn one_line(s: &str) -> String {
    s.split(['\n', '\r', '\t']).collect::<Vec<_>>().join(" ").trim().to_string()
}

fn write_gpl(lib: &Palette) -> String {
    let mut out = format!("{GPL_HEADER}\nName: {}\nColumns: 0\n#\n", one_line(&lib.name));
    let colors = |out: &mut String, list: &[PaletteSwatch]| {
        for w in list {
            let [r, g, b] = w.color.to_rgb8();
            out.push_str(&format!("{r:3} {g:3} {b:3}\t{}\n", one_line(&w.name)));
        }
    };
    colors(&mut out, &lib.swatches);
    for g in &lib.groups {
        out.push_str(&format!("{GPL_GROUP} {}\n", one_line(&g.name)));
        colors(&mut out, &g.swatches);
    }
    out
}

/// `name` as a CSS custom property (`--` and an escaped identifier): lower case, spaces as
/// hyphens, other characters an identifier can't hold escaped with a backslash.
pub fn css_property(name: &str) -> String {
    let mut out = String::from("--");
    for word in name.split_whitespace() {
        if out.len() > 2 {
            out.push('-');
        }
        for ch in word.chars().flat_map(char::to_lowercase) {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || !ch.is_ascii() {
                out.push(ch);
            } else if ch.is_ascii_graphic() {
                out.push('\\');
                out.push(ch);
            } else {
                out.push_str(&format!("\\{:x} ", ch as u32));
            }
        }
    }
    if out.len() == 2 {
        out.push_str("swatch");
    }
    out
}

fn css_comment(s: &str) -> String {
    one_line(s).replace("*/", "* /")
}

fn write_css(lib: &Palette) -> String {
    let mut out = format!("/* {} */\n:root {{\n", css_comment(&lib.name));
    let mut taken: HashSet<String> = HashSet::new();
    let mut next: HashMap<String, usize> = HashMap::new();
    let mut props = |out: &mut String, list: &[PaletteSwatch]| {
        for w in list {
            let [r, g, b] = w.color.to_rgb8();
            let value = format!("#{r:02x}{g:02x}{b:02x}");
            let base = css_property(&w.name);
            let name = if taken.insert(base.clone()) {
                base
            } else {
                let n = next.entry(base.clone()).or_insert(1);
                loop {
                    *n += 1;
                    let candidate = format!("{base}-{n}");
                    if taken.insert(candidate.clone()) {
                        break candidate;
                    }
                }
            };
            let note = if w.spot { " /* spot */" } else { "" };
            out.push_str(&format!("  {name}: {value};{note}\n"));
        }
    };
    props(&mut out, &lib.swatches);
    for g in &lib.groups {
        out.push_str(&format!("  /* {} */\n", css_comment(&g.name)));
        props(&mut out, &g.swatches);
    }
    out.push_str("}\n");
    out
}

fn write_ase(lib: &Palette) -> Result<Vec<u8>, String> {
    let entries = lib.swatches.len() + lib.groups.iter().map(|g| 1 + g.swatches.len()).sum::<usize>();
    if entries > ASE_MAX_ENTRIES {
        return Err(format!("at most {ASE_MAX_ENTRIES} colors and groups are supported in a swatch exchange file, and this library has {entries}"));
    }
    let mut body = vec![];
    let mut blocks = put_ase_colors(&mut body, &lib.swatches)?;
    for g in &lib.groups {
        let mut name = vec![];
        put_ase_name(&mut name, &g.name)?;
        put_ase_block(&mut body, ASE_GROUP_START, &name)?;
        blocks += 2 + put_ase_colors(&mut body, &g.swatches)?;
        put_ase_block(&mut body, ASE_GROUP_END, &[])?;
    }
    let blocks = u32::try_from(blocks).map_err(|_| "too many swatch exchange blocks".to_string())?;
    let mut out = ASE_SIGNATURE.to_vec();
    out.extend(1u16.to_be_bytes());
    out.extend(0u16.to_be_bytes());
    out.extend(blocks.to_be_bytes());
    out.extend(body);
    Ok(out)
}

fn put_ase_colors(out: &mut Vec<u8>, list: &[PaletteSwatch]) -> Result<usize, String> {
    let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let ab = |v: f32| if v.is_finite() { v.clamp(-128.0, 127.0) } else { 0.0 };
    let mut n = 0;
    for w in list {
        let mut body = vec![];
        put_ase_name(&mut body, &w.name)?;
        let (model, values): (&[u8; 4], Vec<f32>) = match w.color {
            PaletteColor::Rgb { r, g, b } => (b"RGB ", vec![unit(r), unit(g), unit(b)]),
            PaletteColor::Cmyk { c, m, y, k } => (b"CMYK", vec![unit(c), unit(m), unit(y), unit(k)]),
            PaletteColor::Lab { l, a, b } => (b"LAB ", vec![unit(l / 100.0), ab(a), ab(b)]),
            PaletteColor::Gray { k } => (b"Gray", vec![1.0 - unit(k)]),
        };
        body.extend(model);
        body.extend(values.iter().flat_map(|v| v.to_be_bytes()));
        let kind: u16 = if w.spot {
            1
        } else if w.global {
            0
        } else {
            2
        };
        body.extend(kind.to_be_bytes());
        put_ase_block(out, ASE_COLOR, &body)?;
        n += 1;
    }
    Ok(n)
}

fn put_ase_block(out: &mut Vec<u8>, kind: u16, body: &[u8]) -> Result<(), String> {
    let len = u32::try_from(body.len()).map_err(|_| "a swatch exchange block is too long".to_string())?;
    out.extend(kind.to_be_bytes());
    out.extend(len.to_be_bytes());
    out.extend_from_slice(body);
    Ok(())
}

fn put_ase_name(out: &mut Vec<u8>, name: &str) -> Result<(), String> {
    let mut units: Vec<u16> = vec![];
    for c in name.chars().filter(|&c| c != '\0') {
        let mut buf = [0; 2];
        let encoded = c.encode_utf16(&mut buf);
        if units.len() + encoded.len() > ASE_MAX_NAME {
            break;
        }
        units.extend_from_slice(encoded);
    }
    units.push(0);
    let count = u16::try_from(units.len()).map_err(|_| "a swatch exchange name is too long".to_string())?;
    out.extend(count.to_be_bytes());
    out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
    Ok(())
}

fn read_gpl(text: &str, fallback: &str) -> Palette {
    let mut lib = Palette { name: fallback.into(), ..Default::default() };
    let mut names = UniqueNames::default();
    for line in text.lines().skip(1) {
        let line = line.trim();
        if let Some(g) = line.strip_prefix(GPL_GROUP) {
            lib.groups.push(PaletteGroup { name: names.unique(group_name(g)), swatches: vec![] });
            continue;
        }
        if let Some(n) = line.strip_prefix("Name:") {
            if !n.trim().is_empty() {
                lib.name = n.trim().to_string();
            }
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let rgb: Vec<u8> = words.iter().take(3).map_while(|v| v.parse().ok()).collect();
        let [r, g, b] = rgb[..] else { continue };
        let rest = words[3..].join(" ");
        let base = if rest.is_empty() { format!("R={r} G={g} B={b}") } else { rest };
        let w = PaletteSwatch { name: names.unique(base), color: PaletteColor::rgb8(r, g, b), global: false, spot: false };
        match lib.groups.last_mut() {
            Some(grp) => grp.swatches.push(w),
            None => lib.swatches.push(w),
        }
    }
    lib
}

fn read_ase(bytes: &[u8], fallback: &str) -> Result<Palette, String> {
    let mut data = AseData(bytes);
    data.bytes(ASE_SIGNATURE.len())?;
    let (major, minor) = (data.u16()?, data.u16()?);
    if major != 1 {
        return Err(format!("swatch exchange version {major}.{minor} isn't supported (only 1.x)"));
    }
    let blocks = data.u32()?;
    let mut lib = Palette { name: fallback.into(), ..Default::default() };
    let mut names = UniqueNames::default();
    let (mut in_group, mut entries) = (false, 0usize);
    for _ in 0..blocks {
        let kind = data.u16()?;
        let len = usize::try_from(data.u32()?).unwrap_or(usize::MAX);
        let mut body = AseData(data.bytes(len)?);
        if matches!(kind, ASE_GROUP_START | ASE_COLOR) {
            entries += 1;
            if entries > ASE_MAX_ENTRIES {
                return Err(format!("the swatch exchange file holds more than {ASE_MAX_ENTRIES} colors and groups"));
            }
        }
        match kind {
            ASE_GROUP_START => {
                lib.groups.push(PaletteGroup { name: names.unique(group_name(&body.name()?)), swatches: vec![] });
                in_group = true;
            }
            ASE_GROUP_END => in_group = false,
            ASE_COLOR => {
                let Some(mut w) = ase_swatch(&mut body)? else { continue };
                w.name = names.unique(std::mem::take(&mut w.name));
                match lib.groups.last_mut().filter(|_| in_group) {
                    Some(g) => g.swatches.push(w),
                    None => lib.swatches.push(w),
                }
            }
            _ => {}
        }
    }
    Ok(lib)
}

fn read_acb(bytes: &[u8], fallback: &str) -> Result<Palette, String> {
    let mut data = AseData(bytes);
    data.bytes(ACB_SIGNATURE.len())?;
    let version = data.u16()?;
    if version != 1 {
        return Err(format!("color book version {version} isn't supported (only 1)"));
    }
    let _book_id = data.u16()?;
    let [title, prefix, suffix, _description] = [data.acb_string()?, data.acb_string()?, data.acb_string()?, data.acb_string()?];
    let count = usize::from(data.u16()?);
    if count > ASE_MAX_ENTRIES {
        return Err(format!("the color book holds more than {ASE_MAX_ENTRIES} colors"));
    }
    let (_page_size, _page_offset) = (data.u16()?, data.u16()?);
    let space = data.u16()?;
    let mut colors = Vec::with_capacity(count);
    for _ in 0..count {
        let name = data.acb_string()?;
        let _code = data.bytes(6)?;
        let color = match space {
            0 => {
                let [r, g, b] = data.array::<3>()?;
                PaletteColor::rgb8(r, g, b)
            }
            2 => {
                let ink = |v: u8| f32::from(255 - v) / 255.0;
                let [c, m, y, k] = data.array::<4>()?;
                PaletteColor::cmyk(ink(c), ink(m), ink(y), ink(k))
            }
            7 => {
                let [l, a, b] = data.array::<3>()?;
                PaletteColor::lab(f32::from(l) * 100.0 / 255.0, f32::from(a) - 128.0, f32::from(b) - 128.0)
            }
            other => return Err(format!("color books in color space {other} aren't supported (only RGB, CMYK and Lab)")),
        };
        colors.push((name, color));
    }
    let spot = data.0.windows(8).last() != Some(b"spflproc".as_slice());
    let mut lib = Palette { name: if title.is_empty() { fallback.into() } else { title }, ..Default::default() };
    let mut names = UniqueNames::default();
    let mut kept = 0usize;
    for (name, color) in colors.into_iter().filter(|(n, _)| !n.trim().is_empty()) {
        kept = kept.saturating_add(2 * (prefix.len() + name.len() + suffix.len()));
        if kept > ACB_MAX_NAME_BYTES {
            return Err(format!("the color book's names take more than {} MB", ACB_MAX_NAME_BYTES >> 20));
        }
        let name = names.unique(format!("{prefix}{name}{suffix}").trim().to_string());
        lib.swatches.push(PaletteSwatch { name, color, global: spot, spot });
    }
    Ok(lib)
}

fn ase_swatch(body: &mut AseData) -> Result<Option<PaletteSwatch>, String> {
    let name = body.name()?;
    let unit = |v: f32| v.clamp(0.0, 1.0);
    let color = match &body.array::<4>()? {
        b"RGB " => {
            let [r, g, b] = [body.f32()?, body.f32()?, body.f32()?];
            PaletteColor::rgb(unit(r), unit(g), unit(b))
        }
        b"CMYK" => {
            let [c, m, y, k] = [body.f32()?, body.f32()?, body.f32()?, body.f32()?];
            PaletteColor::cmyk(unit(c), unit(m), unit(y), unit(k))
        }
        b"LAB " => {
            let [l, a, b] = [body.f32()?, body.f32()?, body.f32()?];
            let l = if l > 1.0 { l.min(100.0) } else { unit(l) * 100.0 };
            PaletteColor::lab(l, a.clamp(-128.0, 127.0), b.clamp(-128.0, 127.0))
        }
        b"Gray" => PaletteColor::gray(1.0 - unit(body.f32()?)),
        _ => return Ok(None),
    };
    let kind = if body.0.len() >= 2 { body.u16()? } else { 2 };
    let (global, spot) = match kind {
        0 => (true, false),
        1 => (true, true),
        _ => (false, false),
    };
    let name = if name.is_empty() { value_name(color) } else { name };
    Ok(Some(PaletteSwatch { name, color, global, spot }))
}

fn value_name(c: PaletteColor) -> String {
    let pct = |v: f32| (v * 100.0).round();
    let byte = |v: f32| (v * 255.0).round();
    match c {
        PaletteColor::Cmyk { c, m, y, k } => format!("C={} M={} Y={} K={}", pct(c), pct(m), pct(y), pct(k)),
        PaletteColor::Rgb { r, g, b } => format!("R={} G={} B={}", byte(r), byte(g), byte(b)),
        PaletteColor::Gray { k } => format!("Gray K={}", pct(k)),
        PaletteColor::Lab { l, a, b } => format!("L={} a={} b={}", l.round() + 0.0, a.round() + 0.0, b.round() + 0.0),
    }
}

struct AseData<'a>(&'a [u8]);

impl<'a> AseData<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let (head, rest) = self.0.split_at_checked(n).ok_or(ASE_CUT_SHORT)?;
        self.0 = rest;
        Ok(head)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let (head, rest) = self.0.split_first_chunk::<N>().ok_or(ASE_CUT_SHORT)?;
        self.0 = rest;
        Ok(*head)
    }
    fn u16(&mut self) -> Result<u16, String> {
        self.array().map(u16::from_be_bytes)
    }
    fn u32(&mut self) -> Result<u32, String> {
        self.array().map(u32::from_be_bytes)
    }
    fn f32(&mut self) -> Result<f32, String> {
        let v = f32::from_be_bytes(self.array()?);
        Ok(if v.is_finite() { v } else { 0.0 })
    }
    fn acb_string(&mut self) -> Result<String, String> {
        let n = usize::try_from(self.u32()?).unwrap_or(usize::MAX);
        let (units, _) = self.bytes(n.checked_mul(2).ok_or(ASE_CUT_SHORT)?)?.as_chunks::<2>();
        let units: Vec<u16> = units.iter().take(ASE_MAX_NAME).map(|u| u16::from_be_bytes(*u)).take_while(|&u| u != 0).collect();
        let s = String::from_utf16_lossy(&units);
        let s = match s.strip_prefix("$$$/") {
            Some(key) => key.split_once('=').map_or("", |(_, text)| text).to_string(),
            None => s,
        };
        Ok(s.replace("^R", "®").replace("^C", "©"))
    }
    fn name(&mut self) -> Result<String, String> {
        if self.0.is_empty() {
            return Ok(String::new());
        }
        let n = usize::from(self.u16()?);
        let (units, _) = self.bytes(n * 2)?.as_chunks::<2>();
        let units: Vec<u16> = units.iter().take(ASE_MAX_NAME).map(|u| u16::from_be_bytes(*u)).take_while(|&u| u != 0).collect();
        Ok(String::from_utf16_lossy(&units).trim().to_string())
    }
}

fn group_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() { "Color Group".into() } else { name.to_string() }
}

#[derive(Default)]
struct UniqueNames {
    taken: HashSet<String>,
    last: HashMap<String, usize>,
}

impl UniqueNames {
    fn unique(&mut self, base: String) -> String {
        if self.taken.insert(base.clone()) {
            return base;
        }
        let n = self.last.entry(base.clone()).or_insert(1);
        loop {
            *n += 1;
            let name = format!("{base} {n}");
            if self.taken.insert(name.clone()) {
                return name;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpl_roundtrip() {
        let pal = Palette {
            name: "Test Palette".into(),
            swatches: vec![PaletteSwatch { name: "Red".into(), color: PaletteColor::rgb8(255, 0, 0), global: false, spot: false }],
            groups: vec![PaletteGroup {
                name: "Blues".into(),
                swatches: vec![PaletteSwatch { name: "Navy".into(), color: PaletteColor::rgb8(0, 0, 128), global: false, spot: false }],
            }],
        };
        let bytes = write_palette(&pal, PaletteFormat::Gpl).expect("write gpl");
        assert!(sniff_palette(&bytes));
        let parsed = read_palette(&bytes, "fallback").expect("read gpl");
        assert_eq!(parsed.name, "Test Palette");
        assert_eq!(parsed.swatches.len(), 1);
        assert_eq!(parsed.swatches[0].name, "Red");
        assert_eq!(parsed.groups.len(), 1);
        assert_eq!(parsed.groups[0].name, "Blues");
        assert_eq!(parsed.groups[0].swatches[0].name, "Navy");
    }

    #[test]
    fn ase_roundtrip() {
        let pal = Palette {
            name: "ASE".into(),
            swatches: vec![
                PaletteSwatch { name: "Spot Red".into(), color: PaletteColor::rgb(1.0, 0.0, 0.0), global: true, spot: true },
                PaletteSwatch { name: "CMYK Cyan".into(), color: PaletteColor::cmyk(1.0, 0.0, 0.0, 0.0), global: true, spot: false },
                PaletteSwatch { name: "Lab Mid".into(), color: PaletteColor::lab(50.0, 10.0, -20.0), global: false, spot: false },
                PaletteSwatch { name: "Gray 40".into(), color: PaletteColor::gray(0.4), global: false, spot: false },
            ],
            groups: vec![PaletteGroup {
                name: "Group 1".into(),
                swatches: vec![PaletteSwatch { name: "Green".into(), color: PaletteColor::rgb(0.0, 1.0, 0.0), global: false, spot: false }],
            }],
        };
        let bytes = write_palette(&pal, PaletteFormat::Ase).expect("write ase");
        assert!(sniff_palette(&bytes));
        let parsed = read_palette(&bytes, "ASE").expect("read ase");
        assert_eq!(parsed.swatches.len(), 4);
        assert!(parsed.swatches[0].spot && parsed.swatches[0].global);
        assert!(!parsed.swatches[1].spot && parsed.swatches[1].global);
        assert_eq!(parsed.groups.len(), 1);
        assert_eq!(parsed.groups[0].name, "Group 1");
    }

    #[test]
    fn css_output() {
        let pal = Palette {
            name: "Brand".into(),
            swatches: vec![
                PaletteSwatch { name: "Primary Blue".into(), color: PaletteColor::rgb8(0, 102, 204), global: true, spot: true },
                PaletteSwatch { name: "Primary Blue".into(), color: PaletteColor::rgb8(0, 0, 0), global: false, spot: false },
            ],
            groups: vec![],
        };
        let bytes = write_palette(&pal, PaletteFormat::Css).expect("write css");
        let css = String::from_utf8(bytes).expect("utf8");
        assert!(css.contains("--primary-blue: #0066cc; /* spot */"));
        assert!(css.contains("--primary-blue-2: #000000;"));
    }

    #[test]
    fn acb_read() {
        let mut acb = Vec::new();
        acb.extend_from_slice(b"8BCB");
        acb.extend_from_slice(&1u16.to_be_bytes());
        acb.extend_from_slice(&3000u16.to_be_bytes());
        let put_str = |buf: &mut Vec<u8>, s: &str| {
            let units: Vec<u16> = s.encode_utf16().collect();
            buf.extend_from_slice(&(units.len() as u32).to_be_bytes());
            for u in units {
                buf.extend_from_slice(&u.to_be_bytes());
            }
        };
        put_str(&mut acb, "$$$/Book/Title=Pantone^R");
        put_str(&mut acb, "PANTONE ");
        put_str(&mut acb, " C");
        put_str(&mut acb, "");
        acb.extend_from_slice(&1u16.to_be_bytes()); // count
        acb.extend_from_slice(&7u16.to_be_bytes()); // page size
        acb.extend_from_slice(&0u16.to_be_bytes()); // page offset
        acb.extend_from_slice(&0u16.to_be_bytes()); // RGB
        put_str(&mut acb, "185");
        acb.extend_from_slice(b"000185");
        acb.extend_from_slice(&[228, 0, 43]);
        acb.extend_from_slice(b"spflspot");

        assert!(sniff_palette(&acb));
        let parsed = read_palette(&acb, "fallback").expect("read acb");
        assert_eq!(parsed.name, "Pantone®");
        assert_eq!(parsed.swatches.len(), 1);
        assert_eq!(parsed.swatches[0].name, "PANTONE 185 C");
        assert!(parsed.swatches[0].spot);
    }
}
