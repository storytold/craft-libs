//! The draw tools' widgets, for egui.
//!
//! - [`gradient`]: the gradient palette ([`GradientPalette`]) and swatch, the on-canvas
//!   [`GradientOverlay`] with the geometry of its line ([`gradient::gradient_line`]), the
//!   [`GradientMapEditor`] and the [`GradientFillEditor`]. The stop bar and the stop rules are
//!   craft-color-ui's (`craft_color_ui::ui::gradient`).
//! - [`pen`]: the `PenDraft` of the Pen tool and the [`BezierOverlay`] that edits paths, with the
//!   path model they share ([`EditPath`]), the anchor tools' [`AnchorTool`], and the [`PathOptionsPopover`].
//! - [`stroke`]: the [`StrokeEditor`], the [`StrokeButton`] that opens it and the [`StrokePreview`].
//! - [`corner`]: the [`CornerWidgets`] for corner radii.
//! - [`crop`]: the [`PerspectiveCropQuad`], the [`CropTool`] with its [`CropSession`], and the [`CropBar`].
//! - [`transform`]: the [`TransformBox`] of Free Transform, the options bar's [`TransformFields`],
//!   the [`TransformSession`] with its [`TransformOverlay`] and the [`TransformBar`].
//! - [`measure`]: the [`RulerTool`] with its [`RulerBar`], and [`box_distances`] between boxes.
//! - [`symmetry`]: the [`Symmetry`] guides with their [`SymmetryOverlay`], [`SymmetryMenu`] and
//!   the [`SymmetryTransform`] that mirrors a stroke.
//! - [`shape`] and [`shape_drag`]: the shape layer's [`ShapePanel`] and [`ShapeBar`]; the
//!   [`ShapeDragTool`] and its [`CreateShapeDialog`].
//! - [`mod@slice`]: the [`SliceTool`], its [`SliceOverlay`], [`SliceBar`] and dialogs.
//! - [`artboard`] and [`add_beside`]: the [`ArtboardTool`], its [`ArtboardBar`] and
//!   [`ArtboardProperties`]; the add-beside buttons ([`AddBesideHandles`], a canvas control).
//! - [`connector`]: the [`ConnectorTool`], its `ConnectorOverlay` and [`ConnectorBar`].
//! - [`pattern`] and [`picture_fill`]: the [`PatternFillEditor`] and [`PictureFillEditor`].
//! - [`brush_presets`] and [`brush_settings`]: the [`BrushPresetPicker`], `BrushPresetChip` and
//!   [`BrushSettingsPanel`].
//! - [`paint_bar`]: the [`PaintBar`] members of the Brush, Pencil, Eraser and Mixer Brush tools.
//! - [`paths`]: the [`PathsPanel`].
//! - [`shape_presets`]: the [`ShapeMenu`] of shape presets.
//!
//! The main types of each module are also re-exported here.

pub mod add_beside;
pub mod artboard;
pub mod brush_presets;
pub mod brush_settings;
pub mod connector;
pub mod corner;
pub mod crop;
pub mod gradient;
pub mod measure;
pub mod paint_bar;
pub mod paths;
pub mod pattern;
pub mod pen;
pub mod picture_fill;
pub mod shape;
pub mod shape_drag;
pub mod shape_presets;
pub mod slice;
pub mod stroke;
pub mod symmetry;
pub mod transform;

pub use add_beside::{AddBesideHandles, AddBesideOutput, AddBesideWords, AddSide, add_beside_places};
pub use artboard::ArtboardBarTarget;
pub use artboard::{
    ArtboardBackground, ArtboardBar, ArtboardBarOutput, ArtboardEvent, ArtboardItem, ArtboardPreset, ArtboardProperties, ArtboardPropertiesOutput,
    ArtboardRules, ArtboardTool,
};
pub use brush_presets::{BrushPick, BrushPickerSettings, BrushPickerState, BrushPickerValues, BrushPresetPicker, BrushPresetPickerOutput};
pub use brush_presets::{BrushesPanel, BrushesPanelOutput};
pub use brush_settings::{
    BrushPanelAction, BrushSection, BrushSectionEvent, BrushSectionKind, BrushSectionList, BrushSettingsPanel, BrushSettingsPanelOutput,
    BrushSizeField, BrushStrokeStrip, DynamicControl, TipEllipse,
};
pub use connector::{
    ConnectionSite, ConnectionTarget, ConnectorBar, ConnectorBarOutput, ConnectorDraft, ConnectorEnd, ConnectorEndKind, ConnectorEvent,
    ConnectorItem, ConnectorLine, ConnectorTool,
};
pub use corner::{CornerEdit, CornerKind, CornerRadii, CornerStyle, CornerWidgets};
pub use crop::{
    CropBar, CropBarOutput, CropBarWords, CropCommit, CropDocument, CropEvent, CropFill, CropInput, CropLength, CropOptionsMenu, CropOutput,
    CropOverlayMenu, CropPresetChoice, CropRatio, CropRatioChange, CropSession, CropSettings, CropShown, CropSizePreset, CropTarget, CropTool,
    CropToolPass, CropUserPreset, CropWords, PerspectiveCropOutput, PerspectiveCropTool, ResolutionUnit,
};
pub use crop::{
    CropPresetList, CropPresetListOutput, CropSizeTarget, PerspectiveCropBar, PerspectiveCropBarOutput, PerspectiveCropPass, PerspectiveInput,
};
pub use crop::{PerspectiveCropQuad, PerspectiveQuad};
pub use gradient::{
    FocusOutline, GradientGroup, GradientItem, GradientLine, GradientMap, GradientMapEditor, GradientMapOutput, GradientOverlay, GradientPalette,
    GradientRef, GradientSwatch, GradientTile, LineGradient, LinePart, RampSampler,
};
pub use gradient::{GradientFill, GradientFillEditor, GradientFillEditorOutput, GradientFillWords, GradientStyle};
pub use measure::{
    MIN_LINE, RulerBar, RulerBarOutput, RulerLine, RulerOverlay, RulerPass, RulerTool, RulerWords, box_distances, ruler_line_form, show_ruler,
};
pub use paint_bar::{
    EraserMode, MIXER_COMBINATIONS, MIXER_GROUP_STARTS, MixerLoad, MixerValues, PaintBar, PaintBarOutput, PaintBarWords, PaintTool, PaintValues,
    Smoothing,
};
pub use paths::{PathKind, PathRow, PathsCommands, PathsEvent, PathsFooterButton, PathsOutput, PathsPanel, PathsWords};
pub use pattern::{PatternColors, PatternFillDialog, PatternFillEditor, PatternFillOptions, PatternFillOutput, PatternSwatch, PatternWords};
pub use pen::{AnchorDelete, AnchorTool, PenEnd};
pub use pen::{
    AnchorsShown, HandleStyle, HitMark, PATH_OPTIONS_COLOR_KEY, PathMode, PathOptionsPopover, PathOptionsSettings, PathTool, PenMode, SegmentMark,
};
pub use pen::{
    BezierHit, BezierOverlay, BezierStyle, BezierTool, EditPath, EditSubpath, HandleSide, HandlesShown, Knot, KnotRef, MAX_ANCHORS, PathEdit,
    PathSelection, PenStyle,
};
pub use picture_fill::{PictureFill, PictureFillEditor, PictureFillOutput, PictureMode};
pub use shape::{
    Arrowheads, ArrowheadsOutput, ArrowheadsPopover, LINE_WEIGHT_RANGE, RADIUS_RANGE, StrokeCells, StrokeCellsOutput, VectorMode, VectorModeSelect,
};
pub use shape::{BoxConstraintsOutput, BoxConstraintsPopover};
pub use shape::{
    ChipSetup, LiveShape, ShapeAppearance, ShapeBar, ShapeBarOutput, ShapeChange, ShapePaint, ShapePanel, ShapePanelOutput, ShapeToolSettings,
    ShapeWords,
};
pub use shape_drag::{
    BoxConstraint, BoxConstraints, CreateShapeDialog, CreateShapeOptions, CreatedShape, ShapeDragEvent, ShapeDragOptions, ShapeDragTool, ShapeKind,
};
pub use shape_presets::{ShapeMenu, ShapePresetWords};
pub use slice::{
    DivideBy, DivideSlices, DivideSlicesDialog, SliceBackground, SliceBar, SliceBarOutput, SliceCellAlign, SliceCellVAlign, SliceCommand, SliceEvent,
    SliceItem, SliceKind, SliceMode, SliceOptions, SliceOptionsDialog, SliceOverlay, SliceTool, SliceToolSettings,
};
pub use stroke::{DashAlign, DashUnit, LineCap, LineJoin, LineStyle, StrokeAlign, StrokeButton, StrokeEditor, StrokePreview, StrokeStyle};
pub use symmetry::{Symmetry, SymmetryChoice, SymmetryCommand, SymmetryEvent, SymmetryMenu, SymmetryMode, SymmetryOverlay, SymmetryTransform};
pub use transform::{
    ApplyBy, FieldArrangement, RotateZone, SizeUnit, TransformBox, TransformField, TransformFields, TransformGroups, TransformRow, TransformSource,
};
pub use transform::{
    Interpolation, TransformBar, TransformBarOutput, TransformChoice, TransformEdit, TransformOverlay, TransformPhase, TransformPreset,
    TransformRequest, TransformResult, TransformSession, TransformSettings, TransformWarp,
};
