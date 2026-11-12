//! The XML of a part: reading with IDML's limits, the `<Properties>` form, and writing.

use craft_xml::{DECL_STANDALONE, El, Local, Node, Options};

use crate::{DOM_VERSION, PACKAGING_NS};

pub(crate) const XML_LIMITS: craft_xml::Limits =
    craft_xml::Limits { max_input: 512 << 20, max_depth: 256, max_nodes: 8_000_000, max_attrs: 1024, max_attrs_total: 32_000_000 };

/// The XML limits a part is read with: 512 MiB of input, 256 levels of nesting (InDesign's
/// packages stay under a few dozen), 8,000,000 nodes and 32,000,000 attributes in one part.
pub const fn xml_limits() -> craft_xml::Limits {
    XML_LIMITS
}

/// `META-INF/container.xml` of a package.
pub const CONTAINER_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n\t<rootfiles>\n\t\t<rootfile full-path=\"designmap.xml\" media-type=\"text/xml\"/>\n\t</rootfiles>\n</container>\n";

/// Parses a part into its root element with [`xml_limits`].
///
/// Names are kept as written (`idPkg:Story`), text is kept exactly (it is story content), CDATA
/// joins the text around it, and processing instructions inside elements are kept as
/// [`Node::Pi`]: InDesign writes special characters as `<?ACE 18?>`. The read is strict: a part
/// that is cut off, or whose tags don't match, is an error rather than half a part.
pub fn parse_part(bytes: &[u8]) -> Result<El, craft_xml::Error> {
    parse_with(bytes, XML_LIMITS)
}

pub(crate) fn parse_with(bytes: &[u8], limits: craft_xml::Limits) -> Result<El, craft_xml::Error> {
    Options { pis: true, ..Options::default() }.strict().with_limits(limits).parse(bytes)
}

/// IDML's two spellings of a property: an attribute (`PointSize="9"`), or a child of the
/// element's `<Properties>` (`<Properties><Leading type="unit">14</Leading></Properties>`).
pub trait Props {
    /// The `<Properties>` child `key`.
    fn prop_el(&self, key: &str) -> Option<&El>;
    /// The property in either form: the attribute, else the text of the `<Properties>` child.
    fn prop(&self, key: &str) -> Option<String>;
    /// [`Props::prop`] trimmed; `None` when blank.
    fn value(&self, key: &str) -> Option<String>;
    /// A finite numeric property; `NaN` and infinities count as absent.
    fn num(&self, key: &str) -> Option<f64>;
    /// `true` or `false`; anything else counts as absent.
    fn boolean(&self, key: &str) -> Option<bool>;
}

impl Props for El {
    fn prop_el(&self, key: &str) -> Option<&El> {
        self.child(Local("Properties")).and_then(|p| p.child(Local(key)))
    }

    fn prop(&self, key: &str) -> Option<String> {
        match self.attr(key) {
            Some(v) => Some(v.to_string()),
            None => self.prop_el(key).map(El::deep_text),
        }
    }

    fn value(&self, key: &str) -> Option<String> {
        self.prop(key).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    }

    fn num(&self, key: &str) -> Option<f64> {
        self.prop(key).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite())
    }

    fn boolean(&self, key: &str) -> Option<bool> {
        match self.prop(key)?.trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }
}

/// A `<Properties>` child: `<Name type="ty">val</Name>`.
pub fn prop(name: &str, ty: &str, val: impl Into<String>) -> El {
    El::new(name).with_attr("type", ty).with_text(val)
}

/// `el` with a `<Properties>` element holding `props` as its first child, when there are any.
pub fn with_props(mut el: El, props: Vec<El>) -> El {
    if !props.is_empty() {
        let mut pe = El::new("Properties");
        pe.children = props.into_iter().map(Node::El).collect();
        el.children.insert(0, Node::El(pe));
    }
    el
}

/// The root element of a part: `idPkg:{kind}` (`Story`, `Spread`, `MasterSpread`, `Graphic`,
/// `Fonts`, `Styles`, `Preferences`, …) with the packaging namespace and [`DOM_VERSION`].
pub fn part_root(kind: &str) -> El {
    El::new(format!("idPkg:{kind}")).with_attr("xmlns:idPkg", PACKAGING_NS).with_attr("DOMVersion", DOM_VERSION)
}

/// A part as bytes: the XML declaration and `root`, an element with only element children one
/// child per line and any other element on one line, so text is never changed.
pub fn part_xml(root: &El) -> Vec<u8> {
    let mut s = String::from(DECL_STANDALONE);
    root.write_xml_pretty(&mut s, "\t");
    s.into_bytes()
}

/// `designmap.xml` as bytes: [`part_xml`] with the `<?aid …?>` instruction that tells InDesign
/// what the file is.
pub fn designmap_xml(root: &El) -> Vec<u8> {
    let mut s = String::from(DECL_STANDALONE);
    s.push_str("<?aid style=\"50\" type=\"document\" readerVersion=\"6.0\" featureSet=\"257\" product=\"");
    s.push_str(DOM_VERSION);
    s.push_str("(1)\" ?>\n");
    root.write_xml_pretty(&mut s, "\t");
    s.into_bytes()
}
