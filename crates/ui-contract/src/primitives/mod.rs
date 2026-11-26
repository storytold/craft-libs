//! Painters, `Draw`, the sensing core, the text adapter and `ImeSurface`.

mod auto_scroll_mark;
pub mod backdrop;
mod content;
mod draw;
mod face;
mod faces;
mod focus_ring;
#[doc(hidden)]
pub mod glyph;
mod handle;
mod host;
mod icon;
mod ime_surface;
mod kit;
mod marker;
mod paint_slot;
pub(crate) mod pointer_art;
mod record;
mod reset_mark;
mod rule;
mod scroll_bar;
pub(crate) mod sensing;
mod state_layer;
mod surface;
mod surround;
mod svg_set;
#[doc(hidden)]
pub mod svg_tree;
pub(crate) mod text;
pub(crate) mod text_adapter;
mod text_input;
pub mod textures;
mod tool;

pub use auto_scroll_mark::AutoScrollMark;
pub use content::{ContentPainterCx, DocRun, DocText, FaceRun};
pub use draw::{Dashes, DataStrokeWidth, DataWidth, Draw, PaintShape, StrokeColor, StrokePlace, StrokeWidth, WellFill};
pub use face::FacePaint;
pub use faces::FacesCx;
pub use focus_ring::{FocusKind, FocusRing};
pub use handle::{Handle, HandleShape};
pub use host::HostStyle;
pub(crate) use icon::SvgIcon;
pub use icon::{Glyph, IconMark, IconSet, IconSource, IconSpec, Mirror, has_icon, icon_names, paint_icon};
pub use ime_surface::ImeSurface;
pub use kit::{
    Checker, CheckerArea, GridAxis, GridCrowding, GridInk, GridLines, GridLook, GridStroke, MAX_LINES, RulerFace, Tick, TickEdge, TickRank, dashed,
    drop_mark, ellipse, fill_polygon, grid_lines, grid_lines_with, marching_ants, outline, outline_with, ramp, swath, ticks,
};
pub use marker::{Marker, MarkerSize};
pub use paint_slot::PaintSlot;
pub use pointer_art::painted_image;
pub use reset_mark::ResetMark;
pub use rule::{RectSide, Rule, RuleKind};
pub use scroll_bar::{HiddenContent, ScrollBar};
pub use sensing::{PartResult, TextOutcome, cancel_press_of, member_gesture_running};
pub use state_layer::{StateLayer, StateLayerKind};
pub(crate) use surface::{Frames as SurfaceFrames, SurfaceSlot, fit_to_frame};
pub use surface::{Surface, frame_surfaces};
pub use surround::paint_surround;
#[cfg(feature = "testing")]
pub(crate) use svg_set::Layer;
pub use svg_set::{AppIcons, register_app_icons};
pub use text::{ChordText, HAlign, LayoutRun, LeastLayout, ReadingMeasure, Span, TextLayout, TextRun, Wrap};
pub use text_input::{
    Completer, Completion, EditorLine, EditorPurpose, EditorRun, EditorState, EditorSurrounding, ImeStamp, SubmitKey, TabKey, TextKey, TextOptions,
};
pub use textures::TextureCache;
pub use tool::ToolIcon;

#[cfg(feature = "testing")]
pub use textures::FakeThumbSource;
pub use textures::{
    CachedTexture, LoadFailure, LoadResult, LoadedPicture, LoaderOptions, MAX_TEXTURE, Priority, Revision, Serials, StateLook, TextureBudget,
    TextureKey, TextureLoader, TextureState, TextureStatistics, TextureStore, TextureStoreOptions, TextureWords, ThumbFit, ThumbJob, ThumbPaint,
    ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails, fit_image, fit_rect, reduce_image, texture_factor, texture_side,
    thumb_image,
};

pub(crate) use draw::DataMode;
#[doc(hidden)]
pub use draw::DrawEnv;
pub(crate) use draw::surface_look_shapes;
#[doc(hidden)]
pub use record::PaintRecord;
pub(crate) use record::{Adopted, FaceRecord, face_id};
pub(crate) use text::TextEnv;
