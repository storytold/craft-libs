#[cfg(not(target_arch = "wasm32"))]
use std::panic::{AssertUnwindSafe, catch_unwind};

#[cfg(not(target_arch = "wasm32"))]
use crate::Layout;
use crate::{Error, ImageRef, Metadata, Result, Samples};

/// Encoder search effort.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Effort {
    /// Quickest search, with larger output files.
    Fast,
    /// Balanced search.
    #[default]
    Normal,
}

/// Color encoding declared by a JPEG XL image.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JxlColour {
    /// sRGB primaries and transfer curve.
    #[default]
    Srgb,
    /// Display P3 primaries, D65, with the sRGB transfer curve.
    DisplayP3,
    /// Rec. 2020 primaries with the BT.709 transfer curve.
    Rec2020,
    /// Rec. 2020 primaries with the SMPTE ST 2084 PQ transfer curve.
    Rec2020Pq,
}

/// JPEG XL encoder settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodeOptions {
    /// Lossy quality on a 1..=100 scale (90 is approximately visually lossless). Ignored for
    /// mathematically lossless output.
    pub quality: u8,
    /// Use mathematically lossless Modular coding.
    pub lossless: bool,
    /// Encoder search effort.
    pub effort: Effort,
    /// Use progressive encoding; lossless output uses the Squeeze transform.
    pub progressive: bool,
    /// Color encoding to write.
    pub colour: JxlColour,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self { quality: 90, lossless: false, effort: Effort::Normal, progressive: false, colour: JxlColour::Srgb }
    }
}

/// SDR reference white used when converting linear float input to HDR PQ, in cd/m².
pub const HDR_REFERENCE_WHITE_NITS: f32 = 203.0;

/// Encode a still image to JPEG XL.
///
/// Gray input is widened to RGB. Alpha is omitted when every sample is opaque. Float input is
/// encoded only with `JxlColour::Rec2020Pq`; its linear values use SDR white at 203 cd/m².
/// ICC data is embedded only for lossless output, where the explicit ICC profile takes precedence
/// over the enum color encoding. Exif and XMP are embedded when supplied.
pub fn encode(image: &ImageRef<'_>, options: EncodeOptions, metadata: &Metadata) -> Result<Vec<u8>> {
    validate(image)?;
    #[cfg(all(not(target_arch = "wasm32"), feature = "encode"))]
    {
        let pixels = Pixels::from(image, options.colour)?;
        catch_unwind(AssertUnwindSafe(|| run(&pixels, image.width as usize, image.height as usize, options, metadata)))
            .unwrap_or_else(|_| Err(Error::Encode("encoder failed while writing this image".into())))
    }
    #[cfg(any(target_arch = "wasm32", not(feature = "encode")))]
    {
        let _ = (image, options, metadata);
        Err(Error::Unsupported("JPEG XL encoding is unavailable on this target".into()))
    }
}

fn validate(image: &ImageRef<'_>) -> Result<()> {
    if image.width == 0 || image.height == 0 {
        return Err(Error::InvalidImage("zero dimension".into()));
    }
    let pixels = u128::from(image.width) * u128::from(image.height);
    let samples = pixels * image.layout.channels() as u128;
    if samples > usize::MAX as u128 {
        return Err(Error::InvalidImage("image sample count exceeds addressable memory".into()));
    }
    let have = match image.samples {
        Samples::U8(data) => data.len(),
        Samples::U16(data) => data.len(),
        Samples::F32(data) => data.len(),
    };
    if have < samples as usize {
        return Err(Error::InvalidImage(format!("sample buffer has {have} samples, expected at least {samples}")));
    }
    Ok(())
}

#[cfg(all(not(target_arch = "wasm32"), feature = "encode"))]
enum Pixels {
    U8 { data: Vec<u8>, alpha: bool },
    U16 { data: Vec<u16>, alpha: bool },
}

#[cfg(all(not(target_arch = "wasm32"), feature = "encode"))]
impl Pixels {
    fn from(image: &ImageRef<'_>, colour: JxlColour) -> Result<Self> {
        let n = usize::try_from(u64::from(image.width) * u64::from(image.height))
            .map_err(|_| Error::InvalidImage("image pixel count exceeds addressable memory".into()))?;
        Ok(match image.samples {
            Samples::U8(samples) => {
                let (data, alpha) = convert(samples, image.layout, n, |a| a == u8::MAX)?;
                Pixels::U8 { data, alpha }
            }
            Samples::U16(samples) => {
                let (data, alpha) = convert(samples, image.layout, n, |a| a == u16::MAX)?;
                Pixels::U16 { data, alpha }
            }
            Samples::F32(samples) if colour == JxlColour::Rec2020Pq => {
                let count = n.checked_mul(4).ok_or_else(|| Error::InvalidImage("pixel buffer size overflows".into()))?;
                let mut encoded = Vec::new();
                encoded.try_reserve_exact(count).map_err(|e| Error::InvalidImage(format!("cannot reserve pixel buffer: {e}")))?;
                for pixel in samples.chunks_exact(image.layout.channels()).take(n) {
                    let (rgb, alpha) = pixel_to_rgba(pixel, image.layout);
                    for channel in rgb {
                        encoded.push(pq_sample(channel));
                    }
                    encoded.push(alpha_sample(alpha));
                }
                let (data, alpha) = convert(&encoded, Layout::Rgba, n, |a| a == u16::MAX)?;
                Pixels::U16 { data, alpha }
            }
            Samples::F32(_) => return Err(Error::InvalidImage("float samples require Rec. 2020 PQ encoding".into())),
        })
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "encode"))]
fn convert<T: Copy>(samples: &[T], layout: Layout, pixels: usize, opaque: impl Fn(T) -> bool) -> Result<(Vec<T>, bool)> {
    let channels = layout.channels();
    let input = samples
        .get(..pixels.checked_mul(channels).ok_or_else(|| Error::InvalidImage("sample count overflows".into()))?)
        .ok_or_else(|| Error::InvalidImage("sample buffer is too short".into()))?;
    let alpha = layout.has_alpha() && input.chunks_exact(channels).any(|p| p.last().is_some_and(|a| !opaque(*a)));
    let output_channels = if alpha { 4 } else { 3 };
    let output_count = pixels.checked_mul(output_channels).ok_or_else(|| Error::InvalidImage("converted sample count overflows".into()))?;
    let mut out = Vec::new();
    out.try_reserve_exact(output_count).map_err(|e| Error::InvalidImage(format!("cannot reserve converted samples: {e}")))?;
    for pixel in input.chunks_exact(channels) {
        match (layout, pixel) {
            (Layout::Gray, [g]) => out.extend_from_slice(&[*g, *g, *g]),
            (Layout::GrayAlpha, [g, a]) if alpha => out.extend_from_slice(&[*g, *g, *g, *a]),
            (Layout::GrayAlpha, [g, _]) => out.extend_from_slice(&[*g, *g, *g]),
            (Layout::Rgb, [r, g, b]) => out.extend_from_slice(&[*r, *g, *b]),
            (Layout::Rgba, [r, g, b, a]) if alpha => out.extend_from_slice(&[*r, *g, *b, *a]),
            (Layout::Rgba, [r, g, b, _]) => out.extend_from_slice(&[*r, *g, *b]),
            _ => return Err(Error::InvalidImage("unsupported pixel layout".into())),
        }
    }
    Ok((out, alpha))
}

#[cfg(all(not(target_arch = "wasm32"), feature = "encode"))]
fn pixel_to_rgba(pixel: &[f32], layout: Layout) -> ([f32; 3], f32) {
    let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
    match (layout, pixel) {
        (Layout::Gray, [g]) => ([finite(*g); 3], 1.0),
        (Layout::GrayAlpha, [g, a]) => ([finite(*g); 3], finite(*a).clamp(0.0, 1.0)),
        (Layout::Rgb, [r, g, b]) => ([finite(*r), finite(*g), finite(*b)], 1.0),
        (Layout::Rgba, [r, g, b, a]) => ([finite(*r), finite(*g), finite(*b)], finite(*a).clamp(0.0, 1.0)),
        _ => ([0.0; 3], 1.0),
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "encode"))]
fn pq_sample(value: f32) -> u16 {
    let linear_nits = (if value.is_finite() { value.max(0.0) } else { 0.0 }) * HDR_REFERENCE_WHITE_NITS;
    (pq_encode(linear_nits) * 65535.0).round().clamp(0.0, 65535.0) as u16
}

#[cfg(all(not(target_arch = "wasm32"), feature = "encode"))]
fn alpha_sample(value: f32) -> u16 {
    ((if value.is_finite() { value.clamp(0.0, 1.0) } else { 1.0 }) * 65535.0).round() as u16
}

#[cfg(all(not(target_arch = "wasm32"), feature = "encode"))]
fn pq_encode(nits: f32) -> f32 {
    const M1: f32 = 2610.0 / 16384.0;
    const M2: f32 = 2523.0 / 4096.0 * 128.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 4096.0 * 32.0;
    const C3: f32 = 2392.0 / 4096.0 * 32.0;
    let y = (nits / 10_000.0).clamp(0.0, 1.0).powf(M1);
    ((C1 + C2 * y) / (1.0 + C3 * y)).powf(M2)
}

#[cfg(all(not(target_arch = "wasm32"), feature = "encode"))]
fn run(pixels: &Pixels, width: usize, height: usize, options: EncodeOptions, metadata: &Metadata) -> Result<Vec<u8>> {
    use jixel::{ColorEncoding, EncodeConfig, Primaries, Speed, TransferFunction};

    let colour = match options.colour {
        JxlColour::Srgb => ColorEncoding::srgb(),
        JxlColour::DisplayP3 => ColorEncoding::display_p3(),
        JxlColour::Rec2020 => ColorEncoding { primaries: Primaries::Bt2020, transfer: TransferFunction::Bt709, ..ColorEncoding::srgb() },
        JxlColour::Rec2020Pq => ColorEncoding::bt2020_pq(),
    };
    let mut config = EncodeConfig::default()
        .with_quality(f32::from(options.quality.clamp(1, 100)))
        .with_color_encoding(colour)
        .with_lossless(options.lossless)
        .with_progressive(options.progressive)
        .with_speed(match options.effort {
            Effort::Fast => Speed::Fastest,
            Effort::Normal => Speed::Fast,
        });
    if options.colour == JxlColour::Rec2020Pq {
        config = config.with_intensity_target(10_000.0);
    }
    if options.lossless
        && let Some(icc) = &metadata.icc
    {
        config = config.with_icc_profile(icc.clone());
    }
    if let Some(exif) = &metadata.exif {
        config = config.with_exif(exif.clone());
    }
    if let Some(xmp) = &metadata.xmp {
        config = config.with_xmp(xmp.clone());
    }
    let result = match pixels {
        Pixels::U8 { data, alpha: false } => jixel::encode_image(data, width, height, &config),
        Pixels::U8 { data, alpha: true } => jixel::encode_image_with_alpha(data, width, height, &config),
        Pixels::U16 { data, alpha: false } => jixel::encode_image_16bit(data, width, height, &config),
        Pixels::U16 { data, alpha: true } => jixel::encode_image_with_alpha_16bit(data, width, height, &config),
    };
    result.map_err(|error| Error::Encode(error.to_string()))
}
