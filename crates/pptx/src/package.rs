//! The package being read: craft-opc's [`craft_opc::Package`] with the limits and lookups the
//! reader uses.

use craft_drawingml::xmlx;
use craft_opc::{ContentTypes, Limits, Oversized, Rels};
use craft_xml::El;

use crate::Error;

/// Most bytes one part inflates to.
pub const MAX_PART: u64 = 512 << 20;
/// Most bytes one XML part may have to be parsed.
pub const MAX_XML_PART: usize = 96 << 20;
/// Most bytes kept from the whole package.
pub const MAX_TOTAL: u64 = 2 << 30;
/// Most bytes inflated for the whole package, parts left out included.
pub const MAX_INFLATE_WORK: u64 = 2 * MAX_TOTAL;
/// Most ZIP entries.
pub const MAX_ENTRIES: usize = 50_000;

/// The package limits: oversized or damaged parts are left out with a warning, and reading stops
/// once [`MAX_INFLATE_WORK`] is spent.
pub const LIMITS: Limits =
    Limits { max_entries: MAX_ENTRIES, max_part: MAX_PART, max_total: MAX_TOTAL, max_work: MAX_INFLATE_WORK, oversized: Oversized::Skip };

/// A parsed part: its root element.
#[derive(Clone, Debug, Default)]
pub(crate) struct Doc {
    pub root: El,
}

impl Doc {
    /// Namespace declarations (`xmlns:x`, `xmlns`) on the root element.
    pub fn ns_decls(&self) -> Vec<(String, String)> {
        xmlx::ns_decls(&self.root)
    }
}

/// Parses a part with [`xmlx::parse`] (names as written, depth and node limits).
pub(crate) fn parse(bytes: &[u8]) -> Result<Doc, Error> {
    xmlx::parse(bytes).map(|root| Doc { root }).map_err(|e| Error::Xml(e.to_string()))
}

pub(crate) struct Package {
    pkg: craft_opc::Package,
    types: ContentTypes,
}

impl Package {
    pub fn open(bytes: &[u8]) -> Result<Self, Error> {
        let pkg = craft_opc::Package::open(bytes, &LIMITS).map_err(|e| Error::NotPptx(e.to_string()))?;
        Ok(Package { types: pkg.content_types(), pkg })
    }
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.pkg.get(name)
    }
    pub fn get_shared(&self, name: &str) -> Option<&std::sync::Arc<Vec<u8>>> {
        self.pkg.get_shared(name)
    }
    /// An XML part's bytes, when it is within [`MAX_XML_PART`].
    pub fn xml_bytes(&self, name: &str) -> Option<&[u8]> {
        let b = self.get(name)?;
        if b.len() > MAX_XML_PART {
            log::warn!("pptx: XML part {name} too large");
            return None;
        }
        Some(b)
    }
    /// An XML part, parsed; `None` (with a warning) when it is missing, too large or malformed.
    pub fn xml(&self, name: &str) -> Option<Doc> {
        match parse(self.xml_bytes(name)?) {
            Ok(d) => Some(d),
            Err(e) => {
                log::warn!("pptx: {name}: {e}");
                None
            }
        }
    }
    /// Relationships of a part (`ppt/slides/slide1.xml` → `ppt/slides/_rels/slide1.xml.rels`).
    pub fn rels(&self, part: &str) -> Rels {
        self.pkg.rels(part)
    }
    pub fn content_type(&self, part: &str) -> Option<String> {
        self.types.of(part).map(str::to_string)
    }
    /// The part name as stored in the archive.
    pub fn original_name(&self, name: &str) -> Option<&str> {
        self.pkg.canonical(name)
    }
}
