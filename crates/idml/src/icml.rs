//! InCopy Markup (ICML): one story, with what it uses, as a single XML document.
//!
//! An ICML file is what InDesign writes for File › Export › InCopy Document and InCopy saves:
//! not a package, but one XML document that holds inline what an IDML package spreads over its
//! parts. Two processing instructions mark it, `<?aid style="50" type="snippet" …?>` and
//! `<?aid SnippetType="InCopyInterchange"?>`, and its `Document` root holds colours and swatches,
//! fonts, the style groups, numbering lists, conditions, cross-reference formats and text
//! variables, then the `Story` (and the stories of text frames anchored in it) and the
//! hyperlinks that point into it. The elements are IDML's, so [`crate::model`] reads them.

use craft_xml::{DECL_STANDALONE, El};

use crate::model::{Story, stories};
use crate::part::parse_part;
use crate::{DOM_VERSION, Error, Result};

/// Most bytes an ICML file may hold: the limit one part of an IDML package has.
pub const MAX_ICML_BYTES: usize = 512 << 20;

/// How far into the file the ICML processing instruction is looked for.
const SNIFF_BYTES: usize = 4096;

/// Does `bytes` look like an ICML file (an XML document marked `SnippetType="InCopyInterchange"`)?
pub fn is_icml(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(SNIFF_BYTES)).unwrap_or_default());
    head.trim_start_matches('\u{FEFF}').trim_start().starts_with('<') && head.contains("SnippetType=\"InCopyInterchange\"")
}

/// An ICML file, read.
#[derive(Clone, Debug, PartialEq)]
pub struct Icml {
    /// The `Document` element as written.
    pub root: El,
    /// Its children, without references to package parts (an ICML file has no package).
    pub top: Vec<El>,
    /// One line per element left out.
    pub warnings: Vec<String>,
}

impl Icml {
    /// The file's stories, in order. The story to place is usually the first one no text frame
    /// of the file holds; the others are the stories of text frames anchored in it.
    pub fn stories(&self) -> Vec<Story> {
        stories(&self.top)
    }
}

/// Reads an ICML file. More than [`MAX_ICML_BYTES`], a root that is not `Document`, and a file
/// without a `Story` are refused; the XML is read as a package part is ([`parse_part`]).
pub fn read_icml(bytes: &[u8]) -> Result<Icml> {
    if bytes.len() > MAX_ICML_BYTES {
        return Err(Error::TooLarge { part: "ICML".into(), limit: MAX_ICML_BYTES as u64 });
    }
    let root = parse_part(bytes).map_err(|e| Error::Part { part: "ICML".into(), msg: e.to_string() })?;
    if root.local() != "Document" {
        return Err(Error::NotIdml(format!("the ICML root is <{}>, not <Document>", root.name)));
    }
    let mut warnings = Vec::new();
    let top: Vec<El> = root
        .elements()
        .filter(|c| {
            let part = c.name.starts_with("idPkg:");
            if part {
                warnings.push(format!("{}: an ICML file has no package parts; it was not read", c.attr("src").unwrap_or(&c.name)));
            }
            !part
        })
        .cloned()
        .collect();
    if !top.iter().any(|e| e.local() == "Story") {
        return Err(Error::NotIdml("the ICML file holds no story".into()));
    }
    Ok(Icml { root, top, warnings })
}

/// An ICML file of `root` (a `Document` holding the resources and the story): the XML
/// declaration, the two instructions that mark the file, then `root`, written as a part is
/// ([`crate::part_xml`]).
pub fn icml_xml(root: &El) -> Vec<u8> {
    let mut s = String::from(DECL_STANDALONE);
    s.push_str("<?aid style=\"50\" type=\"snippet\" readerVersion=\"6.0\" featureSet=\"513\" product=\"");
    s.push_str(DOM_VERSION);
    s.push_str("(1)\" ?>\n<?aid SnippetType=\"InCopyInterchange\"?>\n");
    root.write_xml_pretty(&mut s, "\t");
    s.into_bytes()
}
