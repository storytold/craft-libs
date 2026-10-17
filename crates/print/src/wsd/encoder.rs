//! Producing the document in a format the printer accepts.
//!
//! The job code asks an encoder which formats it can produce (best first), picks the first one
//! the printer lists, and then asks for the bytes in that format together with the settings that
//! belong to the picture (colour, resolution), not the ticket.
//!
//! Implemented here: [`PassThrough`] (bytes already in a printable format, such as a PDF or a
//! ready-made raster), [`Chain`] (several alternatives) and [`RasterEncoder`] (pages rendered by a
//! caller-supplied [`PageSource`], packed as PWG raster, see [`super::pwg`]). Not implemented, and
//! left to follow-ups: JPEG/PNG/TIFF image encoders, XPS and PCL/PostScript generation, and
//! page-set imposition beyond what the renderer already offers.

use std::borrow::Cow;

use super::error::{Result, WsdError};
use super::pwg::{self, Duplex, PwgOptions, RasterPage};
use super::ticket::Sides;

/// Colour or monochrome output. The print schema has no colour setting: this is applied when the
/// picture is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// Colour when the printer says it can, else grey.
    #[default]
    Auto,
    Color,
    Mono,
}

/// What an encoder needs to know besides the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodeContext {
    /// Render in colour (already resolved against the printer's `ColorSupported`).
    pub color: bool,
    /// Pixels per inch, horizontal and vertical.
    pub dpi: (u32, u32),
    pub sides: Option<Sides>,
    /// The media size name asked for, if any (encoders that lay out pages may use it).
    pub media_size: Option<String>,
}

/// Produces the document bytes.
pub trait DocumentEncoder {
    /// The formats this encoder can produce, best first, spelled as MIME types.
    fn formats(&self) -> Vec<String>;
    /// Produce the bytes in `format` (one of [`Self::formats`]). An encoder that already holds the
    /// bytes lends them instead of copying.
    fn encode(&self, format: &str, ctx: &EncodeContext) -> Result<Cow<'_, [u8]>>;
}

/// Bytes that are already in a printable format. The format string is sent as given.
#[derive(Clone, Debug)]
pub struct PassThrough<'a> {
    format: String,
    bytes: Cow<'a, [u8]>,
}

impl<'a> PassThrough<'a> {
    /// `bytes` may be borrowed (`&[u8]`) or owned (`Vec<u8>`).
    pub fn new(format: impl Into<String>, bytes: impl Into<Cow<'a, [u8]>>) -> Self {
        Self { format: format.into(), bytes: bytes.into() }
    }

    /// A PDF file. PDF is not a well-known value in the schema; a printer that accepts it lists
    /// `application/pdf` as an extension value, and PDF is only sent to such a printer.
    pub fn pdf(bytes: impl Into<Cow<'a, [u8]>>) -> Self {
        Self::new("application/pdf", bytes)
    }
}

impl DocumentEncoder for PassThrough<'_> {
    fn formats(&self) -> Vec<String> {
        vec![self.format.clone()]
    }

    fn encode(&self, format: &str, _ctx: &EncodeContext) -> Result<Cow<'_, [u8]>> {
        if format.eq_ignore_ascii_case(&self.format) {
            Ok(Cow::Borrowed(&self.bytes))
        } else {
            Err(WsdError::Encode(format!("cannot produce {format}")))
        }
    }
}

/// Several encoders tried in order: the formats of the first come first.
pub struct Chain<'a>(pub Vec<Box<dyn DocumentEncoder + 'a>>);

impl DocumentEncoder for Chain<'_> {
    fn formats(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for f in self.0.iter().flat_map(|e| e.formats()) {
            if !out.iter().any(|o| o.eq_ignore_ascii_case(&f)) {
                out.push(f);
            }
        }
        out
    }

    fn encode(&self, format: &str, ctx: &EncodeContext) -> Result<Cow<'_, [u8]>> {
        match self.0.iter().find(|e| e.formats().iter().any(|f| f.eq_ignore_ascii_case(format))) {
            Some(e) => e.encode(format, ctx),
            None => Err(WsdError::Encode(format!("cannot produce {format}"))),
        }
    }
}

/// Renders pages to pixels (the application's renderer).
pub trait PageSource {
    fn page_count(&self) -> usize;
    /// Page `index` (0-based) at `dpi`; `gray` asks for 8-bit grey, else 24-bit RGB.
    fn render(&self, index: usize, dpi: (u32, u32), gray: bool) -> Result<RasterPage>;
}

/// Packs the pages of a [`PageSource`] as PWG raster.
pub struct RasterEncoder<S: PageSource> {
    pub source: S,
}

impl<S: PageSource> RasterEncoder<S> {
    pub fn new(source: S) -> Self {
        Self { source }
    }
}

/// The MIME type of PWG raster.
pub const PWG_RASTER: &str = "image/pwg-raster";

impl<S: PageSource> DocumentEncoder for RasterEncoder<S> {
    fn formats(&self) -> Vec<String> {
        vec![PWG_RASTER.to_owned()]
    }

    fn encode(&self, format: &str, ctx: &EncodeContext) -> Result<Cow<'_, [u8]>> {
        if !format.eq_ignore_ascii_case(PWG_RASTER) {
            return Err(WsdError::Encode(format!("cannot produce {format}")));
        }
        let n = self.source.page_count();
        if n == 0 {
            return Err(WsdError::Encode("there are no pages to print".into()));
        }
        if n > pwg::MAX_PAGES {
            return Err(WsdError::Encode(format!("{n} pages is more than the {} one job may carry", pwg::MAX_PAGES)));
        }
        let mut pages = Vec::with_capacity(n);
        for i in 0..n {
            pages.push(self.source.render(i, ctx.dpi, !ctx.color)?);
        }
        let duplex = match ctx.sides {
            Some(Sides::TwoSidedLongEdge) => Duplex::LongEdge,
            Some(Sides::TwoSidedShortEdge) => Duplex::ShortEdge,
            _ => Duplex::Off,
        };
        pwg::encode(&pages, &PwgOptions { dpi: ctx.dpi, duplex }).map(Cow::Owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> EncodeContext {
        EncodeContext { color: true, dpi: (300, 300), sides: None, media_size: None }
    }

    #[test]
    fn pass_through_produces_only_its_own_format() {
        let p = PassThrough::pdf(vec![1u8, 2, 3]);
        assert_eq!(p.formats(), ["application/pdf"]);
        assert_eq!(&*p.encode("Application/PDF", &ctx()).unwrap(), [1, 2, 3]);
        assert!(p.encode("image/jpeg", &ctx()).is_err());
    }

    #[test]
    fn a_chain_lists_formats_once_in_order_and_dispatches() {
        let c = Chain(vec![
            Box::new(PassThrough::new("image/jpeg", vec![9u8])),
            Box::new(PassThrough::pdf(vec![7u8])),
            Box::new(PassThrough::new("IMAGE/JPEG", vec![8u8])),
        ]);
        assert_eq!(c.formats(), ["image/jpeg", "application/pdf"]);
        assert_eq!(&*c.encode("application/pdf", &ctx()).unwrap(), [7]);
        assert_eq!(&*c.encode("image/jpeg", &ctx()).unwrap(), [9]);
        assert!(c.encode("text/plain", &ctx()).is_err());
    }

    struct Blank(usize);
    impl PageSource for Blank {
        fn page_count(&self) -> usize {
            self.0
        }
        fn render(&self, _i: usize, dpi: (u32, u32), gray: bool) -> Result<RasterPage> {
            let (w, h) = (dpi.0 / 10, dpi.1 / 10);
            Ok(RasterPage { width: w, height: h, gray, pixels: vec![255; (w * h) as usize * if gray { 1 } else { 3 }] })
        }
    }

    #[test]
    fn the_raster_encoder_packs_every_page() {
        let e = RasterEncoder::new(Blank(2));
        let bytes = e.encode("image/pwg-raster", &ctx()).unwrap();
        assert_eq!(pwg::decode_for_tests(&bytes).unwrap().len(), 2);
        assert!(e.encode("image/jpeg", &ctx()).is_err());
        assert!(RasterEncoder::new(Blank(0)).encode(PWG_RASTER, &ctx()).is_err());
        assert!(RasterEncoder::new(Blank(pwg::MAX_PAGES + 1)).encode(PWG_RASTER, &ctx()).is_err());
    }
}
