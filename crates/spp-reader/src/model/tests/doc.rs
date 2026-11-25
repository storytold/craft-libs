//! A tiny value tree serialised with the document test writer, so tests can describe documents briefly.

use crate::decode::document::test_writer::DocWriter;

/// One value to write.
pub enum V {
    Obj(&'static str, Vec<(&'static str, V)>),
    /// A list of pointers (tag 0x13).
    List(Vec<V>),
    /// A value list (tag 0x11) of inline objects (tag 0x14), as stroke points are stored.
    Values(Vec<V>),
    /// An object stored inline (tag 0x14).
    Inline(&'static str, Vec<(&'static str, V)>),
    /// A vector or matrix with its tag (0x02 vec2, 0x0e mat4 …).
    Floats(u8, Vec<f32>),
    Absent,
    Float(f32),
    Vec3([f32; 3]),
    Bool(bool),
    /// Tag 0x0c (uid; also `channelTypes` in newer files).
    Uid(u64),
    /// Tag 0x0f.
    Ref(u64),
    /// Tag 0x09.
    Enum(u64),
    /// Tag 0x15 (a channel's bit in the tagged layout).
    Channel(u64),
    /// Tag 0x0b (older `channelTypes`).
    UInt(u32),
    Int(i32),
    Str(&'static str),
}

pub fn obj(class: &'static str, fields: Vec<(&'static str, V)>) -> V {
    V::Obj(class, fields)
}

/// A document with `root` at the given format version.
pub fn document(version: u32, root: V) -> Vec<u8> {
    let mut w = DocWriter::new(version);
    write(&mut w, &root);
    w.bytes
}

fn write(w: &mut DocWriter, v: &V) {
    match v {
        V::Obj(class, fields) => {
            w.object(0x12, class, fields.len() as u16);
            for (name, value) in fields {
                w.field(name);
                write(w, value);
            }
            w.close();
        }
        V::List(items) | V::Values(items) => {
            w.list(if matches!(v, V::List(_)) { 0x13 } else { 0x11 }, items.len() as u32);
            for item in items {
                write(w, item);
            }
            w.close();
        }
        V::Inline(class, fields) => {
            w.object(0x14, class, fields.len() as u16);
            for (name, value) in fields {
                w.field(name);
                write(w, value);
            }
            w.close();
        }
        V::Floats(tag, values) => {
            w.raw(&[*tag]);
            for x in values {
                w.raw(&x.to_le_bytes());
            }
        }
        V::Absent => {
            w.raw(&[0x12, 0xff]);
        }
        V::Float(f) => {
            w.raw(&[0x01]).raw(&f.to_le_bytes());
        }
        V::Vec3(c) => {
            w.raw(&[0x03]);
            for x in c {
                w.raw(&x.to_le_bytes());
            }
        }
        V::Bool(b) => {
            w.raw(&[0x0a, if *b { 0xff } else { 0 }]);
        }
        V::Uid(u) => {
            w.raw(&[0x0c]).raw(&u.to_le_bytes());
        }
        V::Ref(u) => {
            w.raw(&[0x0f]).raw(&u.to_le_bytes());
        }
        V::Channel(u) => {
            w.raw(&[0x15]).raw(&u.to_le_bytes());
        }
        V::Enum(u) => {
            w.raw(&[0x09]).raw(&u.to_le_bytes());
        }
        V::UInt(u) => {
            w.raw(&[0x0b]).raw(&u.to_le_bytes());
        }
        V::Int(i) => {
            w.raw(&[0x05]).raw(&i.to_le_bytes());
        }
        V::Str(s) => {
            w.raw(&[0x10]).raw(&(s.len() as u32).to_le_bytes()).raw(s.as_bytes());
        }
    }
}

/// `DataBlending` for `channels`.
pub fn blending(channels: u64, mode: u64, opacity: f32) -> V {
    obj("DataBlending", vec![("blendingMode", V::Enum(mode)), ("channelTypes", V::Uid(channels)), ("opacity", V::Float(opacity))])
}

/// A `DataStackActions` / `DataStackLayers` holding `items`.
pub fn stack(class: &'static str, uid: u64, items: Vec<V>) -> V {
    obj(class, vec![("items", V::List(items)), ("uid", V::Uid(uid))])
}

/// `DataSourceUniform` for one channel bit.
pub fn uniform(channel: u64, color: [f32; 3]) -> V {
    obj("DataSourceUniform", vec![("channelTypes", V::Uid(channel)), ("color", V::Vec3(color)), ("opacity", V::Float(1.0))])
}

/// `DataActionFill` with sources.
pub fn fill(uid: u64, sources: Vec<V>) -> V {
    obj(
        "DataActionFill",
        vec![
            ("channelTypes", V::Uid(u64::MAX)),
            ("enabled", V::Bool(true)),
            ("perChannelBlending", V::List(vec![])),
            ("projection", V::Enum(0)),
            ("sources", V::List(sources)),
            ("uid", V::Uid(uid)),
        ],
    )
}

/// A layer (`DataLayerColor`) with content effects, blending entries and an optional mask stack.
pub fn layer(uid: u64, label: &'static str, content: Vec<V>, blend: Vec<V>, mask: Option<Vec<V>>) -> V {
    let mask = match mask {
        Some(effects) => stack("DataStackActions", uid + 2, effects),
        None => V::Absent,
    };
    obj(
        "DataLayerColor",
        vec![
            ("actions", stack("DataStackActions", uid + 1, content)),
            ("enabled", V::Bool(true)),
            ("label", V::Str(label)),
            ("maskActions", mask),
            ("maskEnabled", V::Bool(true)),
            ("maskInitial", V::Enum(0)),
            ("perChannelBlending", V::List(blend)),
            ("uid", V::Uid(uid)),
        ],
    )
}

/// A folder (`DataLayerGroup`) holding `children`.
pub fn folder(uid: u64, label: &'static str, children: Vec<V>) -> V {
    obj(
        "DataLayerGroup",
        vec![
            ("enabled", V::Bool(false)),
            ("label", V::Str(label)),
            ("maskActions", V::Absent),
            ("perChannelBlending", V::List(vec![])),
            ("subStack", stack("DataStackLayers", uid + 1, children)),
            ("uid", V::Uid(uid)),
        ],
    )
}

/// A one-set document whose stack holds `layers` (bottom → top).
pub fn single_set(layers: Vec<V>) -> Vec<u8> {
    let channel = obj("DataChannel", vec![("format", V::Enum(0)), ("type", V::Channel(1)), ("userName", V::Str(""))]);
    let material_stack = obj("DataMaterialStack", vec![("channels", V::List(vec![channel])), ("stack", stack("DataStackLayers", 3, layers))]);
    let material = obj(
        "DataMaterial",
        vec![("sceneMaterialName", V::Str("Vase")), ("sceneMaterialUID", V::Uid(77)), ("stacks", V::List(vec![material_stack])), ("uid", V::Uid(2))],
    );
    document(78, obj("DataDocument", vec![("materials", V::List(vec![material]))]))
}
