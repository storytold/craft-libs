//! Finishing as Windows Print Schema options. Each choice has a job-wide and a document-wide
//! feature (the output bin a page-wide one too); a driver lists the one it takes, so the option
//! goes in the first feature whose options in the driver's PrintCapabilities hold it. A choice
//! the driver lists nowhere, and a hole count, which the schema has no keyword for, refuse the job.

use super::{Edge, Finishing, Staple};
use crate::Error;
use crate::ticket::{NS_PSF, NS_PSK};

const STAPLE_FEATURES: &[&str] = &["JobStapleAllDocuments", "DocumentStaple"];
const PUNCH_FEATURES: &[&str] = &["JobHolePunch", "DocumentHolePunch"];
const BIN_FEATURES: &[&str] = &["JobOutputBin", "DocumentOutputBin", "PageOutputBin"];

/// One requested option: the Print Schema feature (a public keyword) and the option's
/// namespace and local name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chosen {
    pub feature: &'static str,
    pub namespace: String,
    pub option: String,
    /// What the job asked for, for an error message.
    pub words: &'static str,
}

impl Staple {
    /// The public-keyword option of the staple features.
    pub(crate) const fn print_schema(self) -> &'static str {
        match self {
            Self::TopLeft => "StapleTopLeft",
            Self::TopRight => "StapleTopRight",
            Self::BottomLeft => "StapleBottomLeft",
            Self::BottomRight => "StapleBottomRight",
            Self::DualLeft => "StapleDualLeft",
            Self::DualRight => "StapleDualRight",
            Self::DualTop => "StapleDualTop",
            Self::DualBottom => "StapleDualBottom",
            Self::Saddle => "SaddleStitch",
        }
    }
}

impl Edge {
    /// The public-keyword option of the hole punch features.
    pub(crate) const fn print_schema(self) -> &'static str {
        match self {
            Self::Left => "LeftEdge",
            Self::Top => "TopEdge",
            Self::Right => "RightEdge",
            Self::Bottom => "BottomEdge",
        }
    }
}

/// Whether the job's finishing decides the Print Schema feature or parameter `name`, so the
/// driver dialog's value for it is left out.
pub(crate) fn decides(name: &str, f: &Finishing) -> bool {
    let any = |features: &[&str]| features.iter().any(|feature| name.starts_with(feature));
    (f.staple.is_some() && any(STAPLE_FEATURES)) || (f.punch.is_some() && any(PUNCH_FEATURES)) || (f.output_bin.is_some() && any(BIN_FEATURES))
}

fn is_psf(node: roxmltree::Node<'_, '_>, local: &str) -> bool {
    node.has_tag_name((NS_PSF, local))
}

/// The namespace and local name of a QName attribute value of `node`; an unprefixed value is
/// in the default namespace.
fn qname<'a>(node: roxmltree::Node<'a, '_>, value: &'a str) -> Option<(&'a str, &'a str)> {
    let (prefix, local) = value.split_once(':').map_or((None, value), |(p, l)| (Some(p), l));
    Some((node.lookup_namespace_uri(prefix)?, local))
}

/// The features directly under the root named `feature` in the public keywords namespace.
fn features<'a, 'i>(root: roxmltree::Node<'a, 'i>, feature: &'a str) -> impl Iterator<Item = roxmltree::Node<'a, 'i>> + 'a {
    root.children().filter(move |n| is_psf(*n, "Feature") && n.attribute("name").and_then(|v| qname(*n, v)) == Some((NS_PSK, feature)))
}

/// The options of a feature, as namespace and local name.
fn options<'a>(feature: roxmltree::Node<'a, '_>) -> impl Iterator<Item = (&'a str, &'a str)> {
    feature.children().filter(|n| is_psf(*n, "Option")).filter_map(|o| qname(o, o.attribute("name")?))
}

/// The first of `features` the capabilities list `matches` an option of, with that option.
fn find(root: roxmltree::Node<'_, '_>, names: &[&'static str], matches: impl Fn(&str, &str) -> bool) -> Option<(&'static str, String, String)> {
    names.iter().find_map(|&name| {
        features(root, name).flat_map(options).find(|(ns, local)| matches(ns, local)).map(|(ns, local)| (name, ns.to_string(), local.to_string()))
    })
}

/// The Print Schema options for `f` on a driver whose PrintCapabilities are `capabilities_xml`.
/// A staple position or punch edge must be a public keyword the driver lists; an output bin is
/// the driver's own option whose local name matches without case. Empty when `f` asks for none.
pub(crate) fn chosen(f: &Finishing, capabilities_xml: &str, printer: &str) -> Result<Vec<Chosen>, Error> {
    if f.is_empty() {
        return Ok(Vec::new());
    }
    let document = crate::xml::parse(capabilities_xml).map_err(|e| Error::Spool(e.to_string()))?;
    let root = document.root_element();
    if !is_psf(root, "PrintCapabilities") {
        return Err(Error::Spool(format!("{printer}'s driver did not say what finishing it has; nothing was sent")));
    }
    let cannot = |what: String| Error::Spool(format!("{printer} can't {what}; nothing was sent"));
    let mut out = Vec::new();
    if let Some(staple) = f.staple {
        let keyword = staple.print_schema();
        let (feature, namespace, option) =
            find(root, STAPLE_FEATURES, |ns, local| ns == NS_PSK && local == keyword).ok_or_else(|| cannot(format!("staple ({staple:?})")))?;
        out.push(Chosen { feature, namespace, option, words: "staple" });
    }
    if let Some(punch) = f.punch {
        if punch.holes.is_some() {
            return Err(Error::Spool("Windows print settings have no hole count; leave the count to the printer".into()));
        }
        let keyword = punch.edge.print_schema();
        let (feature, namespace, option) = find(root, PUNCH_FEATURES, |ns, local| ns == NS_PSK && local == keyword)
            .ok_or_else(|| cannot(format!("punch holes along the {:?} edge", punch.edge)))?;
        out.push(Chosen { feature, namespace, option, words: "hole punch" });
    }
    if let Some(bin) = &f.output_bin {
        let (feature, namespace, option) = find(root, BIN_FEATURES, |_, local| local.eq_ignore_ascii_case(bin))
            .ok_or_else(|| cannot(format!("send pages to an output bin named {bin}")))?;
        out.push(Chosen { feature, namespace, option, words: "output bin" });
    }
    Ok(out)
}

/// A PrintTicket holding just the chosen options, to be merged into the job's ticket.
pub(crate) fn ticket_xml(chosen: &[Chosen]) -> String {
    let mut namespaces = String::new();
    let mut body = String::new();
    for (i, c) in chosen.iter().enumerate() {
        let prefix = if c.namespace == NS_PSK {
            "psk".to_string()
        } else {
            let prefix = format!("ns{i}");
            namespaces.push_str(&format!(" xmlns:{prefix}=\"{}\"", escape(&c.namespace)));
            prefix
        };
        body.push_str(&format!("\n <psf:Feature name=\"psk:{}\"><psf:Option name=\"{prefix}:{}\"/></psf:Feature>", c.feature, escape(&c.option)));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<psf:PrintTicket version=\"1\" xmlns:psf=\"{NS_PSF}\" xmlns:psk=\"{NS_PSK}\"{namespaces}>{body}\n</psf:PrintTicket>"
    )
}

/// Text for an attribute value.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Check that the driver's validated ticket kept every chosen option. Windows drops an option a
/// driver doesn't take while it validates, and the job would then print without it.
pub(crate) fn verify(validated_xml: &str, chosen: &[Chosen]) -> Result<(), Error> {
    if chosen.is_empty() {
        return Ok(());
    }
    let document = crate::xml::parse(validated_xml).map_err(|e| Error::Spool(e.to_string()))?;
    let root = document.root_element();
    for c in chosen {
        if !features(root, c.feature).flat_map(options).any(|(ns, local)| ns == c.namespace && local == c.option) {
            return Err(Error::Spool(format!("the printer's driver did not retain the requested {}; nothing was sent", c.words)));
        }
    }
    Ok(())
}
