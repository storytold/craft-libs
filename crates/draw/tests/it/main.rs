//! The screen tests of craft-draw, folded into the lib's test target.

// PLAN: the sheet (writer note 7) registers `corner`, `shape_cells`, `gradient_fill`, `paths_panel`, `crop_size`, `artboard`, `slice` and `presets`, but their files belong to other sets (tools-w8, w10, w11, w14, w18, w19, framework-01-w11) and do not exist in this tree, so the lines would not compile. Each owning set adds its own `mod` line under the group comment below with its file.

// draw-shape
mod add_beside;

// draw-gradient-b
mod gradient_overlay;

// draw-connector
mod connector;
mod shape_drag;

// draw-pen-a
mod bezier;

// draw-pen-b
mod pen;

// draw-crop-b
mod crop_tool;
mod perspective_crop;

// draw-bars
mod measure;
mod slice_divide;

// draw-stroke
mod stroke_button;
mod stroke_editor;

// draw-brush-presets
mod brush_presets;
mod brush_size;
mod paint_bar;

// draw-transform-a
mod transform_box;
mod transform_session;

// draw-transform-b
mod symmetry;
mod transform_fields;
