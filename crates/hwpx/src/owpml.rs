//! Reading a document another Hangul format rebuilds as OWPML element trees. `craft-hwp` reads
//! HWP 5.0 records into a header and sections of the same elements an HWPX holds, and this
//! crate's reader makes the document, so both formats read the same way.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use craft_wordmodel::Document;

pub use crate::xml::Element;
use crate::{HwpxError, package::Package};

/// A picture or other binary item the sections refer to by its `id` (`binaryItemIDRef`).
#[derive(Clone, Debug)]
pub struct Binary {
    pub id: String,
    /// Its name, with the extension that tells its type ("BinData/BIN0001.png").
    pub name: String,
    pub data: Vec<u8>,
}

/// Read a header (`hh:head`) and sections (`hs:sec`, in order) into the model, as [`crate::read`]
/// reads them from an HWPX. Each paragraph's source is kept in `Document::kept`
/// ([`crate::KEPT_FORMAT`], numbered in document order per section); no file is kept in
/// `Document::passthrough`, so [`crate::write`] makes a new package of such a document.
pub fn read(header: Element, sections: Vec<Element>, binaries: Vec<Binary>) -> Result<Document, HwpxError> {
    let pkg = Package::from_trees(header, sections, binaries);
    crate::read::read(&pkg)
}
