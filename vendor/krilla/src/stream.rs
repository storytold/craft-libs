//! Drawing to a non-page context.
//!
//! In 90% of the cases, you will not have to use streams. In most cases,
//! all you need to do when using this crate is to first construct a document,
//! and then add new pages to the document and use the [`Page::surface`] method to get
//! access to the drawing context. However, there are cases when you don't want to
//! draw on the main page surface, but instead you want to create a "sub-surface"
//! where you can draw independently of the main page contents. This is what streams
//! are there for. Currently, there are only two situations where you need to do that:
//!
//! - When using masks and defining the contents of the mask.
//! - When using a pattern fill or stroke and defining the contents of the pattern.
//!
//! If you want to do any of the two above, you need to call the [`Surface::stream_builder`] method
//! of the current surface. The stream builder represents a kind of sub-context that is
//! independent of the main surface you are working with. Once you have a stream builder, you
//! can once again invoke the [`StreamBuilder::surface`] method, and use this new surface to define the contents
//! of your mask/pattern. In the end, you can call [`StreamBuilder::finish`] which will return a [`Stream`] object.
//! This [`Stream`] object contains the encoded instructions of the mask/pattern, which you can
//! then use to create new [`Pattern`]/[`Mask`] objects.
//!
//! [`Page::surface`]: crate::page::Page::surface
//! [`Surface::stream_builder`]: crate::surface::Surface::stream_builder
//! [`Pattern`]: crate::graphics::paint::Pattern
//! [`Mask`]: crate::graphics::mask::Mask

use std::borrow::Cow;
use std::ops::DerefMut;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use pdf_writer::{Array, Dict, Name};
#[cfg(feature = "raster-images")]
use pdf_writer::{Chunk, Ref, Settings};

use crate::chunk_container::ChunkContainer;
use crate::configure::ValidationError;
use crate::content::ContentBuilder;
use crate::geom::{Rect, Transform};
use crate::resource::{ResourceDictionary, ResourceDictionaryBuilder};
use crate::serialize::SerializeContext;
use crate::surface::Surface;
use crate::SerializeSettings;

/// A stream.
///
/// See the module description for an explanation of its purpose.
// The only reason we implement clone for this type is that in some cases,
// we might need to clone a pattern (including its stream)
#[derive(Debug, Hash, Eq, PartialEq, Clone)]
pub struct Stream {
    pub(crate) content: Vec<u8>,
    pub(crate) bbox: Rect,
    // Important: Each object that uses a stream must ensure to pass on the validation
    // errors to the `SerializeContext` at some point. Currently, only `Mask`,
    // `TilingPattern`, `InternalPage` and `XObject` require that.
    pub(crate) validation_errors: Vec<ValidationError>,
    pub(crate) resource_dictionary: ResourceDictionary,
    pub(crate) uses_mask: bool,
}

impl Stream {
    pub(crate) fn new(
        content: Vec<u8>,
        bbox: Rect,
        validation_errors: Vec<ValidationError>,
        resource_dictionary: ResourceDictionary,
        uses_mask: bool,
    ) -> Self {
        Self {
            content,
            bbox,
            validation_errors,
            resource_dictionary,
            uses_mask,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    pub(crate) fn empty() -> Self {
        Self {
            content: vec![],
            bbox: Rect::from_xywh(0.0, 0.0, 0.0, 0.0).unwrap(),
            validation_errors: vec![],
            resource_dictionary: ResourceDictionaryBuilder::new().finish(),
            uses_mask: false,
        }
    }
}

/// A builder to create streams.
pub struct StreamBuilder<'a> {
    sc: &'a mut SerializeContext,
    chunk_container: &'a mut ChunkContainer,
    stream: Stream,
}

impl<'a> StreamBuilder<'a> {
    pub(crate) fn new(
        sc: &'a mut SerializeContext,
        chunk_container: &'a mut ChunkContainer,
    ) -> Self {
        Self {
            sc,
            chunk_container,
            stream: Stream::empty(),
        }
    }

    /// Get the surface of the stream builder.
    pub fn surface(&mut self) -> Surface<'_> {
        // Stream builders cannot have any tags since we always pass a dummy
        // identifier. Only main page content streams can have one.
        let finish_fn = Box::new(|stream, _| {
            self.stream = stream;
        });

        Surface::new(
            self.sc,
            self.chunk_container,
            ContentBuilder::new(Transform::identity(), true, self.sc),
            None,
            finish_fn,
        )
    }

    /// Turn the stream builder into a stream.
    pub fn finish(self) -> Stream {
        self.stream
    }
}

/// A PDF stream filter.
#[derive(Debug, Copy, Clone)]
pub(crate) enum StreamFilter {
    Flate,
    FlateMemoized,
    AsciiHex,
    #[cfg(feature = "raster-images")]
    Dct,
}

impl StreamFilter {
    pub(crate) fn to_name(self) -> Name<'static> {
        match self {
            Self::AsciiHex => Name(b"ASCIIHexDecode"),
            Self::Flate => Name(b"FlateDecode"),
            Self::FlateMemoized => Name(b"FlateDecode"),
            #[cfg(feature = "raster-images")]
            Self::Dct => Name(b"DCTDecode"),
        }
    }

    pub(crate) fn is_binary(&self) -> bool {
        match self {
            StreamFilter::Flate => true,
            StreamFilter::FlateMemoized => true,
            StreamFilter::AsciiHex => false,
            #[cfg(feature = "raster-images")]
            StreamFilter::Dct => true,
        }
    }
}

impl StreamFilter {
    #[track_caller]
    pub(crate) fn apply(&self, content: &[u8]) -> Vec<u8> {
        match self {
            StreamFilter::Flate => deflate_encode(content),
            StreamFilter::FlateMemoized => deflate_encode_memoized(content),
            StreamFilter::AsciiHex => hex_encode(content),
            // Note: We don't actually encode manually with DCT, because
            // this is only used for JPEG images which are already encoded,
            // so this shouldn't be called at all.
            #[cfg(feature = "raster-images")]
            StreamFilter::Dct => panic!("can't apply dct decode"),
        }
    }
}

/// Allows us to keep track of the filters that a stream has and
/// apply them in an orderly fashion.
#[derive(Debug, Clone)]
pub(crate) enum StreamFilters {
    None,
    Single(StreamFilter),
    Multiple(Vec<StreamFilter>),
}

impl StreamFilters {
    pub(crate) fn add(&mut self, stream_filter: StreamFilter) {
        match self {
            StreamFilters::None => *self = StreamFilters::Single(stream_filter),
            StreamFilters::Single(cur) => {
                *self = StreamFilters::Multiple(vec![*cur, stream_filter])
            }
            StreamFilters::Multiple(cur) => cur.push(stream_filter),
        }
    }

    pub(crate) fn is_binary(&self) -> bool {
        match self {
            StreamFilters::None => false,
            StreamFilters::Single(s) => s.is_binary(),
            StreamFilters::Multiple(m) => m.last().unwrap().is_binary(),
        }
    }
}

/// Parameters for byte-wise PNG Up prediction of one sampled image plane.
#[derive(Debug, Copy, Clone)]
pub(crate) struct ImagePredictor {
    columns: i32,
    colors: i32,
    bits_per_component: i32,
}

impl ImagePredictor {
    #[cfg(feature = "raster-images")]
    pub(crate) fn new(columns: u32, colors: usize, bits_per_component: u8) -> Option<Self> {
        let columns = i32::try_from(columns).ok().filter(|&n| n > 0)?;
        let colors = i32::try_from(colors).ok().filter(|&n| n > 0)?;
        Some(Self {
            columns,
            colors,
            bits_per_component: i32::from(bits_per_component),
        })
    }

    fn write(self, mut dict: Dict<'_>) {
        dict.pair(Name(b"Predictor"), 12);
        dict.pair(Name(b"Columns"), self.columns);
        dict.pair(Name(b"Colors"), self.colors);
        dict.pair(Name(b"BitsPerComponent"), self.bits_per_component);
    }
}

pub(crate) struct FilterStreamBuilder<'a> {
    content: Cow<'a, [u8]>,
    filters: StreamFilters,
    is_binary: bool,
    predictor: Option<ImagePredictor>,
}

impl<'a> FilterStreamBuilder<'a> {
    fn empty(content: &'a [u8]) -> Self {
        Self {
            content: Cow::Borrowed(content),
            filters: StreamFilters::None,
            is_binary: false,
            predictor: None,
        }
    }

    fn binary(content: &'a [u8]) -> Self {
        Self {
            is_binary: true,
            ..Self::empty(content)
        }
    }

    pub(crate) fn new_from_content_stream(
        content: &'a [u8],
        serialize_settings: &SerializeSettings,
    ) -> Self {
        let mut filter_stream = Self::empty(content);

        if serialize_settings.compress_content_streams {
            filter_stream.add_filter(StreamFilter::FlateMemoized);
        }

        filter_stream
    }

    pub(crate) fn new_from_deflated(content: &'a [u8]) -> Self {
        let mut filter_stream = Self::empty(content);
        filter_stream.add_unapplied_filter(StreamFilter::Flate);

        filter_stream
    }

    #[cfg(feature = "raster-images")]
    pub(crate) fn new_from_deflated_image(
        content: &'a [u8],
        predictor: Option<ImagePredictor>,
    ) -> Self {
        let mut filter_stream = Self::new_from_deflated(content);
        filter_stream.predictor = predictor;
        filter_stream
    }

    /// Count the actual filter/parameter dictionary, payload and Length digits without copying
    /// the encoded payload. All other image keys are identical between the two encodings.
    #[cfg(feature = "raster-images")]
    pub(crate) fn image_stream_size(
        len: usize,
        predictor: Option<ImagePredictor>,
        ascii: bool,
        settings: Settings,
    ) -> Option<usize> {
        let encoded_len = if ascii {
            len.checked_mul(2)?.checked_add(len / 35)?
        } else {
            len
        };
        let filters = if ascii {
            StreamFilters::Multiple(vec![StreamFilter::Flate, StreamFilter::AsciiHex])
        } else {
            StreamFilters::Single(StreamFilter::Flate)
        };
        let filter_stream = FilterStream {
            content: Cow::Borrowed(&[]),
            filters,
            predictor,
        };
        // Use the caller's exact pdf-writer settings, not the default pretty-printing mode.
        let mut chunk = Chunk::with_settings(settings);
        {
            let mut stream = chunk.stream(Ref::new(1), &[]);
            filter_stream.write_filters(stream.deref_mut());
        }
        // The empty stream already includes the one digit in /Length 0 and all delimiters.
        chunk
            .len()
            .checked_add(encoded_len)?
            .checked_add(encoded_len.to_string().len() - 1)
    }

    pub(crate) fn new_from_binary_data(content: &'a [u8]) -> Self {
        let mut filter_stream = Self::binary(content);
        filter_stream.add_filter(StreamFilter::Flate);

        filter_stream
    }

    pub(crate) fn new_auto_compressed(content: &'a [u8]) -> Self {
        let filter_stream = Self::new_from_binary_data(content);

        const MAX_COMPRESSED_SIZE: usize = 75;
        if content.is_empty()
            || 100 * filter_stream.content.len() / content.len() > MAX_COMPRESSED_SIZE
        {
            return Self::binary(content);
        }

        filter_stream
    }

    pub(crate) fn new_from_uncompressed(content: &'a [u8]) -> Self {
        Self::binary(content)
    }

    #[cfg(feature = "raster-images")]
    pub(crate) fn new_from_jpeg_data(content: &'a [u8]) -> Self {
        let mut filter_stream = Self::empty(content);
        // JPEG data already is DCT encoded.
        filter_stream.add_unapplied_filter(StreamFilter::Dct);

        filter_stream
    }

    pub(crate) fn finish(mut self, serialize_settings: &SerializeSettings) -> FilterStream<'a> {
        if serialize_settings.ascii_compatible
            && !self.content.is_empty()
            && (self.filters.is_binary()
                || (self.is_binary && self.content.iter().any(|byte| !byte.is_ascii())))
        {
            self.add_filter(StreamFilter::AsciiHex);
        }

        FilterStream {
            content: self.content,
            filters: self.filters,
            predictor: self.predictor,
        }
    }

    fn add_filter(&mut self, filter: StreamFilter) {
        self.content = Cow::Owned(filter.apply(&self.content));
        self.filters.add(filter);
    }

    fn add_unapplied_filter(&mut self, filter: StreamFilter) {
        self.filters.add(filter);
    }
}

pub(crate) struct FilterStream<'a> {
    content: Cow<'a, [u8]>,
    filters: StreamFilters,
    predictor: Option<ImagePredictor>,
}

impl FilterStream<'_> {
    pub(crate) fn encoded_data(&self) -> &[u8] {
        &self.content
    }

    pub(crate) fn write_filters<'b, T>(&self, mut dict: T)
    where
        T: DerefMut<Target = Dict<'b>>,
    {
        match &self.filters {
            StreamFilters::None => {}
            StreamFilters::Single(filter) => {
                dict.deref_mut().pair(Name(b"Filter"), filter.to_name());
            }
            StreamFilters::Multiple(filters) => {
                dict.deref_mut()
                    .insert(Name(b"Filter"))
                    .start::<Array>()
                    .items(filters.iter().map(|f| f.to_name()).rev());
            }
        }
        if let Some(predictor) = self.predictor {
            match &self.filters {
                StreamFilters::Single(StreamFilter::Flate) => {
                    predictor.write(dict.deref_mut().insert(Name(b"DecodeParms")).dict());
                }
                StreamFilters::Multiple(filters) => {
                    let mut parms = dict.deref_mut().insert(Name(b"DecodeParms")).array();
                    for filter in filters.iter().rev() {
                        match filter {
                            StreamFilter::Flate => predictor.write(parms.push().dict()),
                            _ => {
                                parms.item(pdf_writer::Null);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg_attr(feature = "comemo", comemo::memoize)]
fn deflate_encode_memoized(data: &[u8]) -> Vec<u8> {
    deflate_encode(data)
}

pub(crate) fn deflate_encode(data: &[u8]) -> Vec<u8> {
    use std::io::Write;

    const COMPRESSION_LEVEL: u8 = 6;
    let mut e = ZlibEncoder::new(Vec::new(), Compression::new(COMPRESSION_LEVEL as u32));
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn hex_encode(data: &[u8]) -> Vec<u8> {
    use std::fmt::Write;

    const COLUMNS: usize = 35;
    let len = data.len();
    let capacity = 2 * len + len / COLUMNS;
    let mut output = String::with_capacity(capacity);
    for (index, byte) in data.iter().enumerate() {
        write!(output, "{byte:02X}").unwrap();
        if index % COLUMNS == COLUMNS - 1 {
            output.push('\n');
        }
    }
    output.into_bytes()
}
