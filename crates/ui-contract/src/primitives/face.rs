//! The one host painter a value carries.

use super::ContentPainterCx;
use egui::Rect;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// The one host painter a value carries: two faces with one key are the same value.
#[derive(Clone)]
pub struct FacePaint {
    key: u64,
    paint: Arc<dyn Fn(&ContentPainterCx<'_>, Rect) + Send + Sync>,
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

/// By its key: two faces with one key are the same value.
impl crate::contract::MemoProps for FacePaint {
    fn key(&self, key: &mut crate::contract::MemoKey) {
        crate::contract::MemoProps::key(&self.key, key);
    }
}

/// A face is replaced, never written: its painter is shared and its key names what it paints.
impl crate::contract::PlainData for FacePaint {}

impl fmt::Debug for FacePaint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("FacePaint").field(&self.key).finish()
    }
}
