//! [`TrackDecoder`]: one video track's packets in, [`VideoFrame`]s out, whatever the codec. The
//! counterpart of `craft_audio::TrackDecoder`, for an app with its own demuxer.

#[cfg(feature = "av1-decode")]
use crate::av1;
use crate::error::VideoError;
use crate::frame::{FrameError, VideoFrame};
#[cfg(feature = "h264-decode")]
use crate::h264;
#[cfg(feature = "hevc-decode")]
use crate::hevc;
#[cfg(feature = "mpeg2-decode")]
use crate::mpeg2;
#[cfg(feature = "vp9-decode")]
use crate::vp9;
use craft_media::{CodecId, CodecParams};

/// The codec-specific decoder behind a [`TrackDecoder`].
enum Inner {
    /// The decoder, and the `avcC` record or Annex B parameter sets that [`TrackDecoder::reset`]
    /// applies again (the decoder's own reset forgets them).
    #[cfg(feature = "h264-decode")]
    H264 { dec: Box<h264::decode::Decoder>, config: Vec<u8> },
    /// The decoder, and the `hvcC` record or Annex B parameter sets a reset rebuilds it from.
    #[cfg(feature = "hevc-decode")]
    Hevc { dec: Box<hevc::decode::Decoder>, config: Vec<u8> },
    #[cfg(feature = "vp9-decode")]
    Vp9(Box<vp9::decode::Decoder>),
    /// The decoder, and the `av1C` config OBUs a reset rebuilds it from.
    #[cfg(feature = "av1-decode")]
    Av1 { dec: Box<av1::decode::Decoder>, config_obus: Vec<u8> },
    #[cfg(feature = "mpeg2-decode")]
    Mpeg2(Box<mpeg2::Decoder>),
    #[cfg(feature = "prores")]
    ProRes,
    #[cfg(feature = "dnx")]
    Dnx,
    #[cfg(feature = "apv-decode")]
    Apv,
    #[cfg(feature = "ffv1")]
    Ffv1(Box<crate::ffv1::Decoder>),
    #[cfg(feature = "vp8-decode")]
    Vp8(Box<crate::vp8::Decoder>),
    #[cfg(feature = "mjpeg")]
    Mjpeg(crate::mjpeg::Decoder),
}

/// A decoder for one video track, whatever its codec: built from the track's [`CodecParams`],
/// it takes the track's packets in decode order and returns [`VideoFrame`]s in display order.
///
/// | Codec | Feature | `extradata` |
/// |---|---|---|
/// | H.264 | `h264-decode` | an `avcC` record, Annex B parameter sets, or empty |
/// | HEVC | `hevc-decode` | an `hvcC` record, Annex B parameter sets, or empty |
/// | VP9 | `vp9-decode` | ignored (the frame headers repeat the `vpcC` values) |
/// | AV1 | `av1-decode` | an `av1C` record, or empty |
/// | MPEG-2 and MPEG-1 video | `mpeg2-decode` | a sequence header and its extensions, or empty |
/// | ProRes | `prores` | ignored (frame headers carry the format) |
/// | DNxHD / DNxHR | `dnx` | ignored (frame headers carry the format) |
/// | APV | `apv-decode` | ignored (access units carry the format) |
/// | FFV1 | `ffv1` | required version 3 configuration record; container width and height are required too |
/// | Motion JPEG | `mjpeg` | ignored (each packet carries its own picture) |
/// | VP8 | `vp8-decode` | ignored (key frames carry the format) |
///
/// With an `avcC` or `hvcC` record, packets are length-prefixed NAL units, as MP4 and Matroska
/// store them. Otherwise they are Annex B, as MPEG-TS and AVI store them, and the parameter sets
/// may come in the packets themselves. The configuration must not hold picture data: an `avcC`
/// or `hvcC` record holds parameter sets (and SEI for `hvcC`), Annex B `extradata` holds
/// parameter sets, SEI and access unit delimiters, `av1C` config OBUs are sequence headers and
/// metadata, and MPEG-2 `extradata` holds sequence headers, their extensions, user data and GOP
/// headers.
///
/// ProRes, DNx, APV, FFV1 and Motion JPEG return one frame per packet. APV selects the first decodable
/// primary frame and its alpha, as the codec's default decoder does. FFV1 supports version 3,
/// range-coded key frames. VP8 hidden frames update references without returning a picture.
/// These decoders have no delayed output; reset keeps FFV1 configuration and dimensions.
///
/// Other codecs are [`VideoError::Unsupported`], as is a codec whose decoder feature is off.
///
/// A packet carries its presentation timestamp, in any unit; each frame returns with the timestamp
/// of the packet that coded it, through reordering ([`VideoFrame::pts`]).
///
/// ```no_run
/// # fn run(params: &craft_media::CodecParams, packets: &[(Vec<u8>, i64)]) -> Result<(), craft_video::VideoError> {
/// use craft_video::TrackDecoder;
///
/// let mut dec = TrackDecoder::new(params)?;
/// for (packet, pts) in packets {
///     for frame in dec.decode(packet, *pts)? {
///         println!("{}x{} at {:?}", frame.width(), frame.height(), frame.pts);
///     }
/// }
/// for frame in dec.flush()? {
///     let _ = frame;
/// }
/// # Ok(()) }
/// ```
/// Options for [`TrackDecoder::with_options`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackDecoderOptions {
    /// Worker threads for codecs that decode in parallel (`None` uses the codec default;
    /// `Some(1)` decodes on the calling thread). Values of `0` are clamped to `1`.
    pub threads: Option<usize>,
    /// Draft mode (approximate, for reduced-resolution playback only; never for export).
    pub draft: bool,
}

pub struct TrackDecoder {
    inner: Inner,
    codec: CodecId,
    threads: Option<usize>,
    draft: bool,
    /// Frames finished by a call that then failed, which the next call returns first.
    held: Vec<VideoFrame>,
}

impl std::fmt::Debug for TrackDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackDecoder")
            .field("codec", &self.codec)
            .field("threads", &self.threads)
            .field("draft", &self.draft)
            .field("held", &self.held.len())
            .finish_non_exhaustive()
    }
}

impl TrackDecoder {
    /// A decoder for the track `params` describes. `codec` and `extradata` select and configure it;
    /// FFV1 also requires nonzero `width` and `height` from the container. Other codecs get the
    /// picture size and format from the stream.
    ///
    /// Invalid configuration, or configuration containing picture data where only headers are
    /// allowed, is [`VideoError::Malformed`]. Unsupported codec features are
    /// [`VideoError::Unsupported`].
    pub fn new(params: &CodecParams) -> Result<TrackDecoder, VideoError> {
        Self::with_options(params, TrackDecoderOptions::default())
    }

    /// Like [`TrackDecoder::new`], capped at `threads` worker threads (`1` decodes on the calling
    /// thread; `0` is clamped to `1`) for codecs that decode in parallel.
    pub fn with_threads(params: &CodecParams, threads: usize) -> Result<TrackDecoder, VideoError> {
        Self::with_options(params, TrackDecoderOptions { threads: Some(threads), draft: false })
    }

    /// Like [`TrackDecoder::new`], with explicit threading and draft options.
    pub fn with_options(params: &CodecParams, options: TrackDecoderOptions) -> Result<TrackDecoder, VideoError> {
        let threads = options.threads.map(|t| t.max(1));
        let normalized = TrackDecoderOptions { threads, draft: options.draft };
        Ok(TrackDecoder { inner: Inner::new(params, normalized)?, codec: params.codec, threads, draft: options.draft, held: Vec::new() })
    }

    /// The track's codec, as `params` gave it.
    pub fn codec(&self) -> CodecId {
        self.codec
    }

    /// Whether this codec's validated track configuration declares an alpha plane.
    ///
    /// FFV1 v3 can answer this before the first packet, from its [extra plane
    /// flag](https://www.rfc-editor.org/rfc/rfc9043.html#section-4.2.10). Other codecs currently
    /// return `None`. Container alpha and alpha signaled in pictures can require separate inspection.
    pub fn configured_alpha(&self) -> Option<bool> {
        #[cfg(feature = "ffv1")]
        match &self.inner {
            Inner::Ffv1(decoder) => Some(decoder.has_alpha()),
            // FFV1 is the only variant when no other dispatcher codec features are enabled.
            #[allow(unreachable_patterns)]
            _ => None,
        }
        #[cfg(not(feature = "ffv1"))]
        None
    }

    /// The explicit worker-thread cap, or `None` when using the codec's default.
    pub fn threads(&self) -> Option<usize> {
        self.threads
    }

    /// Whether draft mode is enabled (for reduced-resolution playback only; never for export).
    pub fn draft(&self) -> bool {
        self.draft
    }

    /// Toggle draft mode (skipping non-reference loop/deblocking filters on codecs that support
    /// it). Approximate, for reduced-resolution playback only; never use for export.
    pub fn set_draft(&mut self, on: bool) {
        self.draft = on;
        match &mut self.inner {
            #[cfg(feature = "h264-decode")]
            Inner::H264 { dec, .. } => dec.set_draft(on),
            #[cfg(feature = "hevc-decode")]
            Inner::Hevc { dec, .. } => dec.set_draft(on),
            #[cfg(feature = "vp9-decode")]
            Inner::Vp9(dec) => dec.set_draft(on),
            #[cfg(feature = "av1-decode")]
            Inner::Av1 { dec, .. } => dec.set_draft(on),
            #[cfg(feature = "mpeg2-decode")]
            Inner::Mpeg2(_) => {}
            #[cfg(feature = "prores")]
            Inner::ProRes => {}
            #[cfg(feature = "dnx")]
            Inner::Dnx => {}
            #[cfg(feature = "apv-decode")]
            Inner::Apv => {}
            #[cfg(feature = "ffv1")]
            Inner::Ffv1(_) => {}
            #[cfg(feature = "vp8-decode")]
            Inner::Vp8(_) => {}
            #[cfg(feature = "mjpeg")]
            Inner::Mjpeg(_) => {}
        }
    }

    /// Decode one packet (an access unit, a VP9 frame or superframe, an AV1 temporal unit) with
    /// its presentation timestamp. Returns the frames that became ready for display, in display
    /// order; a decoder that reorders or decodes on worker threads returns a packet's frame from a
    /// later call or from [`TrackDecoder::flush`].
    ///
    /// When a call fails because a picture does not convert, the frames finished with it are
    /// kept: the next call returns them first. When the codec itself fails, the pictures it had
    /// finished are up to the codec. H.264 keeps them for later calls. HEVC does too, but only
    /// its newest 32 while calls keep failing, which bounds its memory. VP9 keeps those of earlier
    /// packets. MPEG-2 returns them with the call, unless the packet made too many pictures. AV1
    /// drops those the failing call had finished.
    ///
    /// Decoding can go on with the next packet; a decoder that lost its references waits for the
    /// next random access point or conceals, as the codec does.
    pub fn decode(&mut self, packet: &[u8], pts: i64) -> Result<Vec<VideoFrame>, VideoError> {
        match &mut self.inner {
            #[cfg(feature = "h264-decode")]
            Inner::H264 { dec, .. } => {
                let pics = dec.decode(packet, pts)?;
                self.output(pics, None)
            }
            #[cfg(feature = "hevc-decode")]
            Inner::Hevc { dec, .. } => {
                let pics = dec.decode(packet, pts)?;
                self.output(pics, None)
            }
            #[cfg(feature = "vp9-decode")]
            Inner::Vp9(dec) => {
                let pics = dec.decode(packet, pts)?;
                self.output(pics, None)
            }
            #[cfg(feature = "av1-decode")]
            Inner::Av1 { dec, .. } => {
                let pics = dec.decode_pts(packet, pts)?;
                self.output(pics, None)
            }
            #[cfg(feature = "mpeg2-decode")]
            Inner::Mpeg2(dec) => {
                let pics = dec.decode(packet, pts)?;
                self.output(pics, None)
            }
            #[cfg(feature = "prores")]
            Inner::ProRes => self.single(crate::prores::decode_frame(packet)?, pts),
            #[cfg(feature = "dnx")]
            Inner::Dnx => self.single(crate::dnx::decode_frame(packet)?, pts),
            #[cfg(feature = "apv-decode")]
            Inner::Apv => self.single(crate::apv::decode_frame(packet)?, pts),
            #[cfg(feature = "ffv1")]
            Inner::Ffv1(dec) => {
                let pic = dec.decode(packet)?;
                self.single(pic, pts)
            }
            #[cfg(feature = "mjpeg")]
            Inner::Mjpeg(dec) => {
                let pic = dec.decode(packet)?;
                self.single(pic, pts)
            }
            #[cfg(feature = "vp8-decode")]
            Inner::Vp8(dec) => {
                let pic = dec.decode(packet, pts)?;
                self.output(pic.into_iter().collect(), None)
            }
        }
    }

    /// End of stream: every frame the decoder still holds, in display order.
    ///
    /// A decoding error found while finishing them is returned instead; the H.264 and HEVC frames
    /// are then kept for the next call, while the AV1 decoder drops its own.
    pub fn flush(&mut self) -> Result<Vec<VideoFrame>, VideoError> {
        match &mut self.inner {
            #[cfg(feature = "h264-decode")]
            Inner::H264 { dec, .. } => {
                let pics = dec.flush();
                let err = dec.take_error().map(VideoError::from);
                self.output(pics, err)
            }
            #[cfg(feature = "hevc-decode")]
            Inner::Hevc { dec, .. } => {
                let pics = dec.flush();
                let err = dec.take_error().map(VideoError::from);
                self.output(pics, err)
            }
            #[cfg(feature = "vp9-decode")]
            Inner::Vp9(dec) => {
                let pics = dec.flush();
                self.output(pics, None)
            }
            #[cfg(feature = "av1-decode")]
            Inner::Av1 { dec, .. } => {
                let pics = dec.flush_result()?;
                self.output(pics, None)
            }
            #[cfg(feature = "mpeg2-decode")]
            Inner::Mpeg2(dec) => {
                let pics = dec.flush();
                self.output(pics, None)
            }
            #[cfg(feature = "prores")]
            Inner::ProRes => Ok(std::mem::take(&mut self.held)),
            #[cfg(feature = "dnx")]
            Inner::Dnx => Ok(std::mem::take(&mut self.held)),
            #[cfg(feature = "apv-decode")]
            Inner::Apv => Ok(std::mem::take(&mut self.held)),
            #[cfg(feature = "ffv1")]
            Inner::Ffv1(_) => Ok(std::mem::take(&mut self.held)),
            #[cfg(feature = "vp8-decode")]
            Inner::Vp8(_) => Ok(std::mem::take(&mut self.held)),
            #[cfg(feature = "mjpeg")]
            Inner::Mjpeg(_) => Ok(std::mem::take(&mut self.held)),
        }
    }

    /// Forget the pictures and references, and the frames kept from a failed call, before decoding
    /// from another random access point (after a seek). The track's configuration is kept: H.264
    /// and HEVC parameter sets from `extradata`, the AV1 sequence header from `av1C`, and the
    /// MPEG-2 sequence; parameter sets that came in packets are forgotten (MPEG-2 keeps its
    /// latest sequence header).
    ///
    /// An error is the configuration failing to apply again, which it did not when the decoder was
    /// built.
    pub fn reset(&mut self) -> Result<(), VideoError> {
        self.held.clear();
        match &mut self.inner {
            #[cfg(feature = "h264-decode")]
            Inner::H264 { dec, config } => {
                dec.reset();
                configure_h264(dec, config)
            }
            #[cfg(feature = "hevc-decode")]
            Inner::Hevc { dec, config } => {
                // The HEVC decoder has no reset of its own: a new one starts from nothing.
                let mut fresh = Box::new(self.threads.map_or_else(hevc::decode::Decoder::new, hevc::decode::Decoder::with_threads));
                fresh.set_draft(self.draft);
                configure_hevc(&mut fresh, config)?;
                *dec = fresh;
                Ok(())
            }
            #[cfg(feature = "vp9-decode")]
            Inner::Vp9(dec) => {
                dec.reset();
                Ok(())
            }
            #[cfg(feature = "av1-decode")]
            Inner::Av1 { dec, config_obus } => {
                // The AV1 decoder has no reset of its own: a new one starts from nothing.
                let mut fresh = Box::new(self.threads.map_or_else(av1::decode::Decoder::new, av1::decode::Decoder::with_threads));
                fresh.set_draft(self.draft);
                configure_av1(&mut fresh, config_obus)?;
                *dec = fresh;
                Ok(())
            }
            #[cfg(feature = "mpeg2-decode")]
            Inner::Mpeg2(dec) => {
                dec.reset();
                Ok(())
            }
            #[cfg(feature = "prores")]
            Inner::ProRes => Ok(()),
            #[cfg(feature = "dnx")]
            Inner::Dnx => Ok(()),
            #[cfg(feature = "apv-decode")]
            Inner::Apv => Ok(()),
            #[cfg(feature = "ffv1")]
            Inner::Ffv1(_) => Ok(()),
            #[cfg(feature = "mjpeg")]
            Inner::Mjpeg(_) => Ok(()),
            #[cfg(feature = "vp8-decode")]
            Inner::Vp8(dec) => {
                dec.reset();
                Ok(())
            }
        }
    }

    /// Frame-only decoders do not take timestamps; attach the packet timestamp after conversion.
    #[cfg(any(feature = "prores", feature = "dnx", feature = "apv-decode", feature = "ffv1", feature = "mjpeg"))]
    fn single<P>(&mut self, pic: P, pts: i64) -> Result<Vec<VideoFrame>, VideoError>
    where
        VideoFrame: TryFrom<P, Error = FrameError>,
    {
        let mut frame = VideoFrame::try_from(pic)?;
        frame.pts = Some(pts);
        let mut frames = std::mem::take(&mut self.held);
        frames.push(frame);
        Ok(frames)
    }

    /// The frames kept from a failed call, then `pics` converted. On `err`, or on a picture that
    /// does not convert, the frames are kept for the next call and the (first) error returned.
    #[cfg(any(
        feature = "h264-decode",
        feature = "hevc-decode",
        feature = "vp9-decode",
        feature = "av1-decode",
        feature = "mpeg2-decode",
        feature = "vp8-decode"
    ))]
    fn output<P>(&mut self, pics: Vec<P>, err: Option<VideoError>) -> Result<Vec<VideoFrame>, VideoError>
    where
        VideoFrame: TryFrom<P, Error = FrameError>,
    {
        let mut frames = std::mem::take(&mut self.held);
        frames.reserve(pics.len());
        let mut err = err;
        for pic in pics {
            match VideoFrame::try_from(pic) {
                Ok(frame) => frames.push(frame),
                Err(e) => {
                    err.get_or_insert(e.into());
                }
            }
        }
        match err {
            None => Ok(frames),
            Some(e) => {
                self.held = frames;
                Err(e)
            }
        }
    }
}

impl Inner {
    fn new(params: &CodecParams, options: TrackDecoderOptions) -> Result<Inner, VideoError> {
        let codec = params.codec;
        let extradata = &params.extradata;
        let _ = (extradata, options);
        match codec {
            #[cfg(feature = "h264-decode")]
            CodecId::H264 => {
                let mut dec = Box::new(options.threads.map_or_else(h264::decode::Decoder::new, h264::decode::Decoder::with_threads));
                dec.set_draft(options.draft);
                configure_h264(&mut dec, extradata)?;
                Ok(Inner::H264 { dec, config: extradata.to_vec() })
            }
            #[cfg(feature = "hevc-decode")]
            CodecId::Hevc => {
                let mut dec = Box::new(options.threads.map_or_else(hevc::decode::Decoder::new, hevc::decode::Decoder::with_threads));
                dec.set_draft(options.draft);
                configure_hevc(&mut dec, extradata)?;
                Ok(Inner::Hevc { dec, config: extradata.to_vec() })
            }
            #[cfg(feature = "vp9-decode")]
            CodecId::Vp9 => {
                let mut dec = Box::new(options.threads.map_or_else(vp9::decode::Decoder::new, vp9::decode::Decoder::with_threads));
                dec.set_draft(options.draft);
                Ok(Inner::Vp9(dec))
            }
            #[cfg(feature = "av1-decode")]
            CodecId::Av1 => {
                let config_obus = av1c_config_obus(extradata)?.to_vec();
                let mut dec = Box::new(options.threads.map_or_else(av1::decode::Decoder::new, av1::decode::Decoder::with_threads));
                dec.set_draft(options.draft);
                configure_av1(&mut dec, &config_obus)?;
                Ok(Inner::Av1 { dec, config_obus })
            }
            #[cfg(feature = "mpeg2-decode")]
            CodecId::Mpeg2Video => {
                let mut dec = Box::new(mpeg2::Decoder::new());
                if let Some(threads) = options.threads {
                    dec.set_threads(threads > 1);
                }
                if !extradata.is_empty() {
                    check_mpeg2_config(extradata)?;
                    dec.decode(extradata, 0)?;
                }
                Ok(Inner::Mpeg2(dec))
            }
            #[cfg(feature = "prores")]
            CodecId::ProRes => Ok(Inner::ProRes),
            #[cfg(feature = "dnx")]
            CodecId::Dnxhd => Ok(Inner::Dnx),
            #[cfg(feature = "apv-decode")]
            CodecId::Apv => Ok(Inner::Apv),
            #[cfg(feature = "vp8-decode")]
            CodecId::Vp8 => Ok(Inner::Vp8(Box::default())),
            #[cfg(feature = "mjpeg")]
            CodecId::Mjpeg => Ok(Inner::Mjpeg(crate::mjpeg::Decoder::new())),
            #[cfg(feature = "ffv1")]
            CodecId::Ffv1 => {
                let (width, height) = params
                    .width
                    .zip(params.height)
                    .filter(|&(w, h)| w > 0 && h > 0)
                    .ok_or_else(|| VideoError::Malformed("FFV1: container dimensions are required".into()))?;
                Ok(Inner::Ffv1(Box::new(crate::ffv1::Decoder::new(extradata, width, height)?)))
            }
            other => Err(VideoError::Unsupported(format!("no {} decoder in this build", other.name()))),
        }
    }
}

/// Whether `extradata` is Annex B (starts with a start code) rather than an `avcC` / `hvcC` record.
#[cfg(any(feature = "h264-decode", feature = "hevc-decode"))]
fn is_annex_b(extradata: &[u8]) -> bool {
    extradata.starts_with(&[0, 0, 1]) || extradata.starts_with(&[0, 0, 0, 1])
}

/// Annex B `extradata` holds only NAL units whose type `allowed` accepts (`codec` names it in the
/// error).
#[cfg(any(feature = "h264-decode", feature = "hevc-decode"))]
fn check_annex_b(codec: &str, extradata: &[u8], nal_type: fn(u8) -> u8, allowed: &[u8]) -> Result<(), VideoError> {
    for nal in craft_media::bitstream::annexb_nals(extradata) {
        let t = nal.first().map_or(u8::MAX, |&b| nal_type(b));
        if !allowed.contains(&t) {
            return Err(VideoError::Malformed(format!("{codec}: configuration holds NAL unit type {t}, not a parameter set")));
        }
    }
    Ok(())
}

/// H.264 NAL unit types Annex B `extradata` may hold: SEI, SPS, PPS, access unit delimiter, SPS
/// extension and subset SPS.
#[cfg(feature = "h264-decode")]
const H264_CONFIG_NALS: [u8; 6] = [6, 7, 8, 9, 13, 15];

/// HEVC NAL unit types Annex B `extradata` may hold: VPS, SPS, PPS, access unit delimiter, prefix
/// and suffix SEI.
#[cfg(feature = "hevc-decode")]
const HEVC_CONFIG_NALS: [u8; 6] = [32, 33, 34, 35, 39, 40];

/// Apply H.264 `extradata`: an `avcC` record, Annex B parameter sets, or nothing.
#[cfg(feature = "h264-decode")]
fn configure_h264(dec: &mut h264::decode::Decoder, extradata: &[u8]) -> Result<(), VideoError> {
    if extradata.is_empty() {
        Ok(())
    } else if is_annex_b(extradata) {
        check_annex_b("H.264", extradata, |b| b & 0x1f, &H264_CONFIG_NALS)?;
        dec.decode(extradata, 0)?;
        Ok(())
    } else {
        Ok(dec.configure_avcc(extradata)?)
    }
}

/// Apply HEVC `extradata`: an `hvcC` record, Annex B parameter sets, or nothing.
#[cfg(feature = "hevc-decode")]
fn configure_hevc(dec: &mut hevc::decode::Decoder, extradata: &[u8]) -> Result<(), VideoError> {
    if extradata.is_empty() {
        Ok(())
    } else if is_annex_b(extradata) {
        check_annex_b("HEVC", extradata, |b| (b >> 1) & 0x3f, &HEVC_CONFIG_NALS)?;
        dec.decode(extradata, 0)?;
        Ok(())
    } else {
        Ok(dec.configure_hvcc(extradata)?)
    }
}

/// MPEG-2 start codes `extradata` may hold: user data, sequence header, extension and GOP header.
#[cfg(feature = "mpeg2-decode")]
const MPEG2_CONFIG_CODES: [u8; 4] = [0xb2, 0xb3, 0xb5, 0xb8];

/// MPEG-2 `extradata` holds no picture: every start code in it is one of [`MPEG2_CONFIG_CODES`].
#[cfg(feature = "mpeg2-decode")]
fn check_mpeg2_config(extradata: &[u8]) -> Result<(), VideoError> {
    for w in extradata.windows(4) {
        if let [0, 0, 1, code] = *w
            && !MPEG2_CONFIG_CODES.contains(&code)
        {
            return Err(VideoError::Malformed(format!("MPEG-2: configuration holds start code {code:#04x}, not a sequence header")));
        }
    }
    Ok(())
}

/// The config OBUs of an `av1C` record (empty for empty `extradata`), once they are checked to
/// hold only sequence headers and metadata.
#[cfg(feature = "av1-decode")]
fn av1c_config_obus(extradata: &[u8]) -> Result<&[u8], VideoError> {
    const SEQUENCE_HEADER: u8 = 1;
    const METADATA: u8 = 5;
    if extradata.is_empty() {
        return Ok(&[]);
    }
    // marker (1) and version (1), then three bytes of profile, level and chroma fields
    let obus = match extradata {
        [0x81, _, _, _, obus @ ..] => obus,
        _ => return Err(VideoError::Malformed("AV1: configuration is not an av1C record (version 1)".into())),
    };
    let mut rest = obus;
    while let Some(&header) = rest.first() {
        let obu_type = (header >> 3) & 0xf;
        if obu_type != SEQUENCE_HEADER && obu_type != METADATA {
            return Err(VideoError::Malformed(format!("AV1: av1C holds OBU type {obu_type}, not a sequence header or metadata")));
        }
        // obu_extension_flag adds a byte; without obu_has_size_field the OBU runs to the end
        let start = 1 + usize::from(header & 4 != 0);
        let body = rest.get(start..).ok_or_else(|| VideoError::Malformed("AV1: av1C OBU header truncated".into()))?;
        rest = if header & 2 != 0 {
            let (size, n) = av1::decode::bits::leb128(body)?;
            let end = usize::try_from(size).ok().and_then(|s| s.checked_add(n));
            end.and_then(|end| body.get(end..)).ok_or_else(|| VideoError::Malformed("AV1: av1C OBU truncated".into()))?
        } else {
            &[]
        };
    }
    Ok(obus)
}

/// Apply `av1C` config OBUs (sequence headers and metadata, checked by [`av1c_config_obus`]).
#[cfg(feature = "av1-decode")]
fn configure_av1(dec: &mut av1::decode::Decoder, config_obus: &[u8]) -> Result<(), VideoError> {
    if !config_obus.is_empty() {
        dec.decode_pts(config_obus, 0)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
