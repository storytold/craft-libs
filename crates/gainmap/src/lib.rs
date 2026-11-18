//! Gain map HDR images (ISO 21496-1): an SDR base image plus a gain map that a decoder uses to
//! rebuild the HDR rendition, so the file looks right everywhere and lights up on HDR displays.
//!
//! This crate is the metadata and the arithmetic, independent of the container:
//!
//! - [`GainMapMeta`] reads and writes the ISO 21496-1 binary metadata ([`GainMapMeta::from_iso`],
//!   [`GainMapMeta::to_iso`]) and the Adobe `hdrgm` XMP ([`GainMapMeta::from_hdrgm_xmp`],
//!   [`GainMapMeta::to_gain_map_xmp`], which also writes Apple's description of the same map).
//! - [`compute`] measures a gain map from an SDR and an HDR rendition of the same picture (linear
//!   light, same primaries). Measure it against the base *as written* (decoded from its 8-bit
//!   codes), so quantisation in the base is handed back by the gain map.
//! - [`apply`] rebuilds the HDR rendition from a base and its gain map.
//! - [`apple`] rebuilds the HDR rendition of Apple's legacy single-channel gain maps.
//! - [`primary_xmp`] and [`primary_iso_marker`] are what the primary image of a gain map file
//!   carries to say it has one.
//!
//! Encoding the two images and laying them out in a file (a JPEG with an MPF index, HEIF
//! auxiliary images) is the caller's.
//!
//! All gains are log2. A decoder recovers `log2(hdr + offset_hdr) − log2(sdr + offset_sdr)` as
//! `min + (max − min) · (code / 255)^(1 / gamma)`, scaled by a weight in 0..1 that depends on how
//! much headroom the display has between `base_headroom` and `alternate_headroom`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// Apple's legacy gain maps: a linear gain ratio, not the log2 gain of ISO 21496-1 and Ultra HDR.
pub mod apple;

/// A size or parameter [`compute`] or [`apply`] cannot work with.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("gain map: {0}")]
pub struct Error(pub &'static str);

/// What this crate returns.
pub type Result<T> = std::result::Result<T, Error>;

/// The ISO 21496-1 namespace that opens its APP2 payloads (NUL terminated).
pub const ISO_URN: &[u8] = b"urn:iso:std:iso:ts:21496:-1\0";

/// Fixed denominator for the metadata's rationals: one value per setting, byte-identical output.
const DENOMINATOR: u32 = 1_000_000;

/// Gain map parameters (ISO 21496-1 / Adobe `hdrgm`). Per-channel arrays repeat the first value
/// when the map is single-channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GainMapMeta {
    /// Three gain channels (R, G, B) instead of one luminance gain.
    pub multichannel: bool,
    /// The gain applies in the base image's colour space.
    pub use_base_colour_space: bool,
    /// log2 headroom at which the base is shown as is (0: an SDR base).
    pub base_headroom: f32,
    /// log2 headroom at which the gain map applies in full.
    pub alternate_headroom: f32,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub gamma: [f32; 3],
    pub base_offset: [f32; 3],
    pub alternate_offset: [f32; 3],
}

impl Default for GainMapMeta {
    fn default() -> Self {
        GainMapMeta {
            multichannel: false,
            use_base_colour_space: true,
            base_headroom: 0.0,
            alternate_headroom: 1.0,
            min: [0.0; 3],
            max: [1.0; 3],
            gamma: [1.0; 3],
            base_offset: [1.0 / 64.0; 3],
            alternate_offset: [1.0 / 64.0; 3],
        }
    }
}

impl GainMapMeta {
    fn channels(&self) -> usize {
        if self.multichannel { 3 } else { 1 }
    }

    /// Usable by [`apply`]: finite, positive gammas, max ≥ min, headrooms in order.
    pub fn is_valid(&self) -> bool {
        let fin = |a: &[f32; 3]| a.iter().all(|v| v.is_finite());
        fin(&self.min)
            && fin(&self.max)
            && fin(&self.gamma)
            && fin(&self.base_offset)
            && fin(&self.alternate_offset)
            && self.gamma.iter().all(|g| *g > 0.0)
            && self.min.iter().zip(&self.max).all(|(a, b)| b >= a)
            && self.base_headroom.is_finite()
            && self.alternate_headroom.is_finite()
            && self.alternate_headroom > self.base_headroom
    }

    /// The ISO 21496-1 APP2 payload of the gain map image, URN included.
    pub fn to_iso(&self) -> Vec<u8> {
        let mut p = ISO_URN.to_vec();
        p.extend_from_slice(&0u16.to_be_bytes()); // minimum_version: readers gate on this
        p.extend_from_slice(&0u16.to_be_bytes()); // writer_version
        let mut flags = 0u8;
        if self.multichannel {
            flags |= FLAG_MULTICHANNEL;
        }
        if self.use_base_colour_space {
            flags |= FLAG_BASE_COLOUR_SPACE;
        }
        p.push(flags);
        let unsigned = |p: &mut Vec<u8>, v: f32| {
            let n = (v as f64 * DENOMINATOR as f64).round().clamp(0.0, u32::MAX as f64) as u32;
            p.extend_from_slice(&n.to_be_bytes());
            p.extend_from_slice(&DENOMINATOR.to_be_bytes());
        };
        let signed = |p: &mut Vec<u8>, v: f32| {
            let n = (v as f64 * DENOMINATOR as f64).round().clamp(i32::MIN as f64, i32::MAX as f64) as i32;
            p.extend_from_slice(&n.to_be_bytes());
            p.extend_from_slice(&DENOMINATOR.to_be_bytes());
        };
        unsigned(&mut p, self.base_headroom.max(0.0));
        unsigned(&mut p, self.alternate_headroom.max(0.0));
        for c in 0..self.channels() {
            signed(&mut p, self.min[c]);
            signed(&mut p, self.max[c]);
            unsigned(&mut p, self.gamma[c]);
            signed(&mut p, self.base_offset[c]);
            signed(&mut p, self.alternate_offset[c]);
        }
        p
    }

    /// Parse an ISO 21496-1 payload (the bytes after [`ISO_URN`]). `None` for the versions-only
    /// marker a primary image carries, unknown versions, a base that is the HDR rendition, or
    /// values no decoder could use.
    pub fn from_iso(p: &[u8]) -> Option<GainMapMeta> {
        let mut r = Reader { b: p, at: 0 };
        let min_version = r.u16()?;
        let _writer_version = r.u16()?;
        if min_version != 0 {
            return None;
        }
        let flags = r.u8()?;
        if flags & FLAG_BACKWARD != 0 {
            return None;
        }
        let multichannel = flags & FLAG_MULTICHANNEL != 0;
        let common = flags & FLAG_COMMON_DENOMINATOR != 0;
        let common_d = if common { r.u32()? } else { 0 };
        let unsigned = |r: &mut Reader| -> Option<f32> {
            let n = r.u32()?;
            let d = if common { common_d } else { r.u32()? };
            (d != 0).then(|| (n as f64 / d as f64) as f32)
        };
        let signed = |r: &mut Reader| -> Option<f32> {
            let n = r.u32()? as i32;
            let d = if common { common_d } else { r.u32()? };
            (d != 0).then(|| (n as f64 / d as f64) as f32)
        };
        let mut m = GainMapMeta {
            multichannel,
            use_base_colour_space: flags & FLAG_BASE_COLOUR_SPACE != 0,
            base_headroom: unsigned(&mut r)?,
            alternate_headroom: unsigned(&mut r)?,
            ..GainMapMeta::default()
        };
        for c in 0..m.channels() {
            m.min[c] = signed(&mut r)?;
            m.max[c] = signed(&mut r)?;
            m.gamma[c] = unsigned(&mut r)?;
            m.base_offset[c] = signed(&mut r)?;
            m.alternate_offset[c] = signed(&mut r)?;
        }
        if !multichannel {
            m.fill_from_first();
        }
        m.is_valid().then_some(m)
    }

    /// Copies the first channel's parameters to the other two: what a single-channel map means.
    pub fn fill_from_first(&mut self) {
        for a in [&mut self.min, &mut self.max, &mut self.gamma, &mut self.base_offset, &mut self.alternate_offset] {
            a[1] = a[0];
            a[2] = a[0];
        }
    }

    /// The gain parameters from Adobe `hdrgm` XMP (the gain map image's packet), for files
    /// without ISO 21496-1 metadata (the first Ultra HDR files). Attributes or `rdf:Seq` lists.
    pub fn from_hdrgm_xmp(xmp: &str) -> Option<GainMapMeta> {
        if !xmp.contains("hdrgm:") {
            return None;
        }
        let base_rendition_hdr = xmp_values(xmp, "BaseRenditionIsHDR").is_some_and(|v| v.first().is_some_and(|s| s.eq_ignore_ascii_case("true")));
        if base_rendition_hdr {
            return None;
        }
        let get = |name: &str, default: f32| -> Option<([f32; 3], bool)> {
            let Some(v) = xmp_values(xmp, name) else { return Some(([default; 3], false)) };
            let f: Vec<f32> = v.iter().map(|s| s.trim().parse::<f32>().ok()).collect::<Option<_>>()?;
            match f.as_slice() {
                [a] => Some(([*a; 3], false)),
                [a, b, c] => Some(([*a, *b, *c], true)),
                _ => None,
            }
        };
        let (max, m1) = get("GainMapMax", f32::NAN)?;
        if max.iter().any(|v| v.is_nan()) {
            return None; // the one required parameter
        }
        let (min, m2) = get("GainMapMin", 0.0)?;
        let (gamma, m3) = get("Gamma", 1.0)?;
        let (base_offset, m4) = get("OffsetSDR", 1.0 / 64.0)?;
        let (alternate_offset, m5) = get("OffsetHDR", 1.0 / 64.0)?;
        let (cap_min, _) = get("HDRCapacityMin", 0.0)?;
        let max_gain = max.iter().copied().fold(f32::MIN, f32::max);
        let (cap_max, _) = get("HDRCapacityMax", max_gain)?;
        let m = GainMapMeta {
            multichannel: m1 || m2 || m3 || m4 || m5,
            use_base_colour_space: true,
            base_headroom: cap_min[0].max(0.0),
            alternate_headroom: cap_max[0],
            min,
            max,
            gamma,
            base_offset,
            alternate_offset,
        };
        m.is_valid().then_some(m)
    }

    /// The gain map image's XMP: the Adobe `hdrgm` parameters (read by Android and Chrome, which
    /// ignore the ISO payload) and Apple's description of the same map (what iMessage keeps).
    pub fn to_gain_map_xmp(&self) -> String {
        let names = ["GainMapMin", "GainMapMax", "Gamma", "OffsetSDR", "OffsetHDR"];
        let values = [&self.min, &self.max, &self.gamma, &self.base_offset, &self.alternate_offset];
        let uniform = |v: &[f32; 3]| !self.multichannel || (v[0] == v[1] && v[1] == v[2]);
        let mut s = String::from(XMP_OPEN);
        s.push_str(r#"<rdf:Description rdf:about="" xmlns:hdrgm="http://ns.adobe.com/hdr-gain-map/1.0/" hdrgm:Version="1.0""#);
        s.push_str(&format!(r#" hdrgm:HDRCapacityMin="{}""#, num(self.base_headroom)));
        s.push_str(&format!(r#" hdrgm:HDRCapacityMax="{}""#, num(self.alternate_headroom)));
        s.push_str(r#" hdrgm:BaseRenditionIsHDR="False""#);
        for (n, v) in names.iter().zip(values) {
            if uniform(v) {
                s.push_str(&format!(r#" hdrgm:{n}="{}""#, num(v[0])));
            }
        }
        s.push('>');
        // per-channel values are an rdf:Seq of three, not a comma list: readers parse each item
        for (n, v) in names.iter().zip(values) {
            if !uniform(v) {
                s.push_str(&format!("<hdrgm:{n}><rdf:Seq>"));
                for x in v {
                    s.push_str(&format!("<rdf:li>{}</rdf:li>", num(*x)));
                }
                s.push_str(&format!("</rdf:Seq></hdrgm:{n}>"));
            }
        }
        s.push_str("</rdf:Description>");
        s.push_str(&self.apple_description());
        s.push_str(XMP_CLOSE);
        s
    }

    /// Apple's own description of the map (`apdi`, `HDRGainMap`, `HDRToneMap` namespaces).
    fn apple_description(&self) -> String {
        // 'L008' (8-bit luminance) or '444f'; the stored map must really have that many channels
        let format = if self.multichannel { 0x3434_3466u32 } else { 0x4C30_3038 };
        let mut s = String::from(
            r#"<rdf:Description rdf:about="" xmlns:apdi="http://ns.apple.com/pixeldatainfo/1.0/" xmlns:HDRGainMap="http://ns.apple.com/HDRGainMap/1.0/" xmlns:HDRToneMap="http://ns.apple.com/HDRToneMap/1.0/">"#,
        );
        s.push_str("<apdi:AuxiliaryImageType>urn:com:apple:photo:2020:aux:hdrgainmap</apdi:AuxiliaryImageType>");
        s.push_str(&format!("<apdi:NativeFormat>{format}</apdi:NativeFormat><apdi:StoredFormat>{format}</apdi:StoredFormat>"));
        s.push_str("<HDRGainMap:HDRGainMapVersion>131072</HDRGainMap:HDRGainMapVersion>");
        // linear, not log2: the headroom multiplier itself
        s.push_str(&format!("<HDRGainMap:HDRGainMapHeadroom>{}</HDRGainMap:HDRGainMapHeadroom>", num(self.alternate_headroom.exp2())));
        s.push_str(&format!("<HDRToneMap:AlternateHeadroom>{}</HDRToneMap:AlternateHeadroom>", num(self.alternate_headroom)));
        s.push_str("<HDRToneMap:ChannelMetadata><rdf:Seq>");
        for c in 0..self.channels() {
            s.push_str(&format!(
                r#"<rdf:li rdf:parseType="Resource"><HDRToneMap:GainMapMin>{}</HDRToneMap:GainMapMin><HDRToneMap:GainMapMax>{}</HDRToneMap:GainMapMax><HDRToneMap:Gamma>{}</HDRToneMap:Gamma><HDRToneMap:BaseOffset>{}</HDRToneMap:BaseOffset><HDRToneMap:AlternateOffset>{}</HDRToneMap:AlternateOffset></rdf:li>"#,
                num(self.min[c]),
                num(self.max[c]),
                num(self.gamma[c]),
                num(self.base_offset[c]),
                num(self.alternate_offset[c])
            ));
        }
        s.push_str("</rdf:Seq></HDRToneMap:ChannelMetadata>");
        s.push_str(&format!("<HDRToneMap:BaseHeadroom>{}</HDRToneMap:BaseHeadroom>", num(self.base_headroom)));
        s.push_str("<HDRToneMap:BaseColorIsWorkingColor>True</HDRToneMap:BaseColorIsWorkingColor><HDRToneMap:Version>1</HDRToneMap:Version>");
        s.push_str("</rdf:Description>");
        s
    }
}

const FLAG_MULTICHANNEL: u8 = 0x80;
const FLAG_BASE_COLOUR_SPACE: u8 = 0x40;
const FLAG_COMMON_DENOMINATOR: u8 = 0x08;
const FLAG_BACKWARD: u8 = 0x04;

const XMP_OPEN: &str = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">";
const XMP_CLOSE: &str = "</rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>";

/// A short decimal (up to 6 significant digits, no exponent for ordinary values).
fn num(v: f32) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{:.6}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.to_string() }
}

/// The values of `hdrgm:<name>`: an attribute (`name="v"`) or an element holding an `rdf:Seq`.
fn xmp_values(xmp: &str, name: &str) -> Option<Vec<String>> {
    let attr = format!("hdrgm:{name}=");
    if let Some(i) = xmp.find(&attr) {
        let rest = xmp.get(i + attr.len()..)?;
        let q = rest.chars().next()?;
        if q == '"' || q == '\'' {
            let body = rest.get(1..)?;
            let end = body.find(q)?;
            return Some(vec![body.get(..end)?.to_string()]);
        }
    }
    let open = format!("<hdrgm:{name}>");
    let close = format!("</hdrgm:{name}>");
    let i = xmp.find(&open)?;
    let body = xmp.get(i + open.len()..)?;
    let body = body.get(..body.find(&close)?)?;
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(a) = rest.find("<rdf:li>") {
        let after = rest.get(a + 8..)?;
        let b = after.find("</rdf:li>")?;
        out.push(after.get(..b)?.to_string());
        rest = after.get(b..)?;
        if out.len() > 3 {
            return None;
        }
    }
    if out.is_empty() {
        // a plain element value
        out.push(body.trim().to_string());
    }
    Some(out)
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let s = self.b.get(self.at..self.at.checked_add(N)?)?;
        self.at += N;
        s.try_into().ok()
    }
    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|a| a[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take().map(u16::from_be_bytes)
    }
    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_be_bytes)
    }
}

/// A gain map image: `channels` (1 or 3) interleaved 8-bit codes.
#[derive(Clone, Debug, PartialEq)]
pub struct GainImage {
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub data: Vec<u8>,
}

impl GainImage {
    /// Bilinear code of channel `c` at gain map coordinates (`x`, `y`) (pixel centres at +0.5).
    fn sample(&self, x: f32, y: f32, c: usize) -> f32 {
        let (w, h) = (self.width, self.height);
        let fx = (x - 0.5).clamp(0.0, (w - 1) as f32);
        let fy = (y - 0.5).clamp(0.0, (h - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |x: usize, y: usize| self.data.get((y * w + x) * self.channels + c).copied().unwrap_or(0) as f32;
        let a = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
        let b = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
        a + (b - a) * ty
    }
}

/// How [`compute`] builds the map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GainMapOptions {
    /// Downscale factor of the map (1, 2 or 4).
    pub scale: usize,
    /// Three gains (R, G, B) instead of one luminance gain. Apple's readers want a single
    /// channel, so this is off by default.
    pub multichannel: bool,
    pub gamma: f32,
    /// Offsets added to SDR and HDR before the ratio (keep blacks finite and noise quiet).
    pub offset: f32,
}

impl Default for GainMapOptions {
    fn default() -> Self {
        GainMapOptions { scale: 2, multichannel: false, gamma: 1.0, offset: 1.0 / 64.0 }
    }
}

/// Measure the gain map that turns `sdr` into `hdr` (both `width × height` linear RGB in the same
/// primaries; `luma` = that space's luminance weights).
pub fn compute(
    sdr: &[[f32; 3]],
    hdr: &[[f32; 3]],
    width: usize,
    height: usize,
    luma: [f32; 3],
    o: &GainMapOptions,
) -> Result<(GainImage, GainMapMeta)> {
    let n = width.checked_mul(height).ok_or(Error("image too large"))?;
    if n == 0 || sdr.len() < n || hdr.len() < n {
        return Err(Error("SDR and HDR renditions must match the image size"));
    }
    if !(matches!(o.scale, 1 | 2 | 4) && o.gamma > 0.0 && o.gamma.is_finite() && o.offset > 0.0 && o.offset.is_finite()) {
        return Err(Error("scale must be 1, 2 or 4, gamma and offset positive"));
    }
    let s = o.scale;
    let (gw, gh) = (width.div_ceil(s), height.div_ceil(s));
    let ch = if o.multichannel { 3 } else { 1 };
    let y = |p: &[f32; 3]| luma[0] * p[0] + luma[1] * p[1] + luma[2] * p[2];
    let ratio = |a: f32, b: f32| ((b.max(0.0) + o.offset) / (a.max(0.0) + o.offset)).log2();
    // block means of the log gains
    let mut gains = vec![0f32; gw * gh * ch];
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for gy in 0..gh {
        for gx in 0..gw {
            let mut acc = [0f32; 3];
            let mut count = 0f32;
            for yy in gy * s..((gy + 1) * s).min(height) {
                for xx in gx * s..((gx + 1) * s).min(width) {
                    let i = yy * width + xx;
                    let (Some(a), Some(b)) = (sdr.get(i), hdr.get(i)) else { continue };
                    if o.multichannel {
                        for c in 0..3 {
                            acc[c] += ratio(a[c], b[c]);
                        }
                    } else {
                        acc[0] += ratio(y(a), y(b));
                    }
                    count += 1.0;
                }
            }
            for c in 0..ch {
                let g = if count > 0.0 { acc[c] / count } else { 0.0 };
                let g = if g.is_finite() { g } else { 0.0 };
                gains[(gy * gw + gx) * ch + c] = g;
                lo[c] = lo[c].min(g);
                hi[c] = hi[c].max(g);
            }
        }
    }
    let mut m = GainMapMeta {
        multichannel: o.multichannel,
        gamma: [o.gamma; 3],
        base_offset: [o.offset; 3],
        alternate_offset: [o.offset; 3],
        ..GainMapMeta::default()
    };
    for c in 0..ch {
        // the range always includes 0 (no change) so an SDR-only area encodes exactly
        m.min[c] = lo[c].min(0.0);
        m.max[c] = hi[c].max(m.min[c] + 1e-4);
    }
    if !o.multichannel {
        m.fill_from_first();
    }
    m.base_headroom = 0.0;
    m.alternate_headroom = m.max.iter().copied().fold(0.0f32, f32::max).max(1e-3);
    let data = gains
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let c = i % ch;
            let norm = ((g - m.min[c]) / (m.max[c] - m.min[c])).clamp(0.0, 1.0);
            (norm.powf(o.gamma) * 255.0).round() as u8
        })
        .collect();
    Ok((GainImage { width: gw, height: gh, channels: ch, data }, m))
}

/// Rebuild the HDR rendition in place: `base` (`width × height` linear RGB) is replaced by the
/// rendition at `weight` (0 = the base, 1 = the full alternate rendition; a display with headroom
/// `H` uses `(H − base_headroom) / (alternate_headroom − base_headroom)`, clamped).
pub fn apply(base: &mut [[f32; 3]], width: usize, height: usize, gain: &GainImage, m: &GainMapMeta, weight: f32) -> Result<()> {
    let n = width.checked_mul(height).ok_or(Error("image too large"))?;
    if base.len() < n
        || gain.width == 0
        || gain.height == 0
        || !matches!(gain.channels, 1 | 3)
        || gain.width.checked_mul(gain.height).and_then(|n| n.checked_mul(gain.channels)).is_none_or(|n| gain.data.len() < n)
    {
        return Err(Error("base and gain map sizes don't match"));
    }
    if !m.is_valid() {
        return Err(Error("unusable metadata"));
    }
    let weight = if weight.is_finite() { weight.clamp(0.0, 1.0) } else { 1.0 };
    let (sx, sy) = (gain.width as f32 / width as f32, gain.height as f32 / height as f32);
    let inv_gamma = m.gamma.map(|g| 1.0 / g);
    for (i, p) in base.iter_mut().enumerate().take(n) {
        let (x, y) = ((i % width) as f32 + 0.5, (i / width) as f32 + 0.5);
        let (gx, gy) = (x * sx, y * sy);
        let mut g = [0f32; 3];
        for (c, gc) in g.iter_mut().enumerate() {
            let src = if gain.channels == 3 { c } else { 0 };
            let norm = (gain.sample(gx, gy, src) / 255.0).clamp(0.0, 1.0).powf(inv_gamma[c]);
            *gc = m.min[c] + (m.max[c] - m.min[c]) * norm;
        }
        for c in 0..3 {
            let v = (p[c] + m.base_offset[c]) * (g[c] * weight).exp2() - m.alternate_offset[c];
            p[c] = v.max(0.0);
        }
    }
    Ok(())
}

/// The ISO 21496-1 APP2 payload of the primary image: the namespace and the two version
/// fields only, which says "this image has a gain map".
pub fn primary_iso_marker() -> Vec<u8> {
    let mut p = ISO_URN.to_vec();
    p.extend_from_slice(&[0, 0, 0, 0]);
    p
}

/// The primary image's XMP: `xmp` (the packet it would carry anyway, if any) with the container
/// description added: `hdrgm:Version` (marks the file as a gain map image for Adobe and Google
/// readers) and the Container directory naming the gain map image and its length in bytes.
pub fn primary_xmp(xmp: Option<&str>, gain_len: usize) -> String {
    let description = format!(
        concat!(
            r#"<rdf:Description rdf:about="" xmlns:Container="http://ns.google.com/photos/1.0/container/" xmlns:Item="http://ns.google.com/photos/1.0/container/item/" xmlns:hdrgm="http://ns.adobe.com/hdr-gain-map/1.0/" hdrgm:Version="1.0">"#,
            r#"<Container:Directory><rdf:Seq>"#,
            r#"<rdf:li rdf:parseType="Resource"><Container:Item Item:Semantic="Primary" Item:Mime="image/jpeg"/></rdf:li>"#,
            r#"<rdf:li rdf:parseType="Resource"><Container:Item Item:Semantic="GainMap" Item:Mime="image/jpeg" Item:Length="{}"/></rdf:li>"#,
            r#"</rdf:Seq></Container:Directory></rdf:Description>"#
        ),
        gain_len
    );
    // Added as another `rdf:Description`; a fresh packet when there is none to add to.
    if let Some(x) = xmp
        && let Some(i) = x.rfind("</rdf:RDF>")
        && let (Some(a), Some(b)) = (x.get(..i), x.get(i..))
    {
        return format!("{a}{description}{b}");
    }
    format!("{XMP_OPEN}{description}{XMP_CLOSE}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta3() -> GainMapMeta {
        GainMapMeta {
            multichannel: true,
            use_base_colour_space: true,
            base_headroom: 0.0,
            alternate_headroom: 2.5,
            min: [-0.25, -0.5, 0.0],
            max: [2.5, 2.25, 2.0],
            gamma: [1.0, 1.25, 0.75],
            base_offset: [0.015625; 3],
            alternate_offset: [0.015625, 0.03125, 0.015625],
        }
    }

    #[test]
    fn iso_metadata_round_trips() {
        for m in [GainMapMeta { alternate_headroom: 3.0, max: [3.0; 3], ..GainMapMeta::default() }, meta3()] {
            let p = m.to_iso();
            assert!(p.starts_with(ISO_URN));
            let back = GainMapMeta::from_iso(&p[ISO_URN.len()..]).unwrap();
            for (a, b) in [(back.min, m.min), (back.max, m.max), (back.gamma, m.gamma), (back.alternate_offset, m.alternate_offset)] {
                for c in 0..3 {
                    assert!((a[c] - b[c]).abs() < 1e-6, "{a:?} {b:?}");
                }
            }
            assert_eq!(back.multichannel, m.multichannel);
            assert!((back.alternate_headroom - m.alternate_headroom).abs() < 1e-6);
        }
    }

    #[test]
    fn iso_common_denominator_and_rejections() {
        // common-denominator form, single channel: d = 4, headrooms 0 and 2, min 0, max 2, gamma 1, offsets 0
        let mut p = vec![0, 0, 0, 0, FLAG_COMMON_DENOMINATOR];
        for v in [4u32, 0, 8, 0, 8, 4, 0, 0] {
            p.extend_from_slice(&v.to_be_bytes());
        }
        let m = GainMapMeta::from_iso(&p).unwrap();
        assert_eq!((m.alternate_headroom, m.max[2], m.gamma[1]), (2.0, 2.0, 1.0));
        // versions only (a primary image's marker), unknown version, truncated, backward
        assert!(GainMapMeta::from_iso(&[0, 0, 0, 0]).is_none());
        assert!(GainMapMeta::from_iso(&[0, 1, 0, 0, 0]).is_none());
        assert!(GainMapMeta::from_iso(&p[..p.len() - 1]).is_none());
        let mut back = p.clone();
        back[4] |= FLAG_BACKWARD;
        assert!(GainMapMeta::from_iso(&back).is_none());
        // a zero denominator is rejected, not divided by
        let mut z = p.clone();
        z[5..9].copy_from_slice(&0u32.to_be_bytes());
        assert!(GainMapMeta::from_iso(&z).is_none());
    }

    #[test]
    fn hdrgm_xmp_round_trips_including_per_channel_sequences() {
        for m in [GainMapMeta { alternate_headroom: 2.0, max: [2.0; 3], ..GainMapMeta::default() }, meta3()] {
            let x = m.to_gain_map_xmp();
            assert!(x.contains("apdi:AuxiliaryImageType"));
            if m.multichannel {
                assert!(x.contains("<hdrgm:GainMapMax><rdf:Seq><rdf:li>2.5</rdf:li>"), "{x}");
            }
            let back = GainMapMeta::from_hdrgm_xmp(&x).unwrap();
            assert_eq!(back.multichannel, m.multichannel);
            for c in 0..3 {
                assert!((back.max[c] - m.max[c]).abs() < 1e-5 && (back.gamma[c] - m.gamma[c]).abs() < 1e-5);
            }
            assert!((back.alternate_headroom - m.alternate_headroom).abs() < 1e-5);
        }
        assert!(GainMapMeta::from_hdrgm_xmp("<x hdrgm:Version=\"1.0\"/>").is_none(), "no GainMapMax");
        assert!(GainMapMeta::from_hdrgm_xmp("hdrgm:GainMapMax=\"nope\"").is_none());
    }

    fn renditions(w: usize, h: usize) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        // a ramp: SDR rolls off at 1, HDR keeps going to 6
        let hdr: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let t = (i % w) as f32 / (w - 1) as f32;
                let v = 0.01 + 6.0 * t * t;
                [v, v * 0.8, v * 0.6]
            })
            .collect();
        // a luminance tone map (one scale per pixel, as LightCraft's renders do), so a single
        // luminance gain can rebuild it exactly
        let sdr = hdr
            .iter()
            .map(|p| {
                let y = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
                let k = 1.0 / (1.0 + y * 0.5);
                p.map(|v| v * k)
            })
            .collect();
        (sdr, hdr)
    }

    #[test]
    fn compute_then_apply_recovers_the_hdr_rendition() {
        let (w, h) = (64, 8);
        let (sdr, hdr) = renditions(w, h);
        for multichannel in [false, true] {
            let o = GainMapOptions { scale: 1, multichannel, ..GainMapOptions::default() };
            let (g, m) = compute(&sdr, &hdr, w, h, [0.2126, 0.7152, 0.0722], &o).unwrap();
            assert!(m.is_valid() && m.alternate_headroom > 1.0);
            let mut out = sdr.clone();
            apply(&mut out, w, h, &g, &m, 1.0).unwrap();
            for (a, b) in out.iter().zip(&hdr) {
                for c in 0..3 {
                    // 8-bit codes over the gain range: within ~2 % in linear light
                    assert!((a[c] - b[c]).abs() <= 0.03 * b[c] + 0.01, "{multichannel}: {a:?} vs {b:?}");
                }
            }
            // weight 0 is the base
            let mut base = sdr.clone();
            apply(&mut base, w, h, &g, &m, 0.0).unwrap();
            for (a, b) in base.iter().zip(&sdr) {
                assert!((a[0] - b[0]).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn the_primary_image_says_it_has_a_gain_map() {
        assert_eq!(primary_iso_marker().len(), ISO_URN.len() + 4);
        assert_eq!(GainMapMeta::from_iso(&primary_iso_marker()[ISO_URN.len()..]), None, "versions only: no parameters");
        let fresh = primary_xmp(None, 1234);
        assert!(fresh.starts_with("<?xpacket") && fresh.contains(r#"hdrgm:Version="1.0""#) && fresh.contains(r#"Item:Length="1234""#));
        let kept = primary_xmp(Some("<x:xmpmeta><rdf:RDF><rdf:Description dc:title=\"t\"/></rdf:RDF></x:xmpmeta>"), 7);
        assert!(kept.contains("dc:title") && kept.contains(r#"Item:Length="7""#) && kept.ends_with("</rdf:RDF></x:xmpmeta>"));
        assert!(primary_xmp(Some("not xmp"), 7).starts_with("<?xpacket"), "nothing to add to: a fresh packet");
    }

    #[test]
    fn sizes_and_parameters_that_cannot_work_are_errors() {
        let px = vec![[0.5f32; 3]; 4];
        let o = GainMapOptions::default();
        assert!(compute(&px, &px, 2, 2, [0.2, 0.7, 0.1], &o).is_ok());
        assert!(compute(&px, &px, 3, 2, [0.2, 0.7, 0.1], &o).is_err(), "fewer pixels than the size");
        assert!(compute(&px, &px, 0, 2, [0.2, 0.7, 0.1], &o).is_err());
        assert!(compute(&px, &px, usize::MAX, 2, [0.2, 0.7, 0.1], &o).is_err(), "size overflow");
        for bad in [GainMapOptions { scale: 3, ..o }, GainMapOptions { gamma: 0.0, ..o }, GainMapOptions { offset: f32::NAN, ..o }] {
            assert!(compute(&px, &px, 2, 2, [0.2, 0.7, 0.1], &bad).is_err(), "{bad:?}");
        }
        let (g, m) = compute(&px, &px, 2, 2, [0.2, 0.7, 0.1], &o).unwrap();
        let mut base = px.clone();
        assert!(apply(&mut base, 2, 2, &g, &m, 1.0).is_ok());
        assert!(apply(&mut base, 3, 2, &g, &m, 1.0).is_err(), "base smaller than the size");
        assert!(apply(&mut base, 2, 2, &GainImage { data: vec![], ..g.clone() }, &m, 1.0).is_err(), "short gain map");
        assert!(apply(&mut base, 2, 2, &GainImage { channels: 2, ..g.clone() }, &m, 1.0).is_err());
        assert!(apply(&mut base, 2, 2, &GainImage { width: 0, ..g.clone() }, &m, 1.0).is_err());
        assert!(apply(&mut base, 2, 2, &GainImage { width: usize::MAX, height: 2, ..g.clone() }, &m, 1.0).is_err(), "gain map size overflow");
        assert!(apply(&mut base, 2, 2, &g, &GainMapMeta { gamma: [0.0; 3], ..m }, 1.0).is_err(), "unusable metadata");
    }
}
