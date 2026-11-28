//! [`LivePreview`]: one flow's preview state (session, region, covered area, settle time).

use craft_ui::canvas::PixelRect;

use super::key::{PreviewKey, PreviewLane, PreviewSession};

/// How many keys keep the area they change: as far back as a display's shown key can lag behind the last key given.
const AREA_KEYS: usize = 8;

/// The live preview of one flow (an adjustment dialog, or a Properties-panel drag): the keys of
/// its current session, the area a preview can change, what the canvas has composited of it, and
/// when the settings last changed.
///
/// Rules (PhotoCraft's `adjust_preview.rs`):
/// - Keys: [`key`](Self::key) gives equal keys for equal settings hashes within one session;
///   [`PreviewKey::NONE`] is the committed document. [`begin`](Self::begin) with another session
///   starts empty, so keys of the old one become foreign.
/// - Damage: [`switch`](Self::switch) is the area to recomposite when the shown key changes: the
///   region, or the areas the two keys change when both were keyed with one
///   ([`key_in`](Self::key_in)); [`uncovered`](Self::uncovered) catches up as the view moves
///   (`damage.rs`).
/// - Settle: [`settling`](Self::settling) is true for `within` seconds after the last key change
///   while a preview shows; secondary views (the Navigator) wait for it.
///
/// Times are the host's clock in seconds (egui's `input.time`).
#[derive(Clone, Debug, Default)]
pub struct LivePreview {
    pub(super) lane: PreviewLane,
    pub(super) session: Option<PreviewSession>,
    /// The area a preview of this session can change, in document pixels.
    pub(super) region: PixelRect,
    /// The last key [`key`](Self::key) gave and when it changed (s).
    pub(super) last: Option<(PreviewKey, f64)>,
    /// The canvas shows this key only within this rect; elsewhere in the region it is older.
    pub(super) covered: Option<(PreviewKey, PixelRect)>,
    /// The area each of the last [`AREA_KEYS`] keys given by [`key_in`](Self::key_in) changes, oldest first.
    pub(super) areas: Vec<(PreviewKey, PixelRect)>,
}

impl LivePreview {
    /// An empty preview of `lane`; the first [`begin`](Self::begin) starts its session.
    pub fn new(lane: PreviewLane) -> Self {
        Self { lane, ..Self::default() }
    }

    /// Starts `session` with `region` (document pixels) unless it is the current one; call it
    /// every pass before [`key`](Self::key). A new session forgets the old one's keys, covered
    /// area and settle time, so a reopened document whose id and revision repeat starts clean.
    ///
    /// `region` is what a preview can change: PhotoCraft's dialog takes the target's content
    /// within the document and the selection, grown by the effects' reach; its Properties panel
    /// takes the whole document (a reveal-all mask reports empty content bounds, and a region too
    /// small freezes the preview). The region is read once per session.
    pub fn begin(&mut self, session: PreviewSession, region: PixelRect) {
        // pc: crates/ui-egui/src/dialogs/adjust_preview.rs:241-262 (a new session when document or
        // session differ), :136 `region`, :505 `live_region`, :514 `live_key`.
        if self.session == Some(session) {
            return;
        }
        *self = Self { lane: self.lane, session: Some(session), region, last: None, covered: None, areas: Vec::new() };
    }

    /// Forgets the session (the document closed or the dialog's flow ended for good): every key
    /// is foreign until the next [`begin`](Self::begin).
    pub fn end(&mut self) {
        // pc: crates/ui-egui/src/dialogs/adjust_preview.rs:446 `retain_documents`.
        *self = Self::new(self.lane);
    }

    /// The current session, if any.
    pub fn session(&self) -> Option<PreviewSession> {
        self.session
    }

    /// The current session's region (empty without one).
    pub fn region(&self) -> PixelRect {
        self.region
    }

    /// The key of `settings` (a [`settings_hash`](super::settings_hash)) in the current session:
    /// the lane's tag bit, the session's low bits from bit 40, the hash's low 40 bits. When the
    /// key differs from the last one given, `now` becomes the settle timer's start. Without a
    /// session, [`PreviewKey::NONE`].
    pub fn key(&mut self, settings: u64, now: f64) -> PreviewKey {
        // pc: crates/ui-egui/src/dialogs/adjust_preview.rs:270 (`TAG | (session & 0x7_ffff) << 40 |
        // hash & (1 << 40) - 1`), :274 (`changed_ms`); :519 (`LIVE_TAG | (session & LIVE_SESSION_MASK)
        // << 40 | ..`).
        let Some(session) = self.session else {
            self.last = Some((PreviewKey::NONE, now));
            return PreviewKey::NONE;
        };
        let key = PreviewKey::compose(self.lane, session, settings);
        if self.last.is_none_or(|(last, _)| last != key) {
            self.last = Some((key, now));
        }
        key
    }

    /// [`key`](Self::key), recording `area` (document pixels) as what the preview of this key changes, so
    /// [`switch`](Self::switch) recomposites only that: a tool's preview that changes a small part of the region
    /// (Patch heals around one offset). The last `AREA_KEYS` keys keep their area; an older key falls back to the
    /// region, which damages more, never less.
    pub fn key_in(&mut self, settings: u64, now: f64, area: PixelRect) -> PreviewKey {
        let key = self.key(settings, now);
        if key.is_none() {
            return key;
        }
        self.areas.retain(|(known, _)| *known != key);
        if self.areas.len() >= AREA_KEYS {
            self.areas.remove(0);
        }
        self.areas.push((key, area));
        key
    }

    /// The area [`key_in`](Self::key_in) recorded for `key`; [`PreviewKey::NONE`] changes nothing (an empty rect).
    /// `None` for a key keyed without an area or evicted since.
    fn area(&self, key: PreviewKey) -> Option<PixelRect> {
        if key.is_none() {
            return Some(PixelRect::default());
        }
        self.areas.iter().find(|(known, _)| *known == key).map(|(_, area)| *area)
    }

    /// The pixels that differ between showing `from` and showing `to`: the union of the areas the two keys were
    /// keyed with ([`key_in`](Self::key_in); `NONE` adds nothing). `None` unless both keys are owned and not both
    /// `NONE`, or when a key other than `NONE` has no recorded area. Read-only, for a host that computes its damage
    /// from shared state.
    pub fn changed(&self, from: PreviewKey, to: PreviewKey) -> Option<PixelRect> {
        if !(self.owns(from) && self.owns(to) && (!from.is_none() || !to.is_none())) {
            return None;
        }
        Some(self.area(from)?.union(&self.area(to)?))
    }

    /// Whether `key` is [`PreviewKey::NONE`] or one of the current session's keys.
    pub fn owns(&self, key: PreviewKey) -> bool {
        // pc: crates/ui-egui/src/dialogs/adjust_preview.rs:297-298 (`ours`: key 0 or the session's
        // prefix above bit 40). Without a session nothing is owned, not even `NONE`.
        let Some(session) = self.session else { return false };
        key.is_none() || key.same_session(PreviewKey::compose(self.lane, session, 0))
    }

    /// True while the last key [`key`](Self::key) gave is not [`PreviewKey::NONE`] and changed
    /// less than `within` seconds before `now` (PhotoCraft's Navigator waits 0.2 s).
    pub fn settling(&self, now: f64, within: f64) -> bool {
        // pc: crates/ui-egui/src/dialogs/adjust_preview.rs:339 `settling` (a preview is shown and
        // `now - changed < ms`); crates/ui-egui/src/canvas/navigator.rs:13 `NAVIGATOR_SETTLE_MS` (200).
        self.last.is_some_and(|(key, changed)| !key.is_none() && now - changed < within)
    }

    /// The seconds left until [`settling`](Self::settling) turns false, while it is true: when a
    /// host waiting out the debounce asks for its next frame.
    pub fn settles_in(&self, now: f64, within: f64) -> Option<f64> {
        self.last.filter(|(key, _)| !key.is_none()).map(|(_, changed)| changed + within - now).filter(|left| *left > 0.0)
    }
}
