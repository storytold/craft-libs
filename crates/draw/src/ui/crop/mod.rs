//! The Crop tool: the tool on the canvas tool host ([`CropTool`]) with its pending frame ([`CropSession`]) and options
//! ([`CropSettings`]), the options bar ([`CropBar`]) with its overlay and gear menus, and the Perspective Crop tool
//! ([`PerspectiveCropTool`]) with its quad ([`PerspectiveCropQuad`]). The frame itself is `craft_ui::canvas::CropOverlay`,
//! shown by [`CropToolPass::show_frame`].
//!
//! The tool reports events and never crops the document: the app crops on [`CropEvent::Apply`] with the
//! [`CropCommit`] it carries.

mod bar;
mod bar_words;
mod menus;
mod options;
mod perspective_crop;
mod perspective_tool;
mod session;
mod tool;
mod words;

pub use bar::{CropBar, CropBarOutput, CropRatioChange};
pub use bar_words::CropBarWords;
pub use menus::{CropOptionsMenu, CropOptionsMenuOutput, CropOverlayMenu};
pub use options::{
    CROP_RATIOS, CROP_SIZES, CropFill, CropLength, CropPresetChoice, CropRatio, CropSettings, CropSizeFields, CropSizePreset, CropTarget,
    CropUserPreset, FIELD_MAX, LENGTH_MIN, RATIO_DECIMALS, RATIO_MAX, RATIO_STEP, ResolutionUnit,
};
pub use perspective_crop::{
    PerspectiveCropOutput, PerspectiveCropQuad, PerspectiveCropStyle, PerspectiveQuad, grid_lines, grid_steps, is_valid, ordered,
};
pub use perspective_tool::{PerspectiveCropEvent, PerspectiveCropTool, perspective_to_form};
pub use session::{CropDocKey, CropDocument, CropKept, CropSession, CropStep, LeaveFrame, StraightenState};
pub use tool::{CropCommit, CropEvent, CropInput, CropOutput, CropTool, CropToolPass, crop_to_form};
pub use words::CropWords;
