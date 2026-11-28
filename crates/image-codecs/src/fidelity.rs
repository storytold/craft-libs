//! Encode planning and fidelity warnings. `encode` and `fidelity_warnings`
//! share [`plan`], so a warning is emitted exactly when the encoder will
//! actually transform or drop something.

use std::fmt;

use crate::codecs::{dds, dpx};
use crate::format::{Format, caps};
use crate::image::{ChannelLayout, Image, SampleType};
use crate::options::{DdsCompression, EncodeOptions};

/// Something that will be lost or altered when writing an image to a format.
#[derive(Debug, Clone, PartialEq)]
pub enum FidelityWarning {
    /// The format cannot be written in this build.
    WriteUnsupported {
        format: Format,
    },
    /// Bit depth / precision reduced, e.g. 16-bit → 8-bit or f32 → f16.
    DepthReduced {
        from: SampleType,
        to: SampleType,
    },
    /// Samples will be stored with fewer bits than their sample type holds: DPX at 10 or 12
    /// bits, Cineon at 10.
    BitsReduced {
        from: SampleType,
        bits: u8,
    },
    /// Float samples outside `[0, 1]` will be clipped by integer storage.
    RangeClipped,
    /// Non-opaque alpha will be discarded.
    AlphaDiscarded,
    /// Alpha will be reduced to fully transparent / fully opaque (GIF, DDS BC1, IFF).
    AlphaBinarized,
    /// CMYK data will be converted (naively, not colour-managed) to another model.
    CmykConverted {
        to: ChannelLayout,
    },
    /// L*a*b* data will be converted to another model (sRGB, D50 to D65).
    LabConverted {
        to: ChannelLayout,
    },
    /// Colour data will be reduced to grayscale.
    ColorToGray,
    /// The image will be quantized to a 256-colour palette.
    PaletteQuantized,
    /// The format's encoder is lossy. DDS BC5 also keeps only red and green: blue is dropped and
    /// reads back as 0 (for a normal map, the renderer rebuilds Z from X and Y).
    LossyCompression,
    IccDropped,
    ExifDropped,
    XmpDropped,
    DpiDropped,
    TextDropped,
    /// Metadata larger than the format can hold in one piece (`what`: "EXIF", "XMP") will be
    /// dropped.
    MetadataTooLarge {
        what: &'static str,
        bytes: usize,
    },
    /// Image exceeds the format's maximum size; encoding will fail.
    DimensionsExceeded {
        max_width: u32,
        max_height: u32,
    },
}

impl FidelityWarning {
    /// `true` for warnings that make encoding fail outright.
    pub fn is_fatal(&self) -> bool {
        matches!(self, FidelityWarning::WriteUnsupported { .. } | FidelityWarning::DimensionsExceeded { .. })
    }
}

fn depth_name(s: SampleType) -> &'static str {
    match s {
        SampleType::U8 => "8-bit",
        SampleType::U16 => "16-bit",
        SampleType::F16 => "16-bit float",
        SampleType::F32 => "32-bit float",
    }
}

impl fmt::Display for FidelityWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use FidelityWarning::*;
        match self {
            WriteUnsupported { format } => {
                write!(f, "{} cannot be written in this build", format.name())
            }
            DepthReduced { from, to } => write!(f, "{} will be reduced to {}", depth_name(*from), depth_name(*to)),
            BitsReduced { from, bits } => write!(f, "{} will be reduced to {bits}-bit", depth_name(*from)),
            RangeClipped => write!(f, "HDR values outside 0..1 will be clipped"),
            AlphaDiscarded => write!(f, "alpha will be discarded"),
            AlphaBinarized => write!(f, "alpha will be reduced to on/off transparency"),
            CmykConverted { to } => {
                write!(f, "CMYK will be converted to {to:?} (not colour-managed)")
            }
            LabConverted { to } => write!(f, "Lab will be converted to {to:?} (sRGB)"),
            ColorToGray => write!(f, "colour will be converted to grayscale"),
            PaletteQuantized => write!(f, "colours will be quantized to a 256-colour palette"),
            LossyCompression => write!(f, "lossy compression"),
            IccDropped => write!(f, "ICC profile not supported; it will be dropped"),
            ExifDropped => write!(f, "EXIF not supported; it will be dropped"),
            XmpDropped => write!(f, "XMP not supported; it will be dropped"),
            DpiDropped => write!(f, "resolution (DPI) not supported; it will be dropped"),
            TextDropped => write!(f, "text metadata not supported; it will be dropped"),
            MetadataTooLarge { what, bytes } => write!(f, "{what} ({bytes} bytes) is too large for the format; it will be dropped"),
            DimensionsExceeded { max_width, max_height } => {
                write!(f, "image exceeds the format maximum of {max_width}x{max_height}")
            }
        }
    }
}

/// Target storage chosen for an image/format pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Plan {
    pub layout: ChannelLayout,
    pub sample: SampleType,
}

fn pick_sample(src: SampleType, depths: &[SampleType]) -> SampleType {
    if depths.contains(&src) {
        return src;
    }
    let pref: &[SampleType] = match src {
        // Integers widen into float losslessly; F32 represents U8/U16 exactly.
        SampleType::U8 => &[SampleType::U16, SampleType::F32, SampleType::F16],
        SampleType::U16 => &[SampleType::F32, SampleType::U8, SampleType::F16],
        SampleType::F16 => &[SampleType::F32, SampleType::U16, SampleType::U8],
        SampleType::F32 => &[SampleType::F16, SampleType::U16, SampleType::U8],
    };
    pref.iter().copied().find(|s| depths.contains(s)).unwrap_or(depths[0])
}

fn pick_layout(src: ChannelLayout, layouts: &[ChannelLayout], alpha: bool) -> ChannelLayout {
    let mut cand = src;
    if !alpha {
        cand = cand.without_alpha();
    }
    if layouts.contains(&cand) {
        return cand;
    }
    // CMYK and Lab fall back to RGB; gray expands to RGB.
    if cand.is_cmyk() || cand.is_lab() || cand.is_gray() {
        cand = if cand.has_alpha() { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
    }
    if layouts.contains(&cand) {
        return cand;
    }
    if layouts.contains(&cand.with_alpha()) {
        return cand.with_alpha();
    }
    if layouts.contains(&cand.without_alpha()) {
        return cand.without_alpha();
    }
    layouts[0]
}

pub(crate) fn plan(image: &Image, format: Format, opts: &EncodeOptions) -> Plan {
    let c = caps(format);
    let (layout, sample) = (image.layout(), image.sample_type());
    match format {
        // PNM: integer data can use any layout (PAM); float goes to PFM
        // which only has gray and RGB.
        Format::Pnm if sample.is_float() => {
            Plan { layout: pick_layout(layout, &[ChannelLayout::Gray, ChannelLayout::Rgb], false), sample: SampleType::F32 }
        }
        // DPX has no luma + alpha descriptor: gray with alpha is written as luma, alpha dropped.
        // The sample type follows the chosen bits (U8 for 8, U16 for 10, 12 and 16).
        Format::Dpx => Plan {
            layout: if layout == ChannelLayout::GrayA { ChannelLayout::Gray } else { pick_layout(layout, c.layouts, c.alpha) },
            sample: dpx::plan_sample(opts.dpx_bits, sample),
        },
        // CIELab has no float form: float Lab is written with 16 bits.
        Format::Tiff if layout.is_lab() && sample.is_float() => Plan { layout: pick_layout(layout, c.layouts, c.alpha), sample: SampleType::U16 },
        // A block-compressed DDS stores what its compression holds whatever the source: gray for
        // BC4, red and green for BC5 (written from RGB, blue dropped), RGBA for BC1 to BC3.
        // Uncompressed keeps gray and RGB.
        Format::Dds if opts.dds.compression.is_block() => {
            Plan { layout: opts.dds.compression.layout().map_or(ChannelLayout::Rgba, dds::channel_layout), sample: SampleType::U8 }
        }
        _ => Plan { layout: pick_layout(layout, c.layouts, c.alpha), sample: pick_sample(sample, c.depths) },
    }
}

/// Warnings for writing `image` as `format` with default options.
pub fn fidelity_warnings(image: &Image, format: Format) -> Vec<FidelityWarning> {
    fidelity_warnings_with(image, format, &EncodeOptions::default())
}

/// Warnings for writing `image` as `format` with the given options.
pub fn fidelity_warnings_with(image: &Image, format: Format, opts: &EncodeOptions) -> Vec<FidelityWarning> {
    use FidelityWarning as W;
    let format_caps = caps(format);
    let mut warnings = Vec::new();
    if !format_caps.write {
        warnings.push(W::WriteUnsupported { format });
        return warnings;
    }
    if let Some((max_width, max_height)) = format.max_dimensions()
        && (image.width() > max_width || image.height() > max_height)
    {
        warnings.push(W::DimensionsExceeded { max_width, max_height });
    }
    let encode_plan = plan(image, format, opts);
    let (source_layout, source_sample) = (image.layout(), image.sample_type());

    // Depth
    let lossy_depth = match (source_sample, encode_plan.sample) {
        (source, target) if source == target => false,
        (SampleType::U8, _) => false,
        (SampleType::U16, SampleType::F32) => false,
        (SampleType::F16, SampleType::F32) => false,
        _ => true,
    };
    // 10/12-bit DPX and Cineon drop bits: floats always, U16 unless already code values
    if let Some(bits) = dpx::packed_bits(format, opts).filter(|_| source_sample != SampleType::U8) {
        if source_sample.is_float() || !dpx::keeps_u16_samples(&image.converted(encode_plan.layout, SampleType::U16), bits) {
            warnings.push(W::BitsReduced { from: source_sample, bits });
        }
    } else if lossy_depth {
        warnings.push(W::DepthReduced { from: source_sample, to: encode_plan.sample });
    }
    // FITS stores floats as 0..1 (see `codecs::fits`), so out-of-range values are clipped too
    if source_sample.is_float() && (!encode_plan.sample.is_float() || format == Format::Fits) && image.has_out_of_range() {
        warnings.push(W::RangeClipped);
    }

    // Layout
    if source_layout.has_alpha() && !encode_plan.layout.has_alpha() && image.has_translucency() {
        warnings.push(W::AlphaDiscarded);
    }
    // An IFF/ILBM mask plane holds one bit per pixel
    let binary_alpha = matches!(format, Format::Gif | Format::Iff) || (format == Format::Dds && opts.dds.compression == DdsCompression::Bc1);
    if binary_alpha && encode_plan.layout.has_alpha() && image.has_translucency() {
        warnings.push(W::AlphaBinarized);
    }
    if source_layout.is_cmyk() && !encode_plan.layout.is_cmyk() {
        warnings.push(W::CmykConverted { to: encode_plan.layout });
    }
    if source_layout.is_lab() && !encode_plan.layout.is_lab() {
        warnings.push(W::LabConverted { to: encode_plan.layout });
    }
    if !source_layout.is_gray() && encode_plan.layout.is_gray() {
        warnings.push(W::ColorToGray);
    }

    // Compression
    if format == Format::Gif {
        warnings.push(W::PaletteQuantized);
    } else if format_caps.lossy || (format == Format::WebP && !opts.webp_lossless) || (format == Format::Dds && opts.dds.compression.is_block()) {
        warnings.push(W::LossyCompression);
    }

    // Metadata
    // A raw JPEG 2000 codestream has no colour box
    let raw_jp2 = format == Format::Jp2 && opts.jp2_codestream;
    if image.icc.is_some() && opts.embed_icc && (!format_caps.icc || raw_jp2 || !source_layout.same_model(encode_plan.layout)) {
        warnings.push(W::IccDropped);
    }
    if opts.embed_metadata {
        let meta = &image.meta;
        if meta.exif.is_some() && !format_caps.exif {
            warnings.push(W::ExifDropped);
        }
        if meta.xmp.is_some() && !format_caps.xmp {
            warnings.push(W::XmpDropped);
        }
        if format == Format::Jpeg {
            if let Some(exif) = meta.exif.as_deref().filter(|e| crate::codecs::jpeg::exif_segment(e, None).is_none()) {
                warnings.push(W::MetadataTooLarge { what: "EXIF", bytes: exif.len() });
            }
            if let Some(xmp) = meta.xmp.as_deref().filter(|x| crate::codecs::jpeg::xmp_segment(x, None).is_none()) {
                warnings.push(W::MetadataTooLarge { what: "XMP", bytes: xmp.len() });
            }
        }
        if meta.dpi.is_some() && (!format_caps.dpi || (format == Format::WebP && !crate::codecs::webp::keeps_dpi(image))) {
            warnings.push(W::DpiDropped);
        }
        // DPX keeps its own header fields, not free-form text
        let text_kept = |key: &str| format_caps.text && (format != Format::Dpx || crate::codecs::dpx::KEYS.contains(&key));
        if meta.text.iter().any(|(key, _)| !text_kept(key)) {
            warnings.push(W::TextDropped);
        }
    }
    warnings
}
