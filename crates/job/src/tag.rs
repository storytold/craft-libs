//! The idempotency properties of a job: a [`Tag`] the caller puts in the job where the device
//! lets it, and a [`Fingerprint`] of what was submitted, so a job whose submit reply was lost can
//! be recognised later. This module holds the data and its small helpers; looking jobs up and
//! deciding is the domain crate's business.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use crate::time::Timestamp;

const OPEN: &str = "[ref ";
const HEX_LEN: usize = 8;

/// Why text is not a [`Tag`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TagError;

impl fmt::Display for TagError {
    /// Writes the sentence "a tag is exactly 8 lowercase hexadecimal characters".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a tag is exactly 8 lowercase hexadecimal characters")
    }
}

impl std::error::Error for TagError {}

/// A short token, 8 lowercase hex characters, shown in a job title as ` [ref abc123de]`. No random
/// number generator is used: the caller supplies the seed ([`Self::from_seed`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(into = "String", try_from = "String"))]
pub struct Tag([u8; HEX_LEN]);

impl Tag {
    /// A tag mixed from `seed` (SplitMix64); the same seed gives the same tag.
    pub fn from_seed(seed: u64) -> Self {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        let bits = (z >> 32) as u32;
        let mut out = [b'0'; HEX_LEN];
        for (i, slot) in out.iter_mut().enumerate() {
            let nibble = ((bits >> (28 - 4 * i as u32)) & 0xf) as u8;
            *slot = if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 };
        }
        Self(out)
    }

    /// From text that must be exactly 8 lowercase hex characters.
    pub fn parse(text: &str) -> Result<Self, TagError> {
        let bytes = text.as_bytes();
        if bytes.len() != HEX_LEN || !bytes.iter().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b)) {
            return Err(TagError);
        }
        let mut out = [0u8; HEX_LEN];
        out.copy_from_slice(bytes);
        Ok(Self(out))
    }

    /// The 8 characters.
    pub fn as_str(&self) -> &str {
        // The bytes are validated ASCII by every constructor; fall back to "" rather than panic.
        std::str::from_utf8(&self.0).unwrap_or("")
    }

    /// The text to append to a job title: ` [ref abc123de]`.
    pub fn suffix(&self) -> String {
        format!(" {OPEN}{}]", self.as_str())
    }

    /// Recover a tag from a (possibly device-altered) job title: the **last** well-formed
    /// `[ref xxxxxxxx]` marker in it. Uppercase, short, long or unterminated markers are ignored.
    pub fn parse_from_title(title: &str) -> Option<Self> {
        let mut end = title.len();
        while let Some(pos) = title.get(..end)?.rfind(OPEN) {
            let start = pos + OPEN.len();
            if let (Some(token), Some("]")) = (title.get(start..start + HEX_LEN), title.get(start + HEX_LEN..start + HEX_LEN + 1))
                && let Ok(tag) = Self::parse(token)
            {
                return Some(tag);
            }
            end = pos;
        }
        None
    }
}

#[cfg(feature = "serde")]
impl From<Tag> for String {
    /// The tag as its 8 characters.
    fn from(value: Tag) -> String {
        value.as_str().to_string()
    }
}

#[cfg(feature = "serde")]
impl TryFrom<String> for Tag {
    type Error = TagError;

    /// Parses like [`Tag::parse`].
    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl fmt::Display for Tag {
    /// Writes the 8 characters, without the ` [ref ]` wrapper.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Tag {
    type Err = TagError;

    /// Parses like [`Tag::parse`].
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

/// Where a backend can put a [`Tag`] so it comes back when jobs are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum TagPlacement {
    /// Appended to the job name (the title the queue shows).
    JobNameSuffix,
    /// In the document name.
    DocumentName,
    /// In a native attribute the protocol has for it.
    NativeAttribute,
    /// Nowhere.
    #[cfg_attr(feature = "serde", serde(other))]
    None,
}

/// Whether a backend can carry a [`Tag`] through to a job listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum TagSupport {
    /// No: listings cannot show the tag (WIA has no job tag at all).
    None,
    /// Yes, on every job, at the placement.
    Carried(TagPlacement),
    /// Sometimes: the placement works but a device or driver may rewrite it.
    Partial(TagPlacement),
}

impl TagSupport {
    /// Where the tag goes ([`TagPlacement::None`] when unsupported).
    pub const fn placement(self) -> TagPlacement {
        match self {
            Self::None => TagPlacement::None,
            Self::Carried(p) | Self::Partial(p) => p,
        }
    }

    /// Whether a tag may be found again in a listing (carried or partial).
    pub const fn can_recover(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// What was submitted, kept to recognise the job later when the submit reply was lost: the title,
/// the user, when it was sent and the tag, if one was placed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Fingerprint {
    pub title: String,
    #[cfg_attr(feature = "serde", serde(default))]
    pub user: Option<String>,
    pub submitted: Timestamp,
    /// How far from `submitted` a device's creation time may be and still match (clock skew). On the wire: milliseconds.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::duration_ms", default = "crate::wire::default_window"))]
    pub window: Duration,
    #[cfg_attr(feature = "serde", serde(default))]
    pub tag: Option<Tag>,
}

impl Fingerprint {
    /// The default `window`: five minutes either way.
    pub const DEFAULT_WINDOW: Duration = Duration::from_secs(300);

    /// A fingerprint of the job `title` submitted at `submitted`, with no user or tag and the default window.
    pub fn new(title: impl Into<String>, submitted: Timestamp) -> Self {
        Self { title: title.into(), user: None, submitted, window: Self::DEFAULT_WINDOW, tag: None }
    }

    /// Sets the user the job was submitted as, for matching against the owner a device lists.
    #[must_use]
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Sets how far a device's creation time may be from `submitted` and still match.
    #[must_use]
    pub fn with_window(mut self, window: Duration) -> Self {
        self.window = window;
        self
    }

    /// Sets the tag that was placed in the job.
    #[must_use]
    pub fn with_tag(mut self, tag: Tag) -> Self {
        self.tag = Some(tag);
        self
    }

    /// Whether `created` lies within `window` of the submission, edges included.
    pub fn within_window(&self, created: Timestamp) -> bool {
        self.submitted.abs_diff(created) <= self.window
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses `s` as a tag; the test fails if `s` is not a valid tag.
    fn tag(s: &str) -> Tag {
        Tag::parse(s).expect("valid tag")
    }

    #[test]
    fn from_seed_is_deterministic_hex_and_spread() {
        assert_eq!(Tag::from_seed(7), Tag::from_seed(7));
        let mut seen = std::collections::HashSet::new();
        for seed in 0..2000u64 {
            let t = Tag::from_seed(seed);
            assert_eq!(t.as_str().len(), 8);
            assert!(t.as_str().bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{t}");
            assert_eq!(Tag::parse(t.as_str()), Ok(t));
            seen.insert(t);
        }
        assert!(seen.len() > 1990, "collisions: {}", 2000 - seen.len());
        let _ = Tag::from_seed(0);
        let _ = Tag::from_seed(u64::MAX);
    }

    #[test]
    fn parse_accepts_exactly_eight_lowercase_hex() {
        assert_eq!(tag("abc123de").to_string(), "abc123de");
        assert_eq!("00000000".parse::<Tag>(), Ok(tag("00000000")));
        for bad in ["", "abc123d", "abc123def", "ABC123DE", "abc123dg", "abc 23de", "abc123d\u{e9}", "abc123de\n", " abc123de"] {
            assert_eq!(Tag::parse(bad), Err(TagError), "{bad:?}");
        }
        assert!(TagError.to_string().contains("8 lowercase"));
    }

    #[test]
    fn suffix_has_the_documented_shape_and_round_trips() {
        let t = tag("abc123de");
        assert_eq!(t.suffix(), " [ref abc123de]");
        let title = format!("Quarterly report.pdf{}", t.suffix());
        assert_eq!(Tag::parse_from_title(&title), Some(t));
    }

    #[test]
    fn parse_from_title_takes_the_last_well_formed_marker() {
        let p = Tag::parse_from_title;
        assert_eq!(p("x [ref aaaaaaaa] y [ref bbbbbbbb]"), Some(tag("bbbbbbbb")));
        assert_eq!(p("x [ref aaaaaaaa] y [ref BBBBBBBB]"), Some(tag("aaaaaaaa")), "a malformed later marker is skipped");
        assert_eq!(p("[ref aaaaaaaa]"), Some(tag("aaaaaaaa")));
        assert_eq!(p("[ref aaaaaaaa][ref bbbbbbbb]"), Some(tag("bbbbbbbb")));
    }

    #[test]
    fn parse_from_title_survives_hostile_input() {
        let p = Tag::parse_from_title;
        for bad in [
            "",
            "[ref ",
            "[ref ]",
            "[ref abc123de",
            "[ref abc123de ]",
            "[ref abc123d]",
            "[ref abc123def]",
            "[ref ABC123DE]",
            "[REF abc123de]",
            "[ref  abc123de]",
            "ref abc123de",
            "[ref \u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}]",
            "[ref abc123\u{e9}]",
            "[ref [ref [ref ",
            "\u{0}[ref \u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}]",
        ] {
            assert_eq!(p(bad), None, "{bad:?}");
        }
        let long = format!("{}[ref abc123de]{}", "[ref ".repeat(10_000), "\u{4e2d}".repeat(1000));
        assert_eq!(p(&long), Some(tag("abc123de")));
        assert_eq!(p(&"[ref ".repeat(50_000)), None);
        // a multi-byte character straddling where the token would end must not panic
        assert_eq!(p("[ref abc123d\u{4e2d}]"), None);
    }

    #[test]
    fn tag_support() {
        assert_eq!(TagSupport::None.placement(), TagPlacement::None);
        assert!(!TagSupport::None.can_recover());
        assert_eq!(TagSupport::Carried(TagPlacement::JobNameSuffix).placement(), TagPlacement::JobNameSuffix);
        assert!(TagSupport::Carried(TagPlacement::NativeAttribute).can_recover());
        assert_eq!(TagSupport::Partial(TagPlacement::DocumentName).placement(), TagPlacement::DocumentName);
        assert!(TagSupport::Partial(TagPlacement::DocumentName).can_recover());
    }

    #[test]
    fn fingerprint_window_edges_are_inclusive() {
        let at = Timestamp::from_secs(1000);
        let f = Fingerprint::new("t", at).with_window(Duration::from_secs(10)).with_user("u").with_tag(tag("abc123de"));
        assert_eq!((f.user.as_deref(), f.tag), (Some("u"), Some(tag("abc123de"))));
        assert!(f.within_window(at));
        assert!(f.within_window(Timestamp::from_secs(1010)), "upper edge");
        assert!(f.within_window(Timestamp::from_secs(990)), "lower edge");
        assert!(!f.within_window(Timestamp::from_millis(1_010_001)));
        assert!(!f.within_window(Timestamp::from_millis(989_999)));
        assert_eq!(Fingerprint::new("t", at).window, Fingerprint::DEFAULT_WINDOW);
        let zero = Fingerprint::new("t", at).with_window(Duration::ZERO);
        assert!(zero.within_window(at) && !zero.within_window(Timestamp::from_millis(1_000_001)));
    }
}
