//! Cloud-delegate fallback path: a whole-file read, gated on the blob's size.
//!
//! RFC-0023 required fix 4. The runtime hydrates most reads from the warm tier through a ranged
//! fetch, but some callers fall back to reading a blob whole (no range support on the source, or
//! a range request that itself failed). That fallback must learn the blob's size before touching
//! the delegate's read path at all, and refuse outright when the size would not fit what remains
//! of the mirror's byte budget. The guard this prevents: a whole-file read silently blowing past
//! the budget because nothing checked the size first, which is a backpressure failure mode, not
//! a crash, but with the same OOM-shaped consequence if left unchecked.

use std::fmt;

use craft_tier_store::{CloudBlobRef, MirrorStore, TierPolicy};

/// A cloud fetch the runtime cannot honestly perform on its own: it needs the host's networking
/// (`fetch`/XHR on `wasm32`, or a native HTTP client off the web target). Implemented by whatever
/// embeds `craft-runtime` behind the `cloud-delegate` feature.
pub trait CloudFetchDelegate {
    /// Fetch `length` bytes of `blob` starting at `offset`.
    fn fetch_range(&self, blob: &CloudBlobRef, offset: u64, length: u64) -> Result<Vec<u8>, CloudFetchFault>;

    /// Fetch the entire blob. Only ever called by
    /// [`hydrate_whole_file_fallback`] after [`stat_size`](Self::stat_size) has confirmed the
    /// blob fits the caller's remaining budget: a delegate's `fetch_whole` does not need to guard
    /// against an oversized read itself.
    fn fetch_whole(&self, blob: &CloudBlobRef) -> Result<Vec<u8>, CloudFetchFault>;

    /// Query the blob's size without reading its bytes (a HEAD request, directory-listing
    /// metadata, or similar). The default trusts
    /// [`CloudBlobRef::byte_length`](craft_tier_store::CloudBlobRef) when the host already knows
    /// it; a delegate whose refs never carry that field overrides this with a real stat call.
    fn stat_size(&self, blob: &CloudBlobRef) -> Result<u64, CloudFetchFault> {
        blob.byte_length.ok_or(CloudFetchFault::SizeUnknown)
    }
}

/// Why a cloud fetch did not produce bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudFetchFault {
    /// [`CloudFetchDelegate::stat_size`] could not determine the blob's size.
    SizeUnknown,
    /// The whole-file fallback was refused because the blob's (known) size would not fit what
    /// remains of the byte budget. Backpressure, not a silent OOM path: no bytes were read.
    BudgetExceeded { blob_bytes: u64, remaining_budget: u64 },
    /// The delegate's own fetch or stat call failed (network error, 404, etc).
    Delegate(String),
}

impl fmt::Display for CloudFetchFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeUnknown => write!(f, "cloud fetch: blob size is unknown, refusing a whole-file read"),
            Self::BudgetExceeded { blob_bytes, remaining_budget } => {
                write!(f, "cloud fetch: whole-file read of {blob_bytes} bytes refused, only {remaining_budget} bytes remain in the budget")
            }
            Self::Delegate(message) => write!(f, "cloud fetch: {message}"),
        }
    }
}

impl std::error::Error for CloudFetchFault {}

/// Fall back from a ranged hydration to reading `blob` whole, through `delegate`.
///
/// Stats the blob first and refuses the read outright, before calling
/// [`fetch_whole`](CloudFetchDelegate::fetch_whole) at all, when its size would not fit what
/// remains of `mirror`'s byte budget (see [`MirrorStore::remaining_budget`]). This is the guard
/// RFC-0023 required fix 4 calls for: without it, a whole-file fallback has no idea how big the
/// blob is until the bytes are already in memory.
pub fn hydrate_whole_file_fallback<P: TierPolicy>(
    mirror: &mut MirrorStore<P>,
    blob: &CloudBlobRef,
    delegate: &dyn CloudFetchDelegate,
) -> Result<Vec<u8>, CloudFetchFault> {
    let blob_bytes = delegate.stat_size(blob)?;
    let remaining_budget = mirror.remaining_budget() as u64;
    if blob_bytes > remaining_budget {
        return Err(CloudFetchFault::BudgetExceeded { blob_bytes, remaining_budget });
    }
    // Reserve the stated size against the budget for the duration of the fetch: the bytes are
    // about to exist in memory even though they have not landed in `mirror` via `put`/`seed` yet.
    let blob_bytes_usize = usize::try_from(blob_bytes).unwrap_or(usize::MAX);
    let _guard = mirror.begin_range_hydration(blob_bytes_usize);
    delegate.fetch_whole(blob)
}

#[cfg(test)]
mod tests {
    use craft_tier_store::{DefaultTierPolicy, MirrorStore};

    use super::*;

    struct FixedSizeDelegate {
        size: u64,
        whole_file_calls: std::cell::Cell<u32>,
    }

    impl CloudFetchDelegate for FixedSizeDelegate {
        fn fetch_range(&self, _blob: &CloudBlobRef, _offset: u64, _length: u64) -> Result<Vec<u8>, CloudFetchFault> {
            Err(CloudFetchFault::Delegate("fetch_range not used by this test".into()))
        }

        fn fetch_whole(&self, _blob: &CloudBlobRef) -> Result<Vec<u8>, CloudFetchFault> {
            self.whole_file_calls.set(self.whole_file_calls.get() + 1);
            Ok(vec![0u8; self.size as usize])
        }

        fn stat_size(&self, _blob: &CloudBlobRef) -> Result<u64, CloudFetchFault> {
            Ok(self.size)
        }
    }

    fn blob() -> CloudBlobRef {
        CloudBlobRef { file_id: "blob-1".into(), etag: None, content_type: None, byte_length: None }
    }

    /// RFC-0023 required fix 4: a blob larger than the remaining budget must be refused before
    /// any read happens. Before this fix, nothing stated the size up front, so the only way to
    /// discover the blob was too big was to already be holding its bytes in memory.
    #[test]
    fn a_blob_larger_than_the_remaining_budget_is_refused_before_any_read() {
        let mut mirror = MirrorStore::new(DefaultTierPolicy, 100);
        let delegate = FixedSizeDelegate { size: 101, whole_file_calls: std::cell::Cell::new(0) };

        let result = hydrate_whole_file_fallback(&mut mirror, &blob(), &delegate);

        assert_eq!(result, Err(CloudFetchFault::BudgetExceeded { blob_bytes: 101, remaining_budget: 100 }));
        assert_eq!(delegate.whole_file_calls.get(), 0, "fetch_whole must never be called once stat_size shows it would not fit");
        assert!(!mirror.over_budget(), "a refused fetch must leave no reservation behind");
    }

    /// A blob that fits the remaining budget proceeds to the actual read.
    #[test]
    fn a_blob_that_fits_the_remaining_budget_proceeds() {
        let mut mirror = MirrorStore::new(DefaultTierPolicy, 100);
        let delegate = FixedSizeDelegate { size: 40, whole_file_calls: std::cell::Cell::new(0) };

        let result = hydrate_whole_file_fallback(&mut mirror, &blob(), &delegate);

        assert_eq!(result, Ok(vec![0u8; 40]));
        assert_eq!(delegate.whole_file_calls.get(), 1);
        assert!(!mirror.over_budget(), "the reservation must be released once the fetch has returned");
    }

    /// A blob sized exactly at the remaining budget fits (the refusal is strictly "would
    /// exceed", not "would reach").
    #[test]
    fn a_blob_sized_exactly_at_the_remaining_budget_fits() {
        let mut mirror = MirrorStore::new(DefaultTierPolicy, 100);
        let delegate = FixedSizeDelegate { size: 100, whole_file_calls: std::cell::Cell::new(0) };

        let result = hydrate_whole_file_fallback(&mut mirror, &blob(), &delegate);

        assert_eq!(result, Ok(vec![0u8; 100]));
        assert_eq!(delegate.whole_file_calls.get(), 1);
    }

    /// An unknown size (no `byte_length`, and the delegate's `stat_size` cannot resolve it
    /// either) refuses the same way: never a silent whole-file read of unknown size.
    #[test]
    fn an_unknown_size_is_refused() {
        struct UnknownSizeDelegate;
        impl CloudFetchDelegate for UnknownSizeDelegate {
            fn fetch_range(&self, _blob: &CloudBlobRef, _offset: u64, _length: u64) -> Result<Vec<u8>, CloudFetchFault> {
                Err(CloudFetchFault::SizeUnknown)
            }
            fn fetch_whole(&self, _blob: &CloudBlobRef) -> Result<Vec<u8>, CloudFetchFault> {
                Err(CloudFetchFault::Delegate("fetch_whole must not be reached".into()))
            }
        }

        let mut mirror = MirrorStore::new(DefaultTierPolicy, 100);
        let result = hydrate_whole_file_fallback(&mut mirror, &blob(), &UnknownSizeDelegate);

        assert_eq!(result, Err(CloudFetchFault::SizeUnknown));
    }
}
