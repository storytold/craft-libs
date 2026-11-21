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
    #[allow(dead_code)] // P3(widgets-w6): wg.W04
    paint: Arc<dyn Fn(&ContentPainterCx<'_>, Rect) + Send + Sync>,
}

impl FacePaint {
    /// A face identified by `key` that paints with `paint`.
    pub fn new(key: u64, paint: impl Fn(&ContentPainterCx<'_>, Rect) + Send + Sync + 'static) -> Self {
        let _ = (key, paint);
        todo!("wg.W04")
    }

    /// The key that identifies the face.
    pub fn key(&self) -> u64 {
        todo!("wg.W04")
    }

    /// Paints the face into `rect`.
    pub fn paint(&self, cx: &ContentPainterCx<'_>, rect: Rect) {
        let _ = (cx, rect);
        todo!("wg.W04")
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
