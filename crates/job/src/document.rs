//! The documents a job sends or produced, by reference: names and sizes, never contents or paths.

/// One document: what a print job sends or a scan job produced.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DocumentRef {
    pub name: String,
    /// A format label or media type (`"pdf"`, `"image/jpeg"`), as the domain crate names it.
    #[cfg_attr(feature = "serde", serde(default))]
    pub format: Option<String>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub pages: Option<u32>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub bytes: Option<u64>,
}

impl DocumentRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), format: None, pages: None, bytes: None }
    }

    #[must_use]
    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }

    #[must_use]
    pub fn with_pages(mut self, pages: u32) -> Self {
        self.pages = Some(pages);
        self
    }

    #[must_use]
    pub fn with_bytes(mut self, bytes: u64) -> Self {
        self.bytes = Some(bytes);
        self
    }
}

/// The documents of one job: a small ordered list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct JobDocuments(Vec<DocumentRef>);

impl JobDocuments {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, document: DocumentRef) {
        self.0.push(document);
    }

    #[must_use]
    pub fn with(mut self, document: DocumentRef) -> Self {
        self.0.push(document);
        self
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn first(&self) -> Option<&DocumentRef> {
        self.0.first()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, DocumentRef> {
        self.0.iter()
    }

    /// The sum of the pages, when there is at least one document and every one knows its pages.
    pub fn total_pages(&self) -> Option<u32> {
        self.0.iter().try_fold(0u32, |sum, d| d.pages.map(|p| sum.saturating_add(p))).filter(|_| !self.0.is_empty())
    }

    /// The sum of the sizes, when there is at least one document and every one knows its size.
    pub fn total_bytes(&self) -> Option<u64> {
        self.0.iter().try_fold(0u64, |sum, d| d.bytes.map(|b| sum.saturating_add(b))).filter(|_| !self.0.is_empty())
    }
}

impl From<Vec<DocumentRef>> for JobDocuments {
    fn from(documents: Vec<DocumentRef>) -> Self {
        Self(documents)
    }
}

impl<'a> IntoIterator for &'a JobDocuments {
    type Item = &'a DocumentRef;
    type IntoIter = std::slice::Iter<'a, DocumentRef>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders() {
        let d = DocumentRef::new("report.pdf").with_format("pdf").with_pages(3).with_bytes(1024);
        assert_eq!((d.name.as_str(), d.format.as_deref(), d.pages, d.bytes), ("report.pdf", Some("pdf"), Some(3), Some(1024)));
        let bare = DocumentRef::new("x");
        assert_eq!((bare.format, bare.pages, bare.bytes), (None, None, None));
    }

    #[test]
    fn totals_need_every_document_to_know() {
        let mut docs = JobDocuments::new();
        assert!(docs.is_empty());
        assert_eq!((docs.total_pages(), docs.total_bytes()), (None, None), "no documents, no total");
        docs.push(DocumentRef::new("a").with_pages(2).with_bytes(10));
        docs.push(DocumentRef::new("b").with_pages(3).with_bytes(20));
        assert_eq!((docs.len(), docs.total_pages(), docs.total_bytes()), (2, Some(5), Some(30)));
        docs.push(DocumentRef::new("c"));
        assert_eq!((docs.total_pages(), docs.total_bytes()), (None, None), "one unknown makes the sum unknown");
        assert_eq!(docs.first().map(|d| d.name.as_str()), Some("a"));
        assert_eq!(docs.iter().count(), 3);
        assert_eq!((&docs).into_iter().count(), 3);
    }

    #[test]
    fn totals_saturate() {
        let docs = JobDocuments::from(vec![
            DocumentRef::new("a").with_pages(u32::MAX).with_bytes(u64::MAX),
            DocumentRef::new("b").with_pages(5).with_bytes(5),
        ]);
        assert_eq!((docs.total_pages(), docs.total_bytes()), (Some(u32::MAX), Some(u64::MAX)));
        assert_eq!(JobDocuments::new().with(DocumentRef::new("z")).len(), 1);
    }
}
