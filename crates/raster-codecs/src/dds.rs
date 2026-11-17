//! DDS compatibility facade; parsing, mip layout and BC block codecs live in craft-dds.
use crate::{CodecError, Image, Limits, SampleType, bridge};
use craft_image_codecs::Format;

pub fn looks_like_dds(bytes: &[u8]) -> bool {
    craft_image_codecs::detect(bytes) == Some(Format::Dds)
}
pub fn has_additional_images(bytes: &[u8]) -> Result<bool, CodecError> {
    craft_image_codecs::has_unretained_source_data(bytes).map_err(bridge::from_shared_error)
}
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    bridge::format(Format::Dds, bytes, limits)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DdsCompression {
    #[default]
    None,
    Bc1,
    Bc2,
    Bc3,
}
impl DdsCompression {
    pub const ALL: [Self; 4] = [Self::None, Self::Bc1, Self::Bc2, Self::Bc3];
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bc1 => "bc1",
            Self::Bc2 => "bc2",
            Self::Bc3 => "bc3",
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "bc1" | "dxt1" => Some(Self::Bc1),
            "bc2" | "dxt3" => Some(Self::Bc2),
            "bc3" | "dxt5" => Some(Self::Bc3),
            _ => None,
        }
    }
    pub fn is_block(self) -> bool {
        self != Self::None
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DdsOptions {
    pub compression: DdsCompression,
    pub mipmaps: bool,
}
pub fn encode(image: &Image, options: DdsOptions) -> Result<Vec<u8>, CodecError> {
    if image.sample_type() != SampleType::U8 {
        return Err(CodecError::encode("DDS", "DDS is written at 8 bits per channel"));
    }
    // BC storage encodes RGBA blocks. The generic image API can plan a
    // conversion from other layouts, but this compatibility facade accepts
    // only layouts its former writer stored directly.
    let allowed = if options.compression.is_block() {
        image.layout() == crate::ChannelLayout::Rgba
    } else {
        matches!(image.layout(), crate::ChannelLayout::Gray | crate::ChannelLayout::GrayA | crate::ChannelLayout::Rgb | crate::ChannelLayout::Rgba)
    };
    if !allowed {
        return Err(CodecError::encode("DDS", format!("{:?} pixels can't be written as `{}`", image.layout(), options.compression.name())));
    }
    let compression = match options.compression {
        DdsCompression::None => craft_image_codecs::DdsCompression::None,
        DdsCompression::Bc1 => craft_image_codecs::DdsCompression::Bc1,
        DdsCompression::Bc2 => craft_image_codecs::DdsCompression::Bc2,
        DdsCompression::Bc3 => craft_image_codecs::DdsCompression::Bc3,
    };
    let opts =
        craft_image_codecs::EncodeOptions { dds: craft_image_codecs::DdsOptions { compression, mipmaps: options.mipmaps }, ..Default::default() };
    bridge::encode(image, Format::Dds, &opts)
}

/// Metadata for the supported 2D DDS texture, available without pixel data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsInfo {
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    pub alpha: bool,
    /// `None` for a shared decoder compression not exposed by this legacy writer API.
    pub compression: Option<DdsCompression>,
}

fn dds_error(error: craft_dds::DdsError) -> CodecError {
    match error {
        craft_dds::DdsError::Malformed(reason) => CodecError::malformed("DDS", reason),
        craft_dds::DdsError::Unsupported(reason) => CodecError::unsupported("DDS", reason),
        craft_dds::DdsError::Encode(reason) => CodecError::encode("DDS", reason),
        craft_dds::DdsError::Alloc(len) => CodecError::LimitExceeded(format!("cannot allocate {len} bytes for DDS")),
        craft_dds::DdsError::LimitExceeded { required, limit } => {
            CodecError::LimitExceeded(format!("decoded DDS requires {required} bytes, exceeding limit {limit}"))
        }
        other => CodecError::malformed("DDS", other),
    }
}

fn layout(texture: &craft_dds::Texture<'_>) -> crate::ChannelLayout {
    match texture.layout() {
        craft_dds::Layout::Gray => crate::ChannelLayout::Gray,
        craft_dds::Layout::GrayA => crate::ChannelLayout::GrayA,
        craft_dds::Layout::Rgb => crate::ChannelLayout::Rgb,
        craft_dds::Layout::Rgba => crate::ChannelLayout::Rgba,
    }
}

pub fn info(bytes: &[u8]) -> Result<DdsInfo, CodecError> {
    let texture = craft_dds::Texture::parse(bytes).map_err(dds_error)?;
    let compression = match texture.compression() {
        craft_dds::DdsCompression::None => Some(DdsCompression::None),
        craft_dds::DdsCompression::Bc1 => Some(DdsCompression::Bc1),
        craft_dds::DdsCompression::Bc2 => Some(DdsCompression::Bc2),
        craft_dds::DdsCompression::Bc3 => Some(DdsCompression::Bc3),
        _ => None,
    };
    let alpha = matches!(texture.layout(), craft_dds::Layout::GrayA | craft_dds::Layout::Rgba);
    Ok(DdsInfo { width: texture.width(), height: texture.height(), levels: texture.levels(), alpha, compression })
}

/// Decode the declared mip chain, charging all retained pixels to one output-byte budget.
/// Cancellation is checked before each level; no partial chain is returned.
pub fn decode_levels(bytes: &[u8], limits: &Limits, cancel: &dyn Fn() -> bool) -> Result<Vec<Image>, CodecError> {
    let texture = craft_dds::Texture::parse(bytes).map_err(dds_error)?;
    let mut images = Vec::new();
    images.try_reserve_exact(texture.levels() as usize).map_err(|_| CodecError::LimitExceeded("cannot allocate DDS mip list".into()))?;
    let mut remaining = limits.max_alloc;
    for i in 0..texture.levels() {
        if cancel() {
            return Err(CodecError::malformed("DDS", "DDS import cancelled"));
        }
        let level = texture.mip_level(i).map_err(dds_error)?;
        let layout = layout(&level);
        let level_limits = Limits { max_alloc: remaining, ..*limits };
        level_limits.check(level.width(), level.height(), layout, SampleType::U8)?;
        let pixels = level.decode_with_limit(remaining).map_err(dds_error)?;
        remaining = remaining.checked_sub(pixels.len() as u64).ok_or_else(|| CodecError::LimitExceeded("DDS mip chain exceeds max_alloc".into()))?;
        images.push(Image::from_u8(level.width(), level.height(), layout, pixels)?);
    }
    Ok(images)
}
