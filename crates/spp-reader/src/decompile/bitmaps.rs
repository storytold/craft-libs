//! Images stored inside a compiled graph, converted to the PNG files the graph runtime's `bitmap` node reads. The reader
//! decodes them ([`crate::decode::jpeg`], [`crate::decode::graph_package::compiled::StoredImage`]); this only writes PNGs.

use crate::decode::graph_package::compiled::{RawLayout, StoredChannels};
use crate::decode::jpeg;
use crate::png_out::{ColorType, encode_raw};

/// The JPEG as an 8-bit PNG, keeping its channel count: grayscale stays grayscale.
pub fn jpeg_to_png(jpeg: &[u8]) -> Result<Vec<u8>, String> {
    let img = jpeg::decode(jpeg).map_err(|e| e.to_string())?;
    let color = if img.channels == 1 { ColorType::Gray } else { ColorType::Rgb };
    Ok(encode_raw(img.width, img.height, color, 8, &img.samples))
}

/// A colour JPEG with a greyscale JPEG of the same size as its alpha, as an 8-bit RGBA PNG.
pub fn jpeg_with_alpha_to_png(jpeg: &[u8], alpha: &[u8]) -> Result<Vec<u8>, String> {
    let img = jpeg::decode_with_alpha(jpeg, alpha).map_err(|e| e.to_string())?;
    Ok(encode_raw(img.width, img.height, ColorType::Rgba, 8, &img.samples))
}

/// The file name of the image at section offset `offset`, converting it the first time it is seen; `alpha` the JPEG
/// stored as its alpha, if any.
pub fn add(bitmaps: &mut Vec<(String, Vec<u8>)>, offset: u32, jpeg: &[u8], alpha: Option<&[u8]>) -> Result<String, String> {
    let file = format!("bitmap_{offset:x}.png");
    if !bitmaps.iter().any(|(name, _)| *name == file) {
        let png = match alpha {
            Some(a) => jpeg_with_alpha_to_png(jpeg, a)?,
            None => jpeg_to_png(jpeg)?,
        };
        bitmaps.push((file.clone(), png));
    }
    Ok(file)
}

fn color_type(channels: StoredChannels) -> ColorType {
    match channels {
        StoredChannels::Gray => ColorType::Gray,
        StoredChannels::Rgb => ColorType::Rgb,
        StoredChannels::Rgba => ColorType::Rgba,
    }
}

/// The file name of the uncompressed image at section offset `offset`, as stored by `layout` (16-bit samples little-endian).
pub fn add_raw(bitmaps: &mut Vec<(String, Vec<u8>)>, offset: u32, w: usize, h: usize, layout: RawLayout, samples: &[u8]) -> String {
    let file = format!("bitmap_{offset:x}_{w}x{h}.png");
    if !bitmaps.iter().any(|(name, _)| *name == file) {
        let color = color_type(layout.channels);
        let png = if layout.bytes == 2 {
            let big_endian: Vec<u8> = samples.chunks_exact(2).flat_map(|s| s.iter().rev().copied()).collect();
            encode_raw(w, h, color, 16, &big_endian)
        } else {
            encode_raw(w, h, color, 8, samples)
        };
        bitmaps.push((file.clone(), png));
    }
    file
}

#[cfg(test)]
mod tests {
    use super::{RawLayout, add_raw};
    use crate::decode::graph_package::compiled::StoredChannels;

    #[test]
    fn rgba16_samples_keep_their_values_and_order() {
        let mut files = Vec::new();
        let samples: Vec<u8> = [0x8000u16, 0x4000, 0xffff, 0xffff].iter().flat_map(|v| v.to_le_bytes()).collect();
        let layout = RawLayout::of_flags(0x0718).unwrap();
        add_raw(&mut files, 4, 1, 1, layout, &samples);
        let (w, h, depth, color, samples) = crate::png_out::decode(&files[0].1);
        assert_eq!((w, h, depth, color), (1, 1, 16, 6));
        assert_eq!(samples, [0x80, 0x00, 0x40, 0x00, 0xff, 0xff, 0xff, 0xff], "big-endian, in channel order");
    }

    #[test]
    fn gray16_samples_keep_their_values_in_the_png() {
        let (mut files, samples) = (Vec::new(), [0x00u8, 0x80, 0xff, 0xff]);
        let gray16 = RawLayout { channels: StoredChannels::Gray, bytes: 2 };
        let name = add_raw(&mut files, 4, 2, 1, gray16, &samples);
        assert_eq!(add_raw(&mut files, 4, 2, 1, gray16, &samples), name, "the same image is stored once");
        assert_eq!(files.len(), 1);
        let (w, h, depth, color, samples) = crate::png_out::decode(&files[0].1);
        assert_eq!((w, h, depth, color), (2, 1, 16, 0));
        assert_eq!(samples, [0x80, 0x00, 0xff, 0xff]);
    }
}
