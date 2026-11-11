//! Pure canvas geometry in document units: the transform box ([`quad`]), snapping ([`snap`]), the
//! outline being drawn by a lasso ([`draft`]), how a selection combines with the current one
//! ([`selection_op`]), the marquee drag rules ([`marquee`]), direction locking ([`lock`]), the move gesture and stroke lock ([`move_gesture`]) and
//! plane helpers ([`geom`]). Nothing here reads a `Ui`; modifier keys arrive as
//! [`craft_keymap::Mods`].

pub mod crop;
pub mod draft;
pub mod geom;
pub mod lock;
pub mod marquee;
pub mod move_gesture;
pub mod quad;
pub mod selection_op;
pub mod snap;

pub use crop::{CropBox, CropGuide, GuideShow, crop_view, crop_view_about, hit_rect, screen_step};
pub use draft::{DraftEvent, DraftKind, DraftPoint, DraftReach, OutlineDraft};
pub use lock::{Directions, LOCK_HYSTERESIS_DEGREES, lock_direction, round_to_direction};
pub use marquee::{Constraint, MarqueeGesture, MarqueeRect, fixed_size_pixels, marquee_pixels, square_pixels};
pub use move_gesture::{ConstrainStyle, MoveGesture, MoveValue, StrokeLock};
pub use quad::{BoxAllow, BoxDrag, BoxPart, BoxRules, BoxState, Homography, Parts, SKEW_LIMIT, ScaleRule, TransformMode, drag};
pub use selection_op::SelectionOperation;
pub use snap::{SNAP_PX, SnapGesture, SnapKind, SnapLine, SnapOptions, SnapTarget, SnapTargets, across_axis, coordinate_across, motion_across};
