//! Pure canvas geometry in document units: the transform box ([`quad`]), snapping ([`snap`]), the
//! outline being drawn by a lasso ([`draft`]), how a selection combines with the current one
//! ([`selection_op`]), the marquee drag rules ([`marquee`]), direction locking ([`lock`]), the move gesture and stroke lock ([`move_gesture`]),
//! the crop frame ([`crop`]), the area outside a crop frame as convex pieces ([`outside`]) and
//! plane helpers ([`geom`]). Nothing here reads a `Ui`; modifier keys arrive as
//! [`craft_keymap::Mods`].

pub mod crop;
pub mod draft;
pub mod geom;
pub mod lock;
pub mod marquee;
pub mod move_gesture;
pub mod outside;
pub mod quad;
pub mod selection_op;
pub mod snap;

pub use crop::{AspectHold, CropBox, CropGuide, GuideShow, crop_view, crop_view_about, hit_rect, screen_step, turn_grid};
pub use draft::{DraftEvent, DraftKind, DraftPoint, DraftReach, OutlineDraft};
pub use lock::{
    AXIS_BAND_DEGREES, Directions, LOCK_HYSTERESIS_DEGREES, axis_or_diagonal, lock_direction, polygon_step, round_to_direction, turn_to_45,
};
pub use marquee::{Constraint, MarqueeGesture, MarqueeRect, fixed_size_pixels, marquee_pixels, square_pixels};
pub use move_gesture::{ConstrainStyle, MoveGesture, MoveValue, StrokeLock};
pub use outside::outside_pieces;
pub use quad::{BoxAllow, BoxDrag, BoxHandles, BoxPart, BoxRules, BoxState, Homography, Parts, SKEW_LIMIT, ScaleRule, TransformMode, drag};
pub use selection_op::SelectionOperation;
pub use snap::{
    SNAP_PX, SnapGesture, SnapKind, SnapKinds, SnapLine, SnapOptions, SnapSuspend, SnapTarget, SnapTargets, across_axis, coordinate_across,
    motion_across,
};
