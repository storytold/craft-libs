//! Embedded graph packages: a 7z archive holding, per graph, an XML description and the compiled graph. [`archive`] unpacks the members, [`description`] reads the XML, [`compiled`] the compiled graph.

pub mod archive;
pub mod compiled;
pub mod description;

pub use archive::{GraphPackage, PackageGraph};
pub use description::{Description, GraphInputInfo, GraphOutputInfo, INPUT_TYPE_IMAGE};

/// Test XML with `ROOT` standing for the description's root element ([`crate::format_names::DESCRIPTION_ELEMENT`]).
#[cfg(test)]
pub(crate) fn root_xml(xml: &str) -> String {
    xml.replace("ROOT", crate::format_names::DESCRIPTION_ELEMENT)
}
