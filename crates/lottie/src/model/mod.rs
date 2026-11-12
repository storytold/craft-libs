//! The document model: the objects of a Lottie file as the file has them.
//!
//! Each struct is one object of the lottie-spec 1.0 object model, with a field per key it defines.
//! Every field is optional, because files leave keys out. A key the struct does not name, and a
//! key whose value has another type than the struct expects, stay in `extra` as they were, so
//! reading a document and writing it back keeps all of it.
//!
//! Animatable properties are [`Animated`]: the JSON of the property, read with
//! [`Animated::read`]. The meaning of a value depends on the property, which the file does not
//! say, so the model does not decode values.

mod layer;
mod shape;
mod text;

pub use layer::{Effect, Layer, LayerContent, Mask, PrecompLayer, SolidLayer, Transform};
pub use shape::{
    Dash, Ellipse, Fill, GradientColors, GradientFill, GradientStroke, Group, Merge, OffsetPath, Path, PolyStar, PuckerBloat, Rect, Repeater,
    RepeaterTransform, RoundCorners, Shape, ShapeKind, Stroke, Trim, Twist, ZigZag,
};
pub use text::{TextAlign, TextData, TextDocKey, TextDocs, TextDocument, TextFollowPath, TextRange, TextSelector, TextStyle};

use crate::{Json, KeyOut, LottieError, Prop, read_prop, write_prop};

/// A JSON object.
pub type Map = serde_json::Map<String, Json>;

/// A value of the model as JSON. `from_json` gives the value back when it has another type, so
/// the caller can keep it.
pub(crate) trait Lenient: Sized {
    fn from_json(j: Json) -> Result<Self, Json>;
    fn to_json(&self) -> Json;
}

/// A number as JSON: whole numbers without a fraction part, as files have them.
fn number(v: f64) -> Json {
    if v.fract() == 0.0 && v.abs() < 9.0e15 { Json::from(v as i64) } else { Json::from(v) }
}

impl Lenient for f64 {
    fn from_json(j: Json) -> Result<Self, Json> {
        j.as_f64().ok_or(j)
    }
    fn to_json(&self) -> Json {
        number(*self)
    }
}

impl Lenient for i64 {
    /// A whole number, however it is written (`1`, `1.0`).
    fn from_json(j: Json) -> Result<Self, Json> {
        let whole = j.as_i64().or_else(|| j.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 9.0e15).map(|f| f as i64));
        whole.ok_or(j)
    }
    fn to_json(&self) -> Json {
        Json::from(*self)
    }
}

impl Lenient for bool {
    /// `true` / `false`, or 1 / 0 (older files).
    fn from_json(j: Json) -> Result<Self, Json> {
        match (j.as_bool(), j.as_f64()) {
            (Some(b), _) => Ok(b),
            (None, Some(n)) if n == 0.0 || n == 1.0 => Ok(n == 1.0),
            _ => Err(j),
        }
    }
    fn to_json(&self) -> Json {
        Json::Bool(*self)
    }
}

impl Lenient for String {
    fn from_json(j: Json) -> Result<Self, Json> {
        match j {
            Json::String(s) => Ok(s),
            other => Err(other),
        }
    }
    fn to_json(&self) -> Json {
        Json::String(self.clone())
    }
}

impl Lenient for Map {
    fn from_json(j: Json) -> Result<Self, Json> {
        match j {
            Json::Object(m) => Ok(m),
            other => Err(other),
        }
    }
    fn to_json(&self) -> Json {
        Json::Object(self.clone())
    }
}

impl<T: Lenient> Lenient for Vec<T> {
    /// A list. Entries of another type than `T` are left out.
    fn from_json(j: Json) -> Result<Self, Json> {
        match j {
            Json::Array(a) => Ok(a.into_iter().filter_map(|v| T::from_json(v).ok()).collect()),
            other => Err(other),
        }
    }
    fn to_json(&self) -> Json {
        Json::Array(self.iter().map(T::to_json).collect())
    }
}

/// Takes key `k` out of `m` as a `T`; a value of another type stays in `m`.
fn take<T: Lenient>(m: &mut Map, k: &str) -> Option<T> {
    match T::from_json(m.remove(k)?) {
        Ok(v) => Some(v),
        Err(j) => {
            m.insert(k.to_string(), j);
            None
        }
    }
}

/// A struct of optional fields, each one key of a JSON object: `field / "key": Type`. `take`
/// moves the keys out of an object and `put` writes them back. Fields after `+` are plain.
macro_rules! fields {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $field:ident / $json:literal : $ty:ty,)* } $(+ { $($(#[$xm:meta])* $xf:ident : $xt:ty,)* })?) => {
        $(#[$m])*
        #[derive(Debug, Clone, Default, PartialEq)]
        pub struct $name {
            $($(#[$fm])* pub $field: Option<$ty>,)*
            $($($(#[$xm])* pub $xf: $xt,)*)?
        }

        impl $name {
            #[allow(clippy::needless_update, dead_code)]
            pub(crate) fn take(m: &mut $crate::model::Map) -> Self {
                Self { $($field: $crate::model::take(m, $json),)* ..Default::default() }
            }

            #[allow(dead_code)]
            pub(crate) fn put(&self, m: &mut $crate::model::Map) {
                $(if let Some(v) = &self.$field {
                    m.insert($json.to_string(), $crate::model::Lenient::to_json(v));
                })*
            }
        }
    };
}

/// A [`fields!`] struct that is a whole JSON object: the keys it does not name stay in `extra`.
macro_rules! object {
    ($(#[$m:meta])* $name:ident { $($body:tt)* }) => {
        $crate::model::fields! {
            $(#[$m])*
            $name { $($body)* } + {
                /// Every other key of the object, as it was.
                extra: $crate::model::Map,
            }
        }

        impl $crate::model::Lenient for $name {
            fn from_json(j: $crate::Json) -> Result<Self, $crate::Json> {
                match j {
                    $crate::Json::Object(mut m) => {
                        let mut o = Self::take(&mut m);
                        o.extra = m;
                        Ok(o)
                    }
                    other => Err(other),
                }
            }

            fn to_json(&self) -> $crate::Json {
                let mut m = self.extra.clone();
                self.put(&mut m);
                $crate::Json::Object(m)
            }
        }
    };
}
pub(crate) use {fields, object};

/// An animatable property as the file has it: `{a, k}` with keyframes or one value, sometimes
/// the bare value.
#[derive(Debug, Clone, PartialEq)]
pub struct Animated(pub Json);

impl Animated {
    /// The property's value or keyframes, and its expression.
    pub fn read(&self) -> Prop<'_> {
        read_prop(&self.0)
    }

    /// A property with one value.
    pub fn fixed(value: impl Into<Json>) -> Self {
        Self(write_prop(value.into(), &[], None))
    }

    /// A keyframed property.
    pub fn keyed(keys: &[KeyOut], expression: Option<&str>) -> Self {
        Self(write_prop(Json::Null, keys, expression))
    }

    /// The `x`, `y` and `z` properties of a position split in dimensions (`{s: true, x, y, z}`).
    pub fn split(&self) -> Option<[Option<&Json>; 3]> {
        let on = self.0.get("s").is_some_and(|s| s.as_bool() == Some(true) || s.as_f64() == Some(1.0));
        on.then(|| ["x", "y", "z"].map(|k| self.0.get(k)))
    }
}

impl Lenient for Animated {
    fn from_json(j: Json) -> Result<Self, Json> {
        if j.is_null() { Err(j) } else { Ok(Self(j)) }
    }
    fn to_json(&self) -> Json {
        self.0.clone()
    }
}

object! {
    /// The document.
    Animation {
        /// The version of the exporter that wrote the file.
        version / "v": String,
        name / "nm": String,
        match_name / "mn": String,
        /// Frames per second.
        frame_rate / "fr": f64,
        /// The first frame.
        in_point / "ip": f64,
        /// The frame after the last.
        out_point / "op": f64,
        width / "w": f64,
        height / "h": f64,
        /// 1 when a layer is three-dimensional.
        three_d / "ddd": i64,
        /// Images and precompositions the layers name by id.
        assets / "assets": Vec<Asset>,
        /// The layers, the topmost first.
        layers / "layers": Vec<Layer>,
        markers / "markers": Vec<Marker>,
        fonts / "fonts": Fonts,
        /// Glyph outlines for players without the fonts.
        chars / "chars": Vec<Char>,
        /// Generator, author, keywords, description.
        meta / "meta": Map,
        slots / "slots": Map,
    }
}

object! {
    /// An asset: an image (`p`, `u`, `e`) or a precomposition (`layers`).
    Asset {
        id / "id": String,
        name / "nm": String,
        /// The folder of an image file.
        dir / "u": String,
        /// The file name of an image, or a `data:` URI when `embedded` is 1.
        path / "p": String,
        width / "w": f64,
        height / "h": f64,
        embedded / "e": i64,
        /// `"seq"` for an image sequence.
        kind / "t": String,
        /// The frame rate of a precomposition, when it has its own.
        frame_rate / "fr": f64,
        /// The layers of a precomposition.
        layers / "layers": Vec<Layer>,
    }
}

object! {
    /// A marker on the time line.
    Marker {
        comment / "cm": String,
        /// The frame.
        time / "tm": f64,
        /// Frames.
        duration / "dr": f64,
    }
}

object! {
    /// The fonts the text layers use.
    Fonts {
        list / "list": Vec<Font>,
    }
}

object! {
    Font {
        /// The name text documents refer to.
        name / "fName": String,
        family / "fFamily": String,
        style / "fStyle": String,
        path / "fPath": String,
        weight / "fWeight": String,
        class / "fClass": String,
        /// Where the font comes from (0 local, 1 CSS URL, 2 script URL, 3 font URL).
        origin / "origin": i64,
        ascent / "ascent": f64,
    }
}

object! {
    /// The outline of one character.
    Char {
        character / "ch": String,
        family / "fFamily": String,
        style / "style": String,
        size / "size": f64,
        /// The advance width at size 100.
        width / "w": f64,
        data / "data": CharData,
    }
}

object! {
    CharData {
        shapes / "shapes": Vec<Shape>,
    }
}

/// What a document may hold. Each is checked by [`Animation::read_with_limits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Bytes of JSON.
    pub max_bytes: usize,
    /// Nesting of arrays and objects.
    pub max_depth: usize,
    /// Layers of the document and of every precomposition together.
    pub max_layers: usize,
    pub max_assets: usize,
    /// Shape items of every layer and character together, groups and their contents counted.
    pub max_shapes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_bytes: 256 << 20, max_depth: 128, max_layers: 100_000, max_assets: 20_000, max_shapes: 2_000_000 }
    }
}

/// Whether `j` nests deeper than `max` (a scalar is depth 0). Not recursive.
fn deeper_than(j: &Json, max: usize) -> bool {
    let mut stack = vec![(j, 0usize)];
    while let Some((v, depth)) = stack.pop() {
        match v {
            Json::Array(_) | Json::Object(_) if depth >= max => return true,
            Json::Array(a) => stack.extend(a.iter().map(|c| (c, depth + 1))),
            Json::Object(m) => stack.extend(m.values().map(|c| (c, depth + 1))),
            _ => {}
        }
    }
    false
}

fn count_shapes(shapes: &[Shape]) -> usize {
    shapes.iter().map(|s| 1 + if let ShapeKind::Group(g) = &s.kind { count_shapes(g.items.as_deref().unwrap_or_default()) } else { 0 }).sum()
}

fn count_layer_shapes(layers: &[Layer]) -> usize {
    layers.iter().map(|l| if let LayerContent::Shape(s) = &l.content { count_shapes(s) } else { 0 }).sum()
}

impl Animation {
    /// Reads a document with the default [`Limits`].
    pub fn read(bytes: &[u8]) -> Result<Self, LottieError> {
        Self::read_with_limits(bytes, &Limits::default())
    }

    /// Reads a document. Any JSON object reads: keys of another type than the model expects are
    /// kept in `extra`, and a document without layers is a document without layers.
    pub fn read_with_limits(bytes: &[u8], limits: &Limits) -> Result<Self, LottieError> {
        if bytes.len() > limits.max_bytes {
            return Err(LottieError::TooLarge { what: "bytes", limit: limits.max_bytes });
        }
        Self::from_document(crate::parse_document(bytes)?, limits)
    }

    /// The model of parsed JSON.
    pub fn from_document(doc: Json, limits: &Limits) -> Result<Self, LottieError> {
        if deeper_than(&doc, limits.max_depth) {
            return Err(LottieError::TooLarge { what: "nesting levels", limit: limits.max_depth });
        }
        let doc = <Self as Lenient>::from_json(doc).map_err(|_| LottieError::Parse("the document is not an object".into()))?;
        let assets = doc.assets.as_deref().unwrap_or_default();
        let top = doc.layers.as_deref().unwrap_or_default();
        let nested = || assets.iter().map(|a| a.layers.as_deref().unwrap_or_default());
        let chars = doc.chars.as_deref().unwrap_or_default().iter().filter_map(|c| c.data.as_ref()?.shapes.as_deref());
        let check = |what, n: usize, limit| if n > limit { Err(LottieError::TooLarge { what, limit }) } else { Ok(()) };
        check("assets", assets.len(), limits.max_assets)?;
        check("layers", top.len() + nested().map(<[Layer]>::len).sum::<usize>(), limits.max_layers)?;
        let shapes = count_layer_shapes(top) + nested().map(count_layer_shapes).sum::<usize>() + chars.map(count_shapes).sum::<usize>();
        check("shape items", shapes, limits.max_shapes)?;
        Ok(doc)
    }

    /// The document as JSON.
    pub fn to_json(&self) -> Json {
        Lenient::to_json(self)
    }

    /// The document as compact JSON text.
    pub fn to_vec(&self) -> Vec<u8> {
        // Writing a `Value` to a `Vec` cannot fail.
        serde_json::to_vec(&self.to_json()).unwrap_or_default()
    }

    /// The precomposition or image asset with this id.
    pub fn asset(&self, id: &str) -> Option<&Asset> {
        self.assets.as_deref()?.iter().find(|a| a.id.as_deref() == Some(id))
    }
}
