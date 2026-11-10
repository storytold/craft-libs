//! [`RawObject`]: one object of the serialised tree, generic (class name and named fields), with typed
//! accessors the model builder uses. Nothing here knows what a class means.

use super::packed::PackedList;
use crate::decode::document::Scalar;

/// A field value as stored.
#[derive(Debug, Clone, PartialEq)]
pub enum RawValue {
    Scalar(Scalar),
    Object(Box<RawObject>),
    /// A pointer field whose object is absent (flag `0xff`).
    Absent,
    List(Vec<RawValue>),
    /// A list of flat objects kept packed (the points of brush strokes, see [`PackedList`]).
    Packed(Box<PackedList>),
}

/// One object: its class and its fields in file order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawObject {
    pub class: String,
    pub fields: Vec<(String, RawValue)>,
}

impl RawValue {
    /// A short, stable text for the value, used by the field table's defaults and in reports: floats to 4 places,
    /// enums `e<n>`, unsigned `u<n>`, ints `i<n>`, uids `<uid>`, references `ref:0` / `ref:set`, strings quoted (first
    /// 48 characters), lists by length and first class, objects `obj:<class>`, `absent`.
    pub fn describe(&self) -> String {
        match self {
            RawValue::Scalar(s) => describe_scalar(s),
            RawValue::Object(o) => format!("obj:{}", o.class),
            RawValue::Absent => "absent".into(),
            RawValue::Packed(_) => "packed".into(),
            RawValue::List(items) => {
                let first = items.iter().find_map(|i| match i {
                    RawValue::Object(o) => Some(o.class.as_str()),
                    _ => None,
                });
                let n = match items.len() {
                    n @ 0..=3 => n.to_string(),
                    4..=9 => "4-9".into(),
                    _ => "10+".into(),
                };
                match (items.len(), first) {
                    (0, _) => "list[0]".into(),
                    (1, None) => format!("list[{}]", items.first().map(|i| i.describe()).unwrap_or_default()),
                    (_, Some(c)) => format!("list[{n}]<{c}>"),
                    (_, None) => format!("list[{n}]"),
                }
            }
        }
    }
}

fn describe_scalar(s: &Scalar) -> String {
    let floats = |v: &[f32]| {
        let parts: Vec<String> = v.iter().take(16).map(|x| format!("{x:.4}")).collect();
        format!("[{}]", parts.join(","))
    };
    match s {
        Scalar::Float(v) => format!("{v:.4}"),
        Scalar::Floats(v) => floats(v),
        Scalar::Int(v) => format!("i{v}"),
        Scalar::IVec2(v) => format!("{v:?}"),
        Scalar::Enum(v) => format!("e{v}"),
        Scalar::Bool(v) => format!("{v}"),
        Scalar::UInt(v) => format!("u{v}"),
        Scalar::Uid(_) => "<uid>".into(),
        Scalar::UidRef(0) => "ref:0".into(),
        Scalar::UidRef(_) => "ref:set".into(),
        Scalar::String(v) => {
            let v: String = v.chars().take(48).collect();
            format!("\"{}\"", v.replace(['\t', '\n'], " "))
        }
        Scalar::ChannelType(v) => format!("ch{v}"),
        Scalar::WideMask(v) => format!("w{v:#x}"),
    }
}

impl RawObject {
    pub fn new(class: &str) -> RawObject {
        RawObject { class: class.to_string(), ..RawObject::default() }
    }

    pub fn field(&self, name: &str) -> Option<&RawValue> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    pub fn scalar(&self, name: &str) -> Option<&Scalar> {
        match self.field(name)? {
            RawValue::Scalar(s) => Some(s),
            _ => None,
        }
    }

    /// Integer-like values (uids, references, enums, ints, channel masks).
    pub fn u64(&self, name: &str) -> Option<u64> {
        self.scalar(name)?.as_u64()
    }

    /// A channel's bit (`type`, `channelType`), whichever way the file stores it.
    pub fn channel_bit(&self, name: &str) -> Option<u64> {
        self.scalar(name)?.channel_bit()
    }

    /// A channel mask (`channelTypes`) in the numbering of the tagged layout.
    pub fn channel_mask(&self, name: &str) -> Option<u64> {
        self.scalar(name)?.channel_mask()
    }

    pub fn f32(&self, name: &str) -> Option<f32> {
        match self.scalar(name)? {
            Scalar::Float(v) => Some(*v),
            _ => None,
        }
    }

    pub fn bool(&self, name: &str) -> Option<bool> {
        match self.scalar(name)? {
            Scalar::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn string(&self, name: &str) -> Option<&str> {
        match self.scalar(name)? {
            Scalar::String(v) => Some(v),
            _ => None,
        }
    }

    pub fn floats(&self, name: &str) -> Option<&[f32]> {
        match self.scalar(name)? {
            Scalar::Floats(v) => Some(v),
            _ => None,
        }
    }

    /// The first three components of a vector field.
    pub fn vec3(&self, name: &str) -> Option<[f32; 3]> {
        match self.floats(name)? {
            [a, b, c, ..] => Some([*a, *b, *c]),
            _ => None,
        }
    }

    pub fn ivec2(&self, name: &str) -> Option<[i32; 2]> {
        match self.scalar(name)? {
            Scalar::IVec2(v) => Some(*v),
            _ => None,
        }
    }

    /// The object's own `uid`.
    pub fn uid(&self) -> Option<u64> {
        self.u64("uid")
    }

    /// A pointer or inline object field; `None` if missing or absent.
    pub fn object(&self, name: &str) -> Option<&RawObject> {
        match self.field(name)? {
            RawValue::Object(o) => Some(o),
            _ => None,
        }
    }

    /// The elements of a list field (empty if missing or not a list).
    pub fn list(&self, name: &str) -> &[RawValue] {
        match self.field(name) {
            Some(RawValue::List(items)) => items,
            _ => &[],
        }
    }

    /// The objects of a field that is a list in some formats and a single pointer in others.
    pub fn one_or_many<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a RawObject> + 'a + use<'a> {
        self.object(name).into_iter().chain(self.objects(name))
    }

    /// A packed list field, if the field is one.
    pub fn packed(&self, name: &str) -> Option<&PackedList> {
        match self.field(name)? {
            RawValue::Packed(p) => Some(p),
            _ => None,
        }
    }

    /// The present objects of a list field, in order.
    pub fn objects<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a RawObject> + 'a + use<'a> {
        self.list(name).iter().filter_map(|v| match v {
            RawValue::Object(o) => Some(o.as_ref()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_accessors_check_the_tag() {
        let mut o = RawObject::new("DataSourceUniform");
        o.fields.push(("uid".into(), RawValue::Scalar(Scalar::Uid(7))));
        o.fields.push(("opacity".into(), RawValue::Scalar(Scalar::Float(0.5))));
        o.fields.push(("color".into(), RawValue::Scalar(Scalar::Floats(vec![0.1, 0.2, 0.3]))));
        o.fields.push(("next".into(), RawValue::Absent));
        assert_eq!(o.uid(), Some(7));
        assert_eq!(o.f32("opacity"), Some(0.5));
        assert_eq!(o.vec3("color"), Some([0.1, 0.2, 0.3]));
        assert_eq!(o.bool("opacity"), None);
        assert!(o.object("next").is_none());
        assert!(o.list("missing").is_empty());
    }
}
