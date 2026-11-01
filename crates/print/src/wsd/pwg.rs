//! PWG raster (`image/pwg-raster`) writer. The encoder itself moved to [`crate::pwg`], which is
//! available without the `wsd` feature (an app's [`DocumentConverter`](crate::DocumentConverter)
//! uses it too); this module keeps the old paths and the [`WsdError`] result type of
//! [`encode`].

use super::error::{Result, WsdError};

pub use crate::pwg::{Duplex, MAX_PAGE_BYTES, MAX_PAGES, MAX_SIDE, MAX_STREAM_BYTES, PwgError, PwgOptions, RasterPage};

#[cfg(test)]
pub(crate) use crate::pwg::decode_for_tests;

impl From<PwgError> for WsdError {
    fn from(error: PwgError) -> Self {
        match error {
            PwgError::TooLarge(what, limit) => WsdError::TooLarge(what, limit),
            PwgError::Invalid(why) => WsdError::Encode(why),
        }
    }
}

/// Pack `pages` into a PWG raster stream; see [`crate::pwg::encode`].
pub fn encode(pages: &[RasterPage], opts: &PwgOptions) -> Result<Vec<u8>> {
    crate::pwg::encode(pages, opts).map_err(WsdError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_paths_still_encode_and_errors_keep_their_wsd_shape() {
        let page = RasterPage { width: 2, height: 2, gray: true, pixels: vec![0, 1, 2, 3] };
        let opts = PwgOptions { dpi: (300, 300), duplex: Duplex::Off };
        let bytes = encode(std::slice::from_ref(&page), &opts).unwrap();
        assert_eq!(decode_for_tests(&bytes).unwrap(), [page]);
        assert!(matches!(encode(&[], &opts), Err(WsdError::Encode(_))));
        let huge = RasterPage { width: MAX_SIDE, height: MAX_SIDE, gray: false, pixels: vec![] };
        assert!(matches!(encode(&[huge], &opts), Err(WsdError::TooLarge(..))));
    }
}
