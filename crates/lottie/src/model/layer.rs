//! Layers, their transform, masks and effects.

use super::{Animated, Lenient, Map, Shape, TextData, fields, object, take};
use crate::Json;

object! {
    /// A transform (`ks` of a layer, a `tr` shape item).
    Transform {
        anchor / "a": Animated,
        /// The position; see [`Animated::split`] for one split in dimensions.
        position / "p": Animated,
        /// Percent.
        scale / "s": Animated,
        /// Degrees. A three-dimensional layer has `rx`, `ry`, `rz` and `or` instead.
        rotation / "r": Animated,
        rotation_x / "rx": Animated,
        rotation_y / "ry": Animated,
        rotation_z / "rz": Animated,
        orientation / "or": Animated,
        /// 0..=100.
        opacity / "o": Animated,
        skew / "sk": Animated,
        skew_axis / "sa": Animated,
    }
}

object! {
    Mask {
        name / "nm": String,
        match_name / "mn": String,
        /// `n` none, `a` add, `s` subtract, `i` intersect, `l` lighten, `d` darken, `f` difference.
        mode / "mode": String,
        inverted / "inv": bool,
        /// The path.
        path / "pt": Animated,
        /// 0..=100.
        opacity / "o": Animated,
        expansion / "x": Animated,
        /// Whether the path is closed, in files that keep the flag outside the path.
        closed / "cl": bool,
    }
}

object! {
    /// An effect, or one value of an effect.
    Effect {
        /// The effect type (5 custom, 20 tint, 21 fill, 25 drop shadow, 29 gaussian blur, ...) or the
        /// value type (0 slider, 1 angle, 2 colour, 3 point, 4 checkbox, 7 dropdown, 10 layer).
        kind / "ty": i64,
        name / "nm": String,
        match_name / "mn": String,
        index / "ix": i64,
        /// How many values the effect has.
        count / "np": i64,
        /// 0 when the effect is switched off.
        enabled / "en": i64,
        /// The value of an effect value.
        value / "v": Animated,
        /// The values of an effect.
        values / "ef": Vec<Effect>,
    }
}

fields! {
    /// A layer that shows a precomposition.
    PrecompLayer {
        /// The id of the asset.
        ref_id / "refId": String,
        width / "w": f64,
        height / "h": f64,
        /// Time remapping: the precomposition's time in seconds.
        time_remap / "tm": Animated,
    }
}

fields! {
    SolidLayer {
        /// `#rrggbb`.
        color / "sc": String,
        width / "sw": f64,
        height / "sh": f64,
    }
}

/// What a layer shows, by its `ty`.
#[derive(Debug, Clone, PartialEq)]
pub enum LayerContent {
    /// `ty` 0.
    Precomp(PrecompLayer),
    /// `ty` 1.
    Solid(SolidLayer),
    /// `ty` 2: the id of the image asset (`refId`).
    Image(Option<String>),
    /// `ty` 3.
    Null,
    /// `ty` 4: its `shapes`, the topmost first.
    Shape(Vec<Shape>),
    /// `ty` 5: its `t`.
    Text(Box<TextData>),
    /// Any other `ty` (6 audio, 13 camera, ...), as written; `Null` when the layer has none. The
    /// layer's own keys are in its `extra`.
    Other(Json),
}

impl Default for LayerContent {
    fn default() -> Self {
        Self::Other(Json::Null)
    }
}

fields! {
    /// A layer.
    Layer {
        name / "nm": String,
        match_name / "mn": String,
        /// The number other layers name it by (`parent`, `tp`).
        index / "ind": i64,
        parent / "parent": i64,
        hidden / "hd": bool,
        three_d / "ddd": i64,
        /// The first frame the layer shows at, in composition frames.
        in_point / "ip": f64,
        /// The frame after the last.
        out_point / "op": f64,
        /// The composition frame of the layer's frame 0.
        start_time / "st": f64,
        /// Time stretch: 1 is normal speed, 2 half speed.
        stretch / "sr": f64,
        auto_orient / "ao": i64,
        /// The blend mode, 0 normal.
        blend_mode / "bm": i64,
        transform / "ks": Transform,
        has_mask / "hasMask": bool,
        masks / "masksProperties": Vec<Mask>,
        effects / "ef": Vec<Effect>,
        /// How the layer is matted: 1 alpha, 2 inverted alpha, 3 luma, 4 inverted luma.
        matte_mode / "tt": i64,
        /// The `ind` of the matte layer; without it, the layer above.
        matte_parent / "tp": i64,
        /// 1 when the layer is a matte for another.
        is_matte / "td": i64,
        collapse_transform / "ct": i64,
        /// A CSS class and an element id, for players that render to a page.
        class / "cl": String,
        element_id / "ln": String,
    } + {
        content: LayerContent,
        /// Every other key of the object, as it was.
        extra: Map,
    }
}

impl LayerContent {
    fn take(m: &mut Map) -> Self {
        let Some(ty) = m.remove("ty") else { return Self::default() };
        match ty.as_i64() {
            Some(0) => Self::Precomp(PrecompLayer::take(m)),
            Some(1) => Self::Solid(SolidLayer::take(m)),
            Some(2) => Self::Image(take(m, "refId")),
            Some(3) => Self::Null,
            // A shape or text layer without its content is kept as written.
            Some(4) => take(m, "shapes").map_or(Self::Other(ty), Self::Shape),
            Some(5) => take::<TextData>(m, "t").map_or(Self::Other(ty), |t| Self::Text(Box::new(t))),
            _ => Self::Other(ty),
        }
    }

    fn put(&self, m: &mut Map) {
        let ty = match self {
            Self::Precomp(p) => {
                p.put(m);
                0
            }
            Self::Solid(s) => {
                s.put(m);
                1
            }
            Self::Image(id) => {
                if let Some(id) = id {
                    m.insert("refId".to_string(), id.to_json());
                }
                2
            }
            Self::Null => 3,
            Self::Shape(shapes) => {
                m.insert("shapes".to_string(), shapes.to_json());
                4
            }
            Self::Text(t) => {
                m.insert("t".to_string(), t.to_json());
                5
            }
            Self::Other(ty) => {
                if !ty.is_null() {
                    m.insert("ty".to_string(), ty.clone());
                }
                return;
            }
        };
        m.insert("ty".to_string(), Json::from(ty));
    }
}

impl Lenient for Layer {
    fn from_json(j: Json) -> Result<Self, Json> {
        match j {
            Json::Object(mut m) => {
                let content = LayerContent::take(&mut m);
                let mut l = Self::take(&mut m);
                l.content = content;
                l.extra = m;
                Ok(l)
            }
            other => Err(other),
        }
    }

    fn to_json(&self) -> Json {
        let mut m = self.extra.clone();
        self.put(&mut m);
        self.content.put(&mut m);
        Json::Object(m)
    }
}
