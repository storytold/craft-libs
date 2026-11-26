//! Borrowed, iterative descriptor serialization into an existing output buffer.
//!
//! Views must be finite, stable and acyclic for the duration of serialization.
//! The first absent indexed item ends a container. Counts are derived from emitted
//! items (wrapping u32, like the owned writer), never supplied by a view.
//! Output growth plus O(depth) heap frames are the only proportional allocations.

/// An identifier with its explicit wire encoding.
#[derive(Clone, Copy)]
pub enum IdRef<'a> {
    /// Length zero followed by four bytes.
    Code([u8; 4]),
    /// Explicit byte length, even when four.
    String(&'a [u8]),
}
/// A Unicode string without interpretation or normalization.
#[derive(Clone, Copy)]
pub enum UnicodeRef<'a> {
    /// Encode UTF-8 and optionally append exactly one NUL.
    Text(&'a str, bool),
    /// Preserve UTF-16 units, including unpaired surrogates.
    Units(&'a [u16]),
}
/// A borrowed class.
#[derive(Clone, Copy)]
pub struct ClassRef<'a> {
    /// Display name.
    pub name: UnicodeRef<'a>,
    /// Class identifier.
    pub id: IdRef<'a>,
}
/// A finite, stable, acyclic descriptor view.
pub trait DescriptorView {
    /// Class metadata.
    fn class(&self) -> ClassRef<'_>;
    /// Indexed item; the first None terminates the container.
    fn item(&self, index: usize) -> Option<(IdRef<'_>, ValueRef<'_>)>;
}
/// A finite, stable, acyclic value sequence.
pub trait ValuesView {
    /// Indexed value; the first None terminates the container.
    fn value(&self, index: usize) -> Option<ValueRef<'_>>;
}
/// A finite, stable reference sequence.
pub trait ReferencesView {
    /// Indexed reference; the first None terminates the container.
    fn reference(&self, index: usize) -> Option<ReferenceRef<'_>>;
}
/// A borrowed value. No variant owns a child model or payload.
pub enum ValueRef<'a> {
    /// Objc.
    Descriptor(&'a dyn DescriptorView),
    /// VlLs.
    List(&'a dyn ValuesView),
    /// ObAr prefix and body.
    ObjectArray(u32, &'a dyn DescriptorView),
    /// doub.
    Double(f64),
    /// UntF.
    UnitFloat([u8; 4], f64),
    /// UnFl.
    UnitFloats([u8; 4], &'a [f64]),
    /// TEXT.
    Text(UnicodeRef<'a>),
    /// enum.
    Enumerated(IdRef<'a>, IdRef<'a>),
    /// long.
    Integer(i32),
    /// comp.
    LargeInteger(i64),
    /// bool.
    Boolean(bool),
    /// type.
    Class(ClassRef<'a>),
    /// tdta.
    RawData(&'a [u8]),
    /// obj reference.
    Reference(&'a dyn ReferencesView),
}
/// A borrowed reference item.
#[derive(Clone, Copy)]
pub enum ReferenceRef<'a> {
    /// prop.
    Property(ClassRef<'a>, IdRef<'a>),
    /// Clss.
    Class(ClassRef<'a>),
    /// Enmr.
    Enumerated(ClassRef<'a>, IdRef<'a>, IdRef<'a>),
    /// rele.
    Offset(ClassRef<'a>, i32),
    /// Idnt.
    Identifier(ClassRef<'a>, i32),
    /// indx.
    Index(ClassRef<'a>, i32),
    /// name.
    Name(ClassRef<'a>, UnicodeRef<'a>),
}

fn u32_bytes(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn id(out: &mut Vec<u8>, value: IdRef<'_>) {
    match value {
        IdRef::Code(bytes) => {
            u32_bytes(out, 0);
            out.extend_from_slice(&bytes);
        }
        IdRef::String(bytes) => {
            u32_bytes(out, bytes.len() as u32);
            out.extend_from_slice(bytes);
        }
    }
}
fn unicode(out: &mut Vec<u8>, value: UnicodeRef<'_>) {
    match value {
        UnicodeRef::Text(text, nul) => {
            let count = text.encode_utf16().fold(0u32, |n, _| n.wrapping_add(1)).wrapping_add(u32::from(nul));
            u32_bytes(out, count);
            for unit in text.encode_utf16().chain(nul.then_some(0)) {
                out.extend_from_slice(&unit.to_be_bytes());
            }
        }
        UnicodeRef::Units(units) => {
            u32_bytes(out, units.len() as u32);
            for unit in units {
                out.extend_from_slice(&unit.to_be_bytes());
            }
        }
    }
}
fn class(out: &mut Vec<u8>, value: ClassRef<'_>) {
    unicode(out, value.name);
    id(out, value.id);
}
#[derive(Clone, Copy)]
enum Container<'a> {
    Descriptor(&'a dyn DescriptorView),
    List(&'a dyn ValuesView),
    References(&'a dyn ReferencesView),
}
struct Frame<'a> {
    container: Container<'a>,
    index: usize,
    count: u32,
    offset: usize,
}
fn begin<'a>(out: &mut Vec<u8>, container: Container<'a>) -> Frame<'a> {
    if let Container::Descriptor(d) = &container {
        class(out, d.class());
    }
    let offset = out.len();
    u32_bytes(out, 0);
    Frame { container, index: 0, count: 0, offset }
}
fn finish(out: &mut [u8], frame: Frame<'_>) {
    // This offset denotes the four-byte placeholder emitted by begin, never input data.
    if let Some(slot) = out.get_mut(frame.offset..).and_then(|tail| tail.get_mut(..4)) {
        slot.copy_from_slice(&frame.count.to_be_bytes());
    }
}
fn reference(out: &mut Vec<u8>, value: ReferenceRef<'_>) {
    match value {
        ReferenceRef::Property(c, k) => {
            out.extend_from_slice(b"prop");
            class(out, c);
            id(out, k);
        }
        ReferenceRef::Class(c) => {
            out.extend_from_slice(b"Clss");
            class(out, c);
        }
        ReferenceRef::Enumerated(c, t, v) => {
            out.extend_from_slice(b"Enmr");
            class(out, c);
            id(out, t);
            id(out, v);
        }
        ReferenceRef::Offset(c, n) | ReferenceRef::Identifier(c, n) | ReferenceRef::Index(c, n) => {
            let tag = match value {
                ReferenceRef::Offset(..) => b"rele",
                ReferenceRef::Identifier(..) => b"Idnt",
                _ => b"indx",
            };
            out.extend_from_slice(tag);
            class(out, c);
            out.extend_from_slice(&n.to_be_bytes());
        }
        ReferenceRef::Name(c, n) => {
            out.extend_from_slice(b"name");
            class(out, c);
            unicode(out, n);
        }
    }
}
fn value<'a>(out: &mut Vec<u8>, v: ValueRef<'a>) -> Option<Container<'a>> {
    match v {
        ValueRef::Descriptor(d) => {
            out.extend_from_slice(b"Objc");
            return Some(Container::Descriptor(d));
        }
        ValueRef::List(v) => {
            out.extend_from_slice(b"VlLs");
            return Some(Container::List(v));
        }
        ValueRef::ObjectArray(n, d) => {
            out.extend_from_slice(b"ObAr");
            u32_bytes(out, n);
            return Some(Container::Descriptor(d));
        }
        ValueRef::Reference(v) => {
            out.extend_from_slice(b"obj ");
            return Some(Container::References(v));
        }
        ValueRef::Double(n) => {
            out.extend_from_slice(b"doub");
            out.extend_from_slice(&n.to_be_bytes());
        }
        ValueRef::UnitFloat(u, n) => {
            out.extend_from_slice(b"UntF");
            out.extend_from_slice(&u);
            out.extend_from_slice(&n.to_be_bytes());
        }
        ValueRef::UnitFloats(u, ns) => {
            out.extend_from_slice(b"UnFl");
            out.extend_from_slice(&u);
            u32_bytes(out, ns.len() as u32);
            for n in ns {
                out.extend_from_slice(&n.to_be_bytes());
            }
        }
        ValueRef::Text(s) => {
            out.extend_from_slice(b"TEXT");
            unicode(out, s);
        }
        ValueRef::Enumerated(t, v) => {
            out.extend_from_slice(b"enum");
            id(out, t);
            id(out, v);
        }
        ValueRef::Integer(n) => {
            out.extend_from_slice(b"long");
            out.extend_from_slice(&n.to_be_bytes());
        }
        ValueRef::LargeInteger(n) => {
            out.extend_from_slice(b"comp");
            out.extend_from_slice(&n.to_be_bytes());
        }
        ValueRef::Boolean(b) => {
            out.extend_from_slice(b"bool");
            out.push(u8::from(b));
        }
        ValueRef::Class(c) => {
            out.extend_from_slice(b"type");
            class(out, c);
        }
        ValueRef::RawData(b) => {
            out.extend_from_slice(b"tdta");
            u32_bytes(out, b.len() as u32);
            out.extend_from_slice(b);
        }
    }
    None
}
/// Append without cloning the model or staging serialized output.
///
/// Traversal uses constant call-stack depth and O(nesting depth) heap frames.
/// Views must terminate, remain stable and contain no cycles. Existing bytes in
/// out are untouched; only count placeholders in newly appended data are patched.
pub fn append_descriptor(out: &mut Vec<u8>, descriptor: &dyn DescriptorView) {
    let mut frames = vec![begin(out, Container::Descriptor(descriptor))];
    while let Some(mut frame) = frames.pop() {
        let child = match frame.container {
            Container::Descriptor(d) => match d.item(frame.index) {
                Some((key, v)) => {
                    id(out, key);
                    Some(value(out, v))
                }
                None => None,
            },
            Container::List(list) => list.value(frame.index).map(|v| value(out, v)),
            Container::References(refs) => refs.reference(frame.index).map(|v| {
                reference(out, v);
                None
            }),
        };
        let Some(child) = child else {
            finish(out, frame);
            continue;
        };
        frame.count = frame.count.wrapping_add(1);
        if let Some(index) = frame.index.checked_add(1) {
            frame.index = index;
            frames.push(frame);
        } else {
            // No further index is representable; patch the items actually emitted.
            finish(out, frame);
        }
        if let Some(container) = child {
            frames.push(begin(out, container));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Node {
        next: Option<Box<Node>>,
        kind: u8,
    }
    impl DescriptorView for Node {
        fn class(&self) -> ClassRef<'_> {
            ClassRef { name: UnicodeRef::Units(&[]), id: IdRef::Code(*b"null") }
        }
        fn item(&self, i: usize) -> Option<(IdRef<'_>, ValueRef<'_>)> {
            if i != 0 {
                return None;
            }
            self.next.as_deref().map(|n| {
                (
                    IdRef::Code(*b"next"),
                    match self.kind % 3 {
                        0 => ValueRef::Descriptor(n),
                        1 => ValueRef::List(n),
                        _ => ValueRef::ObjectArray(7, n),
                    },
                )
            })
        }
    }
    impl ValuesView for Node {
        fn value(&self, i: usize) -> Option<ValueRef<'_>> {
            if i != 0 {
                return None;
            }
            self.next.as_deref().map(|n| ValueRef::Descriptor(n))
        }
    }
    #[test]
    fn deeply_mixed_views_use_heap_frames_not_recursive_calls() {
        let mut node = Box::new(Node { next: None, kind: 0 });
        for kind in 0..12000 {
            node = Box::new(Node { next: Some(node), kind: (kind % 3) as u8 });
        }
        let mut out = vec![0xaa];
        append_descriptor(&mut out, node.as_ref());
        assert_eq!(out[0], 0xaa);
        assert!(out.len() > 12000 * 8);
        // Tear down the test model iteratively too; serializer never owns or drops it.
        while let Some(next) = node.next.take() {
            node = next;
        }
    }
    struct Custom;
    impl DescriptorView for Custom {
        fn class(&self) -> ClassRef<'_> {
            ClassRef { name: UnicodeRef::Units(&[]), id: IdRef::Code(*b"null") }
        }
        fn item(&self, index: usize) -> Option<(IdRef<'_>, ValueRef<'_>)> {
            match index {
                0 => Some((IdRef::Code(*b"one "), ValueRef::Integer(-1))),
                1 => Some((IdRef::Code(*b"list"), ValueRef::List(self))),
                2 => Some((IdRef::Code(*b"tail"), ValueRef::Boolean(true))),
                _ => None,
            }
        }
    }
    impl ValuesView for Custom {
        fn value(&self, _index: usize) -> Option<ValueRef<'_>> {
            None
        }
    }
    #[test]
    fn counts_patch_emitted_items_and_empty_children_without_touching_prefix() {
        let mut out = vec![9, 8];
        append_descriptor(&mut out, &Custom);
        let mut expected = vec![9, 8, 0, 0, 0, 0, 0, 0, 0, 0];
        expected.extend(b"null");
        expected.extend([0, 0, 0, 3, 0, 0, 0, 0]);
        expected.extend(b"one long");
        expected.extend([255, 255, 255, 255, 0, 0, 0, 0]);
        expected.extend(b"listVlLs");
        expected.extend([0, 0, 0, 0, 0, 0, 0, 0]);
        expected.extend(b"tailbool");
        expected.push(1);
        assert_eq!(out, expected);
    }
    struct Raw<'a>(&'a [u8]);
    impl DescriptorView for Raw<'_> {
        fn class(&self) -> ClassRef<'_> {
            ClassRef { name: UnicodeRef::Units(&[]), id: IdRef::Code(*b"null") }
        }
        fn item(&self, i: usize) -> Option<(IdRef<'_>, ValueRef<'_>)> {
            (i == 0).then_some((IdRef::Code(*b"data"), ValueRef::RawData(self.0)))
        }
    }
    #[test]
    fn large_payload_is_borrowed_and_appended_without_model_conversion() {
        let bytes = vec![0xa5; 1024 * 1024];
        let view = Raw(&bytes);
        let Some((_, ValueRef::RawData(slice))) = view.item(0) else {
            panic!("raw view");
        };
        assert_eq!(slice.as_ptr(), bytes.as_ptr());
        let mut out = Vec::with_capacity(bytes.len() + 32);
        append_descriptor(&mut out, &view);
        assert_eq!(&out[32..], bytes);
    }

    struct Gapped;
    impl DescriptorView for Gapped {
        fn class(&self) -> ClassRef<'_> {
            ClassRef { name: UnicodeRef::Units(&[]), id: IdRef::Code(*b"null") }
        }
        fn item(&self, index: usize) -> Option<(IdRef<'_>, ValueRef<'_>)> {
            match index {
                0 => Some((IdRef::Code(*b"list"), ValueRef::List(self))),
                1 => Some((IdRef::Code(*b"refs"), ValueRef::Reference(self))),
                3 => panic!("descriptor must stop at first None"),
                _ => None,
            }
        }
    }
    impl ValuesView for Gapped {
        fn value(&self, index: usize) -> Option<ValueRef<'_>> {
            match index {
                0 => Some(ValueRef::Integer(9)),
                2 => panic!("list must stop at first None"),
                _ => None,
            }
        }
    }
    impl ReferencesView for Gapped {
        fn reference(&self, index: usize) -> Option<ReferenceRef<'_>> {
            match index {
                1 => panic!("reference must stop at first None"),
                _ => None,
            }
        }
    }
    #[test]
    fn first_absent_item_terminates_each_custom_container() {
        let mut out = Vec::new();
        append_descriptor(&mut out, &Gapped);
        let expected: Vec<u8> =
            [&b"\0\0\0\0\0\0\0\0null\0\0\0\x02"[..], b"\0\0\0\0listVlLs\0\0\0\x01long\0\0\0\x09", b"\0\0\0\0refsobj \0\0\0\0"].concat();
        assert_eq!(out, expected);
    }
}
