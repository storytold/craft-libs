//! Indexed, range-backed YUV4MPEG2 playback over the shared parser.
//!
//! Adapted to the shared source and frame types from FilmCraft's in-house `Y4mSource`;
//! see the repository's attribution for source history.

use std::sync::Arc;

use craft_audio::AudioBuffer;
use craft_color::{ColorInfo, Range};
use craft_frame::{Chroma, PixelData, VideoFrame, default_matrix};
use craft_time::{FrameRate, Tick};
use craft_y4m::{Error, Layout, Limits, Stream, samples_u16};

use crate::{DEFAULT_MAX_MEDIA_BYTES, FrameRequest, MediaError, MediaInfo, MediaKind, MediaSource, Result, SharedBytes, VideoStreamInfo, read_range};

pub use craft_y4m::sniff;

/// Y4M video whose frame payloads are read on demand from an indexed byte source.
pub struct Y4mSource {
    source: SharedBytes,
    stream: Stream,
    info: MediaInfo,
    rate: FrameRate,
}

impl Y4mSource {
    /// Index complete pictures without loading the video. A picture larger than the shared
    /// byte-reader allocation ceiling is reported as unsupported before this source is returned.
    pub fn open(name: &str, source: SharedBytes) -> Result<Self> {
        let len = source.len();
        let mut read_error = None;
        let stream = Stream::open(
            len,
            |offset, len| match read_range(&*source, offset, len) {
                Ok(bytes) if bytes.len() == len => Some(bytes),
                Ok(_) => {
                    // The parser already clips requests to the length snapshotted at open.
                    // A shorter read here is an I/O failure, not a known incomplete tail.
                    read_error = Some(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "short read while indexing Y4M"));
                    None
                }
                Err(err) => {
                    read_error = Some(err);
                    None
                }
            },
            &Limits::default(),
        );
        if let Some(err) = read_error {
            return Err(MediaError::Io(format!("{name}: Y4M: {err}")));
        }
        let stream = stream.map_err(|err| match err {
            Error::UnsupportedColourSpace(c) => MediaError::Unsupported(format!("{name}: Y4M colour space {c}")),
            Error::LimitExceeded(limit) => MediaError::Unsupported(format!("{name}: Y4M exceeds its {limit} limit")),
            Error::Malformed(reason) => MediaError::Decode(format!("{name}: Y4M: {reason}")),
        })?;
        if stream.frame_bytes > DEFAULT_MAX_MEDIA_BYTES {
            return Err(MediaError::Unsupported(format!("{name}: Y4M frame exceeds the media buffer limit")));
        }

        let header = &stream.header;
        let (width, height) = (header.width, header.height);
        let rate = header.frame_rate.map_or(FrameRate::FPS_25, |(n, d)| FrameRate::new(n, d));
        let color = ColorInfo {
            matrix: default_matrix(width, height),
            range: if header.full_range { Range::Full } else { Range::Limited },
            ..ColorInfo::REC709
        };
        let duration = picture_time(stream.frames.len(), rate);
        let bitrate = (stream.frame_bytes as u64 * 8).checked_mul(rate.num as u64).map(|bits| bits / rate.den as u64);
        let info = MediaInfo {
            name: name.into(),
            kind: MediaKind::Movie,
            duration,
            video: Some(VideoStreamInfo {
                width,
                height,
                frame_rate: rate,
                par: header.pixel_aspect.unwrap_or((1, 1)),
                codec: "Uncompressed YUV".into(),
                pixel_format: layout_label(header.layout),
                color,
                has_alpha: header.layout.alpha,
                bitrate,
                hdr: None,
            }),
            audio_streams: Vec::new(),
            container: "Y4M".into(),
            start_timecode: None,
            file_size: Some(len),
        };
        Ok(Self { source, stream, info, rate })
    }

    fn picture(&self, index: usize) -> Result<VideoFrame> {
        let (Some(payload), Some(info)) = (self.stream.frame(index), self.info.video.as_ref()) else {
            return Err(MediaError::Decode("Y4M: frame index out of range".into()));
        };
        let len = self.stream.frame_bytes;
        let bytes = read_range(&*self.source, payload.start, len).map_err(|err| MediaError::Decode(format!("Y4M: {err}")))?;
        if bytes.len() != len {
            return Err(MediaError::Decode("Y4M: incomplete frame payload".into()));
        }

        let header = &self.stream.header;
        let layout = header.layout;
        let planes = header.planes();
        let grey = header.chroma_samples(craft_y4m::Chroma::C420);
        let chroma = layout.chroma.map(chroma).unwrap_or(Chroma::C420);
        let data = if layout.bits == 8 {
            let y = Arc::new(bytes[planes.y].to_vec());
            let (cb, cr) = if layout.chroma.is_some() {
                (Arc::new(bytes[planes.cb].to_vec()), Arc::new(bytes[planes.cr].to_vec()))
            } else {
                let neutral = Arc::new(vec![128; grey]);
                (neutral.clone(), neutral)
            };
            let alpha = layout.alpha.then(|| Arc::new(bytes[planes.alpha].to_vec()));
            PixelData::Yuv8 { planes: [y, cb, cr], chroma, alpha }
        } else {
            let plane = |range: std::ops::Range<usize>| Arc::new(samples_u16(&bytes[range], layout.bits));
            let y = plane(planes.y);
            let (cb, cr) = if layout.chroma.is_some() {
                (plane(planes.cb), plane(planes.cr))
            } else {
                let neutral = Arc::new(vec![1 << (layout.bits - 1); grey]);
                (neutral.clone(), neutral)
            };
            PixelData::Yuv16 { planes: [y, cb, cr], chroma, bits: layout.bits, alpha: None }
        };
        Ok(VideoFrame { width: info.width, height: info.height, data, color: info.color, par: info.par, pts: picture_time(index, self.rate) })
    }
}

impl MediaSource for Y4mSource {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video_frame(&self, req: FrameRequest) -> Result<Arc<VideoFrame>> {
        let last = self.stream.frames.len().saturating_sub(1);
        let index = usize::try_from(self.rate.frame_at(req.time.max(Tick::ZERO))).unwrap_or(last).min(last);
        self.picture(index).map(Arc::new)
    }

    fn audio(&self, _start: i64, _frames: usize, _sample_rate: u32) -> Result<AudioBuffer> {
        Err(MediaError::NoStream("audio"))
    }
}

fn picture_time(index: usize, rate: FrameRate) -> Tick {
    // The parser's default frame-count cap is below i64::MAX on every target.
    // Round up to the same boundary used by frame_at, within Tick resolution and range.
    rate.tick_of(index as i64)
}

fn chroma(chroma: craft_y4m::Chroma) -> Chroma {
    match chroma {
        craft_y4m::Chroma::C420 => Chroma::C420,
        craft_y4m::Chroma::C422 => Chroma::C422,
        craft_y4m::Chroma::C444 => Chroma::C444,
    }
}

fn layout_label(layout: Layout) -> String {
    match (layout.chroma, layout.alpha) {
        (None, _) => format!("Monochrome {}-bit", layout.bits),
        (Some(_), true) => format!("YUVA 4:4:4:4 {}-bit", layout.bits),
        (Some(subsampling), false) => format!("YUV {} {}-bit", chroma(subsampling).label(), layout.bits),
    }
}
