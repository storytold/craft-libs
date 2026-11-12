//! A typed model of the JSON members of a Penpot export, named as the files name their fields.
//!
//! Every record keeps the fields it has no name for in `extra`, in the JSON they came in, and
//! writes them back: a member read and written loses nothing but `null` fields (left out), and
//! whole numbers come back as floats (`10` as `10.0`). Enumerations keep values they don't know
//! (`Other`). A field of the wrong type fails the read; a reader that must skip bad fields one by
//! one reads the [`serde_json::Value`] instead.
//!
//! Typography sizes and spacings are numbers or numeric strings in the files; they are kept as
//! [`serde_json::Value`]. Text and path `content`, SVG attributes, interactions and layout grid
//! cells are kept as JSON too.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A record: every field optional, plus `extra` for the fields this model has no name for.
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

/// An enumeration Penpot may add values to: the values known here, and `Other` for the rest,
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
            /// The value as the files write it.
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
    /// A shape's `type`.
    ShapeType {
        Frame = "frame", Group = "group", Rect = "rect", Circle = "circle", Path = "path", Text = "text", Image = "image",
        SvgRaw = "svg-raw", Bool = "bool",
    }
}

open_enum! {
    /// A gradient's `type`.
    GradientType { Linear = "linear", Radial = "radial", }
}

open_enum! {
    /// How a shape or a shadow blends with what is under it.
    BlendMode {
        Normal = "normal", Darken = "darken", Multiply = "multiply", ColorBurn = "color-burn", Lighten = "lighten", Screen = "screen",
        ColorDodge = "color-dodge", Overlay = "overlay", SoftLight = "soft-light", HardLight = "hard-light", Difference = "difference",
        Exclusion = "exclusion", Hue = "hue", Saturation = "saturation", Color = "color", Luminosity = "luminosity",
    }
}

/// A point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// An affine matrix: `[a c e; b d f]`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Matrix {
    fn default() -> Self {
        Matrix { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 }
    }
}

record! {
    /// A selection rectangle: its box and its corners.
    SelRect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    }
}

record! {
    /// A gradient stop.
    Stop {
        /// `#rrggbb`.
        color: String,
        opacity: f64,
        offset: f64,
    }
}

record! {
    /// A linear or radial gradient, in 0–1 of the shape's box.
    Gradient {
        #[serde(rename = "type")]
        kind: GradientType,
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        width: f64,
        stops: Vec<Stop>,
    }
}

record! {
    /// An image a fill or stroke shows: `id` names the media record (`files/<file>/media/<id>.json`).
    ImageRef {
        id: String,
        name: String,
        width: f64,
        height: f64,
        mtype: String,
        keep_aspect_ratio: bool,
    }
}

record! {
    /// One fill.
    Fill {
        /// `#rrggbb`.
        fill_color: String,
        fill_opacity: f64,
        fill_color_gradient: Gradient,
        fill_image: ImageRef,
        /// A library colour the fill uses: its id and file.
        fill_color_ref_id: String,
        fill_color_ref_file: String,
    }
}

record! {
    /// One stroke.
    Stroke {
        stroke_color: String,
        stroke_opacity: f64,
        stroke_width: f64,
        /// `inner`, `center` or `outer`.
        stroke_alignment: String,
        /// `solid`, `dotted`, `dashed`, `mixed` or `none`.
        stroke_style: String,
        /// `round`, `square`, `line-arrow`, `triangle-arrow`, `circle-marker`, `diamond-marker`, …
        stroke_cap_start: String,
        stroke_cap_end: String,
        stroke_color_gradient: Gradient,
        stroke_image: ImageRef,
        stroke_color_ref_id: String,
        stroke_color_ref_file: String,
    }
}

record! {
    /// A shadow's colour.
    ShadowColor {
        color: String,
        opacity: f64,
    }
}

record! {
    /// One shadow.
    Shadow {
        id: String,
        /// `drop-shadow` or `inner-shadow`.
        style: String,
        offset_x: f64,
        offset_y: f64,
        blur: f64,
        spread: f64,
        hidden: bool,
        color: ShadowColor,
    }
}

record! {
    /// A blur.
    Blur {
        id: String,
        /// `layer-blur` or `background-blur`.
        #[serde(rename = "type")]
        kind: String,
        value: f64,
        hidden: bool,
    }
}

record! {
    /// One shape (`files/<file>/pages/<page>/<shape>.json`). One record for every shape type: which
    /// fields a shape carries follows from its `type`.
    Shape {
        id: String,
        name: String,
        #[serde(rename = "type")]
        kind: ShapeType,
        /// The shape it is in, and the frame it is in (both the page's root frame on top).
        parent_id: String,
        frame_id: String,
        page_id: String,
        /// Its children's ids, bottom first (frames, groups and booleans).
        shapes: Vec<String>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        /// In degrees.
        rotation: f64,
        selrect: SelRect,
        /// Its corners on the page: top left, top right, bottom right, bottom left.
        points: Vec<Point>,
        transform: Matrix,
        transform_inverse: Matrix,
        flip_x: bool,
        flip_y: bool,
        proportion_lock: bool,
        hidden: bool,
        blocked: bool,
        opacity: f64,
        blend_mode: BlendMode,
        fills: Vec<Fill>,
        strokes: Vec<Stroke>,
        shadow: Vec<Shadow>,
        blur: Blur,
        background_blur: Blur,
        /// Corner radii: top left, top right, bottom right, bottom left.
        r1: f64,
        r2: f64,
        r3: f64,
        r4: f64,
        rx: f64,
        ry: f64,
        /// Text (a tree of root, paragraph sets, paragraphs and spans) or path commands.
        content: Value,
        /// Texts: `fixed`, `auto-width` or `auto-height`.
        grow_type: String,
        /// Booleans: `union`, `difference`, `intersection` or `exclude`.
        bool_type: String,
        masked_group: bool,
        /// `left`, `right`, `leftright`, `center`, `scale`; and `top`, `bottom`, `topbottom`, ….
        constraints_h: String,
        constraints_v: String,
        /// Display grids.
        grids: Vec<Value>,
        /// Flex or grid layout: `flex` or `grid`.
        layout: String,
        /// `row`, `row-reverse`, `column`, `column-reverse`.
        layout_flex_dir: String,
        layout_gap: Value,
        layout_gap_type: String,
        layout_padding: Value,
        layout_padding_type: String,
        layout_wrap_type: String,
        layout_justify_content: String,
        layout_align_items: String,
        layout_align_content: String,
        layout_grid_dir: String,
        layout_grid_rows: Vec<Value>,
        layout_grid_columns: Vec<Value>,
        layout_grid_cells: Value,
        layout_item_h_sizing: String,
        layout_item_v_sizing: String,
        layout_item_absolute: bool,
        layout_item_margin: Value,
        layout_item_align_self: String,
        /// Components: the component a copy is of, its file, and whether this is the main.
        component_id: String,
        component_file: String,
        component_root: bool,
        main_instance: bool,
        shape_ref: String,
        svg_attrs: Value,
        interactions: Vec<Value>,
        /// Design tokens applied to the shape's properties: property → token name.
        applied_tokens: BTreeMap<String, String>,
        hide_fill_on_export: bool,
    }
}

record! {
    /// A ruler guide of a page.
    Guide {
        id: String,
        /// `x` (vertical) or `y` (horizontal).
        axis: String,
        position: f64,
        frame_id: String,
    }
}

record! {
    /// A prototype flow of a page.
    Flow {
        id: String,
        name: String,
        starting_frame: String,
    }
}

record! {
    /// A page (`files/<file>/pages/<page>.json`).
    Page {
        id: String,
        name: String,
        index: f64,
        /// `#rrggbb`.
        background: String,
        guides: BTreeMap<String, Guide>,
        flows: BTreeMap<String, Flow>,
        options: Value,
    }
}

record! {
    /// A file's header (`files/<file>.json`).
    FileHeader {
        id: String,
        name: String,
        features: Vec<String>,
        version: f64,
        migrations: Vec<Value>,
        options: Value,
        is_shared: bool,
        created_at: String,
        modified_at: String,
        revn: f64,
        vern: f64,
    }
}

record! {
    /// One entry of the manifest's `files`.
    ManifestFile {
        id: String,
        name: String,
        features: Vec<String>,
    }
}

record! {
    /// `manifest.json`.
    Manifest {
        #[serde(rename = "type")]
        kind: String,
        version: f64,
        generated_by: String,
        refer: String,
        files: Vec<ManifestFile>,
        relations: Vec<Value>,
    }
}

record! {
    /// A library colour (`files/<file>/colors/<id>.json`).
    LibraryColor {
        id: String,
        name: String,
        path: String,
        color: String,
        opacity: f64,
        gradient: Gradient,
        image: ImageRef,
    }
}

record! {
    /// A library typography (`files/<file>/typographies/<id>.json`). Sizes are numbers or numeric
    /// strings.
    Typography {
        id: String,
        name: String,
        path: String,
        font_id: String,
        font_family: String,
        font_variant_id: String,
        font_size: Value,
        font_weight: Value,
        font_style: String,
        line_height: Value,
        letter_spacing: Value,
        text_transform: String,
    }
}

record! {
    /// A component (`files/<file>/components/<id>.json`).
    Component {
        id: String,
        name: String,
        path: String,
        main_instance_id: String,
        main_instance_page: String,
        variant_id: String,
        variant_properties: Vec<Value>,
    }
}

record! {
    /// A media record (`files/<file>/media/<id>.json`): `media_id` names the stored object.
    Media {
        id: String,
        name: String,
        width: f64,
        height: f64,
        mtype: String,
        media_id: String,
        is_local: bool,
    }
}
