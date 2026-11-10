//! [`RawTree`]: builds the whole object tree of a `document.bin` in memory from the streaming walk.
//! Bulky classes (brush strokes with their thousands of points) keep their lists packed ([`PackedList`]): a
//! few bytes per point instead of an object each. Every object, packed or not, is counted per class.

use super::packed::PackedBuilder;
use super::tree::{RawObject, RawValue};
use crate::decode::DecodeError;
use crate::decode::document::{DocumentHeader, Scalar, ValueTag, Visitor, walk};
use std::collections::BTreeMap;

/// Classes whose list fields are kept packed: strokes hold the bulk of a document.
const PACKED: &[&str] = &["DataStroke", "DataStroke3D"];
/// Upper bound for preallocating a list; real lengths can be larger and simply grow.
const MAX_PREALLOC: usize = 1024;

/// A document's object tree plus what the walk saw.
#[derive(Debug, Clone, PartialEq)]
pub struct RawTree {
    pub format_version: u32,
    /// `None` if the root value is not an object.
    pub root: Option<RawObject>,
    /// Number of objects per class, including those inside pruned objects.
    pub classes: BTreeMap<String, usize>,
    /// Value tags the walker could not size (the rest of their container was skipped).
    pub unknown_tags: usize,
}

impl RawTree {
    pub fn parse(data: &[u8]) -> Result<RawTree, DecodeError> {
        let format_version = DocumentHeader::parse(data)?.format_version;
        let mut builder = Builder::default();
        walk(data, &mut builder)?;
        Ok(RawTree { format_version, root: builder.root, classes: builder.classes, unknown_tags: builder.unknown_tags })
    }
}

/// An open container and, for objects, the field whose value comes next.
enum Frame {
    Object {
        object: RawObject,
        pending: Option<String>,
    },
    List(Vec<RawValue>),
    /// A list field of a [`PACKED`] class.
    Packed(PackedBuilder),
}

#[derive(Default)]
struct Builder {
    frames: Vec<Frame>,
    root: Option<RawObject>,
    classes: BTreeMap<String, usize>,
    unknown_tags: usize,
}

impl Builder {
    /// Adds a finished value to the innermost open container.
    fn attach(&mut self, value: RawValue) {
        match self.frames.last_mut() {
            Some(Frame::Object { object, pending }) => {
                let name = pending.take().unwrap_or_default();
                object.fields.push((name, value));
            }
            Some(Frame::List(items)) => items.push(value),
            // Packed lists take their values themselves; nothing is attached to them.
            Some(Frame::Packed(_)) => {}
            None => {
                if let RawValue::Object(o) = value {
                    self.root = Some(*o);
                }
            }
        }
    }

    /// A field announced without a value was an absent pointer.
    fn close_pending(&mut self) {
        if let Some(Frame::Object { object, pending }) = self.frames.last_mut()
            && let Some(name) = pending.take()
        {
            object.fields.push((name, RawValue::Absent));
        }
    }

    /// The packed list being read, if the innermost container is one.
    fn packing(&mut self) -> Option<&mut PackedBuilder> {
        match self.frames.last_mut() {
            Some(Frame::Packed(p)) => Some(p),
            _ => None,
        }
    }

    /// True if the innermost container is an object of a [`PACKED`] class.
    fn in_packed_object(&self) -> bool {
        matches!(self.frames.last(), Some(Frame::Object { object, .. }) if PACKED.contains(&object.class.as_str()))
    }
}

impl Visitor for Builder {
    fn object(&mut self, class: &str) {
        *self.classes.entry(class.to_string()).or_default() += 1;
        if let Some(p) = self.packing() {
            p.object(class);
            return;
        }
        let object = RawObject::new(class);
        self.frames.push(Frame::Object { object, pending: None });
    }

    fn end_object(&mut self) {
        if let Some(p) = self.packing() {
            p.end_object();
            return;
        }
        self.close_pending();
        if let Some(Frame::Object { object, .. }) = self.frames.pop() {
            self.attach(RawValue::Object(Box::new(object)));
        }
    }

    fn field(&mut self, name: &[u8]) {
        if let Some(p) = self.packing() {
            p.field(name);
            return;
        }
        self.close_pending();
        if let Some(Frame::Object { pending, .. }) = self.frames.last_mut() {
            *pending = Some(String::from_utf8_lossy(name).into_owned());
        }
    }

    fn scalar(&mut self, tag: ValueTag, payload: &[u8]) {
        if let Some(p) = self.packing() {
            p.scalar(tag, payload);
            return;
        }
        // `Scalar::decode` only fails for container tags, which never reach this callback.
        if let Some(s) = Scalar::decode(tag, payload) {
            self.attach(RawValue::Scalar(s));
        }
    }

    fn list(&mut self, count: usize) {
        if let Some(p) = self.packing() {
            p.list();
            return;
        }
        let frame = match self.in_packed_object() {
            true => Frame::Packed(PackedBuilder::new()),
            false => Frame::List(Vec::with_capacity(count.min(MAX_PREALLOC))),
        };
        self.frames.push(frame);
    }

    fn end_list(&mut self) {
        if let Some(p) = self.packing()
            && !p.end_list()
        {
            return;
        }
        match self.frames.pop() {
            Some(Frame::List(items)) => self.attach(RawValue::List(items)),
            Some(Frame::Packed(p)) => self.attach(RawValue::Packed(Box::new(p.finish()))),
            Some(other) => self.frames.push(other),
            None => {}
        }
    }

    fn unknown_tag(&mut self, _tag: u32, _offset: usize) {
        self.unknown_tags += 1;
    }
}
