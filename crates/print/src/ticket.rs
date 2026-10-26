//! The Windows job's PrintTicket, and what is read back from the driver's: plain XML and
//! numbers, so it is tested on every platform. The calls that hand them to the driver are in
//! `windows.rs`.

use crate::{Duplex, Error, Job};

pub(crate) const NS_PSF: &str = "http://schemas.microsoft.com/windows/2003/08/printing/printschemaframework";
pub(crate) const NS_PSK: &str = "http://schemas.microsoft.com/windows/2003/08/printing/printschemakeywords";

/// Paper sizes, in micrometres, this close are the same paper (a quarter of a millimetre).
pub(crate) const PAPER_TOLERANCE: u32 = 250;

fn spool_error(e: impl std::fmt::Display) -> Error {
    Error::Spool(e.to_string())
}

/// Points to micrometres; `None` for a size no printer takes (at most 200 inches, so the
/// conversion can't overflow).
pub(crate) fn micrometres(points: f64) -> Option<u32> {
    (points.is_finite() && points > 0.0 && points <= 14_400.0).then(|| (points * 25_400.0 / 72.0).round() as u32)
}

pub(crate) fn points(micrometres: u32) -> f64 {
    f64::from(micrometres) * 72.0 / 25_400.0
}

/// The PrintTicket the job asks for: the job's options, the orientation, and no scaling (the
/// sheets are laid out at their final size; `sheets.rs` reframes them for shrink-to-fit). The
/// paper is merged in separately, as the driver's own option. Only schema keywords and bounded
/// numbers go into the XML; the printer name and the document title never do. Two-sided is left
/// out when the job doesn't set it, so the queue's default applies.
pub(crate) fn ticket_xml(job: &Job, landscape: bool) -> String {
    let orientation = if landscape { "Landscape" } else { "Portrait" };
    let duplex = match job.duplex {
        None => String::new(),
        Some(duplex) => {
            let option = match duplex {
                Duplex::Off => "OneSided",
                Duplex::LongEdge => "TwoSidedLongEdge",
                Duplex::ShortEdge => "TwoSidedShortEdge",
            };
            format!("\n <psf:Feature name=\"psk:JobDuplexAllDocumentsContiguously\"><psf:Option name=\"psk:{option}\"/></psf:Feature>")
        }
    };
    let collate = if job.collate { "Collated" } else { "Uncollated" };
    // Only grayscale is asked for. Without it the ticket says nothing about colour, so a black and
    // white chosen in the driver's own preferences (Properties…) still applies (storytold/pdfcraft#2422).
    let color = if job.grayscale { "\n <psf:Feature name=\"psk:PageOutputColor\"><psf:Option name=\"psk:Monochrome\"/></psf:Feature>" } else { "" };
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
pub(crate) fn paper_names(width: f64, height: f64) -> &'static [&'static str] {
    let (w, h) = (width.min(height), width.max(height));
    NAMED_PAPERS.iter().find(|((pw, ph), _)| (pw - w).abs() <= 0.5 && (ph - h).abs() <= 0.5).map_or(&[], |(_, names)| names)
}

/// Whether a driver's paper name is `name`: it starts with it, ignoring case, and the name ends
/// there ("A4 210 x 297 mm", not "A4 Extra" for "A4" or "ARCH E1" for "ARCH E").
pub(crate) fn names_paper(display: Option<&str>, name: &str) -> bool {
    let Some(display) = display else { return false };
    display.get(..name.len()).is_some_and(|start| start.eq_ignore_ascii_case(name))
        && display.get(name.len()..).is_some_and(|rest| rest.chars().next().is_none_or(|c| !c.is_alphanumeric()))
}

/// A driver paper offered for the job: its size in micrometres (as the driver lists it), its
/// display name, and whether its option is a Print Schema keyword (`psk:ISOA4`) rather than a
/// vendor's own.
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
    node.attribute("name")
        .and_then(|v| v.split_once(':'))
        .is_some_and(|(prefix, name)| name == local && node.lookup_namespace_uri(Some(prefix)) == Some(NS_PSK))
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
    /// The two-sided feature (`JobDuplexAllDocumentsContiguously`) has any option.
    pub duplex: Option<bool>,
    /// The colour feature (`PageOutputColor`) has an option other than Monochrome and Grayscale.
    pub color: Option<bool>,
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
        collate: Some(options("DocumentCollate").any(|n| named(n, "Collated"))),
    }
}

/// Whether a `pages`-page job asks for collated copies.
pub(crate) fn collated_copies(job: &Job, pages: usize) -> bool {
    job.collate && job.copies.clamp(1, 999) > 1 && pages > 1
}

/// Whether collated copies are made by repeating the document in one job with one copy on the
/// ticket: the job asks for them and the driver can't collate.
pub(crate) fn repeats_collated(job: &Job, pages: usize, offered: Offered) -> bool {
    collated_copies(job, pages) && offered.collate == Some(false)
}

/// Whether a repeated job's validated one-copy ticket ([`crate::sheets::Repeat`]) prints on
/// both sides; `None` when the copies can't share one job and are printed as a job each: a
/// duplex option other than the schema's (the blank backs would be guessed), several pages per
/// sheet, binding (booklets and offset stacking included) or stapling, which would treat every
/// copy as one document.
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
/// job per copy ([`one_job_sides`]). An unknown offer (`None`) is checked strictly.
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
        if !one_sided_only && !option("JobDuplexAllDocumentsContiguously").is_some_and(|n| named(n, wanted)) {
            return Err(refused("simplex/duplex setting"));
        }
    }
    let monochrome_only = offered.color == Some(false);
    if job.grayscale && !monochrome_only && !option("PageOutputColor").is_some_and(|n| named(n, "Monochrome") || named(n, "Grayscale")) {
        return Err(refused("grayscale"));
    }
    Ok(())
}

/// The widest unprintable margin of the paper, in micrometres, from PrintCapabilities computed
/// for the job's ticket (`PageImageableSize`). The largest of the four, so content centred inside
/// it is clear of every edge whichever way the driver measures. `None` when the driver doesn't say.
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
mod tests {
    use super::*;

    #[test]
    fn the_ticket_carries_the_job_options() {
        let job = Job { copies: 3, collate: false, duplex: Some(Duplex::ShortEdge), grayscale: true, title: "<&>".into(), ..Job::default() };
        let xml = ticket_xml(&job, true);
        for value in [">3<", "psk:Uncollated", "psk:TwoSidedShortEdge", "psk:Monochrome", "psk:Landscape", "psk:None"] {
            assert!(xml.contains(value), "missing {value}");
        }
        assert!(!xml.contains("<&>"), "the title never enters the XML");
        assert!(roxmltree::Document::parse(&xml).is_ok());
        let xml = ticket_xml(&Job { copies: 5000, ..Job::default() }, false);
        for value in [">999<", "psk:Collated", "psk:Portrait"] {
            assert!(xml.contains(value), "missing {value}");
        }
        assert!(!xml.contains("JobDuplex"), "no duplex asked: the queue's default applies");
        assert!(!xml.contains("PageOutputColor"), "no grayscale asked: the driver's own colour setting applies");
        assert!(ticket_xml(&Job { duplex: Some(Duplex::Off), ..Job::default() }, false).contains("psk:OneSided"));
        assert!(ticket_xml(&Job { duplex: Some(Duplex::LongEdge), ..Job::default() }, false).contains("psk:TwoSidedLongEdge"));
        assert!(!ticket_xml(&Job { printer: Some("Office \"east\" <1>".into()), ..Job::default() }, false).contains("east"));
    }

    /// A validated ticket as a driver returns it.
    fn validated(paper: (u32, u32), orientation: &str, scaling: Option<&str>) -> String {
        let scaling = scaling.map_or(String::new(), |s| format!(r#"<psf:Feature name="psk:PageScaling"><psf:Option name="{s}"/></psf:Feature>"#));
        format!(
            r#"<psf:PrintTicket version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:ns0000="http://example.com/vendor" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">
<psf:Feature name="psk:PageMediaSize"><psf:Option name="psk:NorthAmericaLetter">
<psf:ScoredProperty name="psk:MediaSizeWidth"><psf:Value xsi:type="xsd:integer">{}</psf:Value></psf:ScoredProperty>
<psf:ScoredProperty name="psk:MediaSizeHeight"><psf:Value xsi:type="xsd:integer">{}</psf:Value></psf:ScoredProperty>
</psf:Option></psf:Feature>
<psf:Feature name="psk:PageOrientation"><psf:Option name="psk:{orientation}"/></psf:Feature>{scaling}
<psf:ParameterInit name="ns0000:Empty"><psf:Value xsi:type="xsd:string"></psf:Value></psf:ParameterInit>
</psf:PrintTicket>"#,
            paper.0, paper.1
        )
    }

    #[test]
    fn a_driver_that_changes_the_sheet_is_refused() {
        let letter = (215_900, 279_400);
        let ticket = validated(letter, "Portrait", Some("psk:None"));
        assert_eq!(verify_ticket(&ticket, Some(letter), false), Ok(letter));
        assert_eq!(verify_ticket(&ticket, Some((279_400, 215_900)), false), Ok(letter), "either way round");
        assert_eq!(verify_ticket(&ticket, None, false), Ok(letter), "shrink-to-fit takes the driver's paper");
        assert!(verify_ticket(&ticket, Some((210_000, 297_000)), false).is_err(), "A4 asked, Letter returned");
        assert!(verify_ticket(&ticket, Some(letter), true).is_err(), "landscape asked, portrait returned");
        assert!(verify_ticket(&validated(letter, "Portrait", Some("psk:FitApplicationBleedSizeToPageImageableSize")), Some(letter), false).is_err());
        assert!(verify_ticket(&validated(letter, "Portrait", Some("ns0000:None")), Some(letter), false).is_ok(), "the IPP class driver's None");
        assert!(verify_ticket(&validated(letter, "Portrait", None), Some(letter), false).is_ok(), "a driver without scaling");
        assert!(verify_ticket(&validated((0, 279_400), "Portrait", None), None, false).is_err(), "a roll has no length");
        assert!(verify_ticket("<not xml", Some(letter), false).is_err());
    }

    #[test]
    fn a_driver_that_drops_explicit_job_options_is_refused() {
        let job = Job { copies: 2, duplex: Some(Duplex::LongEdge), grayscale: true, ..Job::default() };
        let xml = ticket_xml(&job, false);
        assert!(verify_job_options(&xml, &job, 3, Offered::default()).is_ok());
        for (from, to) in
            [(">2<", ">1<"), ("psk:Collated", "psk:Uncollated"), ("psk:TwoSidedLongEdge", "psk:OneSided"), ("psk:Monochrome", "psk:Color")]
        {
            assert!(verify_job_options(&xml.replace(from, to), &job, 3, Offered::default()).is_err(), "{from}");
        }
        assert!(verify_job_options(&xml.replace("psk:Collated", "vendor:Collated"), &job, 3, Offered::default()).is_err(), "namespace matters");
        assert!(verify_job_options(&xml.replace("psk:Monochrome", "psk:Grayscale"), &job, 3, Offered::default()).is_ok());
        for feature in ["DocumentCollate", "JobDuplexAllDocumentsContiguously", "PageOutputColor"] {
            let start = xml.find(&format!("<psf:Feature name=\"psk:{feature}\"")).unwrap();
            let end = start + xml[start..].find("</psf:Feature>").unwrap() + "</psf:Feature>".len();
            let mut dropped = xml.clone();
            dropped.replace_range(start..end, "");
            assert!(verify_job_options(&dropped, &job, 3, Offered::default()).is_err(), "{feature}");
            if feature == "DocumentCollate" {
                assert!(verify_job_options(&dropped, &job, 1, Offered::default()).is_ok(), "one page has no collating order");
            }
        }
        let default = Job::default();
        let xml = ticket_xml(&default, false);
        assert!(verify_job_options(&xml, &default, 3, Offered::default()).is_ok());
        let simplex = Job { duplex: Some(Duplex::Off), ..Job::default() };
        assert!(verify_job_options(&ticket_xml(&simplex, false), &simplex, 3, Offered::default()).is_ok());
        assert!(verify_job_options(&xml, &simplex, 3, Offered::default()).is_err(), "explicit simplex must be confirmed");
        assert!(verify_job_options("<not xml", &default, 1, Offered::default()).is_err());
    }

    /// PrintCapabilities as a driver might write them: with or without a duplex feature, and the
    /// colour and collation options given (none: no such feature).
    fn capabilities_xml(duplex: bool, colors: &[&str], collations: &[&str]) -> String {
        let feature = |name: &str, options: &[&str]| {
            if options.is_empty() {
                return String::new();
            }
            let options: String = options.iter().map(|o| format!(r#"<psf:Option name="{o}"/>"#)).collect();
            format!(r#"<psf:Feature name="psk:{name}">{options}</psf:Feature>"#)
        };
        let duplex = feature("JobDuplexAllDocumentsContiguously", if duplex { &["psk:OneSided", "psk:TwoSidedLongEdge"] } else { &[] });
        format!(
            r#"<psf:PrintCapabilities version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:ns0000="http://example.com/vendor">
{}{duplex}{}
</psf:PrintCapabilities>"#,
            feature("DocumentCollate", collations),
            feature("PageOutputColor", colors)
        )
    }

    const COLOR: &[&str] = &["psk:Color", "psk:Monochrome"];
    const COLLATE: &[&str] = &["psk:Collated", "psk:Uncollated"];

    /// The ticket without one of its features, as a driver that doesn't know it returns it.
    fn without(xml: &str, feature: &str) -> String {
        let start = xml.find(&format!("<psf:Feature name=\"psk:{feature}\"")).unwrap();
        let end = start + xml[start..].find("</psf:Feature>").unwrap() + "</psf:Feature>".len();
        let mut dropped = xml.to_owned();
        dropped.replace_range(start..end, "");
        dropped
    }

    #[test]
    fn a_driver_without_a_duplex_feature_prints_one_side_and_is_not_refused_for_it() {
        let offered = |duplex| offered(&capabilities_xml(duplex, COLOR, COLLATE));
        assert_eq!(offered(true), Offered { duplex: Some(true), color: Some(true), collate: Some(true) });
        assert_eq!(offered(false).duplex, Some(false));
        assert_eq!(super::offered("<psf:PrintCapabilities xmlns:psf=\"x\"/>"), Offered::default(), "another namespace is not capabilities");
        assert_eq!(super::offered("<not xml"), Offered::default());
        let empty = format!(
            r#"<psf:PrintCapabilities xmlns:psf="{NS_PSF}"><psf:Feature name="psk:JobDuplexAllDocumentsContiguously"/></psf:PrintCapabilities>"#
        );
        assert_eq!(super::offered(&empty).duplex, Some(false), "a feature with no option offers nothing");
        let document_duplex = format!(
            r#"<psf:PrintCapabilities xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}"><psf:Feature name="psk:DocumentDuplex"><psf:Option name="psk:OneSided"/><psf:Option name="psk:TwoSidedLongEdge"/></psf:Feature></psf:PrintCapabilities>"#
        );
        assert_eq!(super::offered(&document_duplex).duplex, Some(true), "DocumentDuplex is a duplex feature too");

        let one_sided = Job { duplex: Some(Duplex::Off), ..Job::default() };
        let two_sided = Job { duplex: Some(Duplex::LongEdge), ..Job::default() };
        let kept = ticket_xml(&one_sided, false);
        // The ticket a driver with no duplex feature returns: the setting is gone.
        let none_kept = ticket_xml(&Job::default(), false);

        // The Print dialog's defaults (duplex Off) on a simplex-only queue.
        assert!(verify_job_options(&none_kept, &one_sided, 3, offered(false)).is_ok());
        // Offered and kept.
        assert!(verify_job_options(&kept, &one_sided, 3, offered(true)).is_ok());
        // Offered but the ticket lost it: the driver dropped the choice.
        assert!(verify_job_options(&none_kept, &one_sided, 3, offered(true)).is_err());
        // Two-sided never rests on the missing feature.
        assert!(verify_job_options(&none_kept, &two_sided, 3, offered(false)).is_err());
        // Unknown capabilities stay strict.
        assert!(verify_job_options(&none_kept, &one_sided, 3, Offered::default()).is_err());
        // The other options are still checked on a simplex-only queue.
        let copies = Job { copies: 2, duplex: Some(Duplex::Off), ..Job::default() };
        assert!(verify_job_options(&none_kept, &copies, 3, offered(false)).is_err(), "copy count");
        let gray = Job { grayscale: true, duplex: Some(Duplex::Off), ..Job::default() };
        assert!(verify_job_options(&none_kept, &gray, 3, offered(false)).is_err(), "grayscale");
    }

    #[test]
    fn a_driver_without_colour_prints_grayscale_and_is_not_refused_for_it() {
        let colors = |colors: &[&str]| offered(&capabilities_xml(true, colors, COLLATE)).color;
        assert_eq!(colors(COLOR), Some(true));
        assert_eq!(colors(&["psk:Monochrome"]), Some(false), "monochrome-only");
        assert_eq!(colors(&["psk:Grayscale", "psk:Monochrome"]), Some(false));
        assert_eq!(colors(&[]), Some(false), "no colour feature");
        assert_eq!(colors(&["psk:Monochrome", "ns0000:Vivid"]), Some(true), "a vendor's option may be colour");

        let gray = Job { grayscale: true, ..Job::default() };
        let kept = ticket_xml(&gray, false);
        // A driver without the feature returns a ticket without it.
        let dropped = without(&kept, "PageOutputColor");
        let color = kept.replace("psk:Monochrome", "psk:Color");
        let caps = |colors: &[&str]| offered(&capabilities_xml(true, colors, COLLATE));

        assert!(verify_job_options(&dropped, &gray, 3, caps(&["psk:Monochrome"])).is_ok());
        assert!(verify_job_options(&dropped, &gray, 3, caps(&[])).is_ok());
        assert!(verify_job_options(&kept, &gray, 3, caps(COLOR)).is_ok());
        // A colour driver that dropped or changed the request.
        assert!(verify_job_options(&color, &gray, 3, caps(COLOR)).is_err());
        assert!(verify_job_options(&dropped, &gray, 3, caps(COLOR)).is_err());
        assert!(verify_job_options(&dropped, &gray, 3, Offered::default()).is_err(), "unknown stays strict");
    }

    #[test]
    fn copies_on_a_driver_that_cannot_collate_are_accepted_only_uncollated() {
        let caps = |collations: &[&str]| offered(&capabilities_xml(true, COLOR, collations));
        assert_eq!(caps(COLLATE).collate, Some(true));
        assert_eq!(caps(&["psk:Uncollated"]).collate, Some(false));
        assert_eq!(caps(&[]).collate, Some(false));

        let collated = Job { copies: 2, collate: true, ..Job::default() };
        let uncollated = Job { copies: 2, collate: false, ..Job::default() };
        let dropped = |job: &Job| without(&ticket_xml(job, false), "DocumentCollate");

        // Uncollated copies are what a driver without collation prints.
        assert!(verify_job_options(&dropped(&uncollated), &uncollated, 3, caps(&[])).is_ok());
        assert!(verify_job_options(&ticket_xml(&uncollated, false), &uncollated, 3, caps(&["psk:Uncollated"])).is_ok());
        // Collated copies are refused, saying why.
        let refused = verify_job_options(&dropped(&collated), &collated, 3, caps(&[])).unwrap_err();
        assert!(refused.to_string().contains("can't collate"), "{refused}");
        // A driver that collates and dropped or changed the request.
        assert!(verify_job_options(&dropped(&collated), &collated, 3, caps(COLLATE)).is_err());
        assert!(verify_job_options(&dropped(&uncollated), &uncollated, 3, caps(COLLATE)).is_err());
        let changed = ticket_xml(&collated, false).replace("psk:Collated", "psk:Uncollated");
        assert!(verify_job_options(&changed, &collated, 3, caps(COLLATE)).is_err());
        assert!(verify_job_options(&dropped(&uncollated), &uncollated, 3, Offered::default()).is_err(), "unknown stays strict");
        // The job repeats the document instead of asking for collated copies.
        assert!(repeats_collated(&collated, 3, caps(&[])));
        assert!(!repeats_collated(&collated, 3, caps(COLLATE)) && !repeats_collated(&collated, 3, Offered::default()));
        assert!(!repeats_collated(&uncollated, 3, caps(&[])) && !repeats_collated(&collated, 1, caps(&[])));
        assert!(!repeats_collated(&Job { copies: 1, ..collated.clone() }, 3, caps(&[])));
        // One page or one copy has no collating order.
        assert!(verify_job_options(&dropped(&collated), &collated, 1, caps(&[])).is_ok());
    }

    #[test]
    fn the_driver_paper_named_like_the_standard_size_is_preferred() {
        let paper = |width, height, name, keyword| Paper { width, height, name, keyword };
        // A plotter listing 24 × 36 in three times: a custom entry first, "ARCH D", and one a hair off.
        let papers = [
            paper(609_600, 914_400, Some("Custom 24x36"), false),
            paper(609_700, 914_400, Some("ARCH D 24 x 36 in"), false),
            paper(609_600, 914_400, Some("24 x 36"), true),
        ];
        assert_eq!(matching_paper(&papers, 2592.0, 1728.0), Some(1), "named, either way round");
        assert_eq!(matching_paper(&papers[..1], 1728.0, 2592.0), Some(0));
        assert_eq!(matching_paper(&[papers[0].clone(), papers[2].clone()], 1728.0, 2592.0), Some(1), "then a schema keyword");
        assert_eq!(matching_paper(&papers, 1728.0, 2600.0), None, "3 mm off is another paper");
        assert_eq!(matching_paper(&papers, f64::NAN, 2592.0), None);
        assert_eq!(paper_names(595.0, 842.0), ["A4"], "rounded metric points");
        assert_eq!(paper_names(792.0, 612.0), ["Letter", "US Letter"]);
        assert!(paper_names(360.0, 504.0).is_empty());
        assert!(names_paper(Some("ARCH D 24 x 36 in"), "ARCH D"));
        assert!(names_paper(Some("arch d"), "ARCH D"));
        assert!(!names_paper(Some("ANSI D"), "ARCH D"));
        assert!(!names_paper(Some("ARCH E1"), "ARCH E"));
        assert!(!names_paper(Some("A4Extra"), "A4"));
        assert!(!names_paper(Some("Ä"), "A4"), "never slices inside a character");
        assert!(!names_paper(None, "A4"));
    }

    #[test]
    fn the_unprintable_margin_comes_from_the_capabilities() {
        let caps = |x: u32, y: u32, w: u32, h: u32| {
            format!(
                r#"<psf:PrintCapabilities version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">
<psf:Property name="psk:PageImageableSize">
 <psf:Property name="psk:ImageableSizeWidth"><psf:Value xsi:type="xsd:integer">215900</psf:Value></psf:Property>
 <psf:Property name="psk:ImageableSizeHeight"><psf:Value xsi:type="xsd:integer">279400</psf:Value></psf:Property>
 <psf:Property name="psk:ImageableArea">
  <psf:Property name="psk:OriginWidth"><psf:Value xsi:type="xsd:integer">{x}</psf:Value></psf:Property>
  <psf:Property name="psk:OriginHeight"><psf:Value xsi:type="xsd:integer">{y}</psf:Value></psf:Property>
  <psf:Property name="psk:ExtentWidth"><psf:Value xsi:type="xsd:integer">{w}</psf:Value></psf:Property>
  <psf:Property name="psk:ExtentHeight"><psf:Value xsi:type="xsd:integer">{h}</psf:Value></psf:Property>
 </psf:Property>
</psf:Property>
</psf:PrintCapabilities>"#
            )
        };
        assert_eq!(imageable_margin(&caps(4_233, 4_233, 207_434, 270_934)), Some(4_233));
        assert_eq!(imageable_margin(&caps(3_000, 5_000, 209_900, 270_000)), Some(5_000), "the widest of the four");
        assert_eq!(imageable_margin(&caps(0, 0, 215_900, 279_400)), Some(0), "borderless");
        assert_eq!(imageable_margin(&caps(10_000, 0, 215_900, 279_400)), None, "an area past the paper is not believed");
        assert_eq!(imageable_margin("<psf:PrintCapabilities/>"), None);
    }

    #[test]
    fn points_and_micrometres_convert_both_ways() {
        assert_eq!(micrometres(612.0), Some(215_900));
        assert_eq!(micrometres(841.89), Some(297_000));
        assert!((points(215_900) - 612.0).abs() < 1e-9);
        assert!((points(4_064) - 11.52).abs() < 1e-9, "a 4 mm hardware margin");
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 14_401.0] {
            assert_eq!(micrometres(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_job_file_stays_in_its_folder() {
        let name = job_filename(r"..\..\CON:/unsafe?name");
        assert_eq!(std::path::Path::new(&name).components().count(), 1, "{name}");
        assert!(name.ends_with(".pdf"));
        assert_eq!(job_filename("Invoice 42.pdf"), "Invoice 42.pdf");
        assert_eq!(job_filename("Report (draft) v1.2"), "Report (draft) v1.2.pdf");
        assert_eq!(job_filename("con"), "_con.pdf");
        assert_eq!(job_filename("LPT1.txt"), "_LPT1.txt.pdf");
        assert_eq!(job_filename("COMPLETE"), "COMPLETE.pdf");
        assert_eq!(job_filename("..."), "Print.pdf");
        assert_eq!(job_filename(""), "Print.pdf");
        assert_eq!(job_filename("Отчёт"), "Отчёт.pdf");
    }

    #[test]
    fn a_repeated_job_reads_its_sides_and_leaves_layouts_that_mix_copies_to_separate_jobs() {
        let xml = ticket_xml(&Job { duplex: Some(Duplex::LongEdge), ..Job::default() }, false);
        assert_eq!(one_job_sides(&xml), Some(true));
        assert_eq!(one_job_sides(&xml.replace("JobDuplexAllDocumentsContiguously", "DocumentDuplex")), Some(true));
        assert_eq!(one_job_sides(&ticket_xml(&Job { duplex: Some(Duplex::Off), ..Job::default() }, false)), Some(false));
        assert_eq!(one_job_sides(&ticket_xml(&Job::default(), false)), Some(false), "no duplex feature");
        assert_eq!(one_job_sides(&xml.replace("psk:TwoSidedLongEdge", "vendor:Duplex")), None, "an option it can't read");
        let with = |feature: &str| xml.replace("</psf:PrintTicket>", &format!("{feature}</psf:PrintTicket>"));
        let n_up = |n: u32| {
            with(&format!(
                r#"<psf:Feature name="psk:JobNUpAllDocumentsContiguously"><psf:Option><psf:ScoredProperty name="psk:PagesPerSheet"><psf:Value xsi:type="xsd:integer">{n}</psf:Value></psf:ScoredProperty></psf:Option></psf:Feature>"#
            ))
        };
        assert_eq!(one_job_sides(&n_up(1)), Some(true));
        assert_eq!(one_job_sides(&n_up(2)), None);
        for feature in ["JobBindAllDocuments", "JobStapleAllDocuments", "DocumentStaple"] {
            let finishing = |option: &str| with(&format!(r#"<psf:Feature name="psk:{feature}"><psf:Option name="psk:{option}"/></psf:Feature>"#));
            assert_eq!(one_job_sides(&finishing("None")), Some(true), "{feature}");
            assert_eq!(one_job_sides(&finishing("StapleTopLeft")), None, "{feature}");
        }
    }
}
