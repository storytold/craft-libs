//! The screen tests of craft-draw, folded into the lib's test target.

// draw-shape
pub(crate) mod add_beside;
pub(crate) mod corner;
pub(crate) mod presses;
pub(crate) mod shape_cells;

// draw-gradient-b
pub(crate) mod gradient_fill;
pub(crate) mod gradient_map;
pub(crate) mod gradient_overlay;

// draw-connector
pub(crate) mod connector;
pub(crate) mod shape_drag;

// draw-paths
pub(crate) mod paths_panel;

// draw-pen-a
pub(crate) mod bezier;

// draw-pen-b
pub(crate) mod pen;

// draw-crop-b
pub(crate) mod crop_size;
pub(crate) mod crop_tool;
pub(crate) mod perspective_crop;

// draw-artboard
pub(crate) mod artboard;

// draw-bars
pub(crate) mod measure;
pub(crate) mod slice;

// draw-stroke
pub(crate) mod stroke_button;
pub(crate) mod stroke_editor;

// draw-brush-presets
pub(crate) mod brush_presets;
pub(crate) mod brush_size;
pub(crate) mod paint_bar;
pub(crate) mod tip_ellipse;

// draw-transform-a
pub(crate) mod transform_box;
pub(crate) mod transform_session;

// draw-transform-b
pub(crate) mod symmetry;
pub(crate) mod transform_fields;

// draw-presets
pub(crate) mod presets;

// dialogs-w13
pub(crate) mod pattern_fill;
