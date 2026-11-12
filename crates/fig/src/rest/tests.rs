//! The mapping on a message built in code: a small schema in the compact text form (FigCraft's
//! `.fig` fixture), mapped onto the REST shape and read back with craft-figma-rest.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::cell::RefCell;

use craft_figma_rest::{File, NodeType};
use serde_json::Value;

use super::*;
use crate::kiwi::{Schema, encode};

/// The fixture schema: `enum Name A B …`, `struct|message Name type field, type field[] …`.
/// Message field ids count from 1; enum values from 0.
const SCHEMA: &str = "
enum NodeType NONE DOCUMENT CANVAS GROUP FRAME BOOLEAN_OPERATION VECTOR STAR LINE ELLIPSE RECTANGLE REGULAR_POLYGON ROUNDED_RECTANGLE TEXT SLICE SYMBOL INSTANCE STICKY VARIABLE VARIABLE_SET SHAPE_WITH_TEXT CONNECTOR TABLE
enum PaintType SOLID GRADIENT_LINEAR IMAGE EMOJI
enum ImageScaleMode STRETCH FIT FILL TILE
enum WindingRule NONZERO ODD
enum StyleType NONE FILL TEXT
enum StackMode NONE HORIZONTAL VERTICAL
enum PropType BOOL TEXT
enum VariableField MISSING VISIBLE TEXT_DATA
enum DataType BOOLEAN FLOAT STRING ALIAS COLOR PROP_REF EXPRESSION
enum ResolvedType BOOLEAN FLOAT STRING COLOR
enum InteractionType ON_CLICK ON_HOVER
enum ConnectionType NONE INTERNAL_NODE URL BACK CLOSE SET_VARIABLE CONDITIONAL SET_VARIABLE_MODE
enum StrokeCap NONE ROUND SQUARE ARROW_LINES ARROW_EQUILATERAL
enum StrokeJoin MITER BEVEL ROUND
enum VectorMirror NONE ANGLE ANGLE_AND_LENGTH
enum ShapeWithTextType SQUARE ELLIPSE
enum NodePhase CREATED REMOVED
enum ConnectorLineStyle ELBOWED STRAIGHT CURVED
enum ConnectorMagnet NONE AUTO TOP LEFT BOTTOM RIGHT CENTER
enum ExpressionFunction ADDITION SUBTRACTION RESOLVE_VARIANT MULTIPLY DIVIDE EQUALS
enum NavigationType NAVIGATE OVERLAY
enum TransitionType INSTANT_TRANSITION DISSOLVE SMART_ANIMATE
enum EasingType IN_CUBIC OUT_CUBIC
enum ConstraintType MIN CENTER MAX STRETCH SCALE
enum LeadingTrim NONE CAP_HEIGHT
struct GUID uint sessionID, uint localID
struct Color float r, float g, float b, float a
struct Vector float x, float y
struct Matrix float m00, float m01, float m02, float m10, float m11, float m12
struct ColorStop Color color, float position
message Image byte[] hash, string name, uint dataBlob
message AssetRef string key, string version
message VariableID GUID guid, AssetRef assetRef
message PropRefValue GUID defId
message AnyValue bool boolValue, float floatValue, VariableID alias, Color colorValue, PropRefValue propRefValue, TextData textDataValue, string textValue, Expression expressionValue
message Expression ExpressionFunction expressionFunction, VariableData[] expressionArguments
message VariableTarget VariableID id
message VariableSetID GUID guid, AssetRef assetRef
message ConditionalActions Action[] actions, VariableData condition
message VectorData uint vectorNetworkBlob, Vector normalizedSize, NodeChange[] styleOverrideTable
message ConnectorEndpoint GUID endpointNodeID, Vector position, Vector relativePosition, ConnectorMagnet magnet
message ConnectorControlPoint Vector position, Vector axis
message VariableData AnyValue value, DataType dataType
message Paint PaintType type, Color color, float opacity, bool visible, Image image, Matrix transform, ColorStop[] stops, VariableData colorVar, ImageScaleMode imageScaleMode
message ParentIndex GUID guid, string position
message FontName string family, string style, string postscript
message TextData string characters, uint[] characterStyleIDs, NodeChange[] styleOverrideTable
message Path WindingRule windingRule, uint commandsBlob
message GUIDPath GUID[] guids
message SymbolData GUID symbolID, NodeChange[] symbolOverrides, float uniformScaleFactor
message StyleId GUID guid, AssetRef assetRef
message PropValue bool boolValue, TextData textValue
message PropDef GUID id, string name, PropValue initialValue, PropType type
message PropAssignment GUID defID, PropValue value
message MapEntry VariableData variableData, VariableField variableField
message VariableDataMap MapEntry[] entries
message SetMode GUID id, string name, string sortPosition
message ValuesEntry GUID modeID, VariableData variableData
message Values ValuesEntry[] entries
message ModeEntry VariableID variableSetID, GUID variableModeID
message ModeMap ModeEntry[] entries
message Event InteractionType interactionType
message Action GUID transitionNodeID, TransitionType transitionType, float transitionDuration, EasingType easingType, ConnectionType connectionType, NavigationType navigationType, VariableTarget targetVariable, VariableData targetVariableData, VariableSetID targetVariableSetID, GUID targetVariableModeID, ConditionalActions[] conditionalActions
message Interaction Event event, Action[] actions
message StartingPoint string name, string position
message NodeChange GUID guid, ParentIndex parentIndex, NodeType type, string name, bool visible, float opacity, Vector size, Matrix transform, Paint[] fillPaints, Paint[] strokePaints, float strokeWeight, float cornerRadius, bool resizeToFit, bool internalOnly, float fontSize, FontName fontName, TextData textData, uint styleID, Path[] fillGeometry, Path[] strokeGeometry, StyleType styleType, StyleId styleIdForFill, SymbolData symbolData, PropDef[] componentPropDefs, PropAssignment[] componentPropAssignments, VariableDataMap parameterConsumptionMap, SetMode[] variableSetModes, VariableID variableSetID, ResolvedType variableResolvedType, Values variableDataValues, ModeMap variableModeBySetMap, Interaction[] prototypeInteractions, StartingPoint prototypeStartingPoint, GUIDPath guidPath, StackMode stackMode, float stackSpacing, float stackHorizontalPadding, float stackVerticalPadding, uint count, float starInnerScale, VectorData vectorData, StrokeCap strokeCap, StrokeJoin strokeJoin, VectorMirror handleMirroring, ShapeWithTextType shapeWithTextType, ConnectorEndpoint connectorStart, ConnectorEndpoint connectorEnd, StrokeCap connectorStartCap, StrokeCap connectorEndCap, ConnectorLineStyle connectorLineStyle, ConstraintType horizontalConstraint, ConstraintType verticalConstraint, NodeChange[] derivedSymbolData, string key, string version, LeadingTrim leadingTrim, float[] dashPattern, ConnectorControlPoint[] connectorControlPoints, NodePhase phase, GUID overriddenSymbolID
message Blob byte[] bytes
message Message NodeChange[] nodeChanges, Blob[] blobs
";

fn schema() -> Schema {
    Schema::parse(SCHEMA).unwrap()
}

pub(super) fn o(fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    Kv::Object(fields)
}
fn f(x: f64) -> Kv<'static> {
    Kv::Float(x as f32)
}
fn u(x: u64) -> Kv<'static> {
    Kv::Uint(x)
}
fn e(n: &'static str) -> Kv<'static> {
    Kv::Enum(n)
}
pub(super) fn s(x: impl Into<String>) -> Kv<'static> {
    Kv::Str(x.into())
}
pub(super) fn g(l: u64) -> Kv<'static> {
    o(vec![("sessionID", u(1)), ("localID", u(l))])
}
pub(super) fn arr(v: Vec<Kv<'static>>) -> Kv<'static> {
    Kv::Array(v)
}
fn xy(x: f64, y: f64) -> Kv<'static> {
    o(vec![("x", f(x)), ("y", f(y))])
}
fn at(x: f64, y: f64) -> Kv<'static> {
    o(vec![("m00", f(1.0)), ("m01", f(0.0)), ("m02", f(x)), ("m10", f(0.0)), ("m11", f(1.0)), ("m12", f(y))])
}
fn rgb(r: f64, gr: f64, b: f64) -> Kv<'static> {
    o(vec![("r", f(r)), ("g", f(gr)), ("b", f(b)), ("a", f(1.0))])
}
pub(super) fn solid(r: f64, gr: f64, b: f64) -> Kv<'static> {
    o(vec![("type", e("SOLID")), ("color", rgb(r, gr, b)), ("opacity", f(1.0)), ("visible", Kv::Bool(true))])
}
fn root_guid() -> Kv<'static> {
    o(vec![("sessionID", u(0)), ("localID", u(0))])
}
fn alias_data(l: u64) -> Kv<'static> {
    o(vec![("value", o(vec![("alias", o(vec![("guid", g(l))]))])), ("dataType", e("ALIAS"))])
}
fn prop_ref(def: u64, field: &'static str) -> Kv<'static> {
    let data = o(vec![("value", o(vec![("propRefValue", o(vec![("defId", g(def))]))])), ("dataType", e("PROP_REF"))]);
    o(vec![("entries", arr(vec![o(vec![("variableData", data), ("variableField", e(field))])]))])
}

/// A node: id (0: the document), parent (id, position), type, name and further fields.
pub(super) fn node(id: u64, par: Option<(u64, &str)>, ty: &'static str, name: &str, mut more: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    let mut v = vec![("guid", if id == 0 { root_guid() } else { g(id) })];
    if let Some((p, pos)) = par {
        v.push(("parentIndex", o(vec![("guid", if p == 0 { root_guid() } else { g(p) }), ("position", s(pos))])));
    }
    v.push(("type", e(ty)));
    v.push(("name", s(name)));
    v.append(&mut more);
    o(v)
}

/// A path blob: a closed triangle.
fn triangle_blob() -> Vec<u8> {
    let mut b = Vec::new();
    for (cmd, pts) in [(1u8, vec![0.0f32, 0.0]), (2, vec![10.0, 0.0]), (2, vec![5.0, 8.0]), (0, vec![])] {
        b.push(cmd);
        for p in pts {
            b.extend_from_slice(&p.to_le_bytes());
        }
    }
    b
}

const HASH: [u8; 20] = [0xab; 20];

/// The fixture message: a page with a frame holding a rectangle (image fill, click interaction),
/// a two-style text, a group with a triangle vector and a star, a layer with a linked fill style,
/// one with a variable-bound fill, and an instance of a component with a boolean property
/// (property off, child renamed); a flow and an explicit variable mode; an unsupported sticky
/// note; styles and variables on the internal page.
fn message() -> Kv<'static> {
    let modes = arr(vec![
        o(vec![("id", g(90)), ("name", s("Light")), ("sortPosition", s("!"))]),
        o(vec![("id", g(91)), ("name", s("Dark")), ("sortPosition", s("\""))]),
    ]);
    let color_value = |c| o(vec![("value", o(vec![("colorValue", c)])), ("dataType", e("COLOR"))]);
    let values = o(vec![(
        "entries",
        arr(vec![
            o(vec![("modeID", g(90)), ("variableData", color_value(rgb(1.0, 1.0, 1.0)))]),
            o(vec![("modeID", g(91)), ("variableData", color_value(rgb(0.0, 0.0, 0.0)))]),
        ]),
    )]);
    let click = o(vec![
        ("event", o(vec![("interactionType", e("ON_CLICK"))])),
        (
            "actions",
            arr(vec![o(vec![
                ("transitionNodeID", g(20)),
                ("transitionType", e("SMART_ANIMATE")),
                ("transitionDuration", f(0.25)),
                ("easingType", e("OUT_CUBIC")),
                ("connectionType", e("INTERNAL_NODE")),
                ("navigationType", e("NAVIGATE")),
            ])]),
        ),
    ]);
    let font = |style: &str| o(vec![("family", s("Inter")), ("style", s(style)), ("postscript", s(""))]);
    let text = o(vec![
        ("characters", s("Hi there")),
        ("characterStyleIDs", arr(vec![u(0), u(0), u(0), u(5), u(5), u(5)])),
        ("styleOverrideTable", arr(vec![o(vec![("fontName", font("Bold")), ("styleID", u(5))])])),
    ]);
    let nodes = vec![
        node(0, None, "DOCUMENT", "Document", vec![]),
        node(1, Some((0, "!")), "CANVAS", "Page A", vec![]),
        node(2, Some((0, "~")), "CANVAS", "Internal", vec![("internalOnly", Kv::Bool(true))]),
        node(3, Some((2, "!")), "RECTANGLE", "Brand", vec![("styleType", e("FILL")), ("fillPaints", arr(vec![solid(0.0, 0.0, 1.0)]))]),
        node(4, Some((2, "\"")), "VARIABLE_SET", "Theme", vec![("variableSetModes", modes)]),
        node(
            5,
            Some((2, "#")),
            "VARIABLE",
            "surface",
            vec![("variableSetID", o(vec![("guid", g(4))])), ("variableResolvedType", e("COLOR")), ("variableDataValues", values)],
        ),
        // Listed before the frame, but its position puts it second.
        node(
            20,
            Some((1, "\"")),
            "SYMBOL",
            "Button",
            vec![
                ("size", xy(100.0, 40.0)),
                ("transform", at(500.0, 0.0)),
                ("fillPaints", arr(vec![solid(0.0, 1.0, 0.0)])),
                (
                    "componentPropDefs",
                    arr(vec![o(vec![
                        ("id", g(80)),
                        ("name", s("Show icon")),
                        ("initialValue", o(vec![("boolValue", Kv::Bool(true))])),
                        ("type", e("BOOL")),
                    ])]),
                ),
                ("stackMode", e("HORIZONTAL")),
                ("stackSpacing", f(8.0)),
                ("stackHorizontalPadding", f(12.0)),
                ("stackVerticalPadding", f(4.0)),
            ],
        ),
        node(
            21,
            Some((20, "!")),
            "ELLIPSE",
            "Icon",
            vec![("size", xy(16.0, 16.0)), ("transform", at(12.0, 12.0)), ("parameterConsumptionMap", prop_ref(80, "VISIBLE"))],
        ),
        node(
            10,
            Some((1, "!")),
            "FRAME",
            "Screen",
            vec![
                ("size", xy(400.0, 300.0)),
                ("fillPaints", arr(vec![solid(1.0, 1.0, 1.0)])),
                ("prototypeStartingPoint", o(vec![("name", s("Main flow")), ("position", s("!"))])),
                (
                    "variableModeBySetMap",
                    o(vec![("entries", arr(vec![o(vec![("variableSetID", o(vec![("guid", g(4))])), ("variableModeID", g(91))])]))]),
                ),
            ],
        ),
        node(
            11,
            Some((10, "!")),
            "ROUNDED_RECTANGLE",
            "Photo",
            vec![
                ("size", xy(50.0, 40.0)),
                ("transform", at(10.0, 20.0)),
                ("cornerRadius", f(6.0)),
                ("fillPaints", arr(vec![o(vec![("type", e("IMAGE")), ("image", o(vec![("hash", Kv::Bytes(HASH.to_vec()))]))])])),
                ("strokePaints", arr(vec![solid(0.0, 0.0, 0.0)])),
                ("strokeWeight", f(2.0)),
                ("prototypeInteractions", arr(vec![click])),
            ],
        ),
        node(
            12,
            Some((10, "\"")),
            "TEXT",
            "Title",
            vec![
                ("size", xy(120.0, 20.0)),
                ("transform", at(10.0, 80.0)),
                ("fillPaints", arr(vec![solid(0.0, 0.0, 0.0)])),
                ("fontSize", f(16.0)),
                ("fontName", font("Regular")),
                ("leadingTrim", e("CAP_HEIGHT")),
                ("textData", text),
            ],
        ),
        node(13, Some((10, "#")), "FRAME", "Group", vec![("size", xy(30.0, 30.0)), ("transform", at(100.0, 100.0)), ("resizeToFit", Kv::Bool(true))]),
        node(
            14,
            Some((13, "!")),
            "VECTOR",
            "Tri",
            vec![
                ("size", xy(10.0, 8.0)),
                ("transform", at(5.0, 6.0)),
                ("fillPaints", arr(vec![solid(1.0, 0.0, 0.0)])),
                ("fillGeometry", arr(vec![o(vec![("windingRule", e("NONZERO")), ("commandsBlob", u(0))])])),
            ],
        ),
        node(
            15,
            Some((13, "\"")),
            "STAR",
            "Star",
            vec![("size", xy(20.0, 20.0)), ("transform", at(10.0, 10.0)), ("count", u(6)), ("starInnerScale", f(0.5))],
        ),
        node(
            16,
            Some((10, "$")),
            "ROUNDED_RECTANGLE",
            "Styled",
            vec![
                ("size", xy(10.0, 10.0)),
                ("transform", at(0.0, 200.0)),
                // A stale copy of the style's paint: the style wins.
                ("fillPaints", arr(vec![solid(0.5, 0.5, 0.5)])),
                ("styleIdForFill", o(vec![("guid", g(3))])),
            ],
        ),
        node(
            17,
            Some((10, "%")),
            "ROUNDED_RECTANGLE",
            "Card",
            vec![
                ("size", xy(10.0, 10.0)),
                ("transform", at(20.0, 200.0)),
                ("fillPaints", arr(vec![o(vec![("type", e("SOLID")), ("color", rgb(0.0, 0.0, 0.0)), ("colorVar", alias_data(5))])])),
            ],
        ),
        node(
            18,
            Some((10, "&")),
            "INSTANCE",
            "Button copy",
            vec![
                ("size", xy(100.0, 40.0)),
                ("transform", at(200.0, 200.0)),
                ("fillPaints", arr(vec![solid(1.0, 0.0, 0.0)])),
                ("stackMode", e("HORIZONTAL")),
                (
                    "symbolData",
                    o(vec![
                        ("symbolID", g(20)),
                        ("symbolOverrides", arr(vec![o(vec![("guidPath", o(vec![("guids", arr(vec![g(21)]))])), ("name", s("Renamed icon"))])])),
                    ]),
                ),
                ("componentPropAssignments", arr(vec![o(vec![("defID", g(80)), ("value", o(vec![("boolValue", Kv::Bool(false))]))])])),
            ],
        ),
        node(19, Some((10, "'")), "STICKY", "Note", vec![]),
    ];
    o(vec![("nodeChanges", arr(nodes)), ("blobs", arr(vec![o(vec![("bytes", Kv::Bytes(triangle_blob()))])]))])
}

fn mapped(msg: &Kv) -> Mapped {
    to_rest(msg, "Fixture", &Options::default())
}

/// A node of the mapped document by name.
pub(super) fn named<'v>(json: &'v Value, name: &str) -> &'v Value {
    let mut pending = vec![&json["document"]];
    while let Some(n) = pending.pop() {
        if n["name"] == name {
            return n;
        }
        pending.extend(n["children"].as_array().into_iter().flatten());
    }
    panic!("no layer {name}")
}

#[test]
fn the_fixture_maps_onto_the_rest_shape() {
    let msg = message();
    let out = mapped(&msg);
    let json = &out.json;
    assert_eq!(json["name"], "Fixture");
    // The internal page is skipped; siblings by position.
    let pages = json["document"]["children"].as_array().unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!((pages[0]["type"].as_str(), pages[0]["name"].as_str()), (Some("CANVAS"), Some("Page A")));
    assert_eq!(pages[0]["children"][0]["name"], "Screen");
    let names: Vec<&str> = named(json, "Screen")["children"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Photo", "Title", "Group", "Styled", "Card", "Button copy", "Note"]);
    // An image fill names its image by the hash in hex; corner and stroke come along.
    let photo = named(json, "Photo");
    assert_eq!(photo["fills"][0]["type"], "IMAGE");
    assert_eq!(photo["fills"][0]["imageRef"], "ab".repeat(20));
    assert_eq!((photo["cornerRadius"].as_f64(), photo["strokeWeight"].as_f64()), (Some(6.0), Some(2.0)));
    assert_eq!(photo["interactions"][0]["actions"][0]["destinationId"], "1:20");
    // Text: the bold run over "the" is style 5.
    let title = named(json, "Title");
    assert_eq!(title["characters"], "Hi there");
    assert_eq!(title["style"]["fontSize"], 16.0);
    let runs: Vec<f64> = title["characterStyleOverrides"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    assert_eq!(runs, [0.0, 0.0, 0.0, 5.0, 5.0, 5.0]);
    assert_eq!(title["styleOverrideTable"]["5"]["fontStyle"], "Bold");
    // The vector's outline as SVG path data; the star's points and inner radius (extensions).
    let tri = named(json, "Tri");
    assert_eq!(tri["fillGeometry"][0]["path"], "M 0 0 L 10 0 L 5 8 Z");
    let star = named(json, "Star");
    assert_eq!((star["type"].as_str(), star["pointCount"].as_f64(), star["innerRadius"].as_f64()), (Some("STAR"), Some(6.0), Some(0.5)));
    // Shared style, variable binding, explicit mode.
    assert_eq!(named(json, "Styled")["styles"]["fill"], "1:3");
    assert_eq!(json["styles"]["1:3"]["name"], "Brand");
    assert_eq!(named(json, "Card")["fills"][0]["boundVariables"]["color"]["id"], "1:5");
    assert_eq!(named(json, "Screen")["explicitVariableModes"]["1:4"], "1:91");
    // A component, and an instance of it with its property off and its child renamed.
    assert_eq!(named(json, "Button")["type"], "COMPONENT");
    let inst = named(json, "Button copy");
    assert_eq!((inst["type"].as_str(), inst["componentId"].as_str()), (Some("INSTANCE"), Some("1:20")));
    assert_eq!(inst["children"][0]["name"], "Renamed icon");
    assert_eq!(inst["children"][0]["visible"], false);
    // A flow; the sticky note as a frame, with a warning.
    assert_eq!(pages[0]["flowStartingPoints"][0]["nodeId"], "1:10");
    assert_eq!(named(json, "Note")["type"], "FRAME");
    assert!(out.warnings.iter().any(|w| w.contains("sticky layers import as frames")), "{:?}", out.warnings);
}

#[test]
fn the_result_reads_as_the_typed_model() {
    let msg = message();
    let out = mapped(&msg);
    let file = File::from_value(out.json.clone(), 1000).unwrap();
    let document = file.document.as_ref().unwrap();
    assert_eq!(document.kind, Some(NodeType::Document));
    let tri = document.walk().find(|n| n.name.as_deref() == Some("Tri")).unwrap();
    assert_eq!(tri.kind, Some(NodeType::Vector));
    assert_eq!(tri.fill_geometry.as_ref().unwrap()[0].winding_rule.as_deref(), Some("NONZERO"));
    let star = document.walk().find(|n| n.name.as_deref() == Some("Star")).unwrap();
    assert_eq!(star.extra["pointCount"], 6.0);
    // Written back, the typed model holds the same file (whole numbers aside).
    let back = file.to_value().unwrap();
    assert_eq!(File::from_value(back, 1000).unwrap(), file);
}

#[test]
fn vector_networks_are_handed_over_beside_the_json() {
    // A vector with a network blob: three points, a closed region.
    let blob = crate::network::encode(&crate::network::Network {
        vertices: vec![
            crate::network::Vertex { style: 0, x: 0.0, y: 0.0 },
            crate::network::Vertex { style: 0, x: 1.0, y: 0.0 },
            crate::network::Vertex { style: 0, x: 0.5, y: 1.0 },
        ],
        segments: (0..3).map(|i| crate::network::Segment { style: 0, a: i, ta: (0.0, 0.0), b: (i + 1) % 3, tb: (0.0, 0.0) }).collect(),
        regions: vec![crate::network::Region { style: 0, non_zero: true, loops: vec![vec![0, 1, 2]] }],
    });
    let vd = o(vec![("vectorNetworkBlob", u(0)), ("normalizedSize", xy(1.0, 1.0))]);
    let nodes = vec![
        node(0, None, "DOCUMENT", "D", vec![]),
        node(1, Some((0, "!")), "CANVAS", "P", vec![]),
        node(2, Some((1, "!")), "VECTOR", "V", vec![("size", xy(20.0, 10.0)), ("vectorData", vd)]),
    ];
    let msg = o(vec![("nodeChanges", arr(nodes)), ("blobs", arr(vec![o(vec![("bytes", Kv::Bytes(blob))])]))]);
    let out = mapped(&msg);
    assert_eq!(named(&out.json, "V")["vectorNetworkRef"], 0);
    let net = out.networks[0].as_ref().unwrap();
    // Scaled from the normalized size to the layer's.
    assert_eq!((net.vertices[1].x, net.vertices[2].y), (20.0, 10.0));
    assert_eq!(net.regions[0].loops, [vec![0, 1, 2]]);
    assert!(net.is_consistent());
}

/// Records the connector ends it is asked to route, and routes them straight.
#[derive(Default)]
struct Recording(RefCell<Vec<(RouteEnd, RouteEnd, Routing)>>);

impl Geometry for Recording {
    fn route(&self, from: RouteEnd, to: RouteEnd, routing: Routing, saved: Option<&SavedRuns>) -> Option<Vec<Point>> {
        self.0.borrow_mut().push((from, to, routing));
        Straight.route(from, to, routing, saved)
    }

    fn attach(&self, from: RouteEnd, to: RouteEnd) -> ((Point, Option<Side>), (Point, Option<Side>)) {
        Straight.attach(from, to)
    }
}

#[test]
fn connectors_are_routed_by_the_callers_geometry() {
    let end = |x: f64, y: f64| o(vec![("position", xy(x, y))]);
    let nodes = vec![
        node(0, None, "DOCUMENT", "D", vec![]),
        node(1, Some((0, "!")), "CANVAS", "P", vec![]),
        node(2, Some((1, "!")), "RECTANGLE", "Box", vec![("size", xy(40.0, 20.0)), ("transform", at(100.0, 100.0))]),
        node(
            3,
            Some((1, "\"")),
            "CONNECTOR",
            "Line",
            vec![
                ("connectorStart", end(10.0, 20.0)),
                ("connectorEnd", o(vec![("endpointNodeID", g(2)), ("magnet", e("LEFT"))])),
                ("connectorLineStyle", e("ELBOWED")),
            ],
        ),
    ];
    let msg = o(vec![("nodeChanges", arr(nodes))]);
    let geometry = Recording::default();
    let out = to_rest(&msg, "x", &Options { geometry: &geometry, ..Options::default() });
    let calls = geometry.0.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        (RouteEnd::Point(Point::new(10.0, 20.0)), RouteEnd::Box(kurbo::Rect::new(100.0, 100.0, 140.0, 120.0), Side::Left), Routing::Elbow)
    );
    // The straight route the geometry gave: from the free end to the middle of the box's left side.
    let line = named(&out.json, "Line");
    assert_eq!(line["size"], json!({"x": 90.0, "y": 90.0}));
    assert_eq!(line["vectorNetwork"]["segments"].as_array().map(Vec::len), Some(1));
    assert!(out.warnings.iter().any(|w| w.contains("routed afresh")), "{:?}", out.warnings);
}

#[test]
fn the_outlined_stroke_note_is_read_only_under_the_key_given() {
    let plugin = o(vec![("pluginID", s("writer")), ("key", s("stroke")), ("value", s("{\"weight\":3}"))]);
    let nodes = vec![
        node(0, None, "DOCUMENT", "D", vec![]),
        node(1, Some((0, "!")), "CANVAS", "P", vec![]),
        node(2, Some((1, "!")), "RECTANGLE", "R", vec![("size", xy(4.0, 4.0)), ("pluginData", arr(vec![plugin]))]),
    ];
    let msg = o(vec![("nodeChanges", arr(nodes))]);
    assert!(named(&mapped(&msg).json, "R").get("outlinedStroke").is_none());
    let out = to_rest(&msg, "x", &Options { outlined_stroke: Some(("writer", "stroke")), ..Options::default() });
    assert_eq!(named(&out.json, "R")["outlinedStroke"], json!({"weight": 3}));
}

#[test]
fn no_variable_table_outlives_the_mapping() {
    let msg = message();
    let _ = mapped(&msg);
    assert!(!VarCanon::installed());
}

#[test]
fn the_layer_cap_holds() {
    let mut nodes = vec![node(0, None, "DOCUMENT", "D", vec![]), node(1, Some((0, "!")), "CANVAS", "P", vec![])];
    nodes.extend((2..12).map(|i| node(i, Some((1, "!")), "RECTANGLE", "R", vec![("size", xy(1.0, 1.0))])));
    let msg = o(vec![("nodeChanges", arr(nodes))]);
    let out = to_rest(&msg, "x", &Options { max_nodes: 5, ..Options::default() });
    assert!(out.warnings.iter().any(|w| w.contains("more than 5 layers")), "{:?}", out.warnings);
    assert!(!mapped(&msg).warnings.iter().any(|w| w.contains("more than")));
    // The budget follows the compressed canvas, between its floor and ceiling.
    assert_eq!(layer_budget(0), MAX_NODES);
    assert!(layer_budget(83_491_232) > 894_004);
    assert_eq!(layer_budget(usize::MAX), MAX_FIG_NODES);
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

#[test]
fn mangled_messages_never_panic() {
    let sc = schema();
    let root = sc.find("Message").unwrap();
    let bytes = encode::message(&sc, root, &message()).unwrap();
    let mut r = Lcg(11);
    for cut in (0..bytes.len()).step_by(3) {
        if let Ok(kv) = sc.decode_message(root, &bytes[..cut]) {
            let _ = mapped(&kv);
        }
    }
    for _ in 0..1500 {
        let mut m = bytes.clone();
        for _ in 0..1 + r.next() % 4 {
            let i = (r.next() as usize) % m.len();
            m[i] = r.next() as u8;
        }
        if let Ok(kv) = sc.decode_message(root, &m) {
            let out = mapped(&kv);
            assert!(craft_figma_rest::check(&out.json, usize::MAX).is_ok());
        }
    }
}

/// The GUID written for "none".
fn empty_guid() -> Kv<'static> {
    o(vec![("sessionID", u(4_294_967_295)), ("localID", u(4_294_967_295))])
}

fn symbol(main: u64, overrides: Vec<Kv<'static>>) -> Kv<'static> {
    o(vec![("symbolID", g(main)), ("symbolOverrides", arr(overrides))])
}

fn row(path: &[u64], more: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    let mut fields = vec![("guidPath", o(vec![("guids", arr(path.iter().map(|id| g(*id)).collect()))]))];
    fields.extend(more);
    o(fields)
}

/// A Badge main with a label, an Error variant, a Card main holding a Badge instance whose
/// `overriddenSymbolID` is the empty GUID and a second one the Card swaps to Error, and a Card
/// instance whose override rows write the empty GUID over both.
fn cards() -> Mapped {
    let n = |id: u64, parent: u64, ty: &'static str, name: &str, more| node(id, Some((parent, &format!("{id:04}"))), ty, name, more);
    let msg = o(vec![(
        "nodeChanges",
        arr(vec![
            node(1, None, "DOCUMENT", "Document", vec![]),
            n(2, 1, "CANVAS", "Page", vec![]),
            n(10, 2, "SYMBOL", "Property 1=Informative", vec![]),
            n(11, 10, "RECTANGLE", "Label", vec![]),
            n(20, 2, "SYMBOL", "Property 1=Error", vec![]),
            n(21, 20, "RECTANGLE", "Error label", vec![]),
            n(30, 2, "SYMBOL", "Card", vec![]),
            n(31, 30, "INSTANCE", "Badge", vec![("symbolData", symbol(10, vec![])), ("overriddenSymbolID", empty_guid())]),
            n(32, 30, "INSTANCE", "Swapped badge", vec![("symbolData", symbol(10, vec![])), ("overriddenSymbolID", g(20))]),
            n(
                40,
                2,
                "INSTANCE",
                "Card copy",
                vec![(
                    "symbolData",
                    symbol(30, vec![row(&[31], vec![("overriddenSymbolID", empty_guid())]), row(&[32], vec![("overriddenSymbolID", empty_guid())])]),
                )],
            ),
        ]),
    )]);
    mapped(&msg)
}

/// The main of the `index`th child of `card`.
fn badge_main(json: &Value, card: &str, index: usize) -> Option<String> {
    let badge = &named(json, card)["children"][index];
    (badge["type"] == "INSTANCE").then(|| badge["componentId"].as_str().map(str::to_string)).flatten()
}

#[test]
fn an_empty_swap_guid_keeps_the_instance_of_its_main() {
    let out = cards();
    assert!(out.warnings.is_empty(), "no missing components: {:?}", out.warnings);
    for card in ["Card", "Card copy"] {
        assert_eq!(badge_main(&out.json, card, 0).as_deref(), Some("1:10"), "{card}");
        assert_eq!(named(&out.json, card)["children"][0]["children"][0]["name"], "Label", "{card}: the main's layers");
    }
}

#[test]
fn an_empty_swap_guid_on_a_row_resets_a_swap_below_it_to_the_default_main() {
    let out = cards();
    // In the Card main the badge is swapped to Error; the Card instance's row writes the empty
    // GUID over that swap: back to the main its symbolData names.
    assert_eq!(badge_main(&out.json, "Card", 1).as_deref(), Some("1:20"));
    assert_eq!(badge_main(&out.json, "Card copy", 1).as_deref(), Some("1:10"));
}

/// A three-point network blob whose points have no style of their own.
fn triangle_network() -> Vec<u8> {
    crate::network::encode(&crate::network::Network {
        vertices: vec![
            crate::network::Vertex { style: 0, x: 0.0, y: 0.0 },
            crate::network::Vertex { style: 0, x: 1.0, y: 0.0 },
            crate::network::Vertex { style: 0, x: 0.5, y: 1.0 },
        ],
        segments: (0..3).map(|i| crate::network::Segment { style: 0, a: i, ta: (0.0, 0.0), b: (i + 1) % 3, tb: (0.0, 0.0) }).collect(),
        regions: vec![crate::network::Region { style: 0, non_zero: true, loops: vec![vec![0, 1, 2]] }],
    })
}

#[test]
fn points_without_a_radius_of_their_own_take_the_layers() {
    let vd = || o(vec![("vectorNetworkBlob", u(0)), ("normalizedSize", xy(1.0, 1.0))]);
    let nodes = vec![
        node(0, None, "DOCUMENT", "D", vec![]),
        node(1, Some((0, "!")), "CANVAS", "P", vec![]),
        node(2, Some((1, "!")), "SYMBOL", "Main", vec![("size", xy(20.0, 20.0))]),
        node(3, Some((2, "!")), "VECTOR", "V", vec![("size", xy(20.0, 20.0)), ("vectorData", vd()), ("cornerRadius", f(4.0))]),
        // An instance overriding the vector's radius: its own network, not its main's.
        node(
            4,
            Some((1, "\"")),
            "INSTANCE",
            "I",
            vec![
                ("size", xy(20.0, 20.0)),
                (
                    "symbolData",
                    o(vec![
                        ("symbolID", g(2)),
                        ("symbolOverrides", arr(vec![o(vec![("guidPath", o(vec![("guids", arr(vec![g(3)]))])), ("cornerRadius", f(7.0))])])),
                    ]),
                ),
            ],
        ),
    ];
    let msg = o(vec![("nodeChanges", arr(nodes)), ("blobs", arr(vec![o(vec![("bytes", Kv::Bytes(triangle_network()))])]))]);
    let out = mapped(&msg);
    let radii = |r: &Value| -> Vec<f64> {
        let net = out.networks[r.as_u64().unwrap() as usize].as_ref().unwrap();
        net.vertices.iter().map(|v| v.radius).collect()
    };
    assert_eq!(radii(&named(&out.json, "Main")["children"][0]["vectorNetworkRef"]), [4.0, 4.0, 4.0]);
    let in_instance = &named(&out.json, "I")["children"][0];
    assert_eq!(radii(&in_instance["vectorNetworkRef"]), [7.0, 7.0, 7.0]);
}

/// 270 frames inside one another, or 270 mains each holding an instance of the next: past the
/// nesting limit either way.
fn nested(instances: bool) -> Kv<'static> {
    let mut nodes = vec![node(0, None, "DOCUMENT", "D", vec![]), node(1, Some((0, "!")), "CANVAS", "P", vec![])];
    let levels = 270;
    if instances {
        for i in 0..levels {
            let id = 10 + 2 * i;
            nodes.push(node(id, Some((1, "!")), "SYMBOL", "M", vec![("size", xy(1.0, 1.0))]));
            if i + 1 < levels {
                nodes.push(node(
                    id + 1,
                    Some((id, "!")),
                    "INSTANCE",
                    "I",
                    vec![("size", xy(1.0, 1.0)), ("symbolData", o(vec![("symbolID", g(id + 2))]))],
                ));
            }
        }
        nodes.push(node(5, Some((1, "\"")), "INSTANCE", "Top", vec![("size", xy(1.0, 1.0)), ("symbolData", o(vec![("symbolID", g(10))]))]));
    } else {
        let mut parent = 1;
        for i in 0..levels {
            nodes.push(node(10 + i, Some((parent, "!")), "FRAME", "F", vec![("size", xy(1.0, 1.0))]));
            parent = 10 + i;
        }
    }
    o(vec![("nodeChanges", arr(nodes))])
}

#[test]
fn deep_nesting_stops_at_the_limit_within_a_small_stack() {
    // The walk recurses once per level, in small frames: past the limit of 256 it fits the 1 MiB
    // stack of a WebAssembly build (an unoptimised build maps it on 512 KiB).
    let run = || {
        for instances in [false, true] {
            let msg = nested(instances);
            let out = mapped(&msg);
            assert!(
                out.warnings.iter().any(|w| w.starts_with(&format!("layers nested deeper than {MAX_DEPTH} levels were skipped"))),
                "{:?}",
                out.warnings
            );
            assert!(craft_figma_rest::check(&out.json, usize::MAX).is_ok());
        }
    };
    std::thread::Builder::new().stack_size(1 << 20).spawn(run).unwrap().join().unwrap();
}
