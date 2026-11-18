//! Streams.

use crate::crypto::DecryptionTarget;
use crate::filter::Filter;
use crate::object;
use crate::object::Dict;
use crate::object::Name;
use crate::object::dict::keys::{DECODE_PARMS, DP, F, FILTER, LENGTH, TYPE};
use crate::object::{Array, ObjectIdentifier};
use crate::object::{Object, ObjectLike, ObjectRefLike};
use crate::reader::Reader;
use crate::reader::{Readable, ReaderContext, ReaderExt, Skippable};
use crate::trivia::{is_eol_character, is_regular_character, is_white_space_character};
use crate::util::{OptionLog, find_needle};
use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt::{Debug, Formatter};
use smallvec::SmallVec;

struct FiltersAndParams<'a> {
    filters: SmallVec<[Filter; 2]>,
    params: SmallVec<[Dict<'a>; 2]>,
}

/// PdfCraft patch: how the decoded data of a stream can be produced (see `Stream::incremental`).
pub(crate) enum Incremental {
    /// The stream has no filter, so its data is its own bytes.
    Plain,
    /// The stream is Flate-compressed without a predictor, so it decodes a piece at a time.
    Flate,
    /// Anything else decodes whole.
    Whole,
}

/// A stream of arbitrary data.
#[derive(Clone)]
pub struct Stream<'a> {
    dict: Dict<'a>,
    data: &'a [u8],
}

impl PartialEq for Stream<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.dict == other.dict && self.data == other.data
    }
}

/// Additional parameters for decoding images.
#[derive(Clone, PartialEq, Default)]
pub struct ImageDecodeParams {
    /// Whether the color space of the image is an indexed color space.
    pub is_indexed: bool,
    /// The bits per component of the image, if that information is available.
    pub bpc: Option<u8>,
    /// The components per channel of the image, if that information is available.
    pub num_components: Option<u8>,
    /// A target resolution for the image. Note that this is only a hint so that
    /// in case it's possible, a version of the image will be extracted that
    /// is as close as possible to the hinted dimension.
    pub target_dimension: Option<(u32, u32)>,
    /// The width of the image as indicated by the image dictionary.
    pub width: u32,
    /// The height of the image as indicated by the image dictionary.
    pub height: u32,
}

impl<'a> Stream<'a> {
    pub(crate) fn new(data: &'a [u8], dict: Dict<'a>) -> Self {
        Self { dict, data }
    }

    fn filters_and_params(&self) -> FiltersAndParams<'a> {
        let mut collected_filters = SmallVec::new();
        let mut collected_params = SmallVec::new();

        if let Some(filter) = self
            .dict
            .get::<Name<'_>>(F)
            .or_else(|| self.dict.get::<Name<'_>>(FILTER))
            .and_then(Filter::from_name)
        {
            let params = self
                .dict
                .get::<Dict<'_>>(DP)
                .or_else(|| self.dict.get::<Dict<'_>>(DECODE_PARMS))
                .unwrap_or_default();

            collected_filters.push(filter);
            collected_params.push(params);
        } else if let Some(filters) = self
            .dict
            .get::<Array<'_>>(F)
            .or_else(|| self.dict.get::<Array<'_>>(FILTER))
        {
            let filters = filters.iter::<Name<'_>>().map(Filter::from_name);
            let mut params = self
                .dict
                .get::<Array<'_>>(DP)
                .or_else(|| self.dict.get::<Array<'_>>(DECODE_PARMS))
                .map(|a| a.iter::<Object<'_>>());

            for filter in filters {
                let params = params
                    .as_mut()
                    .and_then(|p| p.next())
                    .and_then(|p| p.into_dict())
                    .unwrap_or_default();

                if let Some(filter) = filter {
                    collected_filters.push(filter);
                    collected_params.push(params);
                }
            }
        }

        FiltersAndParams {
            filters: collected_filters,
            params: collected_params,
        }
    }

    /// Return the raw, decrypted data of the stream.
    ///
    /// Stream filters will not be applied.
    pub fn raw_data(&self) -> Cow<'a, [u8]> {
        let ctx = self.dict.ctx();

        if ctx.xref().needs_decryption(ctx)
            && self
                .dict
                .get::<object::String<'_>>(TYPE)
                .map(|t| t.as_ref() != b"XRef")
                .unwrap_or(true)
        {
            Cow::Owned(
                ctx.xref()
                    .decrypt(self.obj_id(), self.data, DecryptionTarget::Stream)
                    // TODO: MAybe an error would be better?
                    .unwrap_or_default(),
            )
        } else {
            Cow::Borrowed(self.data)
        }
    }

    /// Return the raw, underlying dictionary of the stream.
    pub fn dict(&self) -> &Dict<'a> {
        &self.dict
    }

    /// The immutable ordinary-stream decode policy of this stream's document.
    pub fn decode_limits(&self) -> crate::DecodeLimits {
        self.dict.ctx().decode_limits()
    }

    /// Return the object identifier of the stream.
    pub fn obj_id(&self) -> ObjectIdentifier {
        self.dict.obj_id().unwrap_or_else(|| {
            // In theory shouldn't ever happen, but could theoretically be
            // triggered in a crafted PDF.
            ObjectIdentifier::new(0, 0)
        })
    }

    /// Return the filters that are applied to the stream.
    pub fn filters(&self) -> SmallVec<[Filter; 2]> {
        self.filters_and_params().filters
    }

    /// PdfCraft patch: how the decoded data of the stream can be produced a piece at a time.
    pub(crate) fn incremental(&self) -> Incremental {
        let filters_and_params = self.filters_and_params();

        match (
            filters_and_params.filters.as_slice(),
            filters_and_params.params.first(),
        ) {
            ([], _) => Incremental::Plain,
            ([Filter::FlateDecode], Some(params)) if crate::filter::flate_is_plain(params) => Incremental::Flate,
            _ => Incremental::Whole,
        }
    }

    /// Return the decoded data of the stream.
    ///
    /// Note that the result of this method will not be cached, so calling it multiple
    /// times is expensive.
    pub fn decoded(&self) -> Result<Cow<'a, [u8]>, DecodeFailure> {
        self.decoded_image(&ImageDecodeParams::default())
            .map(|r| r.data)
    }

    /// Return the decoded data of the stream, and return image metadata
    /// if available.
    pub fn decoded_image(
        &self,
        image_params: &ImageDecodeParams,
    ) -> Result<FilterResult<'a>, DecodeFailure> {
        self.decode_within(image_params, usize::MAX)
    }

    /// PdfCraft patch: the decoded data of the stream, for callers that budget what a page
    /// holds. Flate, LZW and RunLength stop at `max` bytes (and at
    /// this PDF's ordinary stream limit in any case), and a JBIG2 page that would decode past it
    /// is refused. A stream without filters is returned as it is; ASCIIHex and ASCII85 barely
    /// expand, and the other image codecs stop at their own pixel limits.
    pub fn decoded_within(&self, max: usize) -> Result<Cow<'a, [u8]>, DecodeFailure> {
        self.decode_within(&ImageDecodeParams::default(), max)
            .map(|r| r.data)
    }

    fn decode_within(
        &self,
        image_params: &ImageDecodeParams,
        max: usize,
    ) -> Result<FilterResult<'a>, DecodeFailure> {
        crate::cancel::check(self.dict.ctx().control()).ok_or(DecodeFailure::StreamDecode)?;
        self.decode_raw_within(self.raw_data(), image_params, max)
    }

    /// PdfCraft patch: decode caller-supplied raw/decrypted bytes with the same filters and limits.
    pub(crate) fn decoded_raw_within(
        &self,
        data: Cow<'a, [u8]>,
        max: usize,
    ) -> Result<Cow<'a, [u8]>, DecodeFailure> {
        self.decode_raw_within(data, &ImageDecodeParams::default(), max)
            .map(|r| r.data)
    }

    fn decode_raw_within(
        &self,
        data: Cow<'a, [u8]>,
        image_params: &ImageDecodeParams,
        max: usize,
    ) -> Result<FilterResult<'a>, DecodeFailure> {
        let control = self.dict.ctx().control();
        crate::cancel::check(control).ok_or(DecodeFailure::StreamDecode)?;
        let filters_and_params = self.filters_and_params();
        crate::cancel::check(control).ok_or(DecodeFailure::StreamDecode)?;
        let count = filters_and_params.filters.len();

        let mut current: Option<FilterResult<'a>> = None;

        for (i, (filter, params)) in filters_and_params
            .filters
            .iter()
            .zip(filters_and_params.params.iter())
            .enumerate()
        {
            // PdfCraft patch: see `filter::decode_limit`.
            let limit = crate::filter::decode_limit(image_params, i + 1 == count, self.dict.ctx().decode_limits()).min(max);
            let new = filter.apply(current.as_ref().map(|c| c.data.as_ref()).unwrap_or(&data), params, image_params, limit, self.dict.ctx().control())?;
            current = Some(new);
        }

        crate::cancel::check(control).ok_or(DecodeFailure::StreamDecode)?;
        Ok(current.unwrap_or(FilterResult {
            data,
            image_data: None,
        }))
    }
}

impl Debug for Stream<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Stream (len: {:?})", self.data.len())
    }
}

impl Skippable for Stream<'_> {
    fn skip(_: &mut Reader<'_>, _: bool) -> Option<()> {
        // A stream can never appear in a dict/array, so it should never be skipped.
        warn!("attempted to skip a stream object");

        None
    }
}

impl<'a> Readable<'a> for Stream<'a> {
    fn read(r: &mut Reader<'a>, ctx: &ReaderContext<'a>) -> Option<Self> {
        let dict = r.read_with_context::<Dict<'_>>(ctx)?;

        if dict.contains_key(F) {
            warn!("encountered stream referencing external file, which is unsupported");

            return None;
        }

        let offset = r.offset();
        parse_proper(r, &dict)
            .or_else(|| {
                // PdfCraft patch (36): before searching on, which could reach a later object.
                r.jump(offset);
                parse_declared(r, &dict, true)
            })
            .or_else(|| {
                crate::cancel::check(ctx.control())?;
                warn!("failed to parse stream, trying to parse it manually");

                r.jump(offset);
                parse_fallback(r, &dict)
            })
            .or_else(|| {
                r.jump(offset);
                parse_declared(r, &dict, false)
            })
            .error_none("was unable to manually parse the stream")
    }
}

#[derive(Debug, Copy, Clone)]
/// A failure that can occur during decoding a data stream.
pub enum DecodeFailure {
    /// An image stream failed to decode.
    ImageDecode,
    /// A data stream failed to decode.
    StreamDecode,
    /// A failure occurred while decrypting a file.
    Decryption,
    /// An unknown failure occurred.
    Unknown,
}

/// An image color space.
#[derive(Debug, Copy, Clone)]
pub enum ImageColorSpace {
    /// Grayscale color space.
    Gray,
    /// RGB color space.
    Rgb,
    /// CMYK color space.
    Cmyk,
    /// An unknown color space.
    Unknown(u8),
}

/// Additional data that is extracted from some image streams.
pub struct ImageData {
    /// An optional alpha channel of the image.
    pub alpha: Option<Vec<u8>>,
    /// The color space of the image.
    pub color_space: Option<ImageColorSpace>,
    /// The bits per component of the image.
    pub bits_per_component: u8,
    /// The width of the image.
    pub width: u32,
    /// The height of the image.
    pub height: u32,
}

/// The result of applying a filter.
pub struct FilterResult<'a> {
    /// The decoded data.
    pub data: Cow<'a, [u8]>,
    /// Additional data that is extracted from JPX image streams.
    pub image_data: Option<ImageData>,
}

impl FilterResult<'_> {
    pub(crate) fn from_data(data: Vec<u8>) -> Self {
        Self {
            data: Cow::Owned(data),
            image_data: None,
        }
    }
}

/// PdfCraft patch (20): the end-of-line after the `stream` keyword. Microsoft Reporting Services
/// writes `stream \r\n`, a space before the line break, which the specification does not allow.
/// Without this the content stream was taken to start at the space, failed to inflate, and the
/// page rendered blank (bank statements, invoices) while other viewers showed it. Only spaces and
/// tabs are skipped, and only when a line break follows, so stream data is never swallowed.
fn forward_stream_eol(r: &mut Reader<'_>) -> Option<()> {
    r.forward_while(|b| b == b' ' || b == b'\t');
    r.forward_tag(b"\n")
        .or_else(|| r.forward_tag(b"\r\n"))
        // Technically not allowed, but no reason to not try it.
        .or_else(|| r.forward_tag(b"\r"))
}

fn parse_proper<'a>(r: &mut Reader<'a>, dict: &Dict<'a>) -> Option<Stream<'a>> {
    let length = dict.get::<u32>(LENGTH)?;

    r.skip_white_spaces_and_comments();
    r.forward_tag(b"stream")?;
    forward_stream_eol(r)?;
    let data = r.read_bytes(length as usize)?;
    r.skip_white_spaces();
    r.forward_tag(b"endstream")?;
    // PdfCraft patch (36): a whole token (ISO 32000-2 7.2.3), so a wrong `/Length` that stops
    // before payload such as `endstreamer` does not end the stream there.
    if r.tail().and_then(|t| t.first().copied()).is_some_and(is_regular_character) {
        return None;
    }

    Some(Stream::new(data, dict.clone()))
}

fn parse_fallback<'a>(r: &mut Reader<'a>, dict: &Dict<'a>) -> Option<Stream<'a>> {
    // PdfCraft patch: `stream` must be the next token. Scanning ahead to a later
    // `stream` made a dictionary look like a stream — in particular an indirect
    // `/AP /N` appearance-state dictionary (`<< /Yes 5 0 R /Off 6 0 R >>`, as Quartz
    // writes checkboxes). The widget then had no form to draw, so the box disappeared.
    // A wrong `/Length` still recovers, because that keyword sits directly after the dict.
    r.skip_white_spaces_and_comments();
    r.forward_tag(b"stream")?;
    forward_stream_eol(r)?;

    let tail = r.tail()?;
    let control = dict.ctx().control();
    let endstream_offset = find_endstream(tail, control)?;
    let data_end = trim_trailing_ascii_whitespace(&tail[..endstream_offset], control)?;
   let data = tail.get(..data_end)?;

    r.read_bytes(endstream_offset)?;
    r.skip_white_spaces();
    r.forward_tag(b"endstream")?;

    Some(Stream::new(data, dict.clone()))
}

/// PdfCraft patch (36): a stream with no `endstream` after it, as a producer that writes
/// `endstrea` leaves every stream, is the bytes its `/Length` gives, "how many bytes of the PDF
/// file are used for the stream's data" (ISO 32000-2 7.3.8.2), as other readers take it. Neither
/// [`parse_proper`] nor [`parse_fallback`] found it, so every font and page content of such a file
/// was lost and its pages drew blank. Only a `/Length` the data holds is read.
///
/// With `at_an_end`, tried before [`parse_fallback`], the `/Length` must also end where the
/// stream would ([`at_stream_end`]): before a whole `endstream`, before a misspelt one
/// (`endstrea`) or `endobj` where the object ends too, or at the end of the data. Otherwise the search for `endstream` would run on into a later object and take its
/// header and data as this stream's. Without it, tried last, after the search found no
/// `endstream` in the rest of the data, so it doesn't search again.
fn parse_declared<'a>(r: &mut Reader<'a>, dict: &Dict<'a>, at_an_end: bool) -> Option<Stream<'a>> {
    let length = dict.get::<u32>(LENGTH)?;

    r.skip_white_spaces_and_comments();
    r.forward_tag(b"stream")?;
    forward_stream_eol(r)?;
    let data = r.read_bytes(usize::try_from(length).ok()?)?;
    if at_an_end && !at_stream_end(r.tail().unwrap_or_default()) {
        return None;
    }

    Some(Stream::new(data, dict.clone()))
}

fn find_endstream_raw(data: &[u8], control: Option<&crate::cancel::Control>) -> Option<usize> {
    const NEEDLE: &[u8] = b"endstream";
    if control.is_none() {
        return find_needle(data, NEEDLE);
    }
    for start in (0..data.len()).step_by(64 * 1024) {
        crate::cancel::check(control)?;
        // Look ahead past each chunk so a keyword crossing its boundary is preserved.
        let end = start.saturating_add(64 * 1024 + NEEDLE.len() - 1).min(data.len());
        if let Some(offset) = find_needle(data.get(start..end)?, NEEDLE) {
            return Some(start + offset);
        }
    }
    None
}

/// PdfCraft patch (36): the first `endstream` in `data` that is a whole token (ISO 32000-2
/// 7.2.3), not the start of payload bytes such as `endstreamer`.
fn find_endstream(data: &[u8], control: Option<&crate::cancel::Control>) -> Option<usize> {
    let mut from = 0;
    loop {
        let at = from + find_endstream_raw(data.get(from..)?, control)?;
        let end = at + b"endstream".len();
        if !data.get(end).copied().is_some_and(is_regular_character) {
            return Some(at);
        }
        from = end;
    }
}

/// PdfCraft patch (36): whether `rest`, the data after a stream's `/Length` bytes, is where the
/// stream ends. A whole `endstream` token is. A misspelt one (`endstrea`), an `endobj` with the
/// `endstream` left out, or both, are only where the object ends as well, before the end of the
/// data, another object's `N G obj` header, or `xref`, `trailer` or `startxref`: payload such as
/// `endobj more`, where a wrong `/Length` stops, is not, and the search for `endstream` runs on.
fn at_stream_end(rest: &[u8]) -> bool {
    let rest = skip_trivia(rest);
    if keyword(rest, b"endstream").is_some() {
        return true;
    }
    let (rest, misspelt) = keyword(rest, b"endstrea").map_or((rest, false), |r| (skip_trivia(r), true));
    let (rest, endobj) = keyword(rest, b"endobj").map_or((rest, false), |r| (skip_trivia(r), true));
    if !(misspelt || endobj) {
        return rest.is_empty();
    }
    rest.is_empty()
        || [&b"xref"[..], b"trailer", b"startxref"].iter().any(|kw| keyword(rest, kw).is_some())
        || object_header(rest)
}

/// Whether `rest` starts with an indirect object's `N G obj` header.
fn object_header(rest: &[u8]) -> bool {
    fn number(r: &[u8]) -> Option<&[u8]> {
        let digits = r.iter().take_while(|b| b.is_ascii_digit()).count();
        (digits > 0).then(|| skip_trivia(r.get(digits..).unwrap_or_default()))
    }
    number(rest).and_then(number).is_some_and(|r| keyword(r, b"obj").is_some())
}

/// What follows `kw` at the start of `data` when it is a whole token there.
fn keyword<'a>(data: &'a [u8], kw: &[u8]) -> Option<&'a [u8]> {
    data.strip_prefix(kw).filter(|tail| !tail.first().copied().is_some_and(is_regular_character))
}

/// `data` with leading white space and comments skipped.
fn skip_trivia(mut data: &[u8]) -> &[u8] {
    loop {
        match data.first() {
            Some(b) if is_white_space_character(*b) => data = data.get(1..).unwrap_or_default(),
            Some(b'%') => {
                let eol = data.iter().position(|b| is_eol_character(*b)).unwrap_or(data.len());
                data = data.get(eol..).unwrap_or_default();
            }
            _ => return data,
        }
    }
}

fn trim_trailing_ascii_whitespace(data: &[u8], control: Option<&crate::cancel::Control>) -> Option<usize> {
   let mut end = data.len();
    let mut checks = crate::cancel::Checkpoints::new(control);

    while data
        .get(end.wrapping_sub(1))
        .copied()
        .is_some_and(is_white_space_character)
    {
        checks.at(data.len() - end)?;
        end -= 1;
    }

    Some(end)
}

impl<'a> TryFrom<Object<'a>> for Stream<'a> {
    type Error = ();

    fn try_from(value: Object<'a>) -> Result<Self, Self::Error> {
        match value {
            Object::Stream(s) => Ok(s),
            _ => Err(()),
        }
    }
}

impl<'a> ObjectLike<'a> for Stream<'a> {}
impl<'a> ObjectRefLike<'a> for Stream<'a> {
    fn cast_ref<'b>(obj: &'b Object<'a>) -> Option<&'b Self> {
        match obj {
            Object::Stream(stream) => Some(stream),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::object::Stream;
    use crate::reader::Reader;
    use crate::reader::{ReaderContext, ReaderExt};

    #[test]
    fn stream() {
        let data = b"<< /Length 10 >> stream\nabcdefghij\nendstream";
        let mut r = Reader::new(data);
        let stream = r
            .read_with_context::<Stream<'_>>(&ReaderContext::dummy())
            .unwrap();

        assert_eq!(stream.data, b"abcdefghij");
    }

    #[test]
    fn stream_fallback() {
        let data = b"<< /Length 999 >> stream\nabcdefghij\nendstream";
        let mut r = Reader::new(data);
        let stream = r
            .read_with_context::<Stream<'_>>(&ReaderContext::dummy())
            .unwrap();

        assert_eq!(stream.data, b"abcdefghij");
    }

    #[test]
    fn fallback_search_polls_before_reaching_a_distant_endstream() {
        use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let armed = alloc::sync::Arc::new(AtomicBool::new(false));
        let polls = alloc::sync::Arc::new(AtomicUsize::new(0));
        let flag = armed.clone();
        let counter = polls.clone();
        let cancel = crate::Cancellation::new(move || flag.load(Ordering::Relaxed) && counter.fetch_add(1, Ordering::Relaxed) + 1 == 2);
        let control = crate::cancel::Control::new(&cancel);
        let ctx = ReaderContext::dummy_with_control(crate::DecodeLimits::default(), Some(control));
        let dict = Reader::new(b"<< >>").read_with_context::<crate::object::Dict<'_>>(&ctx).unwrap();
        let mut data = b"stream\n".to_vec();
        data.extend(core::iter::repeat_n(b'x', 2 << 20));
        data.extend_from_slice(b"\nendstream");
        armed.store(true, Ordering::Relaxed);
        assert!(super::parse_fallback(&mut Reader::new(&data), &dict).is_none());
        assert_eq!(polls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn fallback_preserves_endstream_across_search_boundaries() {
        for length in (64 * 1024 - 9)..(64 * 1024 + 2) {
            let (control, _) = crate::cancel::stop_at(usize::MAX);
            let ctx = ReaderContext::dummy_with_control(crate::DecodeLimits::default(), Some(control));
            let dict = Reader::new(b"<< >>").read_with_context::<crate::object::Dict<'_>>(&ctx).unwrap();
            let mut data = b"stream\n".to_vec();
            data.extend(core::iter::repeat_n(b'x', length));
            data.extend_from_slice(b"endstream");
            let stream = super::parse_fallback(&mut Reader::new(&data), &dict).unwrap();
            assert_eq!(stream.data, alloc::vec![b'x'; length]);
        }
    }

    #[test]
    fn fallback_whitespace_trimming_is_cancellable_too() {
        use core::sync::atomic::Ordering;
        let (control, polls) = crate::cancel::stop_at(2);
        assert!(super::trim_trailing_ascii_whitespace(&alloc::vec![b' '; 2 << 20], Some(&control)).is_none());
        assert_eq!(polls.load(Ordering::Relaxed), 2);
    }

    /// A dictionary followed by some other object's stream is not itself a stream.
    /// `parse_fallback` used to latch onto that later `stream` keyword.
    #[test]
    fn dictionary_is_not_a_later_objects_stream() {
        let data = b"<< /Yes 5 0 R /Off 6 0 R >>\nendobj\n11 0 obj << /Length 5 >> stream\nhello\nendstream";
        let mut r = Reader::new(data);
        assert!(r.read_with_context::<Stream<'_>>(&ReaderContext::dummy()).is_none());
    }

    /// PdfCraft patch (20): spaces or tabs between `stream` and its line break are skipped.
    #[test]
    fn stream_keyword_may_be_followed_by_spaces_before_the_line_break() {
        for data in [
            &b"<< /Length 10 >> stream \r\nabcdefghij\nendstream"[..],
            &b"<< /Length 10 >> stream\t \nabcdefghij\nendstream"[..],
            &b"<< /Length 999 >> stream \r\nabcdefghij\nendstream"[..],
        ] {
            let mut r = Reader::new(data);
            let stream = r
                .read_with_context::<Stream<'_>>(&ReaderContext::dummy())
                .unwrap();

            assert_eq!(stream.data, b"abcdefghij");
        }
    }

    /// PdfCraft patch (36): the bytes of the stream read from `data`, or `None`.
    fn read_data(data: &[u8]) -> Option<alloc::vec::Vec<u8>> {
        Reader::new(data).read_with_context::<Stream<'_>>(&ReaderContext::dummy()).map(|s| s.data.to_vec())
    }

    /// PdfCraft patch (36, ISO 32000-2 7.3.8.2): a stream whose `endstream` is misspelt (`endstrea`)
    /// or left out before `endobj` is its `/Length`, not the bytes up to a later object's `endstream`.
    #[test]
    fn a_stream_without_endstream_is_its_length_before_a_later_one() {
        for end in ["endstrea\nendobj", "endobj"] {
            let data = format!("<< /Length 5 >> stream\nHELLO\n{end}\n5 0 obj << /Length 5 >> stream\nWORLD\nendstream\nendobj\n");
            assert_eq!(read_data(data.as_bytes()).as_deref(), Some(&b"HELLO"[..]), "{end:?}");
        }
        // No `/Length` the data holds, and nothing to search for: not a stream.
        assert_eq!(read_data(b"<< /Length 99 >> stream\nHELLO\nendstrea"), None);
        assert_eq!(read_data(b"<< >> stream\nHELLO\nendstrea"), None);
    }

    /// PdfCraft patch (36): a wrong `/Length` that stops before payload that begins like an end marker
    /// does not end the stream there (a token runs on through regular characters, ISO 32000-2 7.2.3);
    /// an `endobj` or `endstrea` the object does not end after (`endobj more`) does not either, while one
    /// followed by another object's header, `trailer` or the end of the data does.
    #[test]
    fn a_stream_does_not_end_inside_a_longer_word() {
        for word in ["endobject", "endstreak", "endstreamer", "endobj more", "endstrea more", "endstrea endobj more"] {
            let data = format!("<< /Length 5 >> stream\nHELLO {word} more\nendstream\nendobj\n");
            assert_eq!(read_data(data.as_bytes()), Some(format!("HELLO {word} more").into_bytes()), "{word}");
        }
        for end in ["endobj\n", "endstrea\n", "endstrea\nendobj\n%c\n", "endstrea\n6 0 obj << >> endobj\n", "endobj\ntrailer\n"] {
            let data = format!("<< /Length 5 >> stream\nHELLO\n{end}");
            assert_eq!(read_data(data.as_bytes()).as_deref(), Some(&b"HELLO"[..]), "{end:?}");
        }
    }
}
