use super::*;
use craft_ui::testing::trial;

/// The namespace of the Brother driver's own keywords.
const BROTHER: &str = "http://schemas.brother.info/mfc/printing/2006/11/printschemakeywords";

/// A Brother driver's ticket with its vendor namespace declared as `prefix`. The driver's own
/// spelling is `brpsk`; Windows' ticket merge returned `ns0000` for the same ticket, and the
/// printer then failed every page (`0x80040003`).
fn brother(prefix: &str) -> String {
    format!(
        r#"<psf:PrintTicket xmlns:psf="{NS_PSF}" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema" version="1" xmlns:psk="{NS_PSK}" xmlns:{prefix}="{BROTHER}"><psf:Feature name="psk:PageMediaSize"><psf:Option name="psk:NorthAmericaLetter"><psf:ScoredProperty name="psk:MediaSizeWidth"><psf:Value xsi:type="xsd:integer">215900</psf:Value></psf:ScoredProperty><psf:ScoredProperty name="psk:MediaSizeHeight"><psf:Value xsi:type="xsd:integer">279400</psf:Value></psf:ScoredProperty></psf:Option></psf:Feature><psf:Feature name="psk:PageOrientation"><psf:Option name="psk:Portrait"/></psf:Feature><psf:Property name="{prefix}:PageMediaSizeData"><psf:Property name="{prefix}:CustomUnitType"><psf:Value xsi:type="xsd:string">mm</psf:Value></psf:Property></psf:Property><psf:Feature name="{prefix}:JobPrintSpeed"><psf:Option name="{prefix}:PrinterDefault"/></psf:Feature><psf:ParameterInit name="{prefix}:Mode"><psf:Value xsi:type="xsd:QName">{prefix}:Fast</psf:Value></psf:ParameterInit><psf:ParameterInit name="{prefix}:Label"><psf:Value xsi:type="xsd:string">ns0000:text, not a name</psf:Value></psf:ParameterInit></psf:PrintTicket>"#
    )
}

/// The ticket Windows returns (`ns0000`) gets the driver's own `brpsk` back, in the declaration, the names and the QName values, but not in text.
#[trial]
fn a_vendor_namespace_gets_the_prefix_the_driver_declares_it_with() {
    let merged = brother("ns0000");
    assert_eq!(respell(&merged, &brother("brpsk")), Some(brother("brpsk")), "declaration, names and QName values; not the text");
    assert_eq!(respell(&brother("brpsk"), &brother("brpsk")), None, "already the driver's");
    assert_eq!(respell(&brother("brpsk"), &merged), Some(merged.clone()), "whatever the driver calls it");
    // What the job's checks read is the same either way.
    let letter = (215_900, 279_400);
    assert_eq!(verify_ticket(&merged, Some(letter), false), Ok(letter));
    assert_eq!(verify_ticket(&respell(&merged, &brother("brpsk")).unwrap(), Some(letter), false), Ok(letter));
}

/// A second vendor namespace, next to [`BROTHER`].
const OTHER: &str = "http://example.com/other";

/// A ticket that names [`BROTHER`] with the prefix `a` and [`OTHER`] with `b` in its root, uses
/// each in a `name`, and ends with `extra`.
fn two_namespaces(a: &str, b: &str, extra: &str) -> String {
    format!(
        r#"<psf:PrintTicket xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:{a}="{BROTHER}" xmlns:{b}="{OTHER}"><psf:Feature name="{a}:One"/><psf:Feature name="{b}:Two"/>{extra}</psf:PrintTicket>"#
    )
}

/// When two namespaces trade prefixes both prefixes stay declared, so a name that is only
/// written with a prefix (an element or an attribute, unlike a `name` value) would still parse and
/// resolve, but to the other namespace. Each ticket here is well-formed and has every prefix
/// declared, so `respell` leaves it alone for the name, not because the result didn't parse.
#[trial]
fn a_swap_does_not_move_an_element_or_attribute_name_to_the_other_namespace() {
    let driver = two_namespaces("ns0001", "ns0000", "");
    let refused = |extra: &str| {
        let ticket = two_namespaces("ns0000", "ns0001", extra);
        let document = roxmltree::Document::parse(&ticket).expect("the fixture must stay well-formed");
        assert!(prefixes_resolve(&document));
        respell(&ticket, &driver)
    };
    assert!(refused("").is_some(), "control: the same ticket without such a name is respelled");

    // An element: `ns0000:Feature` is BROTHER's until the declarations swap, then OTHER's.
    assert_eq!(refused("<ns0000:Feature/>"), None, "element in the first namespace");
    assert_eq!(refused("<ns0001:Feature/>"), None, "element in the second namespace");
    assert_eq!(refused("<psf:Property name=\"ns0001:Three\"><ns0000:Child/></psf:Property>"), None, "nested element");

    // An attribute, whichever element carries it.
    assert_eq!(refused("<psf:Feature name=\"ns0000:Three\" ns0000:flag=\"1\"/>"), None, "attribute in the first namespace");
    assert_eq!(refused("<psf:Feature name=\"ns0000:Three\" ns0001:flag=\"1\"/>"), None, "attribute in the second namespace");
    assert_eq!(refused("<ns0001:Feature ns0000:flag=\"1\"/>"), None, "both at once");
}

/// Checks that document duplex drivers keep each requested binding.
#[trial]
fn document_duplex_drivers_keep_each_requested_binding() {
    for choice in [Duplex::Off, Duplex::LongEdge, Duplex::ShortEdge] {
        let job = Job { duplex: Some(choice), ..Job::default() };
        let xml = ticket_xml_for_duplex(&job, false, true);
        assert!(xml.contains("psk:DocumentDuplex"));
        assert!(!xml.contains("psk:JobDuplexAllDocumentsContiguously"));
        assert!(verify_job_options(&xml, &job, 3, Offered::default()).is_ok());
        assert!(verify_job_options(&without(&xml, "DocumentDuplex"), &job, 3, Offered::default()).is_err());
        assert!(verify_job_options(&xml.replace("psk:DocumentDuplex", "ns0000:DocumentDuplex"), &job, 3, Offered::default()).is_err());
        for other in ["OneSided", "TwoSidedLongEdge", "TwoSidedShortEdge"] {
            let wanted = match choice {
                Duplex::Off => "OneSided",
                Duplex::LongEdge => "TwoSidedLongEdge",
                Duplex::ShortEdge => "TwoSidedShortEdge",
            };
            assert!(verify_job_options(&xml.replace(&format!("psk:{wanted}"), &format!("ns0000:{wanted}")), &job, 3, Offered::default()).is_err());
            if other != wanted {
                assert!(verify_job_options(&xml.replace(wanted, other), &job, 3, Offered::default()).is_err());
                let conflicting = xml.replace("</psf:PrintTicket>", &format!(
                    "<psf:Feature name=\"psk:JobDuplexAllDocumentsContiguously\"><psf:Option name=\"psk:{other}\"/></psf:Feature></psf:PrintTicket>"
                ));
                assert!(verify_job_options(&conflicting, &job, 3, Offered::default()).is_err());
            }
        }
    }
}

#[trial]
fn duplex_qnames_resolve_their_in_scope_namespace() {
    for prefix in ["psk:", ""] {
        for choice in [Duplex::Off, Duplex::LongEdge, Duplex::ShortEdge] {
            let option = match choice {
                Duplex::Off => "OneSided",
                Duplex::LongEdge => "TwoSidedLongEdge",
                Duplex::ShortEdge => "TwoSidedShortEdge",
            };
            let xml = format!(
                r#"<psf:PrintCapabilities xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns="{NS_PSK}" xmlns:v="urn:vendor"><psf:Feature name="{prefix}DocumentDuplex"><psf:Option name="{prefix}{option}"/></psf:Feature></psf:PrintCapabilities>"#
            );
            assert!(document_duplex_only(&xml, Some(choice)));
            assert!(!document_duplex_only(&xml, None));
            assert_eq!(two_sided(&xml), Some(choice != Duplex::Off));
            let vendor = xml.replace(&format!("name=\"{prefix}{option}\""), &format!("name=\"v:{option}\""));
            assert!(!document_duplex_only(&vendor, Some(choice)));
            assert_eq!(two_sided(&vendor), Some(false));
            let default_vendor = xml.replace(&format!("xmlns=\"{NS_PSK}\""), "xmlns=\"urn:vendor\"");
            assert_eq!(document_duplex_only(&default_vendor, Some(choice)), !prefix.is_empty());
            let validated = ticket_xml_for_duplex(&Job { duplex: Some(choice), ..Job::default() }, false, true)
                .replace(&format!("xmlns:psk=\"{NS_PSK}\""), &format!("xmlns:psk=\"{NS_PSK}\" xmlns=\"{NS_PSK}\""))
                .replace("psk:", prefix);
            assert!(verify_job_options(&validated, &Job { duplex: Some(choice), ..Job::default() }, 1, Offered::default()).is_ok());
        }
    }
    let unknown = format!(
        r#"<psf:PrintCapabilities xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}"><psf:Feature name="psk:DocumentDuplex"><psf:Option name="psk:TwoSidedVendor"/></psf:Feature></psf:PrintCapabilities>"#
    );
    assert_eq!(two_sided(&unknown), Some(false));
    assert_eq!(two_sided("invalid"), None);
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

/// storytold/pdfcraft#5806: a driver whose only grey option is `Grayscale` is asked for that,
/// since Windows drops a `Monochrome` it doesn't list and the driver's colour comes back.
#[trial]
fn the_ticket_asks_for_the_grey_option_the_driver_lists() {
    let grey = |colors: &[&str]| grey_option(&capabilities_xml(true, colors, COLLATE));
    assert_eq!(grey(&["psk:Color", "psk:Grayscale"]), Grey::Grayscale);
    assert_eq!(grey(&["psk:Color", "psk:Monochrome"]), Grey::Monochrome);
    assert_eq!(grey(&["psk:Color", "psk:Grayscale", "psk:Monochrome"]), Grey::Monochrome, "both: as before");
    assert_eq!(grey(&["psk:Color"]), Grey::Monochrome, "none to ask for: as before");
    assert_eq!(grey(&["psk:Color", "ns0000:Grayscale"]), Grey::Monochrome, "a vendor's option is not the schema's");
    assert_eq!(grey_option("<not xml"), Grey::Monochrome);

    let gray = Job { grayscale: true, ..Job::default() };
    let asked = ticket_xml_with(&gray, false, false, Grey::Grayscale);
    assert!(asked.contains("<psf:Option name=\"psk:Grayscale\"/>") && !asked.contains("psk:Monochrome"));
    assert_eq!(ticket_xml_with(&gray, false, false, Grey::Monochrome), ticket_xml(&gray, false));
    assert!(!ticket_xml_with(&Job::default(), false, false, Grey::Grayscale).contains("PageOutputColor"), "colour asks nothing");
    // The driver keeps it, and the check reads it back.
    assert!(verify_job_options(&asked, &gray, 3, Offered::default()).is_ok());
}

/// storytold/pdfcraft#5806: pages the caller has already made grey print on a driver with no
/// grey option (Microsoft Print to PDF lists only Color; an HP driver has only its own
/// `ns0000:JobHPPrintInGrayScale`), and on one that dropped the request. Without them, such a
/// driver is still refused, since the pages would come out in colour.
#[trial]
fn grey_pages_print_on_a_driver_that_has_no_grey_option() {
    let caps = |colors: &[&str]| offered(&capabilities_xml(true, colors, COLLATE));
    assert_eq!(caps(&["psk:Color"]).grey, Some(false));
    assert_eq!(caps(&["psk:Color", "ns0000:Gray"]).grey, Some(false), "a vendor's option can't be asked for by name");
    assert_eq!(caps(COLOR).grey, Some(true));
    assert_eq!(caps(&["psk:Grayscale"]).grey, Some(true));
    assert_eq!(super::offered("<not xml").grey, None);

    let gray = Job { grayscale: true, ..Job::default() };
    let grey_pages = Job { grey_pages: true, ..gray.clone() };
    // What such a driver returns: colour, or nothing about colour.
    let color = ticket_xml(&gray, false).replace("psk:Monochrome", "psk:Color");
    let dropped = without(&ticket_xml(&gray, false), "PageOutputColor");
    for xml in [&color, &dropped] {
        assert!(verify_job_options(xml, &gray, 3, caps(&["psk:Color"])).is_err(), "colour pages are refused");
        assert!(verify_job_options(xml, &grey_pages, 3, caps(&["psk:Color"])).is_ok(), "grey pages print");
        assert!(verify_job_options(xml, &grey_pages, 3, Offered::default()).is_ok(), "unread capabilities too");
        assert!(verify_job_options(xml, &grey_pages, 3, caps(COLOR)).is_ok(), "and a driver that dropped it");
    }
    // The other options are still checked.
    let copies = Job { copies: 2, ..grey_pages.clone() };
    assert!(verify_job_options(&color, &copies, 3, caps(&["psk:Color"])).is_err(), "copy count");
    assert!(!Job::default().grey_pages, "off unless the caller says so");
}
