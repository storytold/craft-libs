//! The records of the file JSON, named as the REST API reference names them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::ext::VectorNetwork;

/// A record: every field optional (the API leaves out what has its default), plus `extra` for the
/// fields this model has no name for.
macro_rules! record {
    ($(#[$meta:meta])* $name:ident { $($(#[$fmeta:meta])* $field:ident: $ty:ty,)* }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct $name {
            $(
                $(#[$fmeta])*
                #[serde(default, skip_serializing_if = "Option::is_none")]
                pub $field: Option<$ty>,
            )*
            /// Fields this model has no name for, as the JSON they came in.
            #[serde(flatten)]
            pub extra: Map<String, Value>,
        }
    };
}

/// An enumeration the API may add values to: the values known here, and `Other` for the rest,
/// written back as it was read.
macro_rules! open_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $text:literal,)* }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(from = "String", into = "String")]
        pub enum $name {
            $($variant,)*
            /// A value this model has no name for.
            Other(String),
        }

        impl $name {
            /// The value as the API writes it.
            pub fn as_str(&self) -> &str {
                match self {
                    $($name::$variant => $text,)*
                    $name::Other(s) => s,
                }
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                match s.as_str() {
                    $($text => $name::$variant,)*
                    _ => $name::Other(s),
                }
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> String {
                match v {
                    $name::Other(s) => s,
                    known => known.as_str().to_string(),
                }
            }
        }
    };
}

open_enum! {
    /// A node's `type`.
    NodeType {
        Document = "DOCUMENT", Canvas = "CANVAS", Frame = "FRAME", Group = "GROUP", Section = "SECTION", Vector = "VECTOR",
        BooleanOperation = "BOOLEAN_OPERATION", Star = "STAR", Line = "LINE", Ellipse = "ELLIPSE", RegularPolygon = "REGULAR_POLYGON",
        Rectangle = "RECTANGLE", Text = "TEXT", Slice = "SLICE", Component = "COMPONENT", ComponentSet = "COMPONENT_SET", Instance = "INSTANCE",
        Sticky = "STICKY", ShapeWithText = "SHAPE_WITH_TEXT", Connector = "CONNECTOR", Table = "TABLE", TableCell = "TABLE_CELL",
        WashiTape = "WASHI_TAPE", Slot = "SLOT",
    }
}

open_enum! {
    /// A paint's `type`.
    PaintType {
        Solid = "SOLID", GradientLinear = "GRADIENT_LINEAR", GradientRadial = "GRADIENT_RADIAL", GradientAngular = "GRADIENT_ANGULAR",
        GradientDiamond = "GRADIENT_DIAMOND", Image = "IMAGE", Emoji = "EMOJI", Video = "VIDEO", Pattern = "PATTERN",
    }
}

open_enum! {
    /// An effect's `type`.
    EffectType {
        InnerShadow = "INNER_SHADOW", DropShadow = "DROP_SHADOW", LayerBlur = "LAYER_BLUR", BackgroundBlur = "BACKGROUND_BLUR",
        Texture = "TEXTURE", Noise = "NOISE", Glass = "GLASS",
    }
}

open_enum! {
    /// How a layer or a paint blends with what is under it.
    BlendMode {
        PassThrough = "PASS_THROUGH", Normal = "NORMAL", Darken = "DARKEN", Multiply = "MULTIPLY", LinearBurn = "LINEAR_BURN",
        ColorBurn = "COLOR_BURN", Lighten = "LIGHTEN", Screen = "SCREEN", LinearDodge = "LINEAR_DODGE", ColorDodge = "COLOR_DODGE",
        Overlay = "OVERLAY", SoftLight = "SOFT_LIGHT", HardLight = "HARD_LIGHT", Difference = "DIFFERENCE", Exclusion = "EXCLUSION",
        Hue = "HUE", Saturation = "SATURATION", Color = "COLOR", Luminosity = "LUMINOSITY",
    }
}

open_enum! {
    /// The kind of a shared style.
    StyleType { Fill = "FILL", Text = "TEXT", Effect = "EFFECT", Grid = "GRID", }
}

/// A colour: red, green, blue and alpha, each 0–1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    #[serde(default = "one")]
    pub a: f64,
}

fn one() -> f64 {
    1.0
}

/// A point or an offset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vector {
    pub x: f64,
    pub y: f64,
}

/// A bounding box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rectangle {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A 2 × 3 affine matrix, row by row: `[[a, c, tx], [b, d, ty]]`.
pub type Transform = [[f64; 3]; 2];

/// A gradient stop.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorStop {
    pub position: f64,
    pub color: Color,
    /// Fields this model has no name for (`boundVariables`).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

record! {
    /// A fill or a stroke paint.
    Paint {
        #[serde(rename = "type")]
        kind: PaintType,
        visible: bool,
        opacity: f64,
        blend_mode: BlendMode,
        /// Solid paints.
        color: Color,
        /// Gradients: the start, the end, and a third point giving the width.
        gradient_handle_positions: Vec<Vector>,
        gradient_stops: Vec<ColorStop>,
        /// Images: `FILL`, `FIT`, `TILE` or `STRETCH`.
        scale_mode: String,
        /// Images: the key of the image (`GET /v1/files/:key/images` gives its URL; a `.fig`
        /// names its `images/` members by it).
        image_ref: String,
        /// Images with `scaleMode` `STRETCH`.
        image_transform: Transform,
        /// Images with `scaleMode` `TILE`.
        scaling_factor: f64,
        /// Images: rotation in degrees.
        rotation: f64,
        gif_ref: String,
    }
}

record! {
    /// A shadow, a blur or another effect.
    Effect {
        #[serde(rename = "type")]
        kind: EffectType,
        visible: bool,
        radius: f64,
        color: Color,
        blend_mode: BlendMode,
        offset: Vector,
        spread: f64,
        show_shadow_behind_node: bool,
    }
}

record! {
    /// How a layer follows its parent when the parent is resized.
    LayoutConstraint {
        /// `TOP`, `BOTTOM`, `CENTER`, `TOP_BOTTOM` or `SCALE`.
        vertical: String,
        /// `LEFT`, `RIGHT`, `CENTER`, `LEFT_RIGHT` or `SCALE`.
        horizontal: String,
    }
}

record! {
    /// A layout guide of a frame.
    LayoutGrid {
        /// `ROWS`, `COLUMNS` or `GRID`.
        pattern: String,
        section_size: f64,
        visible: bool,
        color: Color,
        /// `MIN`, `MAX`, `CENTER` or `STRETCH`.
        alignment: String,
        gutter_size: f64,
        offset: f64,
        count: f64,
    }
}

record! {
    /// One export preset of a layer.
    ExportSetting {
        suffix: String,
        /// `JPG`, `PNG`, `SVG` or `PDF`.
        format: String,
        /// `{type: SCALE | WIDTH | HEIGHT, value}`.
        constraint: Value,
    }
}

record! {
    /// An outline in the node's own space.
    Path {
        /// SVG path data.
        path: String,
        /// `NONZERO` or `EVENODD`.
        winding_rule: String,
        override_id: f64,
    }
}

record! {
    /// The sweep of an ellipse: angles in radians, the inner radius as a fraction.
    ArcData {
        starting_angle: f64,
        ending_angle: f64,
        inner_radius: f64,
    }
}

record! {
    /// The style of a text node, or of a run in its `styleOverrideTable`.
    TypeStyle {
        font_family: String,
        font_post_script_name: String,
        font_style: String,
        italic: bool,
        font_weight: f64,
        font_size: f64,
        /// `ORIGINAL`, `UPPER`, `LOWER`, `TITLE`, `SMALL_CAPS` or `SMALL_CAPS_FORCED`.
        text_case: String,
        /// `NONE`, `STRIKETHROUGH` or `UNDERLINE`.
        text_decoration: String,
        /// `NONE`, `HEIGHT`, `WIDTH_AND_HEIGHT` or `TRUNCATE`.
        text_auto_resize: String,
        /// `DISABLED` or `ENDING`.
        text_truncation: String,
        max_lines: f64,
        /// `LEFT`, `RIGHT`, `CENTER` or `JUSTIFIED`.
        text_align_horizontal: String,
        /// `TOP`, `CENTER` or `BOTTOM`.
        text_align_vertical: String,
        /// In pixels.
        letter_spacing: f64,
        fills: Vec<Paint>,
        /// `{type: URL | NODE, url | nodeID}`.
        hyperlink: Value,
        /// OpenType feature tags and whether each is on (1) or off (0).
        opentype_flags: BTreeMap<String, f64>,
        line_height_px: f64,
        line_height_percent_font_size: f64,
        /// `PIXELS`, `FONT_SIZE_%` or `INTRINSIC_%`.
        line_height_unit: String,
        paragraph_spacing: f64,
        paragraph_indent: f64,
        list_spacing: f64,
    }
}

record! {
    /// A layer's own properties ([`Node`] adds its children). One record for every node type, as
    /// the API's node types share most properties: which ones a node carries follows from its
    /// `type`.
    NodeProps {
        id: String,
        name: String,
        #[serde(rename = "type")]
        kind: NodeType,
        visible: bool,
        locked: bool,
        /// In radians.
        rotation: f64,
        blend_mode: BlendMode,
        opacity: f64,
        absolute_bounding_box: Rectangle,
        absolute_render_bounds: Rectangle,
        /// The node's place in its parent (asked for with `geometry=paths`).
        relative_transform: Transform,
        /// Width and height before the transform (asked for with `geometry=paths`).
        size: Vector,
        constraints: LayoutConstraint,
        clips_content: bool,
        /// A canvas's background.
        background_color: Color,
        fills: Vec<Paint>,
        strokes: Vec<Paint>,
        stroke_weight: f64,
        /// `{top, right, bottom, left}`.
        individual_stroke_weights: Value,
        /// `INSIDE`, `OUTSIDE` or `CENTER`.
        stroke_align: String,
        /// `MITER`, `BEVEL` or `ROUND`.
        stroke_join: String,
        /// `NONE`, `ROUND`, `SQUARE`, `LINE_ARROW`, `TRIANGLE_ARROW`, …
        stroke_cap: String,
        stroke_dashes: Vec<f64>,
        stroke_miter_angle: f64,
        fill_geometry: Vec<Path>,
        stroke_geometry: Vec<Path>,
        corner_radius: f64,
        corner_smoothing: f64,
        /// Top left, top right, bottom right, bottom left.
        rectangle_corner_radii: [f64; 4],
        effects: Vec<Effect>,
        is_mask: bool,
        /// `ALPHA`, `VECTOR` or `LUMINANCE`.
        mask_type: String,
        export_settings: Vec<ExportSetting>,
        layout_grids: Vec<LayoutGrid>,
        /// The shared styles the node uses: `fill`, `stroke`, `text`, `effect`, `grid` → style id.
        styles: BTreeMap<String, String>,

        /// Auto layout: `NONE`, `HORIZONTAL`, `VERTICAL` or `GRID`.
        layout_mode: String,
        /// `NO_WRAP` or `WRAP`.
        layout_wrap: String,
        /// `FIXED` or `AUTO`.
        primary_axis_sizing_mode: String,
        counter_axis_sizing_mode: String,
        /// `MIN`, `CENTER`, `MAX` or `SPACE_BETWEEN`.
        primary_axis_align_items: String,
        /// `MIN`, `CENTER`, `MAX` or `BASELINE`.
        counter_axis_align_items: String,
        /// `AUTO` or `SPACE_BETWEEN`.
        counter_axis_align_content: String,
        padding_left: f64,
        padding_right: f64,
        padding_top: f64,
        padding_bottom: f64,
        item_spacing: f64,
        counter_axis_spacing: f64,
        item_reverse_z_index: bool,
        strokes_included_in_layout: bool,
        /// As a child of an auto layout: `INHERIT`, `STRETCH`, `MIN`, `CENTER`, `MAX`.
        layout_align: String,
        layout_grow: f64,
        /// `AUTO` or `ABSOLUTE`.
        layout_positioning: String,
        /// `FIXED`, `HUG` or `FILL`.
        layout_sizing_horizontal: String,
        layout_sizing_vertical: String,
        min_width: f64,
        max_width: f64,
        min_height: f64,
        max_height: f64,
        /// `NONE`, `HORIZONTAL_SCROLLING`, `VERTICAL_SCROLLING` or `HORIZONTAL_AND_VERTICAL_SCROLLING`.
        overflow_direction: String,

        /// Text: the characters, the base style, and for each character an index into
        /// `styleOverrideTable` (0: the base style).
        characters: String,
        style: TypeStyle,
        character_style_overrides: Vec<f64>,
        style_override_table: BTreeMap<String, TypeStyle>,
        /// Per line: `NONE`, `ORDERED` or `UNORDERED`.
        line_types: Vec<String>,
        line_indentations: Vec<f64>,

        /// An instance's main component.
        component_id: String,
        /// `UNION`, `INTERSECT`, `SUBTRACT` or `EXCLUDE`.
        boolean_operation: String,
        arc_data: ArcData,
        section_contents_hidden: bool,

        /// Extension ([`crate::ext`]): the editable network of a vector.
        vector_network: VectorNetwork,
    }
}

record! {
    /// A component's entry in the file's `components`.
    Component {
        key: String,
        name: String,
        description: String,
        component_set_id: String,
        remote: bool,
    }
}

record! {
    /// A component set's entry in the file's `componentSets`.
    ComponentSet {
        key: String,
        name: String,
        description: String,
        remote: bool,
    }
}

record! {
    /// A shared style's entry in the file's `styles`.
    Style {
        key: String,
        name: String,
        description: String,
        remote: bool,
        style_type: StyleType,
    }
}

record! {
    /// The response of `GET /v1/files/:key`.
    File {
        name: String,
        last_modified: String,
        thumbnail_url: String,
        version: String,
        schema_version: f64,
        /// The `DOCUMENT` node: its children are the `CANVAS` pages.
        document: Node,
        /// By node id.
        components: BTreeMap<String, Component>,
        component_sets: BTreeMap<String, ComponentSet>,
        styles: BTreeMap<String, Style>,
    }
}

record! {
    /// One entry of the `nodes` map of `GET /v1/files/:key/nodes`.
    FileNode {
        document: Node,
        components: BTreeMap<String, Component>,
        component_sets: BTreeMap<String, ComponentSet>,
        styles: BTreeMap<String, Style>,
        schema_version: f64,
    }
}

record! {
    /// The response of `GET /v1/files/:key/nodes`. An entry is `None` for an id the file lacks.
    FileNodes {
        name: String,
        nodes: BTreeMap<String, Option<FileNode>>,
    }
}

/// A layer: its properties (read through the node itself: `node.fills`) and its children.
///
/// Reading and writing a node does not recurse through its layers: the tree is taken apart and
/// put together level by level, so a deep file needs no deep stack.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub props: NodeProps,
    /// `None` for a node without a `children` array (a leaf); `Some(vec![])` for an empty one.
    pub children: Option<Vec<Node>>,
}

impl std::ops::Deref for Node {
    type Target = NodeProps;
    fn deref(&self) -> &NodeProps {
        &self.props
    }
}

impl std::ops::DerefMut for Node {
    fn deref_mut(&mut self) -> &mut NodeProps {
        &mut self.props
    }
}

impl Node {
    /// A leaf with `props`.
    pub fn new(props: NodeProps) -> Node {
        Node { props, children: None }
    }

    /// A node with `props` and `children`.
    pub fn with_children(props: NodeProps, children: Vec<Node>) -> Node {
        Node { props, children: Some(children) }
    }

    /// The node and its subtree from JSON, level by level. A `children` that is not an array is
    /// kept as an unnamed field.
    pub(crate) fn build(v: Value) -> Result<Node, serde_json::Error> {
        // The tree in pre-order, each node without its children, with how many it had.
        let mut order: Vec<(Value, Option<usize>)> = Vec::new();
        let mut pending = vec![v];
        while let Some(mut v) = pending.pop() {
            let kids = match &mut v {
                Value::Object(o) if o.get("children").is_some_and(Value::is_array) => match o.remove("children") {
                    Some(Value::Array(a)) => Some(a),
                    _ => None,
                },
                _ => None,
            };
            let count = kids.as_ref().map(Vec::len);
            pending.extend(kids.into_iter().flatten().rev());
            order.push((v, count));
        }
        // Backwards, every node finds its children built already, the first on top.
        let mut built: Vec<Node> = Vec::new();
        for (v, count) in order.into_iter().rev() {
            let props: NodeProps = serde_json::from_value(v)?;
            let children = count.map(|n| {
                let mut kids = built.split_off(built.len().saturating_sub(n));
                kids.reverse();
                kids
            });
            built.push(Node { props, children });
        }
        Ok(built.pop().unwrap_or_default())
    }

    /// The node and its subtree as JSON, level by level.
    pub(crate) fn unbuild(&self) -> Result<Value, serde_json::Error> {
        let mut order: Vec<&Node> = Vec::new();
        let mut pending = vec![self];
        while let Some(node) = pending.pop() {
            pending.extend(node.children.iter().flatten().rev());
            order.push(node);
        }
        let mut built: Vec<Value> = Vec::new();
        for node in order.into_iter().rev() {
            let mut v = serde_json::to_value(&node.props)?;
            if let (Some(kids), Value::Object(o)) = (&node.children, &mut v) {
                let mut kids = built.split_off(built.len().saturating_sub(kids.len()));
                kids.reverse();
                o.insert("children".into(), Value::Array(kids));
            }
            built.push(v);
        }
        Ok(built.pop().unwrap_or_default())
    }
}

/// Through [`Value`], so that only the JSON tree (small frames) is walked by recursion.
impl Serialize for Node {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.unbuild().map_err(serde::ser::Error::custom)?.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Node, D::Error> {
        Node::build(Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
