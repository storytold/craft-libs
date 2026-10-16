//! The starting point for reading PDF files.

use crate::PdfData;
use crate::object::Object;
use crate::page::Pages;
use crate::page::cached::CachedPages;
use crate::reader::Reader;
use crate::sync::Arc;
use crate::xref::{XRef, XRefError, fallback, root_xref};

pub use crate::crypto::DecryptionError;
use crate::metadata::Metadata;

/// Immutable limits for decoding one PDF's streams.
///
/// These limits apply to ordinary stream expansion (Flate, LZW and RunLength), including
/// object and xref streams. Known-size image pixels and incremental page content keep their
/// separate budgets. In particular, a bare Flate page read through `Page::content_reader`
/// (also used by interpretation and rasterization) is decoded incrementally under its content
/// budget, not this ordinary materialization limit. `Page::page_stream` materializes that
/// same stream and does use this limit, so the two paths can produce different lengths.
/// These limits do not impose a total document memory or execution-time limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    max_decoded_stream: usize,
}

impl DecodeLimits {
    /// The existing ordinary stream limit, 256 MiB.
    pub const DEFAULT: Self = Self { max_decoded_stream: crate::filter::MAX_DECODED_STREAM };

    /// Lower the maximum expanded bytes per ordinary stream. Requests above the existing
    /// 256 MiB ceiling are clamped; zero permits no ordinary materialized expansion.
    /// This does not lower the separate incremental page-content budget.
    pub const fn with_max_decoded_stream(self, max: usize) -> Self {
        Self { max_decoded_stream: if max < self.max_decoded_stream { max } else { self.max_decoded_stream } }
    }

    /// The maximum expanded bytes per ordinary stream.
    pub const fn max_decoded_stream(self) -> usize {
        self.max_decoded_stream
    }
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A PDF file.
pub struct Pdf {
    xref: Arc<XRef>,
    header_version: PdfVersion,
    pages: CachedPages,
    data: PdfData,
}

/// An error that occurred while loading a PDF file.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LoadPdfError {
    /// An error occurred while processing an encrypted document.
    Decryption(DecryptionError),
    /// The PDF was invalid or could not be parsed due to some other unknown reason.
    Invalid,
}

#[allow(clippy::len_without_is_empty)]
impl Pdf {
    /// Try to read the given PDF file.
    ///
    /// Returns `Err` if it was unable to read it.
    pub fn new(data: impl Into<PdfData>) -> Result<Self, LoadPdfError> {
        Self::new_with_password(data, "")
    }

    /// Try to read the given PDF file with a password.
    ///
    /// Returns `Err` if it was unable to read it or if the password is incorrect.
    pub fn new_with_password(data: impl Into<PdfData>, password: &str) -> Result<Self, LoadPdfError> {
        Self::new_with_password_and_limits(data, password, DecodeLimits::default())
    }

    /// Try to read a PDF using document-local stream expansion limits.
    pub fn new_with_limits(data: impl Into<PdfData>, limits: DecodeLimits) -> Result<Self, LoadPdfError> {
        Self::new_with_password_and_limits(data, "", limits)
    }

    /// Try to read a PDF with a password and document-local stream expansion limits.
    /// The same immutable limits apply during initial xref parsing, repair and later reads.
    pub fn new_with_password_and_limits(data: impl Into<PdfData>, password: &str, limits: DecodeLimits) -> Result<Self, LoadPdfError> {
        let data = data.into();
        let password = password.as_bytes();
        let version = find_version(data.as_ref()).unwrap_or(PdfVersion::Pdf10);
        let xref = match root_xref(data.clone(), password, limits) {
            Ok(x) => x,
            Err(e) => match e {
                XRefError::Unknown => fallback(data.clone(), password, limits).ok_or(LoadPdfError::Invalid)?,
                XRefError::Encryption(e) => return Err(LoadPdfError::Decryption(e)),
            },
        };
        let xref = Arc::new(xref);

        let pages = CachedPages::new(xref.clone()).ok_or(LoadPdfError::Invalid)?;

        Ok(Self { xref, header_version: version, pages, data })
    }

    /// Return this document's immutable stream expansion limits.
    pub fn decode_limits(&self) -> DecodeLimits {
        self.xref.decode_limits()
    }

    /// Return the number of objects present in the PDF file.
    pub fn len(&self) -> usize {
        self.xref.len()
    }

    /// Return an iterator over all objects defined in the PDF file.
    pub fn objects(&self) -> impl IntoIterator<Item = Object<'_>> {
        self.xref.objects()
    }

    /// Return the version of the PDF file.
    pub fn version(&self) -> PdfVersion {
        self.xref
            .trailer_data()
            .version
            .unwrap_or(self.header_version)
    }

    /// Return the underlying data of the PDF file.
    pub fn data(&self) -> &PdfData {
        &self.data
    }

    /// Return the pages of the PDF file.
    pub fn pages(&self) -> &Pages<'_> {
        self.pages.get()
    }

    /// Return the xref of the PDF file.
    pub fn xref(&self) -> &XRef {
        &self.xref
    }

    /// Return the metadata in the document information dictionary of the document.
    pub fn metadata(&self) -> &Metadata {
        self.xref.metadata()
    }
}

fn find_version(data: &[u8]) -> Option<PdfVersion> {
    let data = &data[..data.len().min(2000)];
    let mut r = Reader::new(data);

    while r.forward_tag(b"%PDF-").is_none() {
        r.read_byte()?;
    }

    PdfVersion::from_bytes(r.tail()?)
}

/// The version of a PDF document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PdfVersion {
    /// PDF 1.0.
    Pdf10,
    /// PDF 1.1.
    Pdf11,
    /// PDF 1.2.
    Pdf12,
    /// PDF 1.3.
    Pdf13,
    /// PDF 1.4.
    Pdf14,
    /// PDF 1.5.
    Pdf15,
    /// PDF 1.6.
    Pdf16,
    /// PDF 1.7.
    Pdf17,
    /// PDF 2.0.
    Pdf20,
}

impl PdfVersion {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        match bytes.get(..3)? {
            b"1.0" => Some(Self::Pdf10),
            b"1.1" => Some(Self::Pdf11),
            b"1.2" => Some(Self::Pdf12),
            b"1.3" => Some(Self::Pdf13),
            b"1.4" => Some(Self::Pdf14),
            b"1.5" => Some(Self::Pdf15),
            b"1.6" => Some(Self::Pdf16),
            b"1.7" => Some(Self::Pdf17),
            b"2.0" => Some(Self::Pdf20),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::pdf::{Pdf, PdfVersion};

    #[test]
    fn issue_49() {
        let _ = Pdf::new(Vec::new());
    }

    fn version_pdf(header: &str, catalog_version: &str) -> alloc::vec::Vec<u8> {
        alloc::format!(
            "%PDF-{header}\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R {catalog_version} >>\nendobj\n\
             2 0 obj\n<< /Type /Pages /Count 1 /Kids [3 0 R] >>\nendobj\n\
             3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>\nendobj\n\
             trailer\n<< /Root 1 0 R >>\n%%EOF\n"
        )
        .into_bytes()
    }

    #[test]
    fn pdf_version_header() {
        let pdf = Pdf::new(version_pdf("1.7", "")).unwrap();
        assert_eq!(pdf.version(), PdfVersion::Pdf17);
    }

    #[test]
    fn pdf_version_catalog() {
        let pdf = Pdf::new(version_pdf("1.3", "/Version /1.4")).unwrap();
        assert_eq!(pdf.version(), PdfVersion::Pdf14);
    }
}
