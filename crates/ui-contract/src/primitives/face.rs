//! The one host painter a value carries.

use super::ContentPainterCx;
use egui::Rect;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

type Paint = dyn Fn(&ContentPainterCx<'_>, Rect) + Send + Sync;

/// The one host painter a value carries: two faces with one key are the same value.
#[derive(Clone, crate::contract::MemoProps, craft_ui_macros::PlainData)]
pub struct FacePaint {
    key: u64,
    #[memo_props(covered_by = "key", reason = "the key names what the painter paints")]
    #[plain_data(assume = "a face is replaced, never written: its painter is shared")]
    paint: Arc<Paint>,
}

impl FacePaint {
    /// A face identified by `key` that paints with `paint`.
    pub fn new(key: u64, paint: impl Fn(&ContentPainterCx<'_>, Rect) + Send + Sync + 'static) -> Self {
        Self { key, paint: Arc::new(paint) }
    }

    /// The key that identifies the face.
    pub fn key(&self) -> u64 {
        self.key
    }

    /// Paints the face into `rect`.
    pub fn paint(&self, cx: &ContentPainterCx<'_>, rect: Rect) {
        (self.paint)(cx, rect);
    }
}

impl PartialEq for FacePaint {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for FacePaint {}

impl Hash for FacePaint {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

impl fmt::Debug for FacePaint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("FacePaint").field(&self.key).finish()
    }
}
