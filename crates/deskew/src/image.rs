//! A borrowed, validated sample buffer and its layout.

use crate::Error;

/// The largest image accepted, in pixels (16384 × 16384).
pub const MAX_PIXELS: u64 = 1 << 28;

/// The most components a pixel may have.
pub const MAX_COMPONENTS: u8 = 32;

/// Which sample value is blank paper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Paper {
    /// The largest sample value is paper: grey, RGB and other additive spaces.
    Max,
    /// Zero is paper: CMYK and other subtractive spaces, or a grey image decoded inverted.
    Zero,
}

/// How samples are packed (the PDF image layout, ISO 32000-2 §8.9.3): components interleaved,
/// each row padded to a byte boundary, most significant bit first, 16-bit samples big-endian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    /// Components per pixel, 1 to [`MAX_COMPONENTS`].
    pub components: u8,
    /// 1, 2, 4, 8 or 16.
    pub bits_per_component: u8,
    pub paper: Paper,
    /// Samples are palette indices: never blended, and ink is whatever differs from the
    /// background. Requires one component.
    pub indexed: bool,
}

impl Layout {
    /// A layout of direct (not indexed) samples. [`Image::new`] checks it.
    #[must_use]
    pub fn new(width: u32, height: u32, components: u8, bits_per_component: u8, paper: Paper) -> Self {
        Self { width, height, components, bits_per_component, paper, indexed: false }
    }

    /// An 8-bit greyscale layout, paper white.
    #[must_use]
    pub fn grey8(width: u32, height: u32) -> Self {
        Self::new(width, height, 1, 8, Paper::Max)
    }

    /// The same layout with its samples read as palette indices.
    #[must_use]
    pub fn as_indexed(self) -> Self {
        Self { indexed: true, ..self }
    }
}

/// Samples checked against their [`Layout`].
#[derive(Debug, Clone, Copy)]
pub struct Image<'a> {
    layout: Layout,
    samples: &'a [u8],
    row_bytes: usize,
    len: usize,
}

impl<'a> Image<'a> {
    /// Check `samples` against `layout`. Extra trailing bytes are ignored.
    ///
    /// # Errors
    /// [`Error::Layout`] for an empty image, an unsupported depth or component count, or an
    /// indexed image with more than one component; [`Error::TooLarge`] past [`MAX_PIXELS`];
    /// [`Error::ShortBuffer`] when `samples` is shorter than the layout needs.
    pub fn new(layout: Layout, samples: &'a [u8]) -> Result<Self, Error> {
        if layout.width == 0 || layout.height == 0 {
            return Err(Error::Layout("the image is empty"));
        }
        if !matches!(layout.bits_per_component, 1 | 2 | 4 | 8 | 16) {
            return Err(Error::Layout("bits per component must be 1, 2, 4, 8 or 16"));
        }
        if layout.components == 0 || layout.components > MAX_COMPONENTS {
            return Err(Error::Layout("components must be 1 to 32"));
        }
        if layout.indexed && layout.components != 1 {
            return Err(Error::Layout("an indexed image has one component"));
        }
        let pixels = u64::from(layout.width) * u64::from(layout.height);
        let too_large = Error::TooLarge { pixels, max: MAX_PIXELS };
        if pixels > MAX_PIXELS {
            return Err(too_large);
        }
        let row_bits = u64::from(layout.width) * u64::from(layout.components) * u64::from(layout.bits_per_component);
        let row_bytes = usize::try_from(row_bits.div_ceil(8)).map_err(|_| too_large.clone())?;
        let len = row_bytes.checked_mul(layout.height as usize).ok_or(too_large)?;
        if samples.len() < len {
            return Err(Error::ShortBuffer { needed: len, got: samples.len() });
        }
        Ok(Self { layout, samples, row_bytes, len })
    }

    #[must_use]
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// Bytes one packed row takes.
    #[must_use]
    pub fn row_bytes(&self) -> usize {
        self.row_bytes
    }

    /// Bytes the whole image takes.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.len
    }

    pub(crate) fn width(&self) -> u32 {
        self.layout.width
    }

    pub(crate) fn height(&self) -> u32 {
        self.layout.height
    }

    pub(crate) fn components(&self) -> u32 {
        u32::from(self.layout.components)
    }

    /// The largest sample value.
    pub(crate) fn max(&self) -> u32 {
        max_sample(self.layout.bits_per_component)
    }

    /// Whether samples may be blended: 8 bits or more and not palette indices.
    pub(crate) fn blends(&self) -> bool {
        self.layout.bits_per_component >= 8 && !self.layout.indexed
    }

    /// Where sample `c` of pixel `(x, y)` starts, as (byte, bit within it).
    fn locate(&self, x: u32, y: u32, c: u32) -> (usize, usize) {
        let bpc = usize::from(self.layout.bits_per_component);
        let bit = (x as usize * self.components() as usize + c as usize) * bpc;
        (y as usize * self.row_bytes + bit / 8, bit % 8)
    }

    /// Sample `c` of pixel `(x, y)`; 0 outside the buffer.
    pub(crate) fn get(&self, x: u32, y: u32, c: u32) -> u32 {
        read_sample(self.samples, self.layout.bits_per_component, self.locate(x, y, c))
    }

    /// Write sample `c` of pixel `(x, y)` into `out`, a buffer of this layout; ignored outside it.
    pub(crate) fn put(&self, out: &mut [u8], x: u32, y: u32, c: u32, value: u32) {
        write_sample(out, self.layout.bits_per_component, self.locate(x, y, c), value);
    }

    /// How far pixel `(x, y)` is from paper towards full ink, 0 to 255. For an indexed image,
    /// 255 when it differs from `background` and 0 when not.
    pub(crate) fn ink_level(&self, x: u32, y: u32, background: &[u32]) -> u8 {
        if self.layout.indexed {
            let differs = background.first().is_some_and(|&b| self.get(x, y, 0) != b);
            return if differs { 255 } else { 0 };
        }
        let sum: u64 = (0..self.components()).map(|c| u64::from(self.get(x, y, c))).sum();
        let full = u64::from(self.max()) * u64::from(self.components());
        let level = match self.layout.paper {
            Paper::Max => full.saturating_sub(sum),
            Paper::Zero => sum,
        };
        u8::try_from(level.saturating_mul(255) / full.max(1)).unwrap_or(u8::MAX)
    }
}

pub(crate) fn max_sample(bits: u8) -> u32 {
    match bits {
        1 => 1,
        2 => 3,
        4 => 15,
        8 => 255,
        _ => 65535,
    }
}

fn read_sample(samples: &[u8], bits: u8, (at, bit): (usize, usize)) -> u32 {
    let byte = |i: usize| u32::from(samples.get(i).copied().unwrap_or(0));
    match bits {
        16 => (byte(at) << 8) | byte(at.saturating_add(1)),
        8 => byte(at),
        _ => {
            let shift = 8u32.saturating_sub(u32::from(bits) + bit as u32);
            (byte(at) >> shift) & max_sample(bits)
        }
    }
}

fn write_sample(out: &mut [u8], bits: u8, (at, bit): (usize, usize), value: u32) {
    let value = value.min(max_sample(bits));
    match bits {
        16 => {
            let [hi, lo] = u16::try_from(value).unwrap_or(u16::MAX).to_be_bytes();
            if let Some(b) = out.get_mut(at) {
                *b = hi;
            }
            if let Some(b) = out.get_mut(at.saturating_add(1)) {
                *b = lo;
            }
        }
        8 => {
            if let Some(b) = out.get_mut(at) {
                *b = u8::try_from(value).unwrap_or(u8::MAX);
            }
        }
        _ => {
            let shift = 8u32.saturating_sub(u32::from(bits) + bit as u32);
            let mask = max_sample(bits) << shift;
            if let Some(b) = out.get_mut(at) {
                let merged = (u32::from(*b) & !mask) | ((value << shift) & mask);
                *b = u8::try_from(merged & 0xff).unwrap_or(0);
            }
        }
    }
}

/// The most border pixels [`background`] reads along each side.
const BORDER_SAMPLES: u32 = 1024;

/// The pixel (all components) most common along the image's border, read at up to
/// [`BORDER_SAMPLES`] evenly spaced points a side; ties go to the smallest.
pub(crate) fn background(image: &Image<'_>) -> Vec<u32> {
    let (w, h) = (image.width(), image.height());
    let (right, bottom) = (w.saturating_sub(1), h.saturating_sub(1));
    let along = |n: u32| (0..n).step_by(n.div_ceil(BORDER_SAMPLES).max(1) as usize);
    let mut counts: std::collections::BTreeMap<Vec<u32>, usize> = std::collections::BTreeMap::new();
    let border = along(w).flat_map(|x| [(x, 0), (x, bottom)]).chain(along(h).flat_map(|y| [(0, y), (right, y)]));
    for (x, y) in border {
        let pixel: Vec<u32> = (0..image.components()).map(|c| image.get(x, y, c)).collect();
        *counts.entry(pixel).or_default() += 1;
    }
    counts.into_iter().fold((Vec::new(), 0), |best, (pixel, n)| if n > best.1 { (pixel, n) } else { best }).0
}
