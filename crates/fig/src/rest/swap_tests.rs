//! A nested instance swapped to another main takes that main's look, group by group.
#![allow(clippy::unwrap_used, clippy::panic)]

use serde_json::Value;

use super::tests::{arr, g, named, o, s, solid};
use super::*;
use crate::kiwi::{Schema, encode};

/// The schema of the fields these files use, with the ids files carry (FigCraft's `.fig` export
/// schema, plus the older `inheritFillStyleID` files from elsewhere can have).
const SCHEMA: &str = "
enum MessageType JOIN_START=0 NODE_CHANGES=1
enum NodePhase CREATED=0 REMOVED=1
enum NodeType NONE=0 DOCUMENT=1 CANVAS=2 GROUP=3 FRAME=4 BOOLEAN_OPERATION=5 VECTOR=6 STAR=7 LINE=8 ELLIPSE=9 RECTANGLE=10 REGULAR_POLYGON=11 ROUNDED_RECTANGLE=12 TEXT=13 SLICE=14 SYMBOL=15 INSTANCE=16 SECTION=25 VARIABLE=28 VARIABLE_SET=31
enum BlendMode PASS_THROUGH=0 NORMAL=1 DARKEN=2 MULTIPLY=3 LINEAR_BURN=4 COLOR_BURN=5 LIGHTEN=6 SCREEN=7 LINEAR_DODGE=8 COLOR_DODGE=9 OVERLAY=10 SOFT_LIGHT=11 HARD_LIGHT=12 DIFFERENCE=13 EXCLUSION=14 HUE=15 SATURATION=16 COLOR=17 LUMINOSITY=18
enum PaintType SOLID=0 GRADIENT_LINEAR=1 GRADIENT_RADIAL=2 GRADIENT_ANGULAR=3 GRADIENT_DIAMOND=4 IMAGE=5
enum ImageScaleMode STRETCH=0 FIT=1 FILL=2 TILE=3
enum WindingRule NONZERO=0 ODD=1
enum StrokeAlign CENTER=0 INSIDE=1 OUTSIDE=2
enum StrokeCap NONE=0 ROUND=1 SQUARE=2 ARROW_LINES=3 ARROW_EQUILATERAL=4 TRIANGLE_FILLED=6 DIAMOND_FILLED=5 CIRCLE_FILLED=14
enum StrokeJoin MITER=0 BEVEL=1 ROUND=2
enum ConstraintType MIN=0 CENTER=1 MAX=2 STRETCH=3 SCALE=4
enum StackMode NONE=0 HORIZONTAL=1 VERTICAL=2 GRID=3
enum StackAlign MIN=0 CENTER=1 MAX=2 BASELINE=3
enum StackJustify MIN=0 CENTER=1 MAX=2 SPACE_EVENLY=3 SPACE_BETWEEN=4 SPACE_AROUND=5 SPACE_EVENLY_CSS=6
enum StackCounterAlign MIN=0 CENTER=1 MAX=2 STRETCH=3 AUTO=4 BASELINE=5
enum StackSize FIXED=0 RESIZE_TO_FIT=1
enum StackWrap NO_WRAP=0 WRAP=1
enum StackCounterAlignContent AUTO=0 SPACE_BETWEEN=1
enum StackPositioning AUTO=0 ABSOLUTE=1
enum EffectType INNER_SHADOW=0 DROP_SHADOW=1 FOREGROUND_BLUR=2 BACKGROUND_BLUR=3 NOISE=7 GRAIN=6 GLASS=8
enum BlurOpType NORMAL=0 PROGRESSIVE=1
enum NoiseType MONOTONE=1 DUOTONE=2 MULTITONE=0
enum TextAlignHorizontal LEFT=0 CENTER=1 RIGHT=2 JUSTIFIED=3
enum TextAlignVertical TOP=0 CENTER=1 BOTTOM=2
enum TextAutoResize NONE=0 WIDTH_AND_HEIGHT=1 HEIGHT=2
enum TextTruncation DISABLED=0 ENDING=1
enum TextCase ORIGINAL=0 UPPER=1 LOWER=2 TITLE=3 SMALL_CAPS=4
enum TextDecoration NONE=0 UNDERLINE=1 STRIKETHROUGH=2
enum NumberUnits RAW=0 PIXELS=1 PERCENT=2
enum LineType PLAIN=0 ORDERED_LIST=1 UNORDERED_LIST=2
enum SourceDirectionality AUTO=0 LTR=1 RTL=2
enum DocumentColorProfile LEGACY=0 SRGB=1 DISPLAY_P3=2
enum EmojiImageSet APPLE=0 NOTO=1
enum MaskType ALPHA=0 OUTLINE=1 LUMINANCE=2
enum VariableResolvedDataType BOOLEAN=0 FLOAT=1 STRING=2 COLOR=4 SYMBOL_ID=6 TEXT_DATA=8
enum VectorMirror NONE=0 ANGLE=1 ANGLE_AND_LENGTH=2
enum BooleanOperation UNION=0 INTERSECT=1 SUBTRACT=2 XOR=3
enum LayoutGridType MIN=0 CENTER=1 STRETCH=2 MAX=3
enum LayoutGridPattern STRIPES=0 GRID=1
enum Axis X=0 Y=1
enum ImageType PNG=0 JPEG=1 SVG=2 PDF=3
enum ExportConstraintType CONTENT_SCALE=0 CONTENT_WIDTH=1 CONTENT_HEIGHT=2
enum ComponentPropType BOOL=0 TEXT=1 INSTANCE_SWAP=3 VARIANT=4
enum VariableField MISSING=0 CORNER_RADIUS=1 PARAGRAPH_SPACING=2 PARAGRAPH_INDENT=3 STROKE_WEIGHT=4 STACK_SPACING=5 STACK_PADDING_LEFT=6 STACK_PADDING_TOP=7 STACK_PADDING_RIGHT=8 STACK_PADDING_BOTTOM=9 VISIBLE=10 TEXT_DATA=11 WIDTH=12 HEIGHT=13 RECTANGLE_TOP_LEFT_CORNER_RADIUS=14 RECTANGLE_TOP_RIGHT_CORNER_RADIUS=15 RECTANGLE_BOTTOM_LEFT_CORNER_RADIUS=16 RECTANGLE_BOTTOM_RIGHT_CORNER_RADIUS=17 BORDER_TOP_WEIGHT=18 BORDER_BOTTOM_WEIGHT=19 BORDER_LEFT_WEIGHT=20 BORDER_RIGHT_WEIGHT=21 STACK_COUNTER_SPACING=23 MIN_WIDTH=24 MAX_WIDTH=25 MIN_HEIGHT=26 MAX_HEIGHT=27 FONT_FAMILY=28 FONT_STYLE=29 OPACITY=31 FONT_SIZE=32 LETTER_SPACING=34 LINE_HEIGHT=36 OVERRIDDEN_SYMBOL_ID=37
enum VariableDataType BOOLEAN=0 FLOAT=1 STRING=2 ALIAS=3 COLOR=4 SYMBOL_ID=7 TEXT_DATA=9 PROP_REF=13
enum VariableScope ALL_SCOPES=0 TEXT_CONTENT=1 CORNER_RADIUS=2 WIDTH_HEIGHT=3 GAP=4 ALL_FILLS=5 FRAME_FILL=6 SHAPE_FILL=7 TEXT_FILL=8 STROKE=9 STROKE_FLOAT=10 EFFECT_FLOAT=11 EFFECT_COLOR=12 OPACITY=13 FONT_STYLE=14 FONT_FAMILY=15 FONT_SIZE=16 LINE_HEIGHT=17 LETTER_SPACING=18 PARAGRAPH_SPACING=19 PARAGRAPH_INDENT=20
enum StyleType NONE=0 FILL=1 STROKE=2 TEXT=3 EFFECT=4 EXPORT=5 GRID=6
enum InteractionType ON_CLICK=0 DRAG=9 ON_HOVER=4 ON_PRESS=7 MOUSE_IN=2 MOUSE_OUT=3 MOUSE_DOWN=5 MOUSE_UP=6 AFTER_TIMEOUT=1 ON_KEY_DOWN=10
enum TriggerDevice KEYBOARD=0 UNKNOWN_CONTROLLER=1
enum ConnectionType NONE=0 INTERNAL_NODE=1 URL=2 BACK=3 CLOSE=4
enum NavigationType NAVIGATE=0 OVERLAY=1 SWAP=2 SWAP_STATE=3 SCROLL_TO=4
enum TransitionType INSTANT_TRANSITION=0 DISSOLVE=1 SMART_ANIMATE=24 SLIDE_FROM_LEFT=3 SLIDE_FROM_RIGHT=4 SLIDE_FROM_TOP=5 SLIDE_FROM_BOTTOM=6 PUSH_FROM_LEFT=7 PUSH_FROM_RIGHT=8 PUSH_FROM_TOP=9 PUSH_FROM_BOTTOM=10 MOVE_FROM_LEFT=11 MOVE_FROM_RIGHT=12 MOVE_FROM_TOP=13 MOVE_FROM_BOTTOM=14 SLIDE_OUT_TO_LEFT=15 SLIDE_OUT_TO_RIGHT=16 SLIDE_OUT_TO_TOP=17 SLIDE_OUT_TO_BOTTOM=18 MOVE_OUT_TO_LEFT=19 MOVE_OUT_TO_RIGHT=20 MOVE_OUT_TO_TOP=21 MOVE_OUT_TO_BOTTOM=22
enum EasingType IN_CUBIC=0 OUT_CUBIC=1 INOUT_CUBIC=2 LINEAR=3 IN_BACK_CUBIC=4 OUT_BACK_CUBIC=5 INOUT_BACK_CUBIC=6 CUSTOM_CUBIC=7 GENTLE_SPRING=9
enum ScrollDirection NONE=0 HORIZONTAL=1 VERTICAL=2 BOTH=3
enum ScrollBehavior SCROLLS=0 FIXED_WHEN_CHILD_OF_SCROLLING_FRAME=1 STICKY_SCROLLS=2
enum OverlayPositionType CENTER=0 TOP_LEFT=1 TOP_CENTER=2 TOP_RIGHT=3 BOTTOM_LEFT=4 BOTTOM_CENTER=5 BOTTOM_RIGHT=6 MANUAL=7
enum OverlayBackgroundInteraction NONE=0 CLOSE_ON_CLICK_OUTSIDE=1
enum OverlayBackgroundType NONE=0 SOLID_COLOR=1
struct GUID uint sessionID, uint localID
struct Color float r, float g, float b, float a
struct Vector float x, float y
struct Matrix float m00, float m01, float m02, float m10, float m11, float m12
struct ColorStop Color color, float position
struct ParentIndex GUID guid, string position
message Image byte[] hash=1, string name=2
message VariableWidthPoint float position=1, float ascent=2, float descent=3
message PluginData string pluginID=1, string value=2, string key=3
message StyleId GUID guid=1
message VariableID GUID guid=1
message VariableSetID GUID guid=1
message ColorStopVar Color color=1, VariableData colorVar=2, float position=3
message Paint PaintType type=1, Color color=2, float opacity=3, bool visible=4, BlendMode blendMode=5, ColorStop[] stops=6, Matrix transform=7, Image image=8, ImageScaleMode imageScaleMode=10, float rotation=11, float scale=12, VariableData colorVar=21, ColorStopVar[] stopsVar=23
message Effect EffectType type=1, Color color=2, Vector offset=3, float radius=4, bool visible=5, BlendMode blendMode=6, float spread=7, bool showShadowBehindNode=8, BlurOpType blurOpType=19, float startRadius=21, Vector startOffset=20, Vector endOffset=28, NoiseType noiseType=26, Vector noiseSize=22, float density=25, float opacity=27, Color secondaryColor=24, bool clipToShape=23, VariableData radiusVar=9, VariableData colorVar=10, VariableData spreadVar=11, VariableData xVar=12, VariableData yVar=13, VariableData startRadiusVar=45
struct FontName string family, string style, string postscript
struct Number float value, NumberUnits units
message Hyperlink string url=1
message TextLineData LineType lineType=1, int styleId=10, int indentationLevel=2, SourceDirectionality sourceDirectionality=9, int listStartOffset=7, bool isFirstLineOfList=8
message TextData string characters=1, uint[] characterStyleIDs=2, NodeChange[] styleOverrideTable=3, TextLineData[] lines=12
message Path WindingRule windingRule=1, uint commandsBlob=2, uint styleID=3
message VectorData uint vectorNetworkBlob=1, Vector normalizedSize=2, NodeChange[] styleOverrideTable=3
message ArcData float startingAngle=1, float endingAngle=2, float innerRadius=3
message OptionalVector Vector value=1
message LayoutGrid LayoutGridType type=1, Axis axis=2, bool visible=3, int numSections=4, float offset=5, float sectionSize=6, float gutterSize=7, Color color=8, LayoutGridPattern pattern=9, VariableData numSectionsVar=10, VariableData offsetVar=11, VariableData sectionSizeVar=12, VariableData gutterSizeVar=13
struct ExportConstraint ExportConstraintType type, float value
message ExportSettings string suffix=1, ImageType imageType=2, ExportConstraint constraint=3
message GUIDPath GUID[] guids=1
message SymbolData GUID symbolID=1, NodeChange[] symbolOverrides=2, float uniformScaleFactor=3
message ComponentPropValue bool boolValue=1, TextData textValue=2, GUID guidValue=3
message ComponentPropPreferredValues string[] stringValues=1
message ComponentPropDef GUID id=1, string name=2, ComponentPropValue initialValue=3, string sortPosition=4, GUID parentPropDefId=5, ComponentPropType type=6, ComponentPropPreferredValues preferredValues=8, VariableData varValue=9
message ComponentPropAssignment GUID defID=1, ComponentPropValue value=2, VariableData varValue=3
enum ComponentPropNodeField VISIBLE=0 TEXT_DATA=1 OVERRIDDEN_SYMBOL_ID=2
message ComponentPropRef GUID defID=2, ComponentPropNodeField componentPropNodeField=4
message VariantPropSpec GUID propDefId=1, string value=2
message StateGroupPropertyValueOrder string property=1, string[] values=2
message PropRefValue GUID defId=1
message SymbolId GUID guid=1
message VariableAnyValue bool boolValue=1, string textValue=2, float floatValue=3, VariableID alias=4, Color colorValue=5, SymbolId symbolIdValue=8, TextData textDataValue=10, PropRefValue propRefValue=13
message VariableData VariableAnyValue value=1, VariableDataType dataType=2, VariableResolvedDataType resolvedDataType=3
message VariableDataMapEntry uint nodeField=1, VariableData variableData=2, VariableField variableField=3
message VariableDataMap VariableDataMapEntry[] entries=1
message VariableModeBySetMapEntry VariableSetID variableSetID=1, GUID variableModeID=2
message VariableModeBySetMap VariableModeBySetMapEntry[] entries=1
message VariableSetMode GUID id=1, string name=2, string sortPosition=3
message VariableDataValuesEntry GUID modeID=1, VariableData variableData=2
message VariableDataValues VariableDataValuesEntry[] entries=1
message KeyTrigger int[] keyCodes=1, TriggerDevice triggerDevice=2
message PrototypeEvent InteractionType interactionType=1, float transitionTimeout=6, KeyTrigger keyTrigger=4
message PrototypeAction GUID transitionNodeID=1, TransitionType transitionType=2, float transitionDuration=3, EasingType easingType=4, float[] easingFunction=12, ConnectionType connectionType=7, Vector overlayRelativePosition=9, NavigationType navigationType=10, string connectionURL=8, bool transitionPreserveScroll=11
message PrototypeInteraction PrototypeEvent event=2, PrototypeAction[] actions=3
message PrototypeStartingPoint string name=1, string position=3
message OverlayBackgroundAppearance OverlayBackgroundType backgroundType=1, Color backgroundColor=2
message GUIDPositionMapEntry GUID id=1, string position=2
message GUIDPositionMap GUIDPositionMapEntry[] entries=1
enum GridTrackSizingType FLEX=0 FIXED=1 HUG=2 UNKNOWN=3
message GridTrackSizingFunction GridTrackSizingType type=1, float value=2
message GridTrackSize GridTrackSizingFunction minSizing=1, GridTrackSizingFunction maxSizing=2
message GUIDGridTrackSizeMapEntry GUID id=1, GridTrackSize trackSize=2
message GUIDGridTrackSizeMap GUIDGridTrackSizeMapEntry[] entries=1
message NodeChange GUID guid=1, NodePhase phase=2, ParentIndex parentIndex=3, NodeType type=4, string name=5, bool visible=6, bool locked=7, float opacity=8, BlendMode blendMode=9, Vector size=11, Matrix transform=12, bool mask=16, MaskType maskType=317, float backgroundOpacity=19, bool backgroundEnabled=15, bool internalOnly=142, string description=318, StyleType styleType=163, string sortPosition=320, StyleId styleIdForFill=332, StyleId styleIdForStrokeFill=333, StyleId styleIdForText=334, StyleId styleIdForEffect=335, StyleId styleIdForGrid=336, VariableDataMap variableConsumptionMap=307, VariableModeBySetMap variableModeBySetMap=316, VariableSetMode[] variableSetModes=312, VariableSetID variableSetID=313, VariableResolvedDataType variableResolvedType=314, VariableDataValues variableDataValues=315, VariableScope[] variableScopes=353, DocumentColorProfile documentColorProfile=349, Paint[] fillPaints=38, Paint[] strokePaints=39, float strokeWeight=26, StrokeAlign strokeAlign=29, StrokeJoin strokeJoin=31, StrokeCap strokeCap=30, float[] dashPattern=13, float miterLimit=25, bool borderStrokeWeightsIndependent=299, float borderTopWeight=295, float borderRightWeight=298, float borderBottomWeight=296, float borderLeftWeight=297, Effect[] effects=43, float cornerRadius=20, float cornerSmoothing=160, bool rectangleCornerRadiiIndependent=149, float rectangleTopLeftCornerRadius=145, float rectangleTopRightCornerRadius=146, float rectangleBottomRightCornerRadius=148, float rectangleBottomLeftCornerRadius=147, ConstraintType horizontalConstraint=28, ConstraintType verticalConstraint=37, bool frameMaskDisabled=115, bool resizeToFit=117, bool isStateGroup=225, StateGroupPropertyValueOrder[] stateGroupPropertyValueOrders=238, bool sectionContentsHidden=283, Color backgroundColor=50, StackMode stackMode=105, float stackSpacing=107, float stackCounterSpacing=324, StackWrap stackWrap=323, StackCounterAlignContent stackCounterAlignContent=343, float stackHorizontalPadding=209, float stackVerticalPadding=210, float stackPaddingRight=233, float stackPaddingBottom=234, StackJustify stackPrimaryAlignItems=230, StackAlign stackCounterAlignItems=231, StackSize stackPrimarySizing=229, StackSize stackCounterSizing=221, bool bordersTakeSpace=294, bool stackReverseZIndex=271, float gridColumnGap=438, float gridRowGap=437, GUIDPositionMap gridColumns=436, GUIDPositionMap gridRows=435, GUIDGridTrackSizeMap gridColumnsSizing=474, GUIDGridTrackSizeMap gridRowsSizing=475, float stackChildPrimaryGrow=232, StackCounterAlign stackChildAlignSelf=236, StackPositioning stackPositioning=269, OptionalVector minSize=325, OptionalVector maxSize=326, GUID gridColumnAnchor=440, GUID gridRowAnchor=439, uint gridColumnSpan=442, uint gridRowSpan=441, LayoutGrid[] layoutGrids=47, ExportSettings[] exportSettings=45, TextData textData=42, FontName fontName=41, float fontSize=21, Number lineHeight=40, Number letterSpacing=165, TextCase textCase=34, TextDecoration textDecoration=35, TextAlignHorizontal textAlignHorizontal=32, TextAlignVertical textAlignVertical=33, TextAutoResize textAutoResize=46, TextTruncation textTruncation=280, int maxLines=351, float paragraphSpacing=23, float paragraphIndent=22, Hyperlink hyperlink=223, bool autoRename=14, bool fontVariantCommonLigatures=127, bool fontVariantContextualLigatures=128, string fontVersion=202, uint textUserLayoutVersion=203, uint textExplicitLayoutVersion=396, uint textBidiVersion=279, float textTracking=27, bool textDecorationSkipInk=412, EmojiImageSet emojiImageSet=391, uint styleID=49, Path[] fillGeometry=51, Path[] strokeGeometry=52, VectorData vectorData=48, VectorMirror handleMirroring=44, uint count=10, float starInnerScale=24, ArcData arcData=195, BooleanOperation booleanOperation=36, string symbolDescription=144, VariableWidthPoint[] variableWidthPoints=447, PluginData[] pluginData=204, SymbolData symbolData=113, NodeChange[] derivedSymbolData=125, GUID overriddenSymbolID=143, ComponentPropDef[] componentPropDefs=266, ComponentPropRef[] componentPropRefs=267, ComponentPropAssignment[] componentPropAssignments=268, VariantPropSpec[] variantPropSpecs=483, VariableDataMap parameterConsumptionMap=445, GUIDPath guidPath=111, PrototypeInteraction[] prototypeInteractions=226, PrototypeStartingPoint prototypeStartingPoint=249, ScrollDirection scrollDirection=159, ScrollBehavior scrollBehavior=186, OverlayPositionType overlayPositionType=198, OverlayBackgroundInteraction overlayBackgroundInteraction=200, OverlayBackgroundAppearance overlayBackgroundAppearance=201
struct Blob byte[] bytes
message Message MessageType type=1, uint sessionID=2, NodeChange[] nodeChanges=4, Blob[] blobs=6
";

type Fields = Vec<(&'static str, Kv<'static>)>;
type Rgb = (f32, f32, f32);

/// A node under `parent` (the shared `node` with a position from the id).
fn node(id: u64, parent: Option<u64>, ty: &'static str, name: &str, more: Fields) -> Kv<'static> {
    let pos = format!("{id:04}");
    super::tests::node(id, parent.map(|p| (p, pos.as_str())), ty, name, more)
}

fn symbol(main: u64, overrides: Vec<Kv<'static>>) -> Kv<'static> {
    o(vec![("symbolID", g(main)), ("symbolOverrides", arr(overrides))])
}

fn row(path: &[u64], more: Fields) -> Kv<'static> {
    let mut fields = vec![("guidPath", o(vec![("guids", arr(path.iter().map(|id| g(*id)).collect()))]))];
    fields.extend(more);
    o(fields)
}

fn paints((r, gr, b): Rgb) -> Kv<'static> {
    arr(vec![solid(f64::from(r), f64::from(gr), f64::from(b))])
}

/// `paints` whose one paint is bound to the variable `var`.
fn bound_paints((r, gr, b): Rgb, var: u64) -> Kv<'static> {
    let alias = o(vec![("value", o(vec![("alias", o(vec![("guid", g(var))]))])), ("dataType", Kv::Enum("ALIAS"))]);
    let mut paint = match solid(f64::from(r), f64::from(gr), f64::from(b)) {
        Kv::Object(fields) => fields,
        _ => Vec::new(),
    };
    paint.push(("colorVar", alias));
    arr(vec![Kv::Object(paint)])
}

fn style_link(id: u64) -> Kv<'static> {
    o(vec![("guid", g(id))])
}

/// The link a layer writes when its fill is detached from a style.
fn no_style() -> Kv<'static> {
    o(vec![("guid", o(vec![("sessionID", Kv::Uint(4_294_967_295)), ("localID", Kv::Uint(4_294_967_295))]))])
}

const INFO_FILL: Rgb = (0.1, 0.2, 0.9);
const IDEAL_FILL: Rgb = (0.9, 0.2, 0.1);
const INFO_STROKE: Rgb = (0.1, 0.9, 0.2);
const IDEAL_STROKE: Rgb = (0.5, 0.5, 0.0);
const RED: Rgb = (1.0, 0.0, 0.0);
const BLUE: Rgb = (0.0, 0.0, 1.0);
const BOUND: Rgb = (0.3, 0.6, 0.3);

/// A variant main: both paints with their styles and a stroke weight, as the file stores them.
/// `fill` None: the fill links no style.
fn variant(id: u64, name: &str, styles: (Option<u64>, Rgb, u64, Rgb), weight: f32, child: u64) -> Vec<Kv<'static>> {
    let (fill, fill_rgb, stroke, stroke_rgb) = styles;
    let mut fields = vec![("fillPaints", paints(fill_rgb))];
    fields.extend(fill.map(|f| ("styleIdForFill", style_link(f))));
    fields.extend([("strokePaints", paints(stroke_rgb)), ("styleIdForStrokeFill", style_link(stroke)), ("strokeWeight", Kv::Float(weight))]);
    vec![node(id, Some(2), "SYMBOL", name, fields), node(child, Some(id), "RECTANGLE", &format!("{name} child"), vec![])]
}

/// What differs between the payloads: the badge's own fill (None: the copy of its main's), the
/// swap row's extra fields, the main the badge records and whether the Info fill links its style.
struct Case {
    badge_fill: Option<Fields>,
    swap_extra: Fields,
    recorded_main: u64,
    info_fill_style: bool,
}

impl Case {
    fn copy() -> Self {
        Self { badge_fill: None, swap_extra: vec![], recorded_main: 10, info_fill_style: true }
    }
}

fn message(case: Case) -> Kv<'static> {
    let style =
        |id: u64, name: &str, rgb: Rgb| node(id, Some(2), "RECTANGLE", name, vec![("styleType", Kv::Enum("FILL")), ("fillPaints", paints(rgb))]);
    let mut changes = vec![
        node(1, None, "DOCUMENT", "Document", vec![]),
        node(2, Some(1), "CANVAS", "Page", vec![]),
        style(3, "Info fill", INFO_FILL),
        style(4, "Info stroke", INFO_STROKE),
        style(5, "Ideal fill", IDEAL_FILL),
        style(6, "Ideal stroke", IDEAL_STROKE),
        node(
            7,
            Some(2),
            "VARIABLE_SET",
            "Theme",
            vec![("variableSetModes", Kv::Array(vec![o(vec![("id", g(90)), ("name", s("Light")), ("sortPosition", s("!"))])]))],
        ),
        node(
            8,
            Some(2),
            "VARIABLE",
            "badge-bg",
            vec![
                ("variableSetID", o(vec![("guid", g(7))])),
                ("variableResolvedType", Kv::Enum("COLOR")),
                (
                    "variableDataValues",
                    o(vec![(
                        "entries",
                        Kv::Array(vec![o(vec![
                            ("modeID", g(90)),
                            (
                                "variableData",
                                o(vec![
                                    (
                                        "value",
                                        o(vec![(
                                            "colorValue",
                                            o(vec![("r", Kv::Float(0.3)), ("g", Kv::Float(0.6)), ("b", Kv::Float(0.3)), ("a", Kv::Float(1.0))]),
                                        )]),
                                    ),
                                    ("dataType", Kv::Enum("COLOR")),
                                ]),
                            ),
                        ])]),
                    )]),
                ),
            ],
        ),
    ];
    let info_fill = case.info_fill_style.then_some(3);
    changes.extend(variant(10, "Type=Info", (info_fill, INFO_FILL, 4, INFO_STROKE), 1.0, 11));
    changes.extend(variant(20, "Type=Ideal", (Some(5), IDEAL_FILL, 6, IDEAL_STROKE), 2.0, 21));
    changes.push(node(30, Some(2), "SYMBOL", "Card", vec![]));
    let mut badge = vec![
        ("symbolData", symbol(case.recorded_main, vec![])),
        ("strokePaints", paints(INFO_STROKE)),
        ("styleIdForStrokeFill", style_link(4)),
        ("strokeWeight", Kv::Float(1.0)),
    ];
    match case.badge_fill {
        Some(fields) => badge.extend(fields),
        None => {
            badge.push(("fillPaints", paints(INFO_FILL)));
            badge.extend(info_fill.map(|f| ("styleIdForFill", style_link(f))));
        }
    }
    changes.push(node(31, Some(30), "INSTANCE", "Badge", badge));
    let mut swap = vec![("overriddenSymbolID", g(20))];
    swap.extend(case.swap_extra);
    changes.push(node(40, Some(2), "INSTANCE", "Swapped", vec![("symbolData", symbol(30, vec![row(&[31], swap)]))]));
    changes.push(node(41, Some(2), "INSTANCE", "Plain", vec![("symbolData", symbol(30, vec![]))]));
    o(vec![("nodeChanges", Kv::Array(changes))])
}

fn map_case(case: Case) -> Mapped {
    let schema = SCHEMA.replace("message NodeChange GUID guid=1,", "message NodeChange GUID guid=1, GUID inheritFillStyleID=501,");
    let schema = Schema::parse(&schema).unwrap();
    // Through the bytes, as a file's message comes.
    let root = schema.find("Message").unwrap();
    let bytes = encode::message(&schema, root, &message(case)).unwrap();
    let msg = schema.decode_message(root, &bytes).unwrap();
    to_rest(&msg, "Swap.fig", &Options::default())
}

/// The first child of the layer named `outer`: the badge.
fn badge<'j>(json: &'j Value, outer: &str) -> &'j Value {
    &named(json, outer)["children"][0]
}

fn is_rgb(paints: &Value, want: Rgb) -> bool {
    let c = &paints[0]["color"];
    [&c["r"], &c["g"], &c["b"]].iter().zip([want.0, want.1, want.2]).all(|(a, b)| a.as_f64().is_some_and(|a| (a - f64::from(b)).abs() < 1e-3))
}

/// The name of the style `layer` links in `slot`.
fn style_name<'j>(json: &'j Value, layer: &Value, slot: &str) -> Option<&'j str> {
    json["styles"][layer["styles"][slot].as_str()?]["name"].as_str()
}

#[test]
fn a_swapped_nested_instance_takes_the_new_mains_look() {
    let out = map_case(Case::copy());
    let json = &out.json;
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let swapped = badge(json, "Swapped");
    assert!(is_rgb(&swapped["fills"], IDEAL_FILL), "fill {}", swapped["fills"]);
    assert!(is_rgb(&swapped["strokes"], IDEAL_STROKE), "stroke {}", swapped["strokes"]);
    assert_eq!(style_name(json, swapped, "fill"), Some("Ideal fill"));
    assert_eq!(style_name(json, swapped, "stroke"), Some("Ideal stroke"));
    assert_eq!(swapped["strokeWeight"], 2.0);
    assert_eq!(swapped["children"][0]["name"], "Type=Ideal child");
    let plain = badge(json, "Plain");
    assert!(is_rgb(&plain["fills"], INFO_FILL));
    assert_eq!(style_name(json, plain, "fill"), Some("Info fill"));
    assert_eq!(style_name(json, plain, "stroke"), Some("Info stroke"));
    assert_eq!(plain["strokeWeight"], 1.0);
}

#[test]
fn a_real_override_on_the_nested_instance_survives_the_swap() {
    let out = map_case(Case { badge_fill: Some(vec![("fillPaints", paints(RED))]), ..Case::copy() });
    let json = &out.json;
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let swapped = badge(json, "Swapped");
    assert!(is_rgb(&swapped["fills"], RED), "fill {}", swapped["fills"]);
    assert!(swapped["styles"].get("fill").is_none());
    assert!(is_rgb(&swapped["strokes"], IDEAL_STROKE), "stroke {}", swapped["strokes"]);
    assert_eq!(style_name(json, swapped, "stroke"), Some("Ideal stroke"));
    assert!(is_rgb(&badge(json, "Card")["fills"], RED));
}

#[test]
fn a_row_above_the_swap_beats_the_new_main() {
    let out = map_case(Case { swap_extra: vec![("fillPaints", paints(BLUE)), ("styleIdForFill", no_style())], ..Case::copy() });
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let swapped = badge(&out.json, "Swapped");
    assert!(is_rgb(&swapped["fills"], BLUE), "fill {}", swapped["fills"]);
    assert!(is_rgb(&swapped["strokes"], IDEAL_STROKE), "stroke {}", swapped["strokes"]);
}

#[test]
fn a_fill_row_without_a_style_link_beats_the_new_mains_style() {
    // The previous main's fill links no style, so a row that recolours the badge has none to
    // detach: its paints must still win over the style the new main links.
    let out = map_case(Case { info_fill_style: false, swap_extra: vec![("fillPaints", paints(BLUE))], ..Case::copy() });
    let json = &out.json;
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let swapped = badge(json, "Swapped");
    assert!(is_rgb(&swapped["fills"], BLUE), "fill {}", swapped["fills"]);
    assert!(swapped["styles"].get("fill").is_none());
    assert!(is_rgb(&swapped["strokes"], IDEAL_STROKE), "stroke {}", swapped["strokes"]);
    assert_eq!(style_name(json, swapped, "stroke"), Some("Ideal stroke"));
}

#[test]
fn a_rows_older_style_field_beats_the_new_mains_newer_one() {
    // The older `inheritFillStyleID` on the swap row is the topmost word on the fill's style: a
    // detach keeps the row's paints, a link takes that style.
    let detach = o(vec![("sessionID", Kv::Uint(4_294_967_295)), ("localID", Kv::Uint(4_294_967_295))]);
    let out = map_case(Case { swap_extra: vec![("fillPaints", paints(BLUE)), ("inheritFillStyleID", detach)], ..Case::copy() });
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let swapped = badge(&out.json, "Swapped");
    assert!(is_rgb(&swapped["fills"], BLUE), "fill {}", swapped["fills"]);
    assert!(swapped["styles"].get("fill").is_none());
    let out = map_case(Case { swap_extra: vec![("inheritFillStyleID", g(3))], ..Case::copy() });
    let swapped = badge(&out.json, "Swapped");
    assert!(is_rgb(&swapped["fills"], INFO_FILL), "fill {}", swapped["fills"]);
    assert_eq!(style_name(&out.json, swapped, "fill"), Some("Info fill"));
}

#[test]
fn a_swap_whose_previous_main_is_missing_takes_the_new_look() {
    let out = map_case(Case { recorded_main: 99, ..Case::copy() });
    assert_eq!(out.warnings.iter().filter(|n| n.contains("previous component isn't in the file")).count(), 1, "{:?}", out.warnings);
    let swapped = badge(&out.json, "Swapped");
    assert!(is_rgb(&swapped["fills"], IDEAL_FILL), "fill {}", swapped["fills"]);
    assert!(is_rgb(&swapped["strokes"], IDEAL_STROKE), "stroke {}", swapped["strokes"]);
}

#[test]
fn a_bound_fill_under_the_old_mains_style_link_survives_the_swap() {
    // The badge keeps the old main's fill style link but binds its own fill to a variable: the
    // binding is drawn over the style, so the swap must not replace it with the new main's fill.
    let bound = vec![("fillPaints", bound_paints(BOUND, 8)), ("styleIdForFill", style_link(3))];
    let out = map_case(Case { badge_fill: Some(bound), ..Case::copy() });
    let json = &out.json;
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let swapped = badge(json, "Swapped");
    assert!(is_rgb(&swapped["fills"], BOUND), "fill {}", swapped["fills"]);
    assert_eq!(swapped["fills"][0]["boundVariables"]["color"]["id"], "1:8");
    assert_eq!(style_name(json, swapped, "fill"), Some("Info fill"));
    assert!(is_rgb(&swapped["strokes"], IDEAL_STROKE), "stroke {}", swapped["strokes"]);
    assert_eq!(swapped["strokeWeight"], 2.0);
}
