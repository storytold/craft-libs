//! [`PenTool`]: the path being drawn with the Pen and the Curvature Pen, as PhotoCraft's tools draw it;
//! [`PathMode`]: which path overlay a tool and the held keys run; the pen's Path Options
//! ([`PathOptionsSettings`], [`PathOptionsPopover`]).

mod auto;
pub(crate) mod bezier;
mod curvature_mode;
mod mode;
mod options;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;
mod tool;
mod words;

pub use bezier::{
    AnchorsShown, BezierHit, BezierOutput, BezierOverlay, BezierStyle, BezierTool, BezierWords, EditPath, EditSubpath, HandleSide, HandlesShown,
    HitMark, Knot, KnotRef, MAX_ANCHORS, PathEdit, PathSelection, SegmentMark,
};
pub use mode::{PathMode, PathTool, PenMode};
pub use options::{PATH_OPTIONS_COLOR_KEY, PathOptionsOutput, PathOptionsPopover, PathOptionsSettings, PathOptionsWords};
pub use tool::{PenEvent, PenStyle, PenTool};
pub use words::PenWords;
