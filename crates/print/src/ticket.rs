//! The Windows job's PrintTicket, and what is read back from the driver's: plain XML and
//! numbers, so it is tested on every platform. The calls that hand them to the driver are in
//! `windows.rs`.

use crate::Error;
use crate::{Duplex, Job};

pub(crate) const NS_PSF: &str = "http://schemas.microsoft.com/windows/2003/08/printing/printschemaframework";
pub(crate) const NS_PSK: &str = "http://schemas.microsoft.com/windows/2003/08/printing/printschemakeywords";

/// Paper sizes, in micrometres, this close are the same paper (a quarter of a millimetre).
pub(crate) const PAPER_TOLERANCE: u32 = 250;

fn spool_error(e: impl std::fmt::Display) -> Error {
    Error::Spool(e.to_string())
}

/// Points to micrometres; `None` for a size no printer takes (at most 200 inches, so the
/// conversion can't overflow).
#[cfg(windows)]
pub(crate) fn micrometres(points: f64) -> Option<u32> {
    (points.is_finite() && points > 0.0 && points <= 14_400.0).then(|| (points * 25_400.0 / 72.0).round() as u32)
}

#[cfg(windows)]
pub(crate) fn points(micrometres: u32) -> f64 {
    f64::from(micrometres) * 72.0 / 25_400.0
}

/// The PrintTicket the job asks for: the job's options, the orientation, and no scaling (the
/// sheets are laid out at their final size; `sheets.rs` reframes them for shrink-to-fit). The
/// paper is merged in separately, as the driver's own option. Only schema keywords and bounded
/// numbers go into the XML; the printer name and the document title never do. Two-sided is left
/// out when the job doesn't set it, so the queue's default applies.
#[cfg(test)]
pub(crate) fn ticket_xml(job: &Job, landscape: bool) -> String {
    ticket_xml_for_duplex(job, landscape, false)
}

/// [`ticket_xml_with`] asking for grey as `Monochrome`, as every ticket did before drivers with
/// only `Grayscale` were asked for that.
#[cfg(test)]
pub(crate) fn ticket_xml_for_duplex(job: &Job, landscape: bool, document_only: bool) -> String {
    ticket_xml_with(job, landscape, document_only, Grey::Monochrome)
}

/// The Print Schema option a ticket asks for grey output with (`PageOutputColor`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grey {
    Monochrome,
    Grayscale,
}

/// The grey option to ask this driver for: `Grayscale` when its PrintCapabilities list that and
/// not `Monochrome` (Windows drops an option the driver doesn't list when it validates the
/// ticket, and the driver's default, colour, comes back); `Monochrome` otherwise, also when the
/// capabilities can't be read.
pub(crate) fn grey_option(capabilities_xml: &str) -> Grey {
    let Ok(document) = roxmltree::Document::parse(capabilities_xml) else { return Grey::Monochrome };
    let root = document.root_element();
    let offers = |option: &str| {
        root.children()
            .filter(|n| n.has_tag_name((NS_PSF, "Feature")) && named(*n, "PageOutputColor"))
            .flat_map(|f| f.children().filter(|n| n.has_tag_name((NS_PSF, "Option"))))
            .any(|n| named(n, option))
    };
    if offers("Grayscale") && !offers("Monochrome") { Grey::Grayscale } else { Grey::Monochrome }
}

/// Build the job's PrintTicket XML with orientation, copies, collation and no scaling.
/// `document_only` selects `DocumentDuplex`; otherwise an explicit duplex choice uses
/// `JobDuplexAllDocumentsContiguously`. With no duplex choice, neither feature is emitted.
/// Grayscale asks for `grey` ([`grey_option`]); colour asks for nothing, so the driver's own
/// colour setting stands. Paper is added separately by the caller. The caller reads
/// capabilities before choosing the feature and returns an error when those capabilities
/// cannot be read.
pub(crate) fn ticket_xml_with(job: &Job, landscape: bool, document_only: bool, grey: Grey) -> String {
    let orientation = if landscape { "Landscape" } else { "Portrait" };
    let duplex = match job.duplex {
        None => String::new(),
        Some(duplex) => {
            let option = match duplex {
                Duplex::Off => "OneSided",
                Duplex::LongEdge => "TwoSidedLongEdge",
                Duplex::ShortEdge => "TwoSidedShortEdge",
            };
            let feature = if document_only { "DocumentDuplex" } else { "JobDuplexAllDocumentsContiguously" };
            format!("\n <psf:Feature name=\"psk:{feature}\"><psf:Option name=\"psk:{option}\"/></psf:Feature>")
        }
    };
    let collate = if job.collate { "Collated" } else { "Uncollated" };
    // Only grayscale is asked for. Without it the ticket says nothing about colour, so a black and
    // white chosen in the driver's own preferences (Properties…) still applies (storytold/pdfcraft#2422).
    let color = match (job.grayscale, grey) {
        (false, _) => "",
        (true, Grey::Monochrome) => "\n <psf:Feature name=\"psk:PageOutputColor\"><psf:Option name=\"psk:Monochrome\"/></psf:Feature>",
        (true, Grey::Grayscale) => "\n <psf:Feature name=\"psk:PageOutputColor\"><psf:Option name=\"psk:Grayscale\"/></psf:Feature>",
    };
    let copies = job.copies.clamp(1, 999);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<psf:PrintTicket version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">
 <psf:ParameterInit name="psk:JobCopiesAllDocuments"><psf:Value xsi:type="xsd:integer">{copies}</psf:Value></psf:ParameterInit>
 <psf:Feature name="psk:DocumentCollate"><psf:Option name="psk:{collate}"/></psf:Feature>{duplex}{color}
 <psf:Feature name="psk:PageScaling"><psf:Option name="psk:None"/></psf:Feature>
 <psf:Feature name="psk:PageOrientation"><psf:Option name="psk:{orientation}"/></psf:Feature>
</psf:PrintTicket>"#
    )
}

/// Standard paper sizes (points, portrait) and the names drivers give them. A driver can list
/// one size twice, a plotter's "ARCH D" next to a custom 24 × 36 in entry; the named one is
/// preferred (from storytold/pdfcraft#599).
#[cfg(windows)]
const NAMED_PAPERS: &[((f64, f64), &[&str])] = &[
    ((612.0, 792.0), &["Letter", "US Letter"]),
    ((612.0, 1008.0), &["Legal", "US Legal"]),
    ((792.0, 1224.0), &["Tabloid", "Ledger", "ANSI B"]),
    ((419.53, 595.28), &["A5"]),
    ((595.28, 841.89), &["A4"]),
    ((841.89, 1190.55), &["A3"]),
    ((1190.55, 1683.78), &["A2"]),
    ((1683.78, 2383.94), &["A1"]),
    ((2383.94, 3370.39), &["A0"]),
    ((648.0, 864.0), &["ARCH A"]),
    ((864.0, 1296.0), &["ARCH B"]),
    ((1296.0, 1728.0), &["ARCH C"]),
    ((1728.0, 2592.0), &["ARCH D"]),
    ((2160.0, 3024.0), &["ARCH E1"]),
    ((2592.0, 3456.0), &["ARCH E"]),
    ((1224.0, 1584.0), &["ANSI C"]),
    ((1584.0, 2448.0), &["ANSI D"]),
    ((2448.0, 3168.0), &["ANSI E"]),
];

/// The usual names of a paper of this size (either way round, to half a point).
#[cfg(windows)]
pub(crate) fn paper_names(width: f64, height: f64) -> &'static [&'static str] {
    let (w, h) = (width.min(height), width.max(height));
    NAMED_PAPERS.iter().find(|((pw, ph), _)| (pw - w).abs() <= 0.5 && (ph - h).abs() <= 0.5).map_or(&[], |(_, names)| names)
}

/// Whether a driver's paper name is `name`: it starts with it, ignoring case, and the name ends
/// there ("A4 210 x 297 mm", not "A4 Extra" for "A4" or "ARCH E1" for "ARCH E").
#[cfg(windows)]
pub(crate) fn names_paper(display: Option<&str>, name: &str) -> bool {
    let Some(display) = display else { return false };
    display.get(..name.len()).is_some_and(|start| start.eq_ignore_ascii_case(name))
        && display.get(name.len()..).is_some_and(|rest| rest.chars().next().is_none_or(|c| !c.is_alphanumeric()))
}

/// A driver paper offered for the job: its size in micrometres (as the driver lists it), its
/// display name, and whether its option is a Print Schema keyword (`psk:ISOA4`) rather than a
/// vendor's own.
#[cfg(windows)]
#[derive(Clone)]
pub(crate) struct Paper<'a> {
    pub width: u32,
    pub height: u32,
    pub name: Option<&'a str>,
    pub keyword: bool,
}

/// The driver paper for a sheet of `width` × `height` points (either way round): within a
/// quarter of a millimetre, preferring the one named like the standard size, then a Print Schema
/// keyword, then the closest; on a tie the driver's own order. Its index in `papers`.
#[cfg(windows)]
pub(crate) fn matching_paper(papers: &[Paper<'_>], width: f64, height: f64) -> Option<usize> {
    let (w, h) = (micrometres(width.min(height))?, micrometres(width.max(height))?);
    let names = paper_names(width, height);
    let mut candidates: Vec<(usize, &Paper<'_>, u32)> = papers
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let (pw, ph) = (p.width.min(p.height), p.width.max(p.height));
            let off = pw.abs_diff(w).max(ph.abs_diff(h));
            (off <= PAPER_TOLERANCE).then_some((i, p, pw.abs_diff(w) + ph.abs_diff(h)))
        })
        .collect();
    candidates.sort_by_key(|(_, p, off)| (!names.iter().any(|n| names_paper(p.name, n)), !p.keyword, *off));
    candidates.first().map(|(i, _, _)| *i)
}

fn named(node: roxmltree::Node<'_, '_>, local: &str) -> bool {
    node.attribute("name").is_some_and(|v| {
        let (prefix, name) = v.split_once(':').map_or((None, v), |(p, n)| (Some(p), n));
        name == local && node.lookup_namespace_uri(prefix) == Some(NS_PSK)
    })
}

/// Read standard duplex options from the original XML: winprint's DOM loses the default
/// namespace of unprefixed QNames, so it cannot safely choose a duplex scope.
fn duplex_option(document: &roxmltree::Document<'_>, feature: &str, option: &str) -> bool {
    document
        .root_element()
        .children()
        .filter(|n| n.has_tag_name((NS_PSF, "Feature")) && named(*n, feature))
        .flat_map(|n| n.children())
        .any(|n| n.has_tag_name((NS_PSF, "Option")) && named(n, option))
}

/// Prefer job scope when it offers the exact binding; an unspecified request emits neither.
pub(crate) fn document_duplex_only(xml: &str, duplex: Option<Duplex>) -> bool {
    let wanted = match duplex {
        None => return false,
        Some(Duplex::Off) => "OneSided",
        Some(Duplex::LongEdge) => "TwoSidedLongEdge",
        Some(Duplex::ShortEdge) => "TwoSidedShortEdge",
    };
    let Ok(document) = roxmltree::Document::parse(xml) else { return false };
    !duplex_option(&document, "JobDuplexAllDocumentsContiguously", wanted) && duplex_option(&document, "DocumentDuplex", wanted)
}

/// Whether either duplex scope offers a supported standard binding; unreadable stays unknown.
pub(crate) fn two_sided(xml: &str) -> Option<bool> {
    let document = roxmltree::Document::parse(xml).ok()?;
    Some(
        ["JobDuplexAllDocumentsContiguously", "DocumentDuplex"]
            .into_iter()
            .any(|feature| ["TwoSidedLongEdge", "TwoSidedShortEdge"].into_iter().any(|option| duplex_option(&document, feature, option))),
    )
}

/// The integer value of the Print Schema element under `node` with this keyword name.
fn integer(node: roxmltree::Node<'_, '_>, tag: &str, name: &str) -> Option<u32> {
    node.descendants()
        .find(|n| n.has_tag_name((NS_PSF, tag)) && named(*n, name))
        .and_then(|n| n.children().find(|v| v.has_tag_name((NS_PSF, "Value"))))
        .and_then(|n| n.text())
        .and_then(|v| v.trim().parse::<u32>().ok())
}

/// Check the driver's validated ticket: it kept the orientation, added no scaling and has a
/// paper (when `paper` is given, that one, to a quarter of a millimetre). Returns the paper's
/// width and height in micrometres as the driver lists it. Only those schema fields are read:
/// some drivers return empty vendor parameters that a stricter parser would refuse.
pub(crate) fn verify_ticket(xml: &str, paper: Option<(u32, u32)>, landscape: bool) -> Result<(u32, u32), Error> {
    let document = roxmltree::Document::parse(xml).map_err(spool_error)?;
    let feature = |name: &str| document.root_element().children().find(|n| n.has_tag_name((NS_PSF, "Feature")) && named(*n, name));
    let media = feature("PageMediaSize").ok_or_else(|| spool_error("the printer's driver did not confirm the paper size; nothing was sent"))?;
    let size = integer(media, "ScoredProperty", "MediaSizeWidth")
        .zip(integer(media, "ScoredProperty", "MediaSizeHeight"))
        .filter(|(w, h)| *w > 0 && *h > 0)
        .ok_or_else(|| spool_error("the printer's driver gave a paper without a size (a roll?); nothing was sent"))?;
    if let Some((w, h)) = paper {
        let same = |a: (u32, u32), b: (u32, u32)| a.0.abs_diff(b.0) <= PAPER_TOLERANCE && a.1.abs_diff(b.1) <= PAPER_TOLERANCE;
        if !same(size, (w, h)) && !same(size, (h, w)) {
            return Err(spool_error("the printer's driver changed the paper size; nothing was sent (choose a paper size it has)"));
        }
    }
    let orientation = feature("PageOrientation").and_then(|f| f.children().find(|n| n.has_tag_name((NS_PSF, "Option"))));
    if !orientation.is_some_and(|n| named(n, if landscape { "Landscape" } else { "Portrait" })) {
        return Err(spool_error("the printer's driver changed the orientation; nothing was sent"));
    }
    if let Some(scaling) = feature("PageScaling") {
        // Microsoft's IPP class driver names its "None" in a namespace of its own.
        if !scaling.children().any(|n| n.has_tag_name((NS_PSF, "Option")) && n.attribute("name").and_then(|v| v.rsplit(':').next()) == Some("None")) {
            return Err(spool_error("the printer's driver would scale the sheets; nothing was sent"));
        }
    }
    Ok(size)
}

/// What a driver's PrintCapabilities offer for the options [`verify_job_options`] checks. A
/// field is `None` when the capabilities couldn't be read; that option is then checked strictly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Offered {
    /// Either standard duplex feature (`JobDuplexAllDocumentsContiguously` or `DocumentDuplex`) has an option.
    pub duplex: Option<bool>,
    /// The colour feature (`PageOutputColor`) has an option other than Monochrome and Grayscale.
    pub color: Option<bool>,
    /// The colour feature has a grey option (Monochrome or Grayscale). A driver without one may
    /// still have a vendor's own grey setting, which a ticket can't ask for by name.
    pub grey: Option<bool>,
    /// The collation feature (`DocumentCollate`) has a Collated option.
    pub collate: Option<bool>,
}

/// What these PrintCapabilities offer ([`Offered`]); all unknown when the document is not
/// PrintCapabilities.
pub(crate) fn offered(capabilities_xml: &str) -> Offered {
    let Ok(document) = roxmltree::Document::parse(capabilities_xml) else { return Offered::default() };
    let root = document.root_element();
    if !root.has_tag_name((NS_PSF, "PrintCapabilities")) {
        return Offered::default();
    }
    let options = |name: &'static str| {
        root.children()
            .filter(move |n| n.has_tag_name((NS_PSF, "Feature")) && named(*n, name))
            .flat_map(|f| f.children().filter(|n| n.has_tag_name((NS_PSF, "Option"))))
    };
    Offered {
        // The schema's two duplex features are alternatives; a driver offers one or the other.
        duplex: Some(options("JobDuplexAllDocumentsContiguously").chain(options("DocumentDuplex")).next().is_some()),
        // A vendor's own option might be colour, so only the two schema keywords count as not.
        color: Some(options("PageOutputColor").any(|n| !named(n, "Monochrome") && !named(n, "Grayscale"))),
        grey: Some(options("PageOutputColor").any(|n| named(n, "Monochrome") || named(n, "Grayscale"))),
        collate: Some(options("DocumentCollate").any(|n| named(n, "Collated"))),
    }
}

/// Whether a `pages`-page job asks for collated copies.
#[cfg(windows)]
pub(crate) fn collated_copies(job: &Job, pages: usize) -> bool {
    job.collate && job.copies.clamp(1, 999) > 1 && pages > 1
}

/// Whether collated copies are made by repeating the document in one job with one copy on the
/// ticket: the job asks for them and the driver can't collate.
#[cfg(windows)]
pub(crate) fn repeats_collated(job: &Job, pages: usize, offered: Offered) -> bool {
    collated_copies(job, pages) && offered.collate == Some(false)
}

/// Whether a repeated job's validated one-copy ticket ([`crate::sheets::Repeat`]) prints on
/// both sides; `None` when the copies can't share one job and are printed as a job each: a
/// duplex option other than the schema's (the blank backs would be guessed), several pages per
/// sheet, binding (booklets and offset stacking included) or stapling, which would treat every
/// copy as one document.
#[cfg(windows)]
pub(crate) fn one_job_sides(xml: &str) -> Option<bool> {
    let document = roxmltree::Document::parse(xml).ok()?;
    let features = |names: &'static [&'static str]| {
        document.root_element().children().filter(move |n| n.has_tag_name((NS_PSF, "Feature")) && names.iter().any(|name| named(*n, name)))
    };
    fn option<'a, 'i>(feature: roxmltree::Node<'a, 'i>) -> Option<roxmltree::Node<'a, 'i>> {
        feature.children().find(|n| n.has_tag_name((NS_PSF, "Option")))
    }
    let mut two_sided = false;
    for feature in features(&["JobDuplexAllDocumentsContiguously", "DocumentDuplex"]) {
        match option(feature) {
            Some(o) if named(o, "OneSided") => {}
            Some(o) if named(o, "TwoSidedLongEdge") || named(o, "TwoSidedShortEdge") => two_sided = true,
            _ => return None,
        }
    }
    if features(&["JobNUpAllDocumentsContiguously", "DocumentNUp"]).any(|f| integer(f, "ScoredProperty", "PagesPerSheet").is_some_and(|n| n > 1)) {
        return None;
    }
    if features(&["JobBindAllDocuments", "DocumentBinding", "JobStapleAllDocuments", "DocumentStaple"])
        .any(|f| !option(f).is_some_and(|o| named(o, "None")))
    {
        return None;
    }
    Some(two_sided)
}

/// Confirm explicit job options survived driver validation. Collation only changes output
/// when both the document and copy count exceed one. Without a duplex request the queue's
/// default applies; without a grayscale request the driver's own colour setting applies.
///
/// A driver that offers an option ([`Offered`]) and returned a ticket without the requested
/// value dropped it, and the job is refused. A driver without the feature is accepted where it
/// can only do what was asked: one-sided on a driver with no two-sided feature, grayscale on one
/// with no colour option, uncollated copies on one that can't collate. Two-sided on a simplex
/// driver is refused, and so are collated copies on a driver that can't collate: Windows doesn't
/// document collating for such a driver (the standard XPS filters make the copies, and leave
/// collated copies to the device only when its GPD or PPD has a Collate command). The job then
/// asks for one copy and repeats the document instead ([`repeats_collated`]), in one job or a
/// job per copy ([`one_job_sides`]). An unknown offer (`None`) is checked strictly. Explicit
/// duplex must appear with the requested option in at least one of the job or document features;
/// if both appear, both must match. The exception is an explicit one-sided request when the
/// driver is known to have no duplex feature. Malformed XML and unretained options are errors.
pub(crate) fn verify_job_options(xml: &str, job: &Job, pages: usize, offered: Offered) -> Result<(), Error> {
    let document = roxmltree::Document::parse(xml).map_err(spool_error)?;
    let root = document.root_element();
    let option = |name: &str| {
        root.children()
            .find(|n| n.has_tag_name((NS_PSF, "Feature")) && named(*n, name))
            .and_then(|f| f.children().find(|n| n.has_tag_name((NS_PSF, "Option"))))
    };
    let refused = |field: &str| spool_error(format!("the printer's driver did not retain the requested {field}; nothing was sent"));
    let copies = job.copies.clamp(1, 999);
    if integer(root, "ParameterInit", "JobCopiesAllDocuments") != Some(copies) {
        return Err(refused("copy count"));
    }
    if copies > 1 && pages > 1 && !option("DocumentCollate").is_some_and(|n| named(n, if job.collate { "Collated" } else { "Uncollated" })) {
        match (job.collate, offered.collate) {
            (false, Some(false)) => {}
            (true, Some(false)) => {
                return Err(spool_error(
                    "the printer's driver can't collate copies; nothing was sent (turn collation off, or print one copy at a time)",
                ));
            }
            _ => return Err(refused("collation")),
        }
    }
    if let Some(duplex) = job.duplex {
        let wanted = match duplex {
            Duplex::Off => "OneSided",
            Duplex::LongEdge => "TwoSidedLongEdge",
            Duplex::ShortEdge => "TwoSidedShortEdge",
        };
        let one_sided_only = duplex == Duplex::Off && offered.duplex == Some(false);
        let duplex_options = [option("JobDuplexAllDocumentsContiguously"), option("DocumentDuplex")];
        let retained = duplex_options.iter().any(Option::is_some) && duplex_options.into_iter().flatten().all(|n| named(n, wanted));
        if !one_sided_only && !retained {
            return Err(refused("simplex/duplex setting"));
        }
    }
    let monochrome_only = offered.color == Some(false);
    // Pages the caller has already made grey print grey whatever the driver does with colour, so
    // grey on the driver is best-effort: a driver with no grey option (Microsoft Print to PDF), or
    // only a vendor's own (storytold/pdfcraft#5806), still prints.
    if job.grayscale
        && !job.grey_pages
        && !monochrome_only
        && !option("PageOutputColor").is_some_and(|n| named(n, "Monochrome") || named(n, "Grayscale"))
    {
        return Err(refused("grayscale"));
    }
    Ok(())
}

/// XML Schema instance namespace (`xsi:type`).
const NS_XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
/// XML Schema datatypes namespace (`xsd:integer`, `xsd:QName`).
const NS_XSD: &str = "http://www.w3.org/2001/XMLSchema";
/// The namespace XML itself binds to the `xml` prefix.
const NS_XML: &str = "http://www.w3.org/XML/1998/namespace";

/// A PrintTicket that sets nothing. Merged and validated by a driver it comes back as the
/// driver's own default ticket, which spells the driver's vendor namespaces the way the driver does.
#[cfg(windows)]
pub(crate) fn empty_ticket() -> String {
    format!(r#"<psf:PrintTicket version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:xsi="{NS_XSI}" xmlns:xsd="{NS_XSD}"/>"#)
}

/// The (prefix, namespace) pairs the root element declares for namespaces other than the Print
/// Schema's: the driver's vendor namespaces.
fn vendor_prefixes(document: &roxmltree::Document<'_>) -> Vec<(String, String)> {
    document
        .root_element()
        .namespaces()
        .filter_map(|n| n.name().map(|prefix| (prefix.to_string(), n.uri().to_string())))
        .filter(|(_, uri)| ![NS_PSF, NS_PSK, NS_XSI, NS_XSD, NS_XML].contains(&uri.as_str()))
        .collect()
}

/// Whether every prefix in a `name` or `constrained` attribute or a QName value of the ticket is
/// declared: the XML parser does not check a prefix inside an attribute value.
fn prefixes_resolve(document: &roxmltree::Document<'_>) -> bool {
    let unresolved = |element: roxmltree::Node<'_, '_>, value: &str| {
        value.split_once(':').is_some_and(|(prefix, _)| element.lookup_namespace_uri(Some(prefix)).is_none())
    };
    !document.descendants().any(|node| {
        if node.is_element() {
            node.attributes().any(|a| a.namespace().is_none() && matches!(a.name(), "name" | "constrained") && unresolved(node, a.value()))
        } else {
            node.is_text() && node.parent().is_some_and(|p| is_qname_value(p) && node.text().is_some_and(|t| unresolved(p, t.trim())))
        }
    })
}

/// A `Value` whose text is a qualified name (`xsi:type="xsd:QName"`): its prefix is a namespace's.
fn is_qname_value(node: roxmltree::Node<'_, '_>) -> bool {
    node.attribute((NS_XSI, "type")).is_some_and(|t| t.rsplit(':').next() == Some("QName"))
}

/// The start of every `xmlns:prefix` declaration's prefix in the start tag that begins at
/// `start`, with the prefix.
fn declared_prefixes(xml: &str, start: usize) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut quote = None;
    let mut at = start;
    let mut previous = None;
    for c in xml.get(start..).unwrap_or_default().chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '>' => break,
            None => {}
        }
        if quote.is_none()
            && c == 'x'
            && previous.is_some_and(char::is_whitespace)
            && let Some(name) = xml.get(at..).and_then(|rest| rest.strip_prefix("xmlns:"))
        {
            let prefix: String = name.chars().take_while(|c| !c.is_whitespace() && *c != '=').collect();
            found.push((at + "xmlns:".len(), prefix));
        }
        previous = Some(c);
        at += c.len_utf8();
    }
    found
}

/// `ticket` with the driver's own prefix for each of the driver's vendor namespaces, where the
/// ticket spells one differently; `None` when nothing needs changing, or when it can't be done
/// safely (then the ticket stays as it is). `driver` is the driver's own ticket
/// ([`empty_ticket`] validated).
///
/// Windows' ticket merge names a vendor namespace with a prefix of its own (`ns0000`) as soon as the
/// merged ticket uses a name from it, such as the driver's own paper option from its capabilities.
/// The prefix means nothing in XML, but some drivers' print pipelines read it as text: the job is
/// accepted, then every page fails in the XPS print API (`0x80040003`, "No such interface
/// supported"). A Brother driver names its namespace `brpsk` and fails with `ns0000`. Only the
/// declarations and the vendor names in `name`/`constrained` attributes and QName values change.
/// A ticket is left as it is when it redeclares one of the prefixes below its root, or has an
/// element or attribute named in a namespace whose prefix would change.
pub(crate) fn respell(ticket: &str, driver: &str) -> Option<String> {
    let document = roxmltree::Document::parse(ticket).ok()?;
    let own = vendor_prefixes(&roxmltree::Document::parse(driver).ok()?);
    let declared = vendor_prefixes(&document);
    let renames: Vec<(&str, &str)> = declared
        .iter()
        .filter_map(|(old, uri)| own.iter().find(|(_, u)| u == uri).map(|(new, _)| (old.as_str(), new.as_str())))
        .filter(|(old, new)| old != new)
        .collect();
    if renames.is_empty() {
        return None;
    }
    let renamed = |old: &str| renames.iter().find(|(o, _)| *o == old).map(|(_, new)| *new);
    // Each prefix must still name one namespace; a swap of two (ns0000 and ns0001) is fine.
    let mut after: Vec<&str> = document.root_element().namespaces().filter_map(|n| n.name()).map(|p| renamed(p).unwrap_or(p)).collect();
    let declared_count = after.len();
    after.sort_unstable();
    after.dedup();
    if after.len() != declared_count {
        return None;
    }
    // Only the root's declarations are rewritten: an element below it that declares either prefix
    // again would make a name mean something else once the prefixes change.
    let touched = |prefix: &str| renames.iter().any(|(old, new)| prefix == *old || prefix == *new);
    let redeclared = document
        .descendants()
        .filter(|n| n.is_element() && *n != document.root_element())
        .any(|n| declared_prefixes(ticket, n.range().start).iter().any(|(_, prefix)| touched(prefix)));
    if redeclared {
        return None;
    }
    // Changing the root's declarations moves every name written with one of these prefixes, and only
    // the `name`/`constrained` values and QName values are rewritten to follow. An element or attribute
    // name (`<ns0000:Feature/>`, `ns0000:flag="1"`) would not follow: it would be left with an undeclared
    // prefix, or, when two namespaces trade prefixes, quietly resolve to the other namespace. Names
    // in a namespace whose prefix stays as it is are not affected.
    let moved: Vec<&str> = document.root_element().namespaces().filter(|n| n.name().is_some_and(touched)).map(|n| n.uri()).collect();
    let names_moved = document.descendants().filter(|n| n.is_element()).any(|n| {
        n.tag_name().namespace().is_some_and(|uri| moved.contains(&uri))
            || n.attributes().any(|a| a.namespace().is_some_and(|uri| moved.contains(&uri)))
    });
    if names_moved {
        return None;
    }
    let respelled = |at: usize| {
        let rest = ticket.get(at..)?;
        renames.iter().find(|(old, _)| rest.strip_prefix(old).is_some_and(|r| r.starts_with(':'))).map(|(old, new)| (at, at + old.len(), *new))
    };
    let mut edits: Vec<(usize, usize, &str)> = Vec::new();
    for (at, prefix) in declared_prefixes(ticket, document.root_element().range().start) {
        if let Some(new) = renamed(&prefix) {
            edits.push((at, at + prefix.len(), new));
        }
    }
    for node in document.descendants() {
        if node.is_element() {
            for a in
                node.attributes().filter(|a| (a.namespace().is_none() && matches!(a.name(), "name" | "constrained")) || a.namespace() == Some(NS_XSI))
            {
                edits.extend(respelled(a.range_value().start));
            }
        } else if node.is_text() && node.parent().is_some_and(is_qname_value) {
            // A pretty-printed ticket puts whitespace before the value; the prefix starts after it.
            let start = node.range().start;
            let lead = ticket.get(start..).map_or(0, |t| t.len() - t.trim_start().len());
            edits.extend(respelled(start + lead));
        }
    }
    edits.sort_unstable_by_key(|(start, ..)| *start);
    let mut out = String::with_capacity(ticket.len());
    let mut copied = 0;
    for (start, end, new) in edits {
        out.push_str(ticket.get(copied..start)?);
        out.push_str(new);
        copied = end;
    }
    out.push_str(ticket.get(copied..)?);
    // A last check on the result: it must parse, declare each namespace under the prefix wanted and
    // resolve every prefix.
    let check = roxmltree::Document::parse(&out).ok()?;
    let mut now = vendor_prefixes(&check);
    let mut wanted: Vec<(String, String)> = declared.iter().map(|(old, uri)| (renamed(old).unwrap_or(old).to_string(), uri.clone())).collect();
    now.sort_unstable();
    wanted.sort_unstable();
    (now == wanted && prefixes_resolve(&check)).then_some(out)
}

/// The widest unprintable margin of the paper, in micrometres, from PrintCapabilities computed
/// for the job's ticket (`PageImageableSize`). The largest of the four, so content centred inside
/// it is clear of every edge whichever way the driver measures. `None` when the driver doesn't say.
#[cfg(windows)]
pub(crate) fn imageable_margin(capabilities_xml: &str) -> Option<u32> {
    let document = roxmltree::Document::parse(capabilities_xml).ok()?;
    let imageable = document.root_element().children().find(|n| n.has_tag_name((NS_PSF, "Property")) && named(*n, "PageImageableSize"))?;
    let width = integer(imageable, "Property", "ImageableSizeWidth")?;
    let height = integer(imageable, "Property", "ImageableSizeHeight")?;
    let area = imageable.children().find(|n| n.has_tag_name((NS_PSF, "Property")) && named(*n, "ImageableArea"))?;
    let (x, y) = (integer(area, "Property", "OriginWidth")?, integer(area, "Property", "OriginHeight")?);
    let (w, h) = (integer(area, "Property", "ExtentWidth")?, integer(area, "Property", "ExtentHeight")?);
    let right = width.checked_sub(x)?.checked_sub(w)?;
    let bottom = height.checked_sub(y)?.checked_sub(h)?;
    Some(x.max(y).max(right).max(bottom))
}

/// The job file's name, which the spooler shows as the document's: the title, with anything that
/// could leave the folder replaced and Windows' device names (`CON`, `LPT1`) avoided.
#[cfg(windows)]
pub(crate) fn job_filename(title: &str) -> String {
    let stem = title.strip_suffix(".pdf").or_else(|| title.strip_suffix(".PDF")).unwrap_or(title);
    let safe: String =
        stem.chars().take(100).map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '(' | ')') { c } else { '_' }).collect();
    // Windows drops trailing dots and spaces from a name.
    let safe = safe.trim_end_matches(['.', ' ']).trim_start();
    let device = safe.split('.').next().map(str::trim_end).is_some_and(|base| {
        let base = base.to_ascii_uppercase();
        ["CON", "PRN", "AUX", "NUL"].contains(&base.as_str())
            || ((base.starts_with("COM") || base.starts_with("LPT")) && base.len() == 4 && base.ends_with(|c: char| c.is_ascii_digit()))
    });
    match (safe.is_empty(), device) {
        (true, _) => "Print.pdf".into(),
        (false, true) => format!("_{safe}.pdf"),
        (false, false) => format!("{safe}.pdf"),
    }
}

#[cfg(test)]
mod tests;
