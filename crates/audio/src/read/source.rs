//! What the decoders read from: borrowed bytes or a shared [`ByteSource`], with small typed reads.

use crate::{AudioError, Result};
use craft_media::ByteSource;
use std::borrow::Cow;
use std::io;
use std::sync::Arc;

/// The bytes a decoder reads: a borrowed buffer, or a source shared with the caller (a file, a
/// network-backed buffer).
#[derive(Clone)]
pub(crate) enum Src<'a> {
    Slice(&'a [u8]),
    Shared(Arc<dyn ByteSource + Send + Sync>),
}

impl ByteSource for Src<'_> {
    fn len(&self) -> u64 {
        match self {
            Src::Slice(b) => <[u8]>::len(b) as u64,
            Src::Shared(s) => s.len(),
        }
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        match self {
            Src::Slice(b) => b.read_at(offset, buf),
            Src::Shared(s) => s.read_at(offset, buf),
        }
    }
}

impl Src<'_> {
    /// The source's length in bytes.
    pub(crate) fn size(&self) -> u64 {
        ByteSource::len(self)
    }

    /// `n` bytes at `pos`, or `None` when they are not all there.
    pub(crate) fn bytes(&self, pos: u64, n: usize) -> Option<Vec<u8>> {
        if let Src::Slice(b) = self {
            let start = usize::try_from(pos).ok()?;
            return b.get(start..start.checked_add(n)?).map(<[u8]>::to_vec);
        }
        let mut v = vec![0; n];
        self.read_at(pos, &mut v).ok()?;
        Some(v)
    }

    /// Up to `n` bytes at `pos`: fewer near the end of the source.
    pub(crate) fn bytes_upto(&self, pos: u64, n: usize) -> Vec<u8> {
        let n = n.min(usize::try_from(self.size().saturating_sub(pos)).unwrap_or(usize::MAX));
        self.bytes(pos, n).unwrap_or_default()
    }

    fn array<const N: usize>(&self, pos: u64) -> Option<[u8; N]> {
        let mut a = [0; N];
        self.read_at(pos, &mut a).ok()?;
        Some(a)
    }

    pub(crate) fn tag(&self, pos: u64) -> Option<[u8; 4]> {
        self.array(pos)
    }

    pub(crate) fn le_u32(&self, pos: u64) -> Option<u32> {
        self.array(pos).map(u32::from_le_bytes)
    }

    pub(crate) fn be_u32(&self, pos: u64) -> Option<u32> {
        self.array(pos).map(u32::from_be_bytes)
    }

    /// The first bytes of the source, enough to detect its format.
    pub(crate) fn head(&self) -> Vec<u8> {
        self.bytes_upto(0, 64 * 1024)
    }

    /// The whole source in memory: borrowed for a buffer, read once for a shared source.
    pub(crate) fn whole(&self) -> Result<Cow<'_, [u8]>> {
        match self {
            Src::Slice(b) => Ok(Cow::Borrowed(b)),
            Src::Shared(s) => {
                let n = usize::try_from(s.len()).map_err(|_| AudioError::TooLarge("the source does not fit in memory".into()))?;
                let mut v = Vec::new();
                v.try_reserve_exact(n).map_err(|_| AudioError::TooLarge("the source does not fit in memory".into()))?;
                v.resize(n, 0);
                s.read_at(0, &mut v)?;
                Ok(Cow::Owned(v))
            }
        }
    }
}
