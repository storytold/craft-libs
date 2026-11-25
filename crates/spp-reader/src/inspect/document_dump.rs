//! [`DocumentDump`]: an indented text listing of an object-serialisation file (`document.bin` and friends),
//! for finding where the reference app stores things. Long lists are shortened.

use crate::decode::DecodeError;
use crate::decode::document::{DocumentHeader, Scalar, ValueTag, Visitor, walk};
use std::fmt::Write;

/// Elements printed per list before the rest are summarised, unless the caller asks for more.
pub const LIST_PREVIEW: usize = 4;

/// Renders the whole tree as text.
pub struct DocumentDump {
    out: String,
    /// One level per open list or object.
    stack: Vec<Level>,
    pending_field: Option<String>,
    preview: usize,
}

struct Level {
    seen: usize,
    is_list: bool,
    hidden_before: bool,
}

impl DocumentDump {
    /// Renders with lists shortened to [`LIST_PREVIEW`] elements.
    pub fn render(data: &[u8]) -> Result<String, DecodeError> {
        Self::render_with(data, LIST_PREVIEW)
    }

    /// Renders with lists shortened to `preview` elements (`usize::MAX` for everything).
    pub fn render_with(data: &[u8], preview: usize) -> Result<String, DecodeError> {
        let header = DocumentHeader::parse(data)?;
        let mut dump = DocumentDump { out: format!("format version {}\n", header.format_version), stack: Vec::new(), pending_field: None, preview };
        walk(data, &mut dump)?;
        Ok(dump.out)
    }

    fn hidden(&self) -> bool {
        self.stack.iter().any(|l| l.hidden_before)
    }

    fn line(&mut self, text: &str) {
        let field = self.pending_field.take();
        let mut beyond_preview = false;
        if let Some(top) = self.stack.last_mut()
            && top.is_list
        {
            top.seen += 1;
            beyond_preview = top.seen > self.preview;
        }
        if beyond_preview || self.hidden() {
            return;
        }
        let indent = "  ".repeat(self.stack.len());
        let _ = match field {
            Some(name) => writeln!(self.out, "{indent}{name}: {text}"),
            None => writeln!(self.out, "{indent}- {text}"),
        };
    }

    fn push(&mut self, is_list: bool) {
        let hide = self.stack.last().is_some_and(|l| l.is_list && l.seen > self.preview);
        self.stack.push(Level { seen: 0, is_list, hidden_before: hide });
    }
}

impl Visitor for DocumentDump {
    fn object(&mut self, class: &str) {
        self.line(class);
        self.push(false);
    }

    fn end_object(&mut self) {
        self.stack.pop();
    }

    fn field(&mut self, name: &[u8]) {
        self.pending_field = Some(String::from_utf8_lossy(name).into_owned());
    }

    fn scalar(&mut self, tag: ValueTag, payload: &[u8]) {
        let text = Scalar::decode(tag, payload).map_or_else(|| format!("{tag:?}?"), |s| s.to_string());
        self.line(&text);
    }

    fn list(&mut self, count: usize) {
        self.line(&format!("[{count}]"));
        self.push(true);
    }

    fn end_list(&mut self) {
        if let Some(l) = self.stack.pop()
            && l.seen > self.preview
            && !l.hidden_before
            && !self.hidden()
        {
            let indent = "  ".repeat(self.stack.len() + 1);
            let _ = writeln!(self.out, "{indent}… {} more", l.seen - self.preview);
        }
    }

    fn unknown_tag(&mut self, tag: u32, offset: usize) {
        let _ = writeln!(self.out, "!! unknown tag {tag:#04x} at {offset}");
    }
}
