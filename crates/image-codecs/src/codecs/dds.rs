//! DirectDraw Surface (DDS) textures through [`craft_dds`]: its errors and pixel layouts mapped
//! onto this crate's, and the decoder's size checked against [`Limits`] before it allocates.

use crate::SampleType;
use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::format::Format;
use crate::image::{ChannelLayout, Image};
use crate::options::{EncodeOptions, Limits};
use craft_dds::{DdsError, Layout, Texture};

fn error(e: DdsError) -> CodecError {
    match e {
        DdsError::Malformed(reason) => CodecError::malformed(Format::Dds, reason),
        DdsError::Unsupported(reason) => CodecError::unsupported(Format::Dds, reason),
        DdsError::Encode(reason) => CodecError::encode(Format::Dds, reason),
        DdsError::Alloc(len) => CodecError::LimitExceeded(format!("cannot allocate {len} elements for the image")),
        DdsError::LimitExceeded { required, limit } => {
            CodecError::LimitExceeded(format!("decoded texture requires {required} bytes, exceeding limit {limit}"))
        }
        other => CodecError::malformed(Format::Dds, other),
    }
}

pub(crate) fn channel_layout(layout: Layout) -> ChannelLayout {
    match layout {
        Layout::Gray => ChannelLayout::Gray,
        Layout::GrayA => ChannelLayout::GrayA,
        Layout::Rgb => ChannelLayout::Rgb,
        Layout::Rgba => ChannelLayout::Rgba,
    }
}

/// Header-declared extra subresources cannot survive a single-image rewrite, even without presence flags.
pub(crate) fn has_additional_images(bytes: &[u8]) -> Result<bool, CodecError> {
    craft_dds::has_additional_images(bytes).map_err(error)
}

/// Channels wider than 8 bits (the 16-bit formats, 10- and 16-bit masks) open at 16 bits.
pub(crate) fn decode(b: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let texture = Texture::parse(b).map_err(error)?;
    let layout = channel_layout(texture.layout());
    let (width, height) = (texture.width(), texture.height());
    if texture.bits_per_channel() > 8 {
        limits.check(width, height, layout, SampleType::U16)?;
        let pixels = texture.decode16_with_limit(limits.max_alloc).map_err(error)?;
        return Image::from_u16(width, height, layout, &pixels);
    }
    limits.check(width, height, layout, SampleType::U8)?;
    let pixels = texture.decode_with_limit(limits.max_alloc).map_err(error)?;
    Image::from_u8(width, height, layout, pixels)
}

pub(crate) fn encode(image: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let img = image.converted(plan.layout, SampleType::U8);
    let pixels = img.as_u8().ok_or_else(|| CodecError::encode(Format::Dds, "the image was not converted to 8 bits per channel"))?;
    let layout = match img.layout() {
        ChannelLayout::Gray => Layout::Gray,
        ChannelLayout::GrayA => Layout::GrayA,
        ChannelLayout::Rgb => Layout::Rgb,
        ChannelLayout::Rgba => Layout::Rgba,
        // `fidelity::plan` only picks the four layouts above for DDS.
        other => return Err(CodecError::encode(Format::Dds, format!("{other:?} pixels can't be written as `{}`", opts.dds.compression.name()))),
    };
    let (width, height) = img.dimensions();
    craft_dds::encode(width, height, layout, pixels, opts.dds).map_err(error)
}
