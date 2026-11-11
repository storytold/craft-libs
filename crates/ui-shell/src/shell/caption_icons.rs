//! The caption glyphs of a desktop icon theme: the four `window-*-symbolic` SVGs, drawn as a coverage mask in the
//! button's ink colour.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::primitives::svg_tree;

/// The largest side a mask is rendered at, in device pixels.
const MAX_MASK_SIDE: u32 = 256;

/// A caption glyph, as icon themes name them (`window-restore-symbolic`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CaptionIcon {
    /// The minimize glyph.
    Minimize,
    /// The maximize glyph.
    Maximize,
    /// The restore glyph, shown while the window is maximized.
    Restore,
    /// The close glyph.
    Close,
}

impl CaptionIcon {
    /// Every glyph, in the order [`CaptionIcons::from_files`] takes its files.
    pub const ALL: [CaptionIcon; 4] = [CaptionIcon::Minimize, CaptionIcon::Maximize, CaptionIcon::Restore, CaptionIcon::Close];

    fn index(self) -> usize {
        Self::ALL.iter().position(|icon| *icon == self).unwrap_or_default()
    }
}

/// A coverage mask: `side` x `side` bytes, row by row.
pub type CaptionMask = Arc<[u8]>;

/// Renders a glyph as a square coverage mask at a pixel side, or `None`.
pub type RenderIconFn = dyn Fn(CaptionIcon, u32) -> Option<CaptionMask> + Send + Sync;

/// The round buttons' glyphs from the desktop's icon theme (Linux: `window-*-symbolic`), drawn in the glyph colour
/// as symbolic icons are. A glyph the theme lacks draws the family's own.
#[derive(Clone)]
pub struct CaptionIcons {
    theme: String,
    generation: u64,
    render: Arc<RenderIconFn>,
}

impl CaptionIcons {
    /// Icons of `theme` rendered by `render`. `generation` changes whenever `render` may answer differently (a
    /// reinstalled theme, a size that finished rendering); the textures made from it follow. Use 0 for icons that
    /// never change.
    pub fn new(theme: impl Into<String>, generation: u64, render: Arc<RenderIconFn>) -> Self {
        CaptionIcons { theme: theme.into(), generation, render }
    }

    /// Icons rendered from the four SVG files of a theme, in [`CaptionIcon::ALL`] order; `None` where the theme lacks
    /// one. The masks are kept per glyph and size.
    pub fn from_files(theme: impl Into<String>, generation: u64, files: [Option<Arc<[u8]>>; 4]) -> Self {
        type Memo = Mutex<HashMap<(CaptionIcon, u32), Option<CaptionMask>>>;
        let memo: Memo = Mutex::default();
        let render = move |icon: CaptionIcon, px: u32| {
            let mut memo = memo.lock().unwrap_or_else(PoisonError::into_inner);
            memo.entry((icon, px)).or_insert_with(|| files[icon.index()].as_deref().and_then(|svg| rasterize_mask(svg, px))).clone()
        };
        CaptionIcons::new(theme, generation, Arc::new(render))
    }

    /// The theme's name.
    pub fn theme(&self) -> &str {
        &self.theme
    }

    /// Changes whenever [`mask`](Self::mask) may answer differently.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The coverage mask of `icon` at `px` device pixels, or `None` while the theme lacks it or it does not render.
    pub fn mask(&self, icon: CaptionIcon, px: u32) -> Option<CaptionMask> {
        (self.render)(icon, px)
    }
}

impl PartialEq for CaptionIcons {
    fn eq(&self, other: &Self) -> bool {
        self.theme == other.theme && self.generation == other.generation && Arc::ptr_eq(&self.render, &other.render)
    }
}

impl std::fmt::Debug for CaptionIcons {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptionIcons").field("theme", &self.theme).field("generation", &self.generation).finish_non_exhaustive()
    }
}

/// An SVG's coverage (alpha) as a `px` x `px` mask, the drawing fitted and centred. `None` for an unreadable SVG or
/// an absurd size.
fn rasterize_mask(svg: &[u8], px: u32) -> Option<CaptionMask> {
    use resvg::tiny_skia;
    if !(1..=MAX_MASK_SIDE).contains(&px) {
        return None;
    }
    let tree = svg_tree::parse(svg).ok()?;
    let size = tree.size();
    let side = size.width().max(size.height());
    if !side.is_finite() || side <= 0.0 {
        return None;
    }
    let scale = px as f32 / side;
    let (dx, dy) = ((px as f32 - size.width() * scale) / 2.0, (px as f32 - size.height() * scale) / 2.0);
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy), &mut pixmap.as_mut());
    Some(pixmap.data().as_chunks::<4>().0.iter().map(|[_, _, _, a]| *a).collect())
}
