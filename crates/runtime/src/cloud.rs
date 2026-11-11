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
use std::ops::Deref;

use craft_tier_store::{CloudBlobRef, HydrationGuard, MirrorStore, TierPolicy};

/// A cloud fetch the runtime cannot honestly perform on its own: it needs the host's networking
/// (`fetch`/XHR on `wasm32`, or a native HTTP client off the web target). Implemented by whatever
/// embeds `craft-runtime` behind the `cloud-delegate` feature.
pub trait CloudFetchDelegate {
    /// Fetch `length` bytes of `blob` starting at `offset`.
    fn fetch_range(&self, blob: &CloudBlobRef, offset: u64, length: u64) -> Result<Vec<u8>, CloudFetchFault>;

    /// Fetch the entire blob, reading no more than `max_bytes`. Only ever called by
    /// [`hydrate_whole_file_fallback`] after [`stat_size`](Self::stat_size) has confirmed a
    /// stated size at or under `max_bytes`: a delegate's `fetch_whole` does not need to re-derive
    /// that limit itself, only enforce it while streaming the response body. `stat_size` can be
    /// stale (the blob changed between the stat and this call), so a correct implementation
    /// stops reading at `max_bytes` rather than trusting the size it already reported; a
    /// delegate that cannot stop mid-stream should at least check the final length before
    /// returning, so that an oversized response still surfaces as a fault here instead of in the
    /// caller. [`hydrate_whole_file_fallback`] re-checks the returned length against `max_bytes`
    /// either way, as the last line of defense.
    fn fetch_whole(&self, blob: &CloudBlobRef, max_bytes: u64) -> Result<Vec<u8>, CloudFetchFault>;

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
    /// `stat_size` approved a size that fit the budget, but the delegate's `fetch_whole`
    /// actually returned more bytes than the approved limit (a stale stat, or a blob that grew
    /// between the two calls). The oversized buffer is discarded here rather than handed back to
    /// the caller uncharged.
    OversizedResponse { approved_bytes: u64, actual_bytes: u64 },
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
            Self::OversizedResponse { approved_bytes, actual_bytes } => {
                write!(f, "cloud fetch: whole-file read returned {actual_bytes} bytes, past the {approved_bytes}-byte approved limit")
            }
            Self::Delegate(message) => write!(f, "cloud fetch: {message}"),
        }
    }
}

impl std::error::Error for CloudFetchFault {}

/// Bytes returned by [`hydrate_whole_file_fallback`], charged against the mirror's byte budget
/// for as long as this value stays alive.
///
/// CodeRabbit finding on cloud.rs:84: the budget reservation used to live only inside
/// `hydrate_whole_file_fallback`'s own stack frame, released the instant the function returned
/// bytes to its caller. The returned `Vec<u8>` could then sit in memory, uncounted by any
/// budget, for as long as the caller held it. Wrapping the bytes with the reservation means the
/// charge follows the bytes instead of stopping at the function boundary: call
/// [`into_bytes`](Self::into_bytes) once the bytes have been landed somewhere that charges them
/// on its own (for example [`MirrorStore::put`]), or simply drop this value once they are no
/// longer needed. Momentarily double-counting (both the reservation here and a fresh `put`) is
/// the safe direction: it only makes `over_budget` trip earlier, never later.
pub struct ChargedBytes<'a, P: TierPolicy> {
    bytes: Vec<u8>,
    _guard: HydrationGuard<'a, P>,
}

impl<P: TierPolicy> ChargedBytes<'_, P> {
    /// Take the bytes, releasing the reservation. The caller becomes responsible for accounting
    /// for them from this point on.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl<P: TierPolicy> Deref for ChargedBytes<'_, P> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}

/// Fall back from a ranged hydration to reading `blob` whole, through `delegate`.
///
/// Stats the blob first and refuses the read outright, before calling
/// [`fetch_whole`](CloudFetchDelegate::fetch_whole) at all, when its size would not fit what
/// remains of `mirror`'s byte budget (see [`MirrorStore::remaining_budget`]). This is the guard
/// RFC-0023 required fix 4 calls for: without it, a whole-file fallback has no idea how big the
/// blob is until the bytes are already in memory.
///
/// Passes that approved size into `fetch_whole` as a hard ceiling (CodeRabbit finding on
/// cloud.rs:33), and re-checks the actual returned length against it (defense in depth: checking
/// after the fact cannot undo an oversized allocation the delegate already made, but it stops an
/// oversized buffer from ever reaching the caller uncharged and unbounded).
pub fn hydrate_whole_file_fallback<'a, P: TierPolicy>(
    mirror: &'a MirrorStore<P>,
    blob: &CloudBlobRef,
    delegate: &dyn CloudFetchDelegate,
) -> Result<ChargedBytes<'a, P>, CloudFetchFault> {
    let blob_bytes = delegate.stat_size(blob)?;
    let remaining_budget = mirror.remaining_budget() as u64;
    if blob_bytes > remaining_budget {
        return Err(CloudFetchFault::BudgetExceeded { blob_bytes, remaining_budget });
    }
    // Reserve the stated size against the budget for the duration of the fetch: the bytes are
    // about to exist in memory even though they have not landed in `mirror` via `put`/`seed` yet.
    // The guard moves into `ChargedBytes` below, so the reservation now outlives this function.
    let blob_bytes_usize = usize::try_from(blob_bytes).unwrap_or(usize::MAX);
    let guard = mirror.begin_range_hydration(blob_bytes_usize);
    let bytes = delegate.fetch_whole(blob, remaining_budget)?;
    if bytes.len() as u64 > remaining_budget {
        // The delegate ignored (or could not honor) the max_bytes ceiling: refuse rather than
        // hand an unbounded, uncharged buffer back to the caller. `guard` drops here, releasing
        // the reservation along with the discarded bytes.
        return Err(CloudFetchFault::OversizedResponse { approved_bytes: remaining_budget, actual_bytes: bytes.len() as u64 });
    }
    Ok(ChargedBytes { bytes, _guard: guard })
}

#[cfg(test)]
mod tests {
    use craft_tier_store::{DefaultTierPolicy, MirrorStore};

    use super::*;

    struct FixedSizeDelegate {
        size: u64,
        /// Bytes actually returned by `fetch_whole`; defaults to `size` bytes. A test can set
        /// this higher than `size` to simulate a stale stat (the blob grew after `stat_size`).
        actual_bytes: u64,
        whole_file_calls: std::cell::Cell<u32>,
        last_max_bytes: std::cell::Cell<u64>,
    }

    impl FixedSizeDelegate {
        fn new(size: u64) -> Self {
            Self { size, actual_bytes: size, whole_file_calls: std::cell::Cell::new(0), last_max_bytes: std::cell::Cell::new(0) }
        }
    }

    impl CloudFetchDelegate for FixedSizeDelegate {
        fn fetch_range(&self, _blob: &CloudBlobRef, _offset: u64, _length: u64) -> Result<Vec<u8>, CloudFetchFault> {
            Err(CloudFetchFault::Delegate("fetch_range not used by this test".into()))
        }

        fn fetch_whole(&self, _blob: &CloudBlobRef, max_bytes: u64) -> Result<Vec<u8>, CloudFetchFault> {
            self.whole_file_calls.set(self.whole_file_calls.get() + 1);
            self.last_max_bytes.set(max_bytes);
            Ok(vec![0u8; self.actual_bytes as usize])
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
        let mirror = MirrorStore::new(DefaultTierPolicy, 100);
        let delegate = FixedSizeDelegate::new(101);

        let result = hydrate_whole_file_fallback(&mirror, &blob(), &delegate);

        assert_eq!(result.err(), Some(CloudFetchFault::BudgetExceeded { blob_bytes: 101, remaining_budget: 100 }));
        assert_eq!(delegate.whole_file_calls.get(), 0, "fetch_whole must never be called once stat_size shows it would not fit");
        assert!(!mirror.over_budget(), "a refused fetch must leave no reservation behind");
    }

    /// A blob that fits the remaining budget proceeds to the actual read, charges the budget for
    /// as long as the caller holds the result, and releases it once dropped (CodeRabbit finding
    /// on cloud.rs:84).
    #[test]
    fn a_blob_that_fits_the_remaining_budget_proceeds_and_stays_charged_until_dropped() {
        let mirror = MirrorStore::new(DefaultTierPolicy, 100);
        let delegate = FixedSizeDelegate::new(40);

        assert_eq!(mirror.remaining_budget(), 100);
        let result = hydrate_whole_file_fallback(&mirror, &blob(), &delegate).expect("blob fits");
        assert_eq!(&*result, vec![0u8; 40].as_slice());
        assert_eq!(delegate.whole_file_calls.get(), 1);
        assert_eq!(delegate.last_max_bytes.get(), 100, "fetch_whole must receive the approved ceiling");
        assert_eq!(mirror.remaining_budget(), 60, "the 40 returned bytes must still be charged while the caller holds them");

        drop(result);
        assert_eq!(mirror.remaining_budget(), 100, "dropping the result must release the reservation");
    }

    /// A blob sized exactly at the remaining budget fits (the refusal is strictly "would
    /// exceed", not "would reach").
    #[test]
    fn a_blob_sized_exactly_at_the_remaining_budget_fits() {
        let mirror = MirrorStore::new(DefaultTierPolicy, 100);
        let delegate = FixedSizeDelegate::new(100);

        let result = hydrate_whole_file_fallback(&mirror, &blob(), &delegate).expect("blob fits exactly");

        assert_eq!(result.into_bytes(), vec![0u8; 100]);
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
            fn fetch_whole(&self, _blob: &CloudBlobRef, _max_bytes: u64) -> Result<Vec<u8>, CloudFetchFault> {
                Err(CloudFetchFault::Delegate("fetch_whole must not be reached".into()))
            }
        }

        let mirror = MirrorStore::new(DefaultTierPolicy, 100);
        let result = hydrate_whole_file_fallback(&mirror, &blob(), &UnknownSizeDelegate);

        assert_eq!(result.err(), Some(CloudFetchFault::SizeUnknown));
    }

    /// CodeRabbit finding on cloud.rs:33: a stale `stat_size` approved 40 bytes, but the blob
    /// grew to 200 bytes by the time `fetch_whole` actually read it. The oversized response must
    /// be refused, not handed back to the caller uncharged and over budget.
    #[test]
    fn a_response_larger_than_the_approved_size_is_refused() {
        let mirror = MirrorStore::new(DefaultTierPolicy, 100);
        let mut delegate = FixedSizeDelegate::new(40);
        delegate.actual_bytes = 200;

        let result = hydrate_whole_file_fallback(&mirror, &blob(), &delegate);

        assert_eq!(result.err(), Some(CloudFetchFault::OversizedResponse { approved_bytes: 100, actual_bytes: 200 }));
        assert!(!mirror.over_budget(), "a refused oversized response must leave no reservation behind");
    }
}
