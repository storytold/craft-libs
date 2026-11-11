//! The raster image editing widgets, for egui.
//!
//! - [`warp`]: the [`WarpGrid`] on a canvas, the mesh of a mesh warp, its anchors and handles
//!   dragged and nudged; the pin warp ([`PinWarp`]) and perspective warp ([`PerspectiveWarp`]) with
//!   their bars, and the [`WarpFields`] of the options bar.
//! - [`transform_preview`]: the [`TransformPreview`] of a transform or warp in progress.
//! - [`curve`]: the [`CurveEditor`] and [`CurvePreview`]; [`histogram`]: the [`Histogram`] plot.
//! - [`adjust`]: the [`LevelsEditor`] and the editors of the other colour adjustments (curves,
//!   hue and saturation, color balance, channel mixer, selective color and more).
//! - [`live_preview`]: the [`LivePreview`] of an adjustment on a proxy of the image.
//! - [`layer_style`]: the [`LayerStyleDialog`], the [`BlendIfBar`] and the [`ContourField`].
//! - [`blend_keys`]: the [`BlendKeys`] that pick a blend mode from the keyboard.
//! - [`select`]: the [`MarqueeTool`] and the lasso tools; [`selection`]: the [`SelectionOverlay`];
//!   [`mask_view`]: the [`MaskViewControls`] for a mask's views.
//! - [`select_mask`], [`fill`], [`filter_gallery`], [`liquify`], [`wide_angle`] and
//!   [`vanishing_point`]: the full-window editors ([`SelectMaskWorkspace`],
//!   [`ContentAwareFillWorkspace`], [`FilterGalleryWorkspace`], [`LiquifyWorkspace`],
//!   [`WideAngleWorkspace`], [`VanishingPointWorkspace`]), each with its session, panel and options bar.
//! - [`blur_pins`]: the [`BlurPinsOverlay`] and [`BlurPinsPanel`] of the blur effects.
//! - [`channels`]: the Channels panel.
//! - [`scopes`]: the scope panel (histogram, statistics, tone zones, vectorscope) and the
//!   clipping overlay.
//!
//! The main types of each module are also re-exported here.

pub mod adjust;
pub mod blend_keys;
pub mod blur_pins;
pub mod channels;
pub mod curve;
pub mod fill;
pub mod filter_gallery;
pub mod histogram;
pub mod layer_style;
pub mod liquify;
pub mod live_preview;
pub mod mask_view;
#[cfg(feature = "screen-testing")]
pub(crate) mod mock_image;
pub mod scopes;
pub mod select;
pub mod select_mask;
pub mod selection;
pub mod transform_preview;
pub mod vanishing_point;
pub mod warp;
pub mod wide_angle;

pub use adjust::{
    AdjustContext, AdjustField, AdjustSession, AdjustSliders, AdjustTrack, AdjustWords, BlackWhite, BlackWhiteEditor, BrightnessContrast,
    BrightnessContrastEditor, ChannelMixer, ChannelMixerEditor, ColorBalance, ColorBalanceEditor, CurvesMode, CurvesPanel, CurvesPanelOutput,
    HueSaturation, HueSaturationEditor, Levels, LevelsClip, LevelsEditor, LevelsEditorOutput, LevelsPanel, LevelsPanelOutput, LevelsWords,
    PhotoFilter, PhotoFilterEditor, SelectiveColor, SelectiveColorEditor, ThresholdEditor, ToneCurves, TonePick, TonePickTool, TonePicker,
    TonePickerRow, TonePickerRowOutput,
};
pub use blend_keys::{BlendKeyEntry, BlendKeyPick, BlendKeys, BlendShortcut, BlendStep};
pub use blur_pins::{
    BlurKind, BlurPin, BlurPins, BlurPinsCommand, BlurPinsOutput, BlurPinsOverlay, BlurPinsPanel, BlurPinsPanelOutput, BlurPinsWords, IrisShape,
    TiltBand,
};
pub use channels::{
    ChannelKind, ChannelRow, ChannelsCommands, ChannelsEvent, ChannelsFooterButton, ChannelsOutput, ChannelsPanel, ChannelsWords, ColorIndicates,
    OVERLAY_OPACITIES, OverlayPreset, targeted_row,
};
pub use curve::{Curve, CurveChannel, CurveEditor, CurveEditorOutput, CurvePreview, CurveRows, CurveRules, CurveSampler, CurveWords, ToneRamp};
pub use fill::{
    ContentAwareFillWorkspace, FillBar, FillBarOutput, FillEvent, FillLimit, FillLimits, FillOpen, FillOutput, FillOverlay, FillPanel,
    FillPanelOutput, FillSession, FillSettings, FillSource, FillTool, FillWords,
};
pub use filter_gallery::{
    EffectParameters, EffectStack, FilterCatalog, FilterGalleryEvent, FilterGalleryOpen, FilterGalleryOutput, FilterGallerySession,
    FilterGalleryWords, FilterGalleryWorkspace, FilterTree, GalleryFilter,
};
pub use histogram::{
    BinCount, ChannelColor, Histogram, HistogramData, HistogramInks, HistogramLook, HistogramOutput, HistogramScale, LevelRange, PlotFrame,
    PlotHeight, StaleMark,
};
pub use layer_style::{
    BlendIfBar, BlendIfWords, ContourField, ContourFieldOutput, ContourWords, LayerStyleDialog, LayerStyleOutput, LayerStyleWords, StylePresets,
};
pub use liquify::{
    LiquifyCommands, LiquifyEvent, LiquifyOpen, LiquifyOutput, LiquifyPanel, LiquifyPanelOutput, LiquifySession, LiquifySettings, LiquifySource,
    LiquifyTiming, LiquifyTool, LiquifyWords, LiquifyWorkspace,
};
pub use live_preview::{DraftScale, LivePreview, PreviewKey, PreviewLane, PreviewSession, ProxyPlan};
pub use mask_view::{MaskViewControls, MaskViewControlsOutput, MaskViewKeys, MaskViewMode, MaskViewRecipe, MaskViewState, MaskViewWords};
pub use scopes::{
    COLORS_CHANNEL, ChannelOverlay, ClipWarning, ClippingDiagnostic, ClippingFrame, ClippingMode, ClippingOverlay, ClippingWarnings, HistogramSet,
    ScopeChannel, ScopeCommand, ScopeFrame, ScopeMenuAction, ScopePanel, ScopePanelOutput, ScopePanelState, ScopeReadout, ScopeSettings, ScopeSource,
    ScopeView, ScopeWords, ScopeZoneEdit, ToneZone, ToneZones, VectorscopeFrame, VectorscopePlot, ZoneChange, scope_point,
};
pub use select::{
    LassoTool, MagneticLassoTool, MagneticSettings, MarqueeKind, MarqueeTool, PolygonLassoTool, SelectEvent, SelectWords, SelectionShape,
};
pub use select_mask::{
    LimitRefused, SelectMaskBar, SelectMaskBarOutput, SelectMaskEdit, SelectMaskEvent, SelectMaskLimits, SelectMaskOpen, SelectMaskOutput,
    SelectMaskPanel, SelectMaskPanelOutput, SelectMaskSession, SelectMaskSettings, SelectMaskSource, SelectMaskWords, SelectMaskWorkspace,
};
pub use selection::{OutlineSource, SelectionOverlay, SelectionOverlayOutput, SelectionWords};
pub use transform_preview::{PreviewMap, PreviewQuality, StyledPreview, TransformPreview};
pub use vanishing_point::{
    StampLimits, StampSettings, VanishingPlane, VanishingPointBar, VanishingPointBarOutput, VanishingPointEditor, VanishingPointEvent,
    VanishingPointOpen, VanishingPointOutput, VanishingPointSession, VanishingPointSettings, VanishingPointSource, VanishingPointWords,
    VanishingPointWorkspace, VanishingScene,
};
pub use warp::perspective::{
    PerspectiveBar, PerspectiveBarOutput, PerspectiveBarWords, PerspectiveCommands, PerspectiveEdit, PerspectiveSession, PerspectiveWarp,
    PerspectiveWarpOutput, PerspectiveWarpWords,
};
pub use warp::pin::{
    Pin, PinWarp, PinWarpBar, PinWarpBarOutput, PinWarpBarWords, PinWarpCommands, PinWarpOutput, PinWarpSession, PinWarpSettings, PinWarpWords,
};
pub use warp::{
    WarpEdit, WarpFieldValues, WarpFields, WarpFieldsEdit, WarpFieldsOutput, WarpFieldsWords, WarpGrid, WarpGridChoice, WarpGridOutput,
    WarpGridStyle, WarpMesh, WarpSide, WarpSplit, WarpStyleEntry, WarpWords,
};
pub use wide_angle::{
    PreviewDone, WideAngleConstraint, WideAngleEvent, WideAngleModel, WideAngleOpen, WideAngleOutput, WideAnglePanel, WideAnglePanelOutput,
    WideAngleSession, WideAngleSource, WideAngleTool, WideAngleWords, WideAngleWorkspace,
};
