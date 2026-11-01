//! The idempotency tag: a short random reference that one [`PrintJob`](crate::PrintJob) puts on
//! every attempt of one print, so that a later reconcile can find a job that an attempt created even
//! when its reply was lost.
//!
//! The data types ([`Tag`], [`TagSupport`], [`TagPlacement`]) are `craft-job`'s, shared with
//! scanning; this module adds what printing needs on top: a random tag ([`TagExt::random`]), putting
//! it on a name within a length limit ([`TagExt::apply_to`]) and reading it back
//! ([`TagExt::find_in`]), plus [`TagPolicy`].
//!
//! The tag is a *correlation id*, not a secret: where the protocol has no dedicated field it travels
//! as a suffix of the job name (`"Quarterly report [ref abc123de]"`), visible to anyone who can see
//! the queue. See `docs/job-tracking.md` for why it is on by default for network backends and off
//! for the local spooler.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

use craft_job::{Tag, TagPlacement, TagSupport};

/// Printing's additions to [`Tag`]. Import the trait (it is re-exported from the crate root) to
/// call these as `Tag::random()`, `tag.apply_to(..)` and so on.
pub trait TagExt: Sized {
    /// The number of characters in a tag.
    const LEN: usize = 8;

    /// A new tag from the process's randomly keyed hasher and a counter: unique within the process,
    /// unpredictable across runs. It is a correlation id, not a security token.
    fn random() -> Self;

    /// The tag a name ends with, or carries as the last well-formed `[ref xxxxxxxx]` marker
    /// ([`Tag::parse_from_title`]: a device may have appended to the name).
    fn find_in(name: &str) -> Option<Self>;

    /// `name` without a trailing tag suffix.
    fn strip(name: &str) -> &str;

    /// `name` with this tag as its suffix, **exactly once**: a suffix the name already ends with is
    /// replaced, never doubled. With `max_chars` the name is cut so that the whole result fits and
    /// the suffix survives; a limit shorter than the suffix itself leaves the tag out (the cut name
    /// alone is returned).
    fn apply_to(&self, name: &str, max_chars: Option<usize>) -> String;
}

const PREFIX: &str = " [ref ";

impl TagExt for Tag {
    fn random() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        Self::from_seed(hasher.finish())
    }

    fn find_in(name: &str) -> Option<Self> {
        Tag::parse_from_title(name)
    }

    fn strip(name: &str) -> &str {
        split_suffix(name).map_or(name, |(base, _)| base)
    }

    fn apply_to(&self, name: &str, max_chars: Option<usize>) -> String {
        let base = Self::strip(name);
        let suffix = self.suffix();
        let Some(max) = max_chars else {
            return format!("{base}{suffix}");
        };
        let room = max.saturating_sub(suffix.chars().count());
        if room == 0 && max < suffix.chars().count() {
            return base.chars().take(max).collect();
        }
        let mut out: String = base.chars().take(room).collect();
        out.push_str(&suffix);
        out
    }
}

fn split_suffix(name: &str) -> Option<(&str, Tag)> {
    let body = name.strip_suffix(']')?;
    let start = body.rfind(PREFIX)?;
    let tag = Tag::parse(body.get(start + PREFIX.len()..)?).ok()?;
    Some((body.get(..start)?, tag))
}

/// What printing asks of a [`TagSupport`].
pub trait TagSupportExt {
    /// Whether the backend carries a tag at all (any placement but [`TagPlacement::None`]).
    fn carries(&self) -> bool;
    /// Whether a job listing returns the tag reliably, so that a reconcile can match on it
    /// ([`TagSupport::Carried`]). A [`TagSupport::Partial`] backend carries the tag, but the device
    /// may rewrite it, so a reconcile tries the tag first and falls back to title, user and time
    /// only when no listed job still carries it.
    fn matchable(&self) -> bool;
}

impl TagSupportExt for TagSupport {
    fn carries(&self) -> bool {
        !matches!(self.placement(), TagPlacement::None)
    }

    fn matchable(&self) -> bool {
        matches!(self, TagSupport::Carried(placement) if !matches!(placement, TagPlacement::None))
    }
}

/// Whether a [`PrintJob`](crate::PrintJob) puts its tag on the requests it sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TagPolicy {
    /// On for [`BackendClass::Network`](crate::BackendClass::Network) backends, off for local ones.
    #[default]
    Auto,
    /// On wherever the backend can carry a tag.
    Always,
    /// Never.
    Never,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_random_tag_is_eight_hex_characters_and_differs_between_calls() {
        let a = Tag::random();
        let b = Tag::random();
        assert_eq!(a.as_str().len(), Tag::LEN);
        assert_eq!(Tag::parse(a.as_str()), Ok(a));
        assert_ne!(a, b);
    }

    #[test]
    fn the_suffix_is_applied_once_and_found_again() {
        let tag = Tag::parse("abc123de").unwrap_or_else(|_| Tag::random());
        let named = tag.apply_to("Report", None);
        assert_eq!(named, "Report [ref abc123de]");
        assert_eq!(tag.apply_to(&named, None), named, "applying twice does not double the suffix");
        assert_eq!(Tag::find_in(&named), Some(tag));
        assert_eq!(Tag::strip(&named), "Report");
        assert_eq!(Tag::strip("Report"), "Report");
        assert_eq!(Tag::find_in("Report [ref nothex!!]"), None);
    }

    #[test]
    fn a_marker_a_device_left_in_the_middle_of_the_name_is_still_found_but_not_stripped() {
        // craft-job's rule: the last well-formed marker anywhere counts, so a name that a device
        // extended is still recognised. Only a trailing suffix is removed from the display title.
        let tag = Tag::parse("abc123de").unwrap_or_else(|_| Tag::random());
        assert_eq!(Tag::find_in("Report [ref abc123de] more"), Some(tag));
        assert_eq!(Tag::strip("Report [ref abc123de] more"), "Report [ref abc123de] more");
    }

    #[test]
    fn a_length_limit_cuts_the_title_and_keeps_the_suffix() {
        let tag = Tag::parse("abc123de").unwrap_or_else(|_| Tag::random());
        let long = "x".repeat(400);
        let named = tag.apply_to(&long, Some(255));
        assert_eq!(named.chars().count(), 255);
        assert!(named.ends_with(" [ref abc123de]"));
        assert_eq!(Tag::find_in(&named), Some(tag));
        assert_eq!(tag.apply_to("abc", Some(5)), "abc", "a limit below the suffix leaves the tag out");
        let multibyte = tag.apply_to(&"é".repeat(300), Some(255));
        assert_eq!(multibyte.chars().count(), 255, "limits count characters, not bytes");
    }

    #[test]
    fn support_helpers() {
        assert!(!TagSupport::None.carries() && !TagSupport::None.matchable());
        assert!(TagSupport::Carried(TagPlacement::NativeAttribute).carries());
        assert!(TagSupport::Carried(TagPlacement::NativeAttribute).matchable());
        let partial = TagSupport::Partial(TagPlacement::JobNameSuffix);
        assert!(partial.carries() && !partial.matchable(), "carried, but a reconcile does not rely on finding it");
        assert!(!TagSupport::Carried(TagPlacement::None).carries());
    }
}
