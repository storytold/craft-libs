//! [`PreviewKey`], [`PreviewLane`], [`PreviewSession`] and [`settings_hash`]: what names one
//! preview.

/// FNV-1a (64-bit: offset basis `0xcbf2_9ce4_8422_2325`, prime `0x0100_0000_01b3`) over `parts`
/// in order, with the lowest bit set so the result is never 0. The host hashes everything that
/// changes the render: PhotoCraft's dialog hashes the kind, its parameters and the Preview check
/// box; the Properties panel the parameters alone.
pub fn settings_hash(parts: &[&[u8]]) -> u64 {
    // pc: crates/ui-egui/src/dialogs/adjust_preview.rs:198 `hash` (FNV-1a, `| 1`); :232 (session
    // parts), :233 (kind, params, Preview flag).
    parts.iter().flat_map(|p| p.iter()).fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)) | 1
}

/// Which flow a [`LivePreview`](super::LivePreview) keys, so a key of one is never mistaken for
/// the other's. The lane is the key's tag bit; the session takes the bits from 40 up to it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PreviewLane {
    /// An Image › Adjustments dialog: tag `1 << 59`, session 19 bits (40..59).
    #[default]
    Dialog,
    /// A Properties-panel drag on an adjustment layer: tag `1 << 58`, session 18 bits (40..58).
    Properties,
}

impl PreviewLane {
    /// The lane's tag bit (59 or 58) and the session's width in bits (19 or 18).
    pub(crate) fn bits(self) -> (u32, u32) {
        // pc: crates/ui-egui/src/dialogs/adjust_preview.rs:52 `TAG` (1 << 59), :55 `SESSION_SHIFT`
        // (40), :272 (`session & 0x7_ffff`); :476 `LIVE_TAG` (1 << 58), :479, :481
        // `LIVE_SESSION_MASK` (18 bits).
        match self {
            PreviewLane::Dialog => (59, 19),
            PreviewLane::Properties => (58, 18),
        }
    }
}

/// What one preview shows: the committed document ([`NONE`](Self::NONE)) or one settings value
/// within one session of one lane.
///
/// Layout: the lane's tag bit, the session's low bits from bit 40 up to the tag, and the low 40
/// bits of the [`settings_hash`]. Equal settings in one session give equal keys; a key of another
/// session or lane is foreign to a [`LivePreview`](super::LivePreview). `Copy + Eq + Hash`, so
/// it keys the host's render jobs and texture caches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PreviewKey(u64);

impl PreviewKey {
    /// The bits from 40 up: the lane's tag and the session's low bits. Equal for the keys of one
    /// session and lane.
    pub(super) const SESSION_SHIFT: u32 = 40;

    /// The key of `settings` in `session` of `lane`: the lane's tag bit, the session's low bits
    /// from bit 40 up to the tag, and the low 40 bits of `settings`.
    pub(super) fn compose(lane: PreviewLane, session: PreviewSession, settings: u64) -> Self {
        let (tag_bit, session_bits) = lane.bits();
        let mask = (1u64 << session_bits) - 1;
        Self((1 << tag_bit) | ((session.0 & mask) << Self::SESSION_SHIFT) | (settings & ((1 << Self::SESSION_SHIFT) - 1)))
    }

    /// Whether `self` carries the lane and session bits of `prefix` (a key of the same session).
    pub(super) fn same_session(self, prefix: PreviewKey) -> bool {
        self.0 >> Self::SESSION_SHIFT == prefix.0 >> Self::SESSION_SHIFT
    }

    /// The committed document, no preview (PhotoCraft's key 0). Also the key while the Preview
    /// check box is off.
    pub const NONE: PreviewKey = PreviewKey(0);

    /// Whether this is [`NONE`](Self::NONE).
    pub fn is_none(self) -> bool {
        self == Self::NONE
    }

    /// The key as a number, for a host that passes it through its own canvas code.
    pub fn raw(self) -> u64 {
        self.0
    }

    /// A key from [`raw`](Self::raw).
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// The document state a preview belongs to. A new session (another document, revision or target)
/// starts a [`LivePreview`](super::LivePreview) afresh: no keys, no covered area, a new region.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PreviewSession(u64);

impl PreviewSession {
    /// The session of `target` (the layer the adjustment applies to; the host's id) in `document`
    /// at `revision`: [`settings_hash`] of the three numbers as little-endian bytes.
    pub fn new(document: u64, revision: u64, target: u64) -> Self {
        // pc: crates/ui-egui/src/dialogs/adjust_preview.rs:232 (document id, revision, target id,
        // channel-target flag); :514 `live_key` (document id, revision, layer id).
        Self(settings_hash(&[&document.to_le_bytes(), &revision.to_le_bytes(), &target.to_le_bytes()]))
    }

    /// Folds one more value into the session, one that starts a new session when it changes
    /// (PhotoCraft's dialog: whether the command targets a channel, the Quick Mask or a mask).
    pub fn and(self, part: u64) -> Self {
        // pc: crates/ui-egui/src/dialogs/adjust_preview.rs:229-232 (`channel_target`).
        Self(settings_hash(&[&self.0.to_le_bytes(), &part.to_le_bytes()]))
    }
}
