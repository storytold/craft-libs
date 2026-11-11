//! Painters, `Draw`, the sensing core, the text adapter and `ImeSurface`.

mod content;
mod draw;
mod faces;
mod focus_ring;
#[doc(hidden)]
pub mod glyph;
mod handle;
mod icon;
mod ime_surface;
mod kit;
mod record;
mod reset_mark;
mod rule;
mod scroll_bar;
pub(crate) mod sensing;
mod state_layer;
mod surface;
mod svg_set;
#[doc(hidden)]
pub mod svg_tree;
mod text;
pub(crate) mod text_adapter;
mod text_input;
pub mod textures;
mod tool;

pub use content::{ContentPainterCx, DocRun, DocText};
pub use draw::{Dashes, DataStrokeWidth, DataWidth, Draw, PaintShape, StrokeColor, StrokeWidth};
pub use faces::FacesCx;
pub use focus_ring::{FocusKind, FocusRing};
pub use handle::{Handle, HandleShape};
pub use icon::{Glyph, IconSet, IconSource, IconSpec, Mirror, has_icon, icon_names};
pub use ime_surface::ImeSurface;
pub use kit::{
    Checker, GridAxis, GridInk, GridLines, GridLook, GridStroke, MAX_LINES, Tick, TickEdge, TickRank, checker, dashed, drop_mark, ellipse,
    grid_lines, grid_lines_with, marching_ants, outline, ramp, ticks,
};
pub use reset_mark::ResetMark;
pub use rule::{Rule, RuleKind};
pub use scroll_bar::{BarAxis, HiddenContent, ScrollBar};
pub use sensing::{PartResult, TextOutcome};
pub use state_layer::{StateLayer, StateLayerKind};
pub use surface::Surface;
pub use text::{ChordText, HAlign, LayoutRun, ReadingMeasure, Span, TextLayout, TextRun, Wrap};
pub use text_input::{Completer, Completion, EditorLine, EditorPurpose, EditorRun, EditorState, TextKey, TextOptions};
pub use textures::TextureCache;
pub use tool::ToolIcon;

#[cfg(feature = "testing")]
pub use textures::FakeThumbSource;
pub use textures::{
    CachedTexture, LoadFailure, LoadResult, LoadedPicture, LoaderOptions, MAX_TEXTURE, Priority, Revision, TextureBudget, TextureKey, TextureLoader,
    TextureState, TextureStatistics, TextureStore, TextureStoreOptions, TextureWords, ThumbFit, ThumbJob, ThumbPaint, ThumbRender, ThumbRequest,
    ThumbSource, ThumbnailOptions, Thumbnails, fit_image, fit_rect, reduce_image, texture_factor, texture_side, thumb_image,
};

#[doc(hidden)]
pub use draw::DrawEnv;
#[doc(hidden)]
pub use record::PaintRecord;
pub(crate) use record::{Adopted, FaceRecord, face_id};
pub(crate) use text::TextEnv;
