//! Test-only writers for small synthetic files in the reference app's object serialisation: [`DocWriter`] for the tagged
//! layout, [`CompactWriter`] for the compact one.

pub struct DocWriter {
    pub bytes: Vec<u8>,
    open: Vec<usize>,
}

impl DocWriter {
    /// Header with the given format version; the root value comes next.
    pub fn new(version: u32) -> Self {
        let mut bytes = vec![0xdd, 0x2f, 0x7c, 0x1b, 0, 0, 0, 0];
        bytes.extend_from_slice(&version.to_le_bytes());
        DocWriter { bytes, open: Vec::new() }
    }

    fn u32(&mut self, v: u32) -> &mut Self {
        self.bytes.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn string(&mut self, s: &str) -> &mut Self {
        self.u32(s.len() as u32);
        self.bytes.extend_from_slice(s.as_bytes());
        self
    }

    fn end_placeholder(&mut self) {
        self.open.push(self.bytes.len());
        self.u32(0);
    }

    /// Starts a field; the value written next belongs to it.
    pub fn field(&mut self, name: &str) -> &mut Self {
        self.string(name)
    }

    /// Writes raw bytes, e.g. a tag and its payload.
    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    /// Opens an object with `tag` (0x12 or 0x14); close it with [`DocWriter::close`].
    pub fn object(&mut self, tag: u8, class: &str, fields: u16) -> &mut Self {
        self.bytes.extend_from_slice(&[tag, 0x00]);
        self.end_placeholder();
        self.string(class);
        self.bytes.extend_from_slice(&fields.to_le_bytes());
        self
    }

    /// Opens a list with `tag` (0x11 or 0x13); close it with [`DocWriter::close`].
    pub fn list(&mut self, tag: u8, count: u32) -> &mut Self {
        self.bytes.push(tag);
        self.end_placeholder();
        self.u32(count)
    }

    /// Closes the innermost open object or list, filling in its end offset.
    pub fn close(&mut self) -> &mut Self {
        if let Some(at) = self.open.pop() {
            let end = (self.bytes.len() as u32).to_le_bytes();
            self.bytes[at..at + 4].copy_from_slice(&end);
        }
        self
    }
}

/// Writes the compact layout. Field tables are written where a class first appears, as the reference app does.
pub struct CompactWriter {
    pub bytes: Vec<u8>,
    open: Vec<usize>,
    tables: Vec<String>,
}

impl CompactWriter {
    /// Header and root tag; the root object comes next, via [`CompactWriter::object`]. `tables` is the header's count.
    pub fn new(version: u32, tables: u32) -> Self {
        let mut bytes = vec![0xdd, 0x2f, 0x7c, 0x1b, 1, 0, 0, 0];
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(&0x12u32.to_le_bytes());
        bytes.extend_from_slice(&tables.to_le_bytes());
        CompactWriter { bytes, open: Vec::new(), tables: Vec::new() }
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.bytes.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn name(&mut self, s: &str) -> &mut Self {
        self.u32(s.len() as u32);
        self.bytes.extend_from_slice(s.as_bytes());
        self
    }

    /// The class reference: an earlier table's index, or -1 and the table.
    fn class(&mut self, class: &str, fields: &[(&str, u32)]) -> &mut Self {
        if let Some(index) = self.tables.iter().position(|t| t == class) {
            return self.u32(index as u32);
        }
        self.tables.push(class.to_string());
        self.u32(u32::MAX).name(class).u32(fields.len() as u32);
        for (name, tag) in fields {
            self.name(name).u32(*tag);
        }
        self
    }

    /// Opens an object (its values come next); close it with [`CompactWriter::close`].
    pub fn object(&mut self, class: &str, fields: &[(&str, u32)]) -> &mut Self {
        self.open.push(self.bytes.len());
        self.u32(0);
        self.class(class, fields)
    }

    pub fn absent(&mut self) -> &mut Self {
        self.u32(0)
    }

    /// Starts a value list; the elements' values come next, with no header of their own.
    pub fn value_list(&mut self, count: u32, class: &str, fields: &[(&str, u32)]) -> &mut Self {
        self.u32(count).class(class, fields)
    }

    pub fn string(&mut self, s: &str) -> &mut Self {
        self.name(s)
    }

    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    /// Closes the innermost open object, filling in its end offset.
    pub fn close(&mut self) -> &mut Self {
        if let Some(at) = self.open.pop() {
            let end = (self.bytes.len() as u32).to_le_bytes();
            self.bytes[at..at + 4].copy_from_slice(&end);
        }
        self
    }
}

/// The document of [`sample`] in the compact layout, plus a value list of two blending entries on the root.
pub fn compact_sample() -> Vec<u8> {
    let root: &[(&str, u32)] = &[("colorManagementACE", 0x12), ("enabled", 0x0a), ("label", 0x10), ("layers", 0x13), ("blending", 0x11)];
    let layer: &[(&str, u32)] = &[("viewMatrix", 0x0e), ("uvTransformation", 0x14)];
    let transform: &[(&str, u32)] = &[("GUIPivot", 0x02)];
    let blending: &[(&str, u32)] = &[("blendingMode", 0x09), ("channelTypes", 0x16), ("opacity", 0x01)];
    let mut w = CompactWriter::new(92, 4);
    w.object("DataDocument", root);
    w.absent().raw(&[0xff]).string("Ink").u32(2);
    for _ in 0..2 {
        w.object("DataLayerColor", layer).raw(&[0; 64]);
        w.object("Data2DTransformation", transform).raw(&[0; 8]).close().close();
    }
    w.value_list(2, "DataBlending", blending);
    for mode in [11u32, 29] {
        w.u32(mode).raw(&128u128.to_le_bytes()).raw(&1f32.to_le_bytes());
    }
    w.close();
    w.bytes
}

/// A small document: a root with a bool, a null pointer, a string, and a list of two layers,
/// one of which holds a matrix and an inline value object.
pub fn sample() -> Vec<u8> {
    let mut w = DocWriter::new(78);
    w.object(0x12, "DataDocument", 4);
    w.field("colorManagementACE").raw(&[0x12, 0xff]);
    w.field("enabled").raw(&[0x0a, 0xff]);
    w.field("label").raw(&[0x10, 3, 0, 0, 0]).raw(b"Ink");
    w.field("layers").list(0x13, 2);
    for _ in 0..2 {
        w.object(0x12, "DataLayerColor", 2);
        w.field("viewMatrix").raw(&[0x0e]).raw(&[0; 64]);
        w.field("uvTransformation").object(0x14, "Data2DTransformation", 1);
        w.field("GUIPivot").raw(&[0x02]).raw(&[0; 8]);
        w.close().close();
    }
    w.close().close();
    w.bytes
}
