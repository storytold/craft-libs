//! [`Context`]: where a walk currently is, as the chain of open objects and the fields that led to them.
//! Visitors that pick values out of the tree keep one and ask it questions such as "is the innermost
//! object a `DataChannel` whose parent is a `DataMaterialStack`?".

/// One open object: its class and the field of the parent that holds it (empty for list elements).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub class: String,
    /// The parent's field this object is stored in (for list elements, the list's field).
    pub via: String,
    /// The most recent field name read inside this object.
    pub field: String,
}

/// The stack of open objects during a walk. Feed it from the matching [`super::Visitor`] callbacks.
#[derive(Debug, Clone, Default)]
pub struct Context {
    frames: Vec<Frame>,
}

impl Context {
    pub fn enter(&mut self, class: &str) {
        let via = self.frames.last().map(|f| f.field.clone()).unwrap_or_default();
        self.frames.push(Frame { class: class.to_string(), via, field: String::new() });
    }

    pub fn leave(&mut self) {
        self.frames.pop();
    }

    pub fn field(&mut self, name: &[u8]) {
        if let Some(top) = self.frames.last_mut() {
            top.field = String::from_utf8_lossy(name).into_owned();
        }
    }

    /// Number of open objects.
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    /// The `n`-th object from the innermost (0 = innermost).
    pub fn frame(&self, n: usize) -> Option<&Frame> {
        self.frames.iter().rev().nth(n)
    }

    /// True if the innermost objects have these classes, innermost first.
    pub fn is(&self, classes: &[&str]) -> bool {
        classes.iter().enumerate().all(|(n, c)| self.frame(n).is_some_and(|f| f.class == *c))
    }

    /// The field currently being read in the innermost object.
    pub fn current_field(&self) -> &str {
        self.frames.last().map_or("", |f| f.field.as_str())
    }

    /// Index (from the outermost) of the innermost open object of `class`.
    pub fn innermost(&self, class: &str) -> Option<usize> {
        self.frames.iter().rposition(|f| f.class == class)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_classes_and_fields() {
        let mut c = Context::default();
        c.enter("DataMaterial");
        c.field(b"stacks");
        c.enter("DataMaterialStack");
        c.field(b"channels");
        c.enter("DataChannel");
        c.field(b"type");
        assert!(c.is(&["DataChannel", "DataMaterialStack", "DataMaterial"]));
        assert_eq!(c.frame(0).map(|f| f.via.as_str()), Some("channels"));
        assert_eq!(c.current_field(), "type");
        assert_eq!(c.innermost("DataMaterial"), Some(0));
        c.leave();
        assert!(c.is(&["DataMaterialStack"]));
        assert_eq!(c.depth(), 2);
        assert!(c.frame(5).is_none());
    }
}
