//! A small PDF writer for print output: pages of a given size in points, each with raster images
//! placed on it, tagged with their ICC profile (`ICCBased`) so the numbers mean the same colour in
//! every reader and RIP, Flate-compressed (lossless) at 8 or 16 bits per component, or carried as
//! JPEG (`DCTDecode`) where small files matter more than exact samples. Optionally an output intent:
//! the printing condition (the printer / paper profile) the file was made for, as PDF/X readers
//! expect it in the catalog. Profiles shared by several images are written once.
//!
//! The file is PDF 1.6 (16-bit images need 1.5): classic cross-reference table, no object streams,
//! deterministic (no dates or IDs), so the same pages always give the same bytes.
//!
//! Sources: ISO 32000-1:2008 (§7.3 objects, §7.5 file structure, §8.6.5.5 ICCBased colour spaces,
//! §8.9 images, §14.11.5 output intents). Own design.

use std::collections::HashMap;
use std::sync::Arc;

/// Pages per document: a print job beyond this is a mistake, not a book.
pub const MAX_PAGES: usize = 1000;
/// Images per page.
pub const MAX_IMAGES_PER_PAGE: usize = 256;
/// Largest page side in points (200 inches): more than any printer's roll.
pub const MAX_PAGE_POINTS: f64 = 14_400.0;
/// Largest image, in pixels per side.
pub const MAX_IMAGE_SIDE: u32 = 65_535;

/// How an image's samples are stored.
#[derive(Clone, Debug, PartialEq)]
pub enum ImageData {
    /// Interleaved samples, `bits` (8 or 16) per component, 16-bit values big-endian: compressed
    /// losslessly in the file.
    Raw { bits: u8, samples: Vec<u8> },
    /// A baseline JPEG stream, copied into the file as it is.
    Jpeg(Vec<u8>),
    /// Samples already compressed with zlib (see [`PdfImage::deflated`]), `bits` per component.
    Flate { bits: u8, data: Vec<u8> },
}

/// A raster image and the colour space its samples are in.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfImage {
    pub width: u32,
    pub height: u32,
    /// Components per pixel: 1 (grey), 3 (RGB) or 4 (CMYK).
    pub channels: u8,
    pub data: ImageData,
    /// The ICC profile the samples are in; without one the image is device colour.
    pub icc: Option<Arc<Vec<u8>>>,
}

impl PdfImage {
    /// An image from interleaved 16-bit samples.
    pub fn from_u16(width: u32, height: u32, channels: u8, samples: &[u16], icc: Option<Arc<Vec<u8>>>) -> PdfImage {
        let bytes = samples.iter().flat_map(|v| v.to_be_bytes()).collect();
        PdfImage { width, height, channels, data: ImageData::Raw { bits: 16, samples: bytes }, icc }
    }

    /// The same image with its samples compressed now (checked first), so a long print doesn't
    /// hold every page's uncompressed samples until the file is written.
    pub fn deflated(self) -> Result<PdfImage, String> {
        self.check()?;
        let data = match self.data {
            ImageData::Raw { bits, samples } => ImageData::Flate { bits, data: deflate(&samples) },
            other => other,
        };
        Ok(PdfImage { data, ..self })
    }

    fn check(&self) -> Result<(), String> {
        if self.width == 0 || self.height == 0 || self.width > MAX_IMAGE_SIDE || self.height > MAX_IMAGE_SIDE {
            return Err(format!("an image of {} × {} pixels can't be written", self.width, self.height));
        }
        if !matches!(self.channels, 1 | 3 | 4) {
            return Err(format!("images have 1, 3 or 4 components, not {}", self.channels));
        }
        match &self.data {
            ImageData::Raw { bits, samples } => {
                if !matches!(bits, 8 | 16) {
                    return Err(format!("images have 8 or 16 bits per component, not {bits}"));
                }
                let want = u64::from(self.width) * u64::from(self.height) * u64::from(self.channels) * u64::from(bits / 8);
                if samples.len() as u64 != want {
                    return Err(format!("the image needs {want} bytes of samples, not {}", samples.len()));
                }
            }
            ImageData::Jpeg(data) => {
                if !data.starts_with(&[0xFF, 0xD8]) {
                    return Err("the image isn't a JPEG stream".into());
                }
            }
            ImageData::Flate { bits, data } => {
                if !matches!(bits, 8 | 16) || data.is_empty() {
                    return Err("a compressed image has 8 or 16 bits per component and some data".into());
                }
            }
        }
        Ok(())
    }
}

/// Where an image goes on its page, in points from the page's bottom-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One page: its size in points and the images on it, drawn in order (later ones on top).
#[derive(Clone, Debug, PartialEq)]
pub struct PdfPage {
    pub width: f64,
    pub height: f64,
    pub images: Vec<(PdfImage, Placement)>,
}

/// The printing condition the document is made for (PDF/X-style output intent).
#[derive(Clone, Debug, PartialEq)]
pub struct OutputIntent {
    /// The printer / paper profile.
    pub icc: Arc<Vec<u8>>,
    /// Its components: 1, 3 or 4.
    pub channels: u8,
    /// A name for the condition (the profile's description).
    pub condition: String,
}

/// Document-level settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfInfo {
    pub title: Option<String>,
    pub output_intent: Option<OutputIntent>,
}

fn finite(v: f64) -> bool {
    v.is_finite()
}

/// A number as PDF writes it: up to four decimals, no exponent, no trailing zeros.
fn num(v: f64) -> String {
    let s = format!("{:.4}", if finite(v) { v } else { 0.0 });
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.into() }
}

/// A text string: plain ASCII as a literal string, anything else as UTF-16BE with a byte order mark.
fn text(s: &str) -> String {
    if s.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('(');
        for c in s.chars() {
            if matches!(c, '(' | ')' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push(')');
        out
    } else {
        let mut out = String::from("<FEFF");
        for unit in s.encode_utf16() {
            out.push_str(&format!("{unit:04X}"));
        }
        out.push('>');
        out
    }
}

struct Writer {
    out: Vec<u8>,
    offsets: Vec<usize>,
}

impl Writer {
    /// Reserve an object number (filled later by [`Writer::object`]).
    fn reserve(&mut self) -> usize {
        self.offsets.push(0);
        self.offsets.len()
    }

    fn object(&mut self, id: usize, dict: &str, stream: Option<&[u8]>) {
        if let Some(slot) = self.offsets.get_mut(id.wrapping_sub(1)) {
            *slot = self.out.len();
        }
        self.out.extend_from_slice(format!("{id} 0 obj\n{dict}").as_bytes());
        if let Some(data) = stream {
            self.out.extend_from_slice(b"\nstream\n");
            self.out.extend_from_slice(data);
            self.out.extend_from_slice(b"\nendstream");
        }
        self.out.extend_from_slice(b"\nendobj\n");
    }
}

fn deflate(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, 6)
}

/// Write a document. Errors name the first page or image that can't be written.
pub fn write(pages: &[PdfPage], info: &PdfInfo) -> Result<Vec<u8>, String> {
    if pages.is_empty() {
        return Err("there are no pages to write".into());
    }
    if pages.len() > MAX_PAGES {
        return Err(format!("a document holds at most {MAX_PAGES} pages"));
    }
    for (n, page) in pages.iter().enumerate() {
        let n = n + 1;
        if !(finite(page.width) && finite(page.height))
            || page.width <= 0.0
            || page.height <= 0.0
            || page.width > MAX_PAGE_POINTS
            || page.height > MAX_PAGE_POINTS
        {
            return Err(format!("page {n} has no usable size"));
        }
        if page.images.len() > MAX_IMAGES_PER_PAGE {
            return Err(format!("page {n} has more than {MAX_IMAGES_PER_PAGE} images"));
        }
        for (image, at) in &page.images {
            image.check().map_err(|e| format!("page {n}: {e}"))?;
            if ![at.x, at.y, at.width, at.height].into_iter().all(finite) || at.width <= 0.0 || at.height <= 0.0 {
                return Err(format!("page {n}: an image has no usable placement"));
            }
        }
    }
    if let Some(intent) = &info.output_intent
        && !matches!(intent.channels, 1 | 3 | 4)
    {
        return Err("the output intent's profile has 1, 3 or 4 components".into());
    }

    let mut w = Writer { out: Vec::new(), offsets: Vec::new() };
    w.out.extend_from_slice(b"%PDF-1.6\n%\xE2\xE3\xCF\xD3\n");
    let catalog = w.reserve();
    let pages_id = w.reserve();
    let info_id = w.reserve();

    // ICC profiles, each written once (keyed by content) with its component count
    let mut profiles: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut profile = |w: &mut Writer, icc: &Arc<Vec<u8>>, n: u8| -> usize {
        if let Some(&id) = profiles.get(icc.as_slice()) {
            return id;
        }
        let id = w.reserve();
        let data = deflate(icc);
        let alternate = match n {
            1 => "/DeviceGray",
            4 => "/DeviceCMYK",
            _ => "/DeviceRGB",
        };
        w.object(id, &format!("<< /N {n} /Alternate {alternate} /Filter /FlateDecode /Length {} >>", data.len()), Some(&data));
        profiles.insert(icc.to_vec(), id);
        id
    };

    let intent_id = info.output_intent.as_ref().map(|intent| profile(&mut w, &intent.icc, intent.channels));

    let mut kids = Vec::with_capacity(pages.len());
    for page in pages {
        let page_id = w.reserve();
        let mut xobjects = String::new();
        let mut content = String::new();
        for (k, (image, at)) in page.images.iter().enumerate() {
            let space = match &image.icc {
                Some(icc) => format!("[/ICCBased {} 0 R]", profile(&mut w, icc, image.channels)),
                None => match image.channels {
                    1 => "/DeviceGray".into(),
                    4 => "/DeviceCMYK".into(),
                    _ => "/DeviceRGB".into(),
                },
            };
            let id = w.reserve();
            let (bits, filter, data): (u8, &str, std::borrow::Cow<[u8]>) = match &image.data {
                ImageData::Raw { bits, samples } => (*bits, "/FlateDecode", deflate(samples).into()),
                ImageData::Flate { bits, data } => (*bits, "/FlateDecode", data.as_slice().into()),
                ImageData::Jpeg(jpeg) => (8, "/DCTDecode", jpeg.as_slice().into()),
            };
            w.object(
                id,
                &format!(
                    "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {space} /BitsPerComponent {bits} /Filter {filter} /Length {} >>",
                    image.width,
                    image.height,
                    data.len()
                ),
                Some(&data),
            );
            xobjects.push_str(&format!("/Im{k} {id} 0 R "));
            content.push_str(&format!("q {} 0 0 {} {} {} cm /Im{k} Do Q\n", num(at.width), num(at.height), num(at.x), num(at.y)));
        }
        let content_id = w.reserve();
        let stream = deflate(content.as_bytes());
        w.object(content_id, &format!("<< /Filter /FlateDecode /Length {} >>", stream.len()), Some(&stream));
        w.object(
            page_id,
            &format!(
                "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {} {}] /Resources << /XObject << {xobjects}>> >> /Contents {content_id} 0 R >>",
                num(page.width),
                num(page.height)
            ),
            None,
        );
        kids.push(format!("{page_id} 0 R"));
    }
    w.object(pages_id, &format!("<< /Type /Pages /Count {} /Kids [{}] >>", kids.len(), kids.join(" ")), None);

    let intents = match (&info.output_intent, intent_id) {
        (Some(intent), Some(id)) => format!(
            " /OutputIntents [<< /Type /OutputIntent /S /GTS_PDFX /OutputConditionIdentifier (Custom) /Info {} /DestOutputProfile {id} 0 R >>]",
            text(&intent.condition)
        ),
        _ => String::new(),
    };
    w.object(catalog, &format!("<< /Type /Catalog /Pages {pages_id} 0 R{intents} >>"), None);
    let title = info.title.as_deref().map(|t| format!(" /Title {}", text(t))).unwrap_or_default();
    w.object(info_id, &format!("<< /Producer (LightCraft){title} >>"), None);

    let xref = w.out.len();
    let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", w.offsets.len() + 1);
    for offset in &w.offsets {
        table.push_str(&format!("{offset:010} 00000 n \n"));
    }
    table.push_str(&format!("trailer\n<< /Size {} /Root {catalog} 0 R /Info {info_id} 0 R >>\nstartxref\n{xref}\n%%EOF\n", w.offsets.len() + 1));
    w.out.extend_from_slice(table.as_bytes());
    Ok(w.out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_codecs::NamedSpace;

    /// The body of object `id` (dictionary and raw stream bytes), found through the xref table.
    fn object(pdf: &[u8], id: usize) -> (String, Vec<u8>) {
        // byte offsets: the streams are binary, so search the bytes, not a lossy string
        let start = pdf.windows(10).rposition(|w| w == b"startxref\n").unwrap() + 10;
        let tail = std::str::from_utf8(&pdf[start..]).unwrap();
        let xref: usize = tail.lines().next().unwrap().trim().parse().unwrap();
        let table = std::str::from_utf8(&pdf[xref..]).unwrap();
        let line = table.lines().nth(2 + id).unwrap();
        let offset: usize = line[..10].parse().unwrap();
        let head = format!("{id} 0 obj\n");
        assert_eq!(&pdf[offset..offset + head.len()], head.as_bytes(), "xref entry for {id}");
        let rest = &pdf[offset + head.len()..];
        let end = rest.windows(7).position(|w| w == b"\nendobj").unwrap();
        let body = &rest[..end];
        match body.windows(8).position(|w| w == b"\nstream\n") {
            Some(p) => {
                let data = &body[p + 8..body.len() - "\nendstream".len()];
                (String::from_utf8_lossy(&body[..p]).into_owned(), data.to_vec())
            }
            None => (String::from_utf8_lossy(body).into_owned(), vec![]),
        }
    }

    fn inflate(data: &[u8]) -> Vec<u8> {
        miniz_oxide::inflate::decompress_to_vec_zlib(data).unwrap()
    }

    /// Object ids of every image XObject, in file order.
    fn images(pdf: &[u8]) -> Vec<usize> {
        let s = String::from_utf8_lossy(pdf);
        s.match_indices(" 0 obj\n<< /Type /XObject /Subtype /Image").map(|(i, _)| s[..i].rsplit('\n').next().unwrap().parse().unwrap()).collect()
    }

    fn reference(dict: &str, key: &str) -> usize {
        let at = dict.find(key).unwrap() + key.len();
        dict[at..].trim_start().split(' ').next().unwrap().parse().unwrap()
    }

    fn srgb() -> Arc<Vec<u8>> {
        Arc::new(lightcraft_codecs::icc::write_named(NamedSpace::Srgb))
    }

    fn page(images: Vec<(PdfImage, Placement)>) -> PdfPage {
        PdfPage { width: 612.0, height: 792.0, images }
    }

    fn full(w: f64, h: f64) -> Placement {
        Placement { x: 0.0, y: 0.0, width: w, height: h }
    }

    #[test]
    fn a_16_bit_icc_image_round_trips() {
        let samples: Vec<u16> = (0..2 * 3 * 3).map(|i| (i * 3851) as u16).collect();
        let img = PdfImage::from_u16(3, 2, 3, &samples, Some(srgb()));
        let pdf = write(&[page(vec![(img, full(612.0, 792.0))])], &PdfInfo::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.6\n"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let ids = images(&pdf);
        assert_eq!(ids.len(), 1);
        let (dict, data) = object(&pdf, ids[0]);
        assert!(dict.contains("/Width 3 /Height 2") && dict.contains("/BitsPerComponent 16") && dict.contains("/FlateDecode"), "{dict}");
        let back: Vec<u16> = inflate(&data).chunks(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect();
        assert_eq!(back, samples);
        let (profile, icc) = object(&pdf, reference(&dict, "/ICCBased"));
        assert!(profile.contains("/N 3") && profile.contains("/Alternate /DeviceRGB"), "{profile}");
        assert_eq!(inflate(&icc), *srgb());
        assert_eq!(lightcraft_codecs::icc::parse(&inflate(&icc)).and_then(|i| i.named), Some(NamedSpace::Srgb));
    }

    #[test]
    fn pages_sizes_placements_and_a_shared_profile() {
        let px = |v: u16| PdfImage::from_u16(1, 1, 3, &[v, v, v], Some(srgb()));
        let at = Placement { x: 36.0, y: 72.5, width: 288.0, height: 216.0 };
        let pages = [page(vec![(px(1), at), (px(2), full(612.0, 792.0))]), PdfPage { width: 841.89, height: 595.28, images: vec![(px(3), at)] }];
        let pdf = write(&pages, &PdfInfo::default()).unwrap();
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains("/Type /Pages /Count 2"), "two pages");
        assert!(s.contains("/MediaBox [0 0 612 792]") && s.contains("/MediaBox [0 0 841.89 595.28]"));
        // the sRGB profile is written once for all three images
        assert_eq!(s.matches("/N 3 /Alternate").count(), 1);
        let first: Vec<_> = images(&pdf).iter().map(|&id| reference(&object(&pdf, id).0, "/ICCBased")).collect();
        assert!(first.windows(2).all(|w| w[0] == w[1]), "{first:?}");
        // placements reach the content streams
        let contents: Vec<String> = s
            .match_indices("/Contents ")
            .map(|(i, _)| s[i + 10..].split(' ').next().unwrap().parse::<usize>().unwrap())
            .map(|id| String::from_utf8(inflate(&object(&pdf, id).1)).unwrap())
            .collect();
        assert_eq!(contents[0], "q 288 0 0 216 36 72.5 cm /Im0 Do Q\nq 612 0 0 792 0 0 cm /Im1 Do Q\n");
        assert_eq!(write(&pages, &PdfInfo::default()).unwrap(), pdf, "deterministic");
    }

    #[test]
    fn output_intent_and_title() {
        let cmyk_like = srgb();
        let info = PdfInfo {
            title: Some("Garden – prints (2)".into()),
            output_intent: Some(OutputIntent { icc: cmyk_like.clone(), channels: 3, condition: "Glossy (Photo) \\ test".into() }),
        };
        let img = PdfImage::from_u16(1, 1, 3, &[0, 0, 0], Some(cmyk_like));
        let pdf = write(&[page(vec![(img, full(10.0, 10.0))])], &info).unwrap();
        let (catalog, _) = object(&pdf, 1);
        assert!(catalog.contains("/OutputIntents [<< /Type /OutputIntent /S /GTS_PDFX"), "{catalog}");
        assert!(catalog.contains("/Info (Glossy \\(Photo\\) \\\\ test)"), "{catalog}");
        // the intent and the image share one profile object
        let dest = reference(&catalog, "/DestOutputProfile");
        assert_eq!(reference(&object(&pdf, images(&pdf)[0]).0, "/ICCBased"), dest);
        let (info_dict, _) = object(&pdf, 3);
        assert!(info_dict.contains("/Title <FEFF"), "non-ASCII titles are UTF-16: {info_dict}");
        assert!(info_dict.contains("2013"), "the en dash: {info_dict}");
    }

    #[test]
    fn cmyk_grey_jpeg_and_device_images() {
        let jpeg = vec![0xFF, 0xD8, 0xFF, 0xD9];
        let imgs = vec![
            (
                PdfImage { width: 1, height: 1, channels: 4, data: ImageData::Raw { bits: 8, samples: vec![0, 10, 20, 30] }, icc: None },
                full(1.0, 1.0),
            ),
            (PdfImage { width: 2, height: 1, channels: 1, data: ImageData::Raw { bits: 8, samples: vec![0, 255] }, icc: None }, full(1.0, 1.0)),
            (PdfImage { width: 4, height: 4, channels: 3, data: ImageData::Jpeg(jpeg.clone()), icc: None }, full(1.0, 1.0)),
        ];
        let pdf = write(&[page(imgs)], &PdfInfo::default()).unwrap();
        let ids = images(&pdf);
        let dicts: Vec<_> = ids.iter().map(|&id| object(&pdf, id)).collect();
        assert!(dicts[0].0.contains("/ColorSpace /DeviceCMYK") && dicts[0].0.contains("/BitsPerComponent 8"));
        assert!(dicts[1].0.contains("/ColorSpace /DeviceGray"));
        assert!(dicts[2].0.contains("/DCTDecode") && dicts[2].0.contains("/DeviceRGB"));
        assert_eq!(dicts[2].1, jpeg, "JPEG streams are copied as they are");
    }

    #[test]
    fn compressing_early_writes_the_same_file() {
        let samples: Vec<u16> = (0..4 * 3 * 3).map(|i| (i * 977) as u16).collect();
        let raw = PdfImage::from_u16(4, 3, 3, &samples, Some(srgb()));
        let early = raw.clone().deflated().unwrap();
        assert!(matches!(early.data, ImageData::Flate { bits: 16, .. }));
        let at = full(100.0, 75.0);
        let a = write(&[page(vec![(raw, at)])], &PdfInfo::default()).unwrap();
        let b = write(&[page(vec![(early, at)])], &PdfInfo::default()).unwrap();
        assert_eq!(a, b);
        let mut short = PdfImage::from_u16(4, 3, 3, &samples, None);
        short.width = 5;
        assert!(short.deflated().is_err(), "checked before compressing");
    }

    #[test]
    fn hostile_documents_are_errors() {
        let ok = || PdfImage::from_u16(1, 1, 3, &[1, 2, 3], None);
        let at = full(10.0, 10.0);
        let bad = |pages: Vec<PdfPage>| write(&pages, &PdfInfo::default()).is_err();
        assert!(bad(vec![]));
        assert!(bad(vec![PdfPage { width: f64::NAN, height: 10.0, images: vec![] }]));
        assert!(bad(vec![PdfPage { width: -1.0, height: 10.0, images: vec![] }]));
        assert!(bad(vec![PdfPage { width: 1e9, height: 10.0, images: vec![] }]));
        assert!(bad(vec![page(vec![(ok(), Placement { x: f64::INFINITY, ..at })])]));
        assert!(bad(vec![page(vec![(ok(), Placement { width: 0.0, ..at })])]));
        let mut short = ok();
        short.data = ImageData::Raw { bits: 16, samples: vec![0; 5] };
        assert!(bad(vec![page(vec![(short, at)])]));
        let mut empty = ok();
        empty.width = 0;
        assert!(bad(vec![page(vec![(empty, at)])]));
        let mut odd = ok();
        odd.channels = 2;
        assert!(bad(vec![page(vec![(odd, at)])]));
        let mut not_jpeg = ok();
        not_jpeg.data = ImageData::Jpeg(vec![1, 2, 3]);
        assert!(bad(vec![page(vec![(not_jpeg, at)])]));
        let too_many = (0..MAX_PAGES + 1).map(|_| page(vec![])).collect();
        assert!(bad(too_many));
        let info = PdfInfo { title: None, output_intent: Some(OutputIntent { icc: srgb(), channels: 2, condition: String::new() }) };
        assert!(write(&[page(vec![])], &info).is_err());
    }

    #[test]
    fn numbers_and_strings() {
        assert_eq!(num(612.0), "612");
        assert_eq!(num(72.5), "72.5");
        assert_eq!(num(-0.00001), "0");
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(num(1.0 / 3.0), "0.3333");
        assert_eq!(text("a(b)c\\"), "(a\\(b\\)c\\\\)");
        assert_eq!(text("é"), "<FEFF00E9>");
    }
}
