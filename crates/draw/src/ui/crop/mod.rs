//! The Crop tool: the tool on the canvas tool host ([`CropTool`]) with its pending frame ([`CropSession`]) and options
//! ([`CropSettings`]), the options bar ([`CropBar`]) with its preset list ([`CropPresetList`]), overlay and gear menus,
//! and the Perspective Crop tool ([`PerspectiveCropTool`], run each pass as [`PerspectiveCropPass`] over a
//! [`PerspectiveInput`]) with its quad ([`PerspectiveCropQuad`]) and options bar ([`PerspectiveCropBar`]). Both bars
//! share the W × H × Resolution cells over [`CropSizeFields`], whose target is a [`CropSizeTarget`]. The frame itself
//! is `craft_ui::canvas::CropOverlay`, shown by [`CropTool::show_frame`].
//!
//! The tool reports events and never crops the document: the app crops on [`CropEvent::Apply`] with the
//! [`CropCommit`] it carries.

pub(crate) mod bar;
mod bar_words;
pub(crate) mod menus;
mod options;
pub(crate) mod perspective_bar;
pub(crate) mod perspective_crop;
mod perspective_tool;
pub(crate) mod preset_list;
mod session;
mod size_cells;
pub(crate) mod tool;
mod words;

pub use bar::{CropBar, CropBarOutput, CropRatioChange};
pub use bar_words::CropBarWords;
pub use menus::{CropOptionsMenu, CropOptionsMenuOutput, CropOverlayMenu};
pub use options::{
    CROP_RATIOS, CROP_SIZES, CropFill, CropLength, CropPresetChoice, CropRatio, CropSettings, CropSizeFields, CropSizePreset, CropTarget,
    CropUserPreset, FIELD_MAX, LENGTH_MIN, RATIO_DECIMALS, RATIO_MAX, RATIO_STEP, ResolutionUnit,
};
pub use perspective_bar::{PerspectiveCropBar, PerspectiveCropBarOutput};
pub use perspective_crop::{
    PerspectiveCropOutput, PerspectiveCropQuad, PerspectiveCropStyle, PerspectiveQuad, grid_lines, grid_steps, is_valid, ordered,
};
pub use perspective_tool::{PerspectiveCropEvent, PerspectiveCropPass, PerspectiveCropTool, PerspectiveInput, perspective_to_form};
pub use preset_list::{CropPresetList, CropPresetListOutput};
pub use session::{CropDocKey, CropDocument, CropKept, CropSession, CropStep, LeaveFrame, StraightenState};
pub use size_cells::CropSizeTarget;
pub use tool::{CropCommit, CropEvent, CropInput, CropOutput, CropShown, CropTool, CropToolPass, crop_to_form};
pub use words::CropWords;
