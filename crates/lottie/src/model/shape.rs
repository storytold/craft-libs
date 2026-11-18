//! Shape items: the contents of a shape layer.

use super::{Animated, Lenient, Map, Transform, fields, object};
use crate::Json;

fields! {
    /// `gr`: a group. Its last item is usually its transform (`tr`).
    Group {
        /// How many items it has.
        count / "np": f64,
        items / "it": Vec<Shape>,
    }
}

fields! {
    /// `rc`.
    Rect {
        /// The drawing direction: 1 clockwise, 3 counter-clockwise.
        direction / "d": i64,
        /// The centre.
        position / "p": Animated,
        size / "s": Animated,
        /// The corner radius.
        roundness / "r": Animated,
    }
}

fields! {
    /// `el`.
    Ellipse {
        direction / "d": i64,
        position / "p": Animated,
        size / "s": Animated,
    }
}

fields! {
    /// `sr`: a star or a polygon.
    PolyStar {
        direction / "d": i64,
        /// 1 star, 2 polygon.
        star_type / "sy": i64,
        position / "p": Animated,
        points / "pt": Animated,
        rotation / "r": Animated,
        outer_radius / "or": Animated,
        /// Percent.
        outer_roundness / "os": Animated,
        /// Stars only.
        inner_radius / "ir": Animated,
        inner_roundness / "is": Animated,
    }
}

fields! {
    /// `sh`: a bezier path.
    Path {
        direction / "d": i64,
        path / "ks": Animated,
        /// Whether the path is closed, in files that keep the flag outside the path.
        closed / "closed": bool,
    }
}

fields! {
    /// `fl`.
    Fill {
        color / "c": Animated,
        /// 0..=100.
        opacity / "o": Animated,
        /// 1 non-zero, 2 even-odd.
        rule / "r": i64,
    }
}

object! {
    /// One entry of a dash pattern.
    Dash {
        /// `d` dash, `g` gap, `o` offset.
        kind / "n": String,
        name / "nm": String,
        match_name / "mn": String,
        length / "v": Animated,
    }
}

fields! {
    /// `st`.
    Stroke {
        color / "c": Animated,
        opacity / "o": Animated,
        width / "w": Animated,
        /// 1 butt, 2 round, 3 square.
        cap / "lc": i64,
        /// 1 miter, 2 round, 3 bevel.
        join / "lj": i64,
        miter_limit / "ml": f64,
        /// The miter limit as a property.
        miter_limit_animated / "ml2": Animated,
        dashes / "d": Vec<Dash>,
    }
}

object! {
    /// The colours of a gradient: the stop list is read with [`Gradient`](crate::Gradient).
    GradientColors {
        /// How many colour stops the list has.
        count / "p": i64,
        stops / "k": Animated,
    }
}

fields! {
    /// `gf`.
    GradientFill {
        colors / "g": GradientColors,
        start / "s": Animated,
        end / "e": Animated,
        /// 1 linear, 2 radial.
        gradient_type / "t": i64,
        /// Radial: where the highlight is along the radius, percent.
        highlight_length / "h": Animated,
        highlight_angle / "a": Animated,
        opacity / "o": Animated,
        rule / "r": i64,
    }
}

fields! {
    /// `gs`.
    GradientStroke {
        colors / "g": GradientColors,
        start / "s": Animated,
        end / "e": Animated,
        gradient_type / "t": i64,
        highlight_length / "h": Animated,
        highlight_angle / "a": Animated,
        opacity / "o": Animated,
        width / "w": Animated,
        cap / "lc": i64,
        join / "lj": i64,
        miter_limit / "ml": f64,
        miter_limit_animated / "ml2": Animated,
        dashes / "d": Vec<Dash>,
    }
}

fields! {
    /// `tm`: trim paths.
    Trim {
        /// Percent.
        start / "s": Animated,
        end / "e": Animated,
        /// Degrees.
        offset / "o": Animated,
        /// 1 all paths together, 2 each on its own.
        multiple / "m": i64,
    }
}

fields! {
    /// `rd`.
    RoundCorners {
        radius / "r": Animated,
    }
}

object! {
    /// The transform a repeater applies to each copy.
    RepeaterTransform {
        anchor / "a": Animated,
        position / "p": Animated,
        scale / "s": Animated,
        rotation / "r": Animated,
        start_opacity / "so": Animated,
        end_opacity / "eo": Animated,
        skew / "sk": Animated,
        skew_axis / "sa": Animated,
    }
}

fields! {
    /// `rp`.
    Repeater {
        copies / "c": Animated,
        offset / "o": Animated,
        /// 1 each copy above the one before, 2 below.
        composite / "m": i64,
        transform / "tr": RepeaterTransform,
    }
}

fields! {
    /// `mm`: merge paths.
    Merge {
        /// 1 merge, 2 add, 3 subtract, 4 intersect, 5 exclude intersections.
        mode / "mm": i64,
    }
}

fields! {
    /// `pb`.
    PuckerBloat {
        /// Percent.
        amount / "a": Animated,
    }
}

fields! {
    /// `tw`.
    Twist {
        angle / "a": Animated,
        center / "c": Animated,
    }
}

fields! {
    /// `zz`.
    ZigZag {
        /// The size of a ridge.
        amount / "r": Animated,
        /// Ridges per segment.
        frequency / "s": Animated,
        /// 1 corner, 2 smooth.
        point_type / "pt": Animated,
    }
}

fields! {
    /// `op`.
    OffsetPath {
        amount / "a": Animated,
        join / "lj": i64,
        miter_limit / "ml": Animated,
    }
}

/// What a shape item is, by its `ty`.
#[derive(Debug, Clone, PartialEq)]
pub enum ShapeKind {
    Group(Group),
    Rect(Rect),
    Ellipse(Ellipse),
    PolyStar(PolyStar),
    Path(Path),
    Fill(Fill),
    Stroke(Stroke),
    GradientFill(GradientFill),
    GradientStroke(GradientStroke),
    /// `tr`: the transform of the group it is in. Its keys are those of a layer transform.
    Transform(Box<Transform>),
    Trim(Trim),
    RoundCorners(RoundCorners),
    Repeater(Repeater),
    Merge(Merge),
    PuckerBloat(PuckerBloat),
    Twist(Twist),
    ZigZag(ZigZag),
    OffsetPath(OffsetPath),
    /// Any other `ty`, as written; `Null` when the item has none. The item's own keys are in its
    /// `extra`.
    Other(Json),
}

impl Default for ShapeKind {
    fn default() -> Self {
        Self::Other(Json::Null)
    }
}

fields! {
    /// A shape item.
    Shape {
        name / "nm": String,
        match_name / "mn": String,
        hidden / "hd": bool,
        blend_mode / "bm": i64,
        /// The property index expressions refer to.
        index / "ix": i64,
        class / "cl": String,
        element_id / "ln": String,
    } + {
        kind: ShapeKind,
        /// Every other key of the object, as it was.
        extra: Map,
    }
}

impl ShapeKind {
    fn take(m: &mut Map) -> Self {
        let Some(ty) = m.remove("ty") else { return Self::default() };
        match ty.as_str().unwrap_or_default() {
            "gr" => Self::Group(Group::take(m)),
            "rc" => Self::Rect(Rect::take(m)),
            "el" => Self::Ellipse(Ellipse::take(m)),
            "sr" => Self::PolyStar(PolyStar::take(m)),
            "sh" => Self::Path(Path::take(m)),
            "fl" => Self::Fill(Fill::take(m)),
            "st" => Self::Stroke(Stroke::take(m)),
            "gf" => Self::GradientFill(GradientFill::take(m)),
            "gs" => Self::GradientStroke(GradientStroke::take(m)),
            // Only the transform's keys: the item's others (`nm`, `hd`, ...) stay with the item.
            "tr" => Self::Transform(Box::new(Transform::take(m))),
            "tm" => Self::Trim(Trim::take(m)),
            "rd" => Self::RoundCorners(RoundCorners::take(m)),
            "rp" => Self::Repeater(Repeater::take(m)),
            "mm" => Self::Merge(Merge::take(m)),
            "pb" => Self::PuckerBloat(PuckerBloat::take(m)),
            "tw" => Self::Twist(Twist::take(m)),
            "zz" => Self::ZigZag(ZigZag::take(m)),
            "op" => Self::OffsetPath(OffsetPath::take(m)),
            _ => Self::Other(ty),
        }
    }

    /// The `ty` of the item; `None` for [`ShapeKind::Other`].
    pub fn tag(&self) -> Option<&'static str> {
        Some(match self {
            Self::Group(_) => "gr",
            Self::Rect(_) => "rc",
            Self::Ellipse(_) => "el",
            Self::PolyStar(_) => "sr",
            Self::Path(_) => "sh",
            Self::Fill(_) => "fl",
            Self::Stroke(_) => "st",
            Self::GradientFill(_) => "gf",
            Self::GradientStroke(_) => "gs",
            Self::Transform(_) => "tr",
            Self::Trim(_) => "tm",
            Self::RoundCorners(_) => "rd",
            Self::Repeater(_) => "rp",
            Self::Merge(_) => "mm",
            Self::PuckerBloat(_) => "pb",
            Self::Twist(_) => "tw",
            Self::ZigZag(_) => "zz",
            Self::OffsetPath(_) => "op",
            Self::Other(_) => return None,
        })
    }

    fn put(&self, m: &mut Map) {
        match self {
            Self::Group(v) => v.put(m),
            Self::Rect(v) => v.put(m),
            Self::Ellipse(v) => v.put(m),
            Self::PolyStar(v) => v.put(m),
            Self::Path(v) => v.put(m),
            Self::Fill(v) => v.put(m),
            Self::Stroke(v) => v.put(m),
            Self::GradientFill(v) => v.put(m),
            Self::GradientStroke(v) => v.put(m),
            Self::Transform(v) => v.put(m),
            Self::Trim(v) => v.put(m),
            Self::RoundCorners(v) => v.put(m),
            Self::Repeater(v) => v.put(m),
            Self::Merge(v) => v.put(m),
            Self::PuckerBloat(v) => v.put(m),
            Self::Twist(v) => v.put(m),
            Self::ZigZag(v) => v.put(m),
            Self::OffsetPath(v) => v.put(m),
            Self::Other(ty) => {
                if !ty.is_null() {
                    m.insert("ty".to_string(), ty.clone());
                }
            }
        }
        if let Some(tag) = self.tag() {
            m.insert("ty".to_string(), Json::from(tag));
        }
    }
}

impl Lenient for Shape {
    fn from_json(j: Json) -> Result<Self, Json> {
        match j {
            Json::Object(mut m) => {
                let kind = ShapeKind::take(&mut m);
                let mut s = Self::take(&mut m);
                s.kind = kind;
                s.extra = m;
                Ok(s)
            }
            other => Err(other),
        }
    }

    fn to_json(&self) -> Json {
        let mut m = self.extra.clone();
        self.put(&mut m);
        self.kind.put(&mut m);
        Json::Object(m)
    }
}
