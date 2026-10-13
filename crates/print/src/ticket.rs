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
    let color = if job.grayscale { "Monochrome" } else { "Color" };
    let copies = job.copies.clamp(1, 999);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<psf:PrintTicket version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">
 <psf:ParameterInit name="psk:JobCopiesAllDocuments"><psf:Value xsi:type="xsd:integer">{copies}</psf:Value></psf:ParameterInit>
 <psf:Feature name="psk:DocumentCollate"><psf:Option name="psk:{collate}"/></psf:Feature>{duplex}
 <psf:Feature name="psk:PageOutputColor"><psf:Option name="psk:{color}"/></psf:Feature>
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
        for value in [">999<", "psk:Collated", "psk:Color", "psk:Portrait"] {
            assert!(xml.contains(value), "missing {value}");
        }
        assert!(!xml.contains("JobDuplex"), "no duplex asked: the queue's default applies");
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
}
