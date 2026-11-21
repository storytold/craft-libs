//! [`VideoError`], the error of the codec-agnostic layer ([`crate::TrackDecoder`]).

use crate::frame::FrameError;

/// Errors from the codec-agnostic layer.
///
/// Codec errors convert into it, so code mixing [`crate::TrackDecoder`] with the codec modules can
/// use `?` throughout: a codec's "unsupported" stays [`VideoError::Unsupported`], anything else in
/// the input is [`VideoError::Malformed`]. The message starts with the codec's name.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    /// The codec, or a feature of the stream, is valid but not supported (or its decoder is not
    /// in this build).
    #[error("unsupported video: {0}")]
    Unsupported(String),
    /// The input is corrupt or not what it claims to be.
    #[error("malformed video: {0}")]
    Malformed(String),
    /// A decoded picture did not convert into a [`crate::frame::VideoFrame`].
    #[error(transparent)]
    Frame(#[from] FrameError),
}

#[cfg(feature = "h264-decode")]
impl From<crate::h264::decode::Error> for VideoError {
    fn from(e: crate::h264::decode::Error) -> Self {
        match e {
            crate::h264::decode::Error::Unsupported(_) => VideoError::Unsupported(format!("H.264: {e}")),
            _ => VideoError::Malformed(format!("H.264: {e}")),
        }
    }
}

#[cfg(feature = "hevc-decode")]
impl From<crate::hevc::decode::Error> for VideoError {
    fn from(e: crate::hevc::decode::Error) -> Self {
        match e {
            crate::hevc::decode::Error::Unsupported(_) => VideoError::Unsupported(format!("HEVC: {e}")),
            _ => VideoError::Malformed(format!("HEVC: {e}")),
        }
    }
}

#[cfg(feature = "vp9-decode")]
impl From<crate::vp9::decode::Error> for VideoError {
    fn from(e: crate::vp9::decode::Error) -> Self {
        match e {
            crate::vp9::decode::Error::Unsupported(_) => VideoError::Unsupported(format!("VP9: {e}")),
            _ => VideoError::Malformed(format!("VP9: {e}")),
        }
    }
}

#[cfg(feature = "av1-decode")]
impl From<crate::av1::decode::Error> for VideoError {
    fn from(e: crate::av1::decode::Error) -> Self {
        match e {
            crate::av1::decode::Error::Unsupported(_) => VideoError::Unsupported(format!("AV1: {e}")),
            _ => VideoError::Malformed(format!("AV1: {e}")),
        }
    }
}

#[cfg(feature = "mpeg2-decode")]
impl From<crate::mpeg2::Error> for VideoError {
    fn from(e: crate::mpeg2::Error) -> Self {
        match e {
            crate::mpeg2::Error::Unsupported(_) => VideoError::Unsupported(format!("MPEG-2: {e}")),
            crate::mpeg2::Error::Invalid(_) => VideoError::Malformed(format!("MPEG-2: {e}")),
        }
    }
}

#[cfg(feature = "prores")]
impl From<crate::prores::Error> for VideoError {
    fn from(e: crate::prores::Error) -> Self {
        match e {
            crate::prores::Error::Unsupported(_) => VideoError::Unsupported(format!("ProRes: {e}")),
            _ => VideoError::Malformed(format!("ProRes: {e}")),
        }
    }
}

#[cfg(feature = "dnx")]
impl From<crate::dnx::Error> for VideoError {
    fn from(e: crate::dnx::Error) -> Self {
        match e {
            crate::dnx::Error::UnknownCid(_) => VideoError::Unsupported(format!("DNx: {e}")),
            _ => VideoError::Malformed(format!("DNx: {e}")),
        }
    }
}

#[cfg(feature = "apv-decode")]
impl From<crate::apv::Error> for VideoError {
    fn from(e: crate::apv::Error) -> Self {
        VideoError::Malformed(format!("APV: {e}"))
    }
}

#[cfg(feature = "ffv1")]
impl From<crate::ffv1::Error> for VideoError {
    fn from(e: crate::ffv1::Error) -> Self {
        match e {
            crate::ffv1::Error::Unsupported(_) => VideoError::Unsupported(format!("FFV1: {e}")),
            _ => VideoError::Malformed(format!("FFV1: {e}")),
        }
    }
}

#[cfg(feature = "vp8-decode")]
impl From<crate::vp8::Error> for VideoError {
    fn from(e: crate::vp8::Error) -> Self {
        VideoError::Malformed(format!("VP8: {e}"))
    }
}

#[cfg(feature = "mjpeg")]
impl From<crate::mjpeg::Error> for VideoError {
    fn from(e: crate::mjpeg::Error) -> Self {
        match e {
            crate::mjpeg::Error::Unsupported(_) => VideoError::Unsupported(format!("Motion JPEG: {e}")),
            _ => VideoError::Malformed(format!("Motion JPEG: {e}")),
        }
    }
}
