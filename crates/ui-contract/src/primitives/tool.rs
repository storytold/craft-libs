//! `ToolIcon`: the tool icons every app shares, by tool, drawn from the SVG set so a theme's icon pack restyles
//! them with the rest.
//!
//! Each tool maps to an icon named by meaning (`marquee_row`, `pen_add_anchor`); the base set has a file for
//! every one.

use craft_behavior::IconId;

/// A tool, for its icon: a raster editor's tool set, which other apps share variant for variant (a vector
/// editor's Pen is [`ToolIcon::Pen`]). A tool the set lacks takes [`ToolIcon::Custom`] with its own icon.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ToolIcon {
    /// A tool of the app's own, drawn by `icon`; it is in no id catalog.
    Custom(IconId),
    /// The `move` icon.
    Move,
    /// The `artboard` icon.
    Artboard,
    /// The `square-dashed` icon.
    RectangularMarquee,
    /// The `ellipse-dashed` icon.
    EllipticalMarquee,
    /// The `marquee-row` icon.
    SingleRowMarquee,
    /// The `marquee-column` icon.
    SingleColumnMarquee,
    /// The `brush` icon.
    Brush,
    /// The `pencil` icon.
    Pencil,
    /// The `palette` icon.
    MixerBrush,
    /// The `brush-replace` icon.
    ColorReplacement,
    /// The `eraser` icon.
    Eraser,
    /// The `eraser-background` icon.
    BackgroundEraser,
    /// The `eraser-magic` icon.
    MagicEraser,
    /// The `pipette` icon.
    Eyedropper,
    /// The `sampler` icon.
    ColorSampler,
    /// The `ruler` icon.
    Ruler,
    /// The `message-square` icon.
    Note,
    /// The `circle-dot` icon.
    Count,
    /// The `lasso` icon.
    Lasso,
    /// The `pentagon` icon.
    PolygonalLasso,
    /// The `lasso-magnetic` icon.
    MagneticLasso,
    /// The `wand-sparkles` icon.
    MagicWand,
    /// The `crop` icon.
    Crop,
    /// The `perspective-crop` icon.
    PerspectiveCrop,
    /// The `slice-knife` icon.
    Slice,
    /// The `slice-select` icon.
    SliceSelect,
    /// The `frame-placeholder` icon.
    Frame,
    /// The `gradient` icon.
    Gradient,
    /// The `paint-bucket` icon.
    PaintBucket,
    /// The `type` icon.
    HorizontalType,
    /// The `type` icon.
    VerticalType,
    /// The `type-mask` icon.
    HorizontalTypeMask,
    /// The `type-mask-vertical` icon.
    VerticalTypeMask,
    /// The `hand` icon.
    Hand,
    /// The `compass` icon.
    RotateView,
    /// The `zoom-in` icon.
    Zoom,
    /// The `bandage` icon.
    SpotHealing,
    /// The `bandage` icon.
    Healing,
    /// The `bandage-sparkle` icon.
    Remove,
    /// The `lasso-select` icon.
    Patch,
    /// The `arrow-left-right` icon.
    ContentAwareMove,
    /// The `eye` icon.
    RedEye,
    /// The `stamp` icon.
    CloneStamp,
    /// The `grid-3x3` icon.
    PatternStamp,
    /// The `clock` icon.
    HistoryBrush,
    /// The `sparkles` icon.
    ArtHistoryBrush,
    /// The `droplet` icon.
    Blur,
    /// The `triangle` icon.
    Sharpen,
    /// The `pointer` icon.
    Smudge,
    /// The `lollipop` icon.
    Dodge,
    /// The `flame` icon.
    Burn,
    /// The `cloud` icon.
    Sponge,
    /// The `quick-selection` icon.
    QuickSelection,
    /// The `square-dashed-mouse-pointer` icon.
    ObjectSelection,
    /// The `pen-tool` icon.
    Pen,
    /// The `pen-freeform` icon.
    FreeformPen,
    /// The `pen-curvature` icon.
    CurvaturePen,
    /// The `pen-add-anchor` icon.
    AddAnchorPoint,
    /// The `pen-delete-anchor` icon.
    DeleteAnchorPoint,
    /// The `convert-point` icon.
    ConvertPoint,
    /// The `mouse-pointer-2` icon.
    PathSelection,
    /// The `direct-select` icon.
    DirectSelection,
    /// The `rectangle-horizontal` icon.
    Rectangle,
    /// The `circle` icon.
    Ellipse,
    /// The `triangle` icon.
    Triangle,
    /// The `pentagon` icon.
    Polygon,
    /// The `slash` icon.
    Line,
    /// The `cloud` icon.
    CustomShape,
}

impl ToolIcon {
    /// Every tool, in this enum's order.
    pub const ALL: [ToolIcon; 68] = [
        ToolIcon::Move,
        ToolIcon::Artboard,
        ToolIcon::RectangularMarquee,
        ToolIcon::EllipticalMarquee,
        ToolIcon::SingleRowMarquee,
        ToolIcon::SingleColumnMarquee,
        ToolIcon::Brush,
        ToolIcon::Pencil,
        ToolIcon::MixerBrush,
        ToolIcon::ColorReplacement,
        ToolIcon::Eraser,
        ToolIcon::BackgroundEraser,
        ToolIcon::MagicEraser,
        ToolIcon::Eyedropper,
        ToolIcon::ColorSampler,
        ToolIcon::Ruler,
        ToolIcon::Note,
        ToolIcon::Count,
        ToolIcon::Lasso,
        ToolIcon::PolygonalLasso,
        ToolIcon::MagneticLasso,
        ToolIcon::MagicWand,
        ToolIcon::Crop,
        ToolIcon::PerspectiveCrop,
        ToolIcon::Slice,
        ToolIcon::SliceSelect,
        ToolIcon::Frame,
        ToolIcon::Gradient,
        ToolIcon::PaintBucket,
        ToolIcon::HorizontalType,
        ToolIcon::VerticalType,
        ToolIcon::HorizontalTypeMask,
        ToolIcon::VerticalTypeMask,
        ToolIcon::Hand,
        ToolIcon::RotateView,
        ToolIcon::Zoom,
        ToolIcon::SpotHealing,
        ToolIcon::Healing,
        ToolIcon::Remove,
        ToolIcon::Patch,
        ToolIcon::ContentAwareMove,
        ToolIcon::RedEye,
        ToolIcon::CloneStamp,
        ToolIcon::PatternStamp,
        ToolIcon::HistoryBrush,
        ToolIcon::ArtHistoryBrush,
        ToolIcon::Blur,
        ToolIcon::Sharpen,
        ToolIcon::Smudge,
        ToolIcon::Dodge,
        ToolIcon::Burn,
        ToolIcon::Sponge,
        ToolIcon::QuickSelection,
        ToolIcon::ObjectSelection,
        ToolIcon::Pen,
        ToolIcon::FreeformPen,
        ToolIcon::CurvaturePen,
        ToolIcon::AddAnchorPoint,
        ToolIcon::DeleteAnchorPoint,
        ToolIcon::ConvertPoint,
        ToolIcon::PathSelection,
        ToolIcon::DirectSelection,
        ToolIcon::Rectangle,
        ToolIcon::Ellipse,
        ToolIcon::Triangle,
        ToolIcon::Polygon,
        ToolIcon::Line,
        ToolIcon::CustomShape,
    ];

    /// The icon that draws the tool.
    pub fn icon(&self) -> IconId {
        match self {
            ToolIcon::Custom(icon) => icon.clone(),
            ToolIcon::Move => IconId::new("move"),
            ToolIcon::Artboard => IconId::new("artboard"),
            ToolIcon::RectangularMarquee => IconId::new("marquee_rect"),
            ToolIcon::EllipticalMarquee => IconId::new("ellipse_dashed"),
            ToolIcon::SingleRowMarquee => IconId::new("marquee_row"),
            ToolIcon::SingleColumnMarquee => IconId::new("marquee_column"),
            ToolIcon::Brush => IconId::new("brush"),
            ToolIcon::Pencil => IconId::new("pencil"),
            ToolIcon::MixerBrush => IconId::new("palette"),
            ToolIcon::ColorReplacement => IconId::new("brush_replace"),
            ToolIcon::Eraser => IconId::new("eraser"),
            ToolIcon::BackgroundEraser => IconId::new("eraser_background"),
            ToolIcon::MagicEraser => IconId::new("eraser_magic"),
            ToolIcon::Eyedropper => IconId::new("eyedropper"),
            ToolIcon::ColorSampler => IconId::new("sampler"),
            ToolIcon::Ruler => IconId::new("ruler"),
            ToolIcon::Note => IconId::new("note"),
            ToolIcon::Count => IconId::new("target"),
            ToolIcon::Lasso => IconId::new("lasso"),
            ToolIcon::PolygonalLasso => IconId::new("pentagon"),
            ToolIcon::MagneticLasso => IconId::new("lasso_magnetic"),
            ToolIcon::MagicWand => IconId::new("wand_sparkles"),
            ToolIcon::Crop => IconId::new("crop"),
            ToolIcon::PerspectiveCrop => IconId::new("perspective_crop"),
            ToolIcon::Slice => IconId::new("slice_knife"),
            ToolIcon::SliceSelect => IconId::new("slice_select"),
            ToolIcon::Frame => IconId::new("frame_placeholder"),
            ToolIcon::Gradient => IconId::new("gradient"),
            ToolIcon::PaintBucket => IconId::new("paint_bucket"),
            ToolIcon::HorizontalType => IconId::new("text"),
            ToolIcon::VerticalType => IconId::new("text"),
            ToolIcon::HorizontalTypeMask => IconId::new("type_mask"),
            ToolIcon::VerticalTypeMask => IconId::new("type_mask_vertical"),
            ToolIcon::Hand => IconId::new("hand"),
            ToolIcon::RotateView => IconId::new("compass"),
            ToolIcon::Zoom => IconId::new("zoom_in"),
            ToolIcon::SpotHealing => IconId::new("bandage"),
            ToolIcon::Healing => IconId::new("bandage"),
            ToolIcon::Remove => IconId::new("bandage_sparkle"),
            ToolIcon::Patch => IconId::new("lasso_select"),
            ToolIcon::ContentAwareMove => IconId::new("swap_horizontal"),
            ToolIcon::RedEye => IconId::new("eye"),
            ToolIcon::CloneStamp => IconId::new("stamp"),
            ToolIcon::PatternStamp => IconId::new("grid"),
            ToolIcon::HistoryBrush => IconId::new("clock"),
            ToolIcon::ArtHistoryBrush => IconId::new("sparkles"),
            ToolIcon::Blur => IconId::new("droplet"),
            ToolIcon::Sharpen => IconId::new("triangle"),
            ToolIcon::Smudge => IconId::new("pointer"),
            ToolIcon::Dodge => IconId::new("lollipop"),
            ToolIcon::Burn => IconId::new("flame"),
            ToolIcon::Sponge => IconId::new("cloud"),
            ToolIcon::QuickSelection => IconId::new("quick_selection"),
            ToolIcon::ObjectSelection => IconId::new("object_select"),
            ToolIcon::Pen => IconId::new("pen_tool"),
            ToolIcon::FreeformPen => IconId::new("pen_freeform"),
            ToolIcon::CurvaturePen => IconId::new("pen_curvature"),
            ToolIcon::AddAnchorPoint => IconId::new("pen_add_anchor"),
            ToolIcon::DeleteAnchorPoint => IconId::new("pen_delete_anchor"),
            ToolIcon::ConvertPoint => IconId::new("convert_point"),
            ToolIcon::PathSelection => IconId::new("mouse_pointer_2"),
            ToolIcon::DirectSelection => IconId::new("direct_select"),
            ToolIcon::Rectangle => IconId::new("rectangle"),
            ToolIcon::Ellipse => IconId::new("circle"),
            ToolIcon::Triangle => IconId::new("triangle"),
            ToolIcon::Polygon => IconId::new("pentagon"),
            ToolIcon::Line => IconId::new("slash"),
            ToolIcon::CustomShape => IconId::new("cloud"),
        }
    }

    /// The tool's id as the apps' tool commands spell it (`tool.rectangularMarquee` without the
    /// `tool.` prefix), for automation and the keymap catalog; a custom tool's is its icon's name.
    pub fn id(&self) -> &str {
        match self {
            ToolIcon::Custom(icon) => icon.name(),
            ToolIcon::Move => "move",
            ToolIcon::Artboard => "artboard",
            ToolIcon::RectangularMarquee => "rectangularMarquee",
            ToolIcon::EllipticalMarquee => "ellipticalMarquee",
            ToolIcon::SingleRowMarquee => "singleRowMarquee",
            ToolIcon::SingleColumnMarquee => "singleColumnMarquee",
            ToolIcon::Brush => "brush",
            ToolIcon::Pencil => "pencil",
            ToolIcon::MixerBrush => "mixerBrush",
            ToolIcon::ColorReplacement => "colorReplacement",
            ToolIcon::Eraser => "eraser",
            ToolIcon::BackgroundEraser => "backgroundEraser",
            ToolIcon::MagicEraser => "magicEraser",
            ToolIcon::Eyedropper => "eyedropper",
            ToolIcon::ColorSampler => "colorSampler",
            ToolIcon::Ruler => "ruler",
            ToolIcon::Note => "note",
            ToolIcon::Count => "count",
            ToolIcon::Lasso => "lasso",
            ToolIcon::PolygonalLasso => "polygonalLasso",
            ToolIcon::MagneticLasso => "magneticLasso",
            ToolIcon::MagicWand => "magicWand",
            ToolIcon::Crop => "crop",
            ToolIcon::PerspectiveCrop => "perspectiveCrop",
            ToolIcon::Slice => "slice",
            ToolIcon::SliceSelect => "sliceSelect",
            ToolIcon::Frame => "frame",
            ToolIcon::Gradient => "gradient",
            ToolIcon::PaintBucket => "paintBucket",
            ToolIcon::HorizontalType => "horizontalType",
            ToolIcon::VerticalType => "verticalType",
            ToolIcon::HorizontalTypeMask => "horizontalTypeMask",
            ToolIcon::VerticalTypeMask => "verticalTypeMask",
            ToolIcon::Hand => "hand",
            ToolIcon::RotateView => "rotateView",
            ToolIcon::Zoom => "zoom",
            ToolIcon::SpotHealing => "spotHealing",
            ToolIcon::Healing => "healing",
            ToolIcon::Remove => "remove",
            ToolIcon::Patch => "patch",
            ToolIcon::ContentAwareMove => "contentAwareMove",
            ToolIcon::RedEye => "redEye",
            ToolIcon::CloneStamp => "cloneStamp",
            ToolIcon::PatternStamp => "patternStamp",
            ToolIcon::HistoryBrush => "historyBrush",
            ToolIcon::ArtHistoryBrush => "artHistoryBrush",
            ToolIcon::Blur => "blur",
            ToolIcon::Sharpen => "sharpen",
            ToolIcon::Smudge => "smudge",
            ToolIcon::Dodge => "dodge",
            ToolIcon::Burn => "burn",
            ToolIcon::Sponge => "sponge",
            ToolIcon::QuickSelection => "quickSelection",
            ToolIcon::ObjectSelection => "objectSelection",
            ToolIcon::Pen => "pen",
            ToolIcon::FreeformPen => "freeformPen",
            ToolIcon::CurvaturePen => "curvaturePen",
            ToolIcon::AddAnchorPoint => "addAnchorPoint",
            ToolIcon::DeleteAnchorPoint => "deleteAnchorPoint",
            ToolIcon::ConvertPoint => "convertPoint",
            ToolIcon::PathSelection => "pathSelection",
            ToolIcon::DirectSelection => "directSelection",
            ToolIcon::Rectangle => "rectangle",
            ToolIcon::Ellipse => "ellipse",
            ToolIcon::Triangle => "triangle",
            ToolIcon::Polygon => "polygon",
            ToolIcon::Line => "line",
            ToolIcon::CustomShape => "customShape",
        }
    }

    /// The built-in tool for an id, `None` for an unknown one and for a custom tool.
    pub fn from_id(id: &str) -> Option<ToolIcon> {
        ToolIcon::ALL.into_iter().find(|tool| tool.id() == id)
    }
}
