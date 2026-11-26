//! [`PenTool`]: the path being drawn with the Pen and the Curvature Pen, as PhotoCraft's tools draw it;
//! [`AnchorTool`]: the canvas tool of the anchor tools and Direct Selection;
//! [`PathMode`]: which path overlay a tool and the held keys run; the pen's Path Options
//! ([`PathOptionsSettings`], [`PathOptionsPopover`]).

mod anchor_tool;
mod auto;
pub(crate) mod bezier;
mod curvature_mode;
mod mode;
pub(crate) mod options;
#[cfg(feature = "suite")]
pub(crate) mod stories;
mod tool;
mod words;

pub use anchor_tool::AnchorTool;
pub use bezier::{
    AnchorDelete, AnchorsShown, BezierHit, BezierOutput, BezierOverlay, BezierStyle, BezierTool, BezierWords, EditPath, EditSubpath, HandleSide,
    HandlesShown, HitMark, Knot, KnotRef, MAX_ANCHORS, PathEdit, PathSelection, SegmentMark,
};
pub use mode::{PathMode, PathTool, PenMode};
pub use options::{HandleStyle, PATH_OPTIONS_COLOR_KEY, PathOptionsOutput, PathOptionsPopover, PathOptionsSettings, PathOptionsWords};
pub use tool::{PenDraft, PenEnd, PenEvent, PenStyle, PenTool};
pub use words::PenWords;
