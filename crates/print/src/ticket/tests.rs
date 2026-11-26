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

/// A ticket written for these tests in the shape of a converted driver dialog's settings: the
/// job-scope schema features, a custom paper with its parameters, pages per sheet, binding
/// and a poster, a media type, a vendor's quality and the driver's private settings.
fn converted_settings() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<psf:PrintTicket version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:ns0000="http://example.com/vendor" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">
<psf:ParameterInit name="psk:JobCopiesAllDocuments"><psf:Value xsi:type="xsd:integer">4</psf:Value></psf:ParameterInit>
<psf:Feature name="psk:DocumentCollate"><psf:Option name="psk:Uncollated"/></psf:Feature>
<psf:Feature name="psk:JobDuplexAllDocumentsContiguously"><psf:Option name="psk:TwoSidedShortEdge"/></psf:Feature>
<psf:Feature name="psk:DocumentDuplex"><psf:Option name="psk:TwoSidedShortEdge"/></psf:Feature>
<psf:Feature name="psk:PageMediaSize"><psf:Option name="psk:CustomMediaSize">
<psf:ScoredProperty name="psk:MediaSizeWidth"><psf:ParameterRef name="psk:PageMediaSizeMediaSizeWidth"/></psf:ScoredProperty>
</psf:Option></psf:Feature>
<psf:ParameterInit name="psk:PageMediaSizeMediaSizeWidth"><psf:Value xsi:type="xsd:integer">914400</psf:Value></psf:ParameterInit>
<psf:Feature name="psk:PageOrientation"><psf:Option name="psk:Landscape"/></psf:Feature>
<psf:Feature name="psk:PageScaling"><psf:Option name="psk:CustomSquare"/></psf:Feature>
<psf:ParameterInit name="psk:PageScalingScale"><psf:Value xsi:type="xsd:integer">50</psf:Value></psf:ParameterInit>
<psf:Feature name="psk:DocumentNUp"><psf:Option><psf:ScoredProperty name="psk:PagesPerSheet"><psf:Value xsi:type="xsd:integer">2</psf:Value></psf:ScoredProperty></psf:Option></psf:Feature>
<psf:Feature name="psk:JobBindAllDocuments"><psf:Option name="psk:Booklet"/></psf:Feature>
<psf:ParameterInit name="psk:JobBindAllDocumentsGutter"><psf:Value xsi:type="xsd:integer">0</psf:Value></psf:ParameterInit>
<psf:Feature name="psk:PagePoster"><psf:Option><psf:ScoredProperty name="psk:SheetsPerPage"><psf:Value xsi:type="xsd:integer">4</psf:Value></psf:ScoredProperty></psf:Option></psf:Feature>
<psf:Feature name="psk:PageOutputColor"><psf:Option name="psk:Color"/></psf:Feature>
<psf:Feature name="psk:PageMediaType"><psf:Option name="psk:PhotographicGlossy"/></psf:Feature>
<psf:Feature name="psk:PageResolution"><psf:Option name="ns0000:Fine"/></psf:Feature>
<psf:Feature name="ns0000:PageMediaSizeSource"><psf:Option name="ns0000:Roll"/></psf:Feature>
<psf:ParameterInit name="ns0000:PageDevmodeSnapshot"><psf:Value xsi:type="xsd:string">AAECAw==</psf:Value></psf:ParameterInit>
</psf:PrintTicket>"#
    )
}

/// The top-level schema names left in a ticket.
fn schema_names(xml: &str) -> Vec<String> {
    let document = roxmltree::Document::parse(xml).unwrap();
    document.root_element().children().filter(|n| named_in(*n, NS_PSK)).map(|n| n.attribute("name").unwrap_or_default().to_string()).collect()
}

fn named_in(node: roxmltree::Node<'_, '_>, namespace: &str) -> bool {
    node.attribute("name").and_then(|v| v.split_once(':')).is_some_and(|(prefix, _)| node.lookup_namespace_uri(Some(prefix)) == Some(namespace))
}

#[trial]
fn the_driver_dialogs_settings_keep_what_the_job_leaves_open() {
    let settings = converted_settings();
    let kept = driver_settings_xml(&settings, &Job::default()).unwrap();
    assert_eq!(
        schema_names(&kept),
        ["psk:JobDuplexAllDocumentsContiguously", "psk:DocumentDuplex", "psk:PageOutputColor", "psk:PageMediaType", "psk:PageResolution"],
        "without a two-sided or grayscale choice of the job, the dialog's apply"
    );
    for vendor in ["ns0000:Fine", "ns0000:PageMediaSizeSource", "ns0000:PageDevmodeSnapshot", "AAECAw=="] {
        assert!(kept.contains(vendor), "{vendor} is kept");
    }
    for job_owned in [
        "JobCopiesAllDocuments",
        "DocumentCollate",
        "PageMediaSize\"",
        "MediaSizeWidth",
        "Landscape",
        "CustomSquare",
        "PageScalingScale",
        "PagesPerSheet",
        "Booklet",
        "JobBindAllDocumentsGutter",
        "SheetsPerPage",
    ] {
        assert!(!kept.contains(job_owned), "{job_owned} is the job's");
    }
    // Everything kept is the driver's own text, unchanged.
    let line = r#"<psf:Feature name="psk:PageMediaType"><psf:Option name="psk:PhotographicGlossy"/></psf:Feature>"#;
    assert!(kept.contains(line));

    let job = Job { duplex: Some(Duplex::Off), grayscale: true, ..Job::default() };
    assert_eq!(
        schema_names(&driver_settings_xml(&settings, &job).unwrap()),
        ["psk:PageMediaType", "psk:PageResolution"],
        "the job's two-sided and grayscale win"
    );
}

#[trial]
fn the_dialogs_settings_are_matched_by_namespace_not_by_prefix() {
    // The schema keywords under another prefix, or unprefixed in the schema's default
    // namespace, are still the job's; a vendor feature with a schema-like name is the driver's.
    let settings = format!(
        r#"<PrintTicket version="1" xmlns="{NS_PSF}" xmlns:k="{NS_PSK}" xmlns:v="http://example.com/vendor">
<Feature name="k:PageOrientation"><Option name="k:Landscape"/></Feature>
<Feature name="v:PageOrientation"><Option name="v:Mirrored"/></Feature>
<p:Feature xmlns:p="{NS_PSF}" xmlns="{NS_PSK}" name="PageMediaSize"/>
</PrintTicket>"#
    );
    let kept = driver_settings_xml(&settings, &Job::default()).unwrap();
    assert!(!kept.contains("k:Landscape"));
    assert!(kept.contains("v:Mirrored"));
    assert!(!kept.contains("name=\"PageMediaSize\""), "{kept}");
    assert!(roxmltree::Document::parse(&kept).is_ok());
}

#[trial]
fn a_media_type_or_quality_the_driver_replaced_refuses_the_job() {
    let chosen = driver_settings_xml(&converted_settings(), &Job::default()).unwrap();
    // The vendor's namespace under another prefix than the dialog's `ns0000`.
    let ticket = |media: &str, resolution: &str, quality: &str| {
        format!(
            r#"<psf:PrintTicket version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:ns0001="http://example.com/vendor">
<psf:Feature name="psk:PageMediaType"><psf:Option name="{media}"/></psf:Feature>
<psf:Feature name="psk:PageResolution"><psf:Option name="{resolution}"/></psf:Feature>{quality}
</psf:PrintTicket>"#
        )
    };
    assert!(verify_chosen_options(&chosen, &ticket("psk:PhotographicGlossy", "ns0001:Fine", "")).is_ok());
    let respelled = format!(
        r#"<PrintTicket version="1" xmlns="{NS_PSF}" xmlns:k="{NS_PSK}"><Feature name="k:PageMediaType"><Option name="k:PhotographicGlossy"/></Feature><Feature name="k:PageResolution"><Option xmlns:v="http://example.com/vendor" name="v:Fine"/></Feature></PrintTicket>"#
    );
    assert!(verify_chosen_options(&chosen, &respelled).is_ok(), "the same options under other prefixes");
    let replaced = [
        (ticket("psk:Plain", "ns0001:Fine", ""), "media type"),
        (ticket("ns0001:PhotographicGlossy", "ns0001:Fine", ""), "media type"),
        (ticket("psk:Plain", "ns0001:Fine", "").replace("PageMediaType", "Other"), "media type"),
        (ticket("psk:PhotographicGlossy", "ns0001:Draft", ""), "resolution"),
        (ticket("psk:PhotographicGlossy", "psk:Fine", ""), "resolution"),
    ];
    for (replaced, words) in replaced {
        let refused = verify_chosen_options(&chosen, &replaced);
        assert!(matches!(&refused, Err(Error::Spool(m)) if m.contains(words) && m.contains("nothing was sent")), "{replaced}: {refused:?}");
    }

    let with_quality = format!(
        r#"<psf:PrintTicket version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}"><psf:Feature name="psk:PageOutputQuality"><psf:Option name="psk:High"/></psf:Feature></psf:PrintTicket>"#
    );
    let quality = |option: &str| {
        ticket("psk:Plain", "psk:Other", &format!(r#"<psf:Feature name="psk:PageOutputQuality"><psf:Option name="{option}"/></psf:Feature>"#))
    };
    assert!(verify_chosen_options(&with_quality, &quality("psk:High")).is_ok(), "no media type or resolution chosen: not checked");
    let refused = verify_chosen_options(&with_quality, &quality("psk:Draft"));
    assert!(matches!(&refused, Err(Error::Spool(m)) if m.contains("print quality")), "{refused:?}");

    // A resolution is also its dots per inch, with or without an option name.
    let resolution = |name: &str, x: u32, y: u32| {
        format!(
            r#"<psf:PrintTicket version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema"><psf:Feature name="psk:PageResolution"><psf:Option{name}>
<psf:ScoredProperty name="psk:ResolutionX"><psf:Value xsi:type="xsd:integer">{x}</psf:Value></psf:ScoredProperty>
<psf:ScoredProperty name="psk:ResolutionY"><psf:Value xsi:type="xsd:integer"> {y} </psf:Value></psf:ScoredProperty>
</psf:Option></psf:Feature></psf:PrintTicket>"#
        )
    };
    for name in ["", r#" name="psk:Fine""#] {
        let chosen = resolution(name, 600, 600);
        assert!(verify_chosen_options(&chosen, &resolution(name, 600, 600)).is_ok(), "{name}");
        assert!(verify_chosen_options(&chosen, &resolution(name, 600, 600).replace(">600<", ">+0600<")).is_ok(), "{name}: the same number");
        for replaced in [resolution(name, 300, 300), resolution(name, 600, 300), quality("psk:High")] {
            let refused = verify_chosen_options(&chosen, &replaced);
            assert!(matches!(&refused, Err(Error::Spool(m)) if m.contains("resolution")), "{name} {replaced}: {refused:?}");
        }
    }
    assert!(verify_chosen_options(&resolution(r#" name="psk:Fine""#, 600, 600), &resolution("", 600, 600)).is_err(), "the name was dropped");

    // Values the option refers to as parameters.
    let by_reference = |x: u32| {
        format!(
            r#"<psf:PrintTicket version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:v="http://example.com/vendor" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema"><psf:Feature name="psk:PageResolution"><psf:Option name="v:Custom">
<psf:ScoredProperty name="psk:ResolutionX"><psf:ParameterRef name="v:Dpi"/></psf:ScoredProperty>
</psf:Option></psf:Feature><psf:ParameterInit name="v:Dpi"><psf:Value xsi:type="xsd:integer">{x}</psf:Value></psf:ParameterInit></psf:PrintTicket>"#
        )
    };
    assert!(verify_chosen_options(&by_reference(600), &by_reference(600)).is_ok());
    assert!(verify_chosen_options(&by_reference(600), &by_reference(300)).is_err(), "the parameter's value changed");

    // A QName value under another prefix is the same value.
    let qname = |prefix: &str, value: &str| {
        format!(
            r#"<psf:PrintTicket version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:{prefix}="http://example.com/vendor" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema"><psf:Feature name="psk:PageOutputQuality"><psf:Option name="psk:High">
<psf:ScoredProperty name="{prefix}:Mode"><psf:Value xsi:type="xsd:QName">{prefix}:{value}</psf:Value></psf:ScoredProperty>
</psf:Option></psf:Feature></psf:PrintTicket>"#
        )
    };
    assert!(verify_chosen_options(&qname("ns0000", "Photo"), &qname("brpsk", "Photo")).is_ok());
    assert!(verify_chosen_options(&qname("ns0000", "Photo"), &qname("brpsk", "Text")).is_err());
}

#[trial]
fn settings_that_are_not_a_ticket_are_refused() {
    for bad in ["", "<not xml", r#"<psf:PrintCapabilities xmlns:psf="http://schemas.microsoft.com/windows/2003/08/printing/printschemaframework"/>"#]
    {
        let refused = driver_settings_xml(bad, &Job::default());
        assert!(matches!(&refused, Err(Error::Spool(m)) if m.contains("choose them again")), "{bad:?}: {refused:?}");
    }
}

#[trial]
fn the_driver_dialog_opens_on_the_jobs_copies_and_collation() {
    let job = Job { copies: 3, collate: false, duplex: Some(Duplex::LongEdge), grayscale: true, ..Job::default() };
    let xml = dialog_xml(&job).unwrap();
    assert_eq!(schema_names(&xml), ["psk:JobCopiesAllDocuments", "psk:DocumentCollate"]);
    for value in [">3<", "psk:Uncollated"] {
        assert!(xml.contains(value), "missing {value}");
    }
    // The job's two-sided and grayscale choices stay out of the settings the dialog returns,
    // so they do not outlast the job's own choice.
    for value in ["TwoSided", "Grayscale", "Monochrome"] {
        assert!(!xml.contains(value), "{value} is the job's");
    }
    assert_eq!(schema_names(&dialog_xml(&Job::default()).unwrap()), ["psk:JobCopiesAllDocuments", "psk:DocumentCollate"]);
}

/// A `name` with more than 255 spaces around its `=`, and an `xsi` attribute with a name over
/// 64 KiB, are respelled where their values are, not where roxmltree's `range_value` says.
#[trial]
fn values_past_a_long_name_or_many_spaces_are_respelled_in_place() {
    let spaced = |prefix: &str| {
        let pad = " ".repeat(200);
        brother(prefix).replace(&format!(r#"name="{prefix}:JobPrintSpeed""#), &format!(r#"name{pad}={pad}"{prefix}:JobPrintSpeed""#))
    };
    assert_eq!(respell(&spaced("ns0000"), &brother("brpsk")), Some(spaced("brpsk")));
    let long = |prefix: &str| {
        let p = "x".repeat(65_536);
        brother(prefix).replace("</psf:PrintTicket>", &format!(r#"<psf:Value xmlns:{p}="{NS_XSI}" {p}:type="{prefix}:Custom"/></psf:PrintTicket>"#))
    };
    assert_eq!(respell(&long("ns0000"), &brother("brpsk")), Some(long("brpsk")));
}

/// A validated ticket that keeps only the sides its DEVMODE extension could read, as a legacy
/// host-based driver (HP LaserJet P1102, storytold/craft-libs#7230) returns one.
fn validated_partial(width: Option<u32>, height: Option<u32>) -> String {
    let side = |name: &str, value: Option<u32>| {
        value.map_or(String::new(), |v| {
            format!(r#"<psf:ScoredProperty name="psk:{name}"><psf:Value xsi:type="xsd:integer">{v}</psf:Value></psf:ScoredProperty>"#)
        })
    };
    format!(
        r#"<psf:PrintTicket version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">
<psf:Feature name="psk:PageMediaSize"><psf:Option name="psk:ISOA4">{}{}</psf:Option></psf:Feature>
<psf:Feature name="psk:PageOrientation"><psf:Option name="psk:Portrait"/></psf:Feature>
<psf:Feature name="psk:PageScaling"><psf:Option name="psk:None"/></psf:Feature>
</psf:PrintTicket>"#,
        side("MediaSizeWidth", width),
        side("MediaSizeHeight", height)
    )
}

/// storytold/craft-libs#7230: the driver reports one side of the paper as zero for a real
/// sheet. With the sheet the request names, the job goes on with the size it asked for; a
/// side the driver did report must still agree with it, and a request that names no sheet is
/// refused as before.
#[trial]
fn a_legacy_v3_drivers_zero_side_falls_back_to_the_requested_paper() {
    let a4 = (210_000, 297_000);
    assert_eq!(verify_ticket(&validated_partial(None, Some(297_000)), Some(a4), false), Ok(a4));
    assert_eq!(verify_ticket(&validated_partial(Some(0), Some(297_000)), Some(a4), false), Ok(a4), "a zero side is a missing one");
    assert_eq!(verify_ticket(&validated_partial(Some(297_000), None), Some(a4), false), Ok(a4), "either way round");
    // 0.5 pt (176 µm) inside the tolerance still agrees.
    assert_eq!(verify_ticket(&validated_partial(Some(0), Some(297_176)), Some(a4), false), Ok(a4));
    assert_eq!(verify_ticket(&validated_partial(Some(0), Some(297_250)), Some(a4), false), Ok(a4), "exactly the tolerance");
    // A side that disagrees means the driver changed the paper, not that it has a roll.
    let changed = verify_ticket(&validated_partial(Some(0), Some(279_400)), Some(a4), false).unwrap_err();
    assert!(changed.to_string().contains("changed the paper size"), "{changed}");
    assert!(!changed.to_string().contains("roll"), "{changed}");
    assert!(verify_ticket(&validated_partial(Some(0), Some(297_251)), Some(a4), false).is_err(), "one µm past the tolerance");
    // Without a sheet named the zero side stays roll-like.
    assert!(verify_ticket(&validated_partial(Some(0), Some(297_000)), None, false).is_err());
    assert!(verify_ticket(&validated_partial(None, Some(297_000)), None, false).is_err());
    // Both sides gone is refused whatever the request names.
    for wanted in [Some(a4), None] {
        for (width, height) in [(None, None), (Some(0), Some(0))] {
            let refused = verify_ticket(&validated_partial(width, height), wanted, false).unwrap_err();
            assert!(refused.to_string().contains("roll"), "{refused}");
        }
    }
}

/// A validated ticket whose width the option refers to a parameter the ticket sets
/// (`MediaSizeWidth` a `ParameterRef`, storytold/craft-libs#1225), as a driver writes one.
fn validated_referenced(width: u32, height: u32) -> String {
    format!(
        r#"<psf:PrintTicket version="1" xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">
<psf:ParameterInit name="psk:PageMediaSizeWidth"><psf:Value xsi:type="xsd:integer">{width}</psf:Value></psf:ParameterInit>
<psf:Feature name="psk:PageMediaSize"><psf:Option name="psk:ISOA4">
<psf:ScoredProperty name="psk:MediaSizeWidth"><psf:ParameterRef name="psk:PageMediaSizeWidth"/></psf:ScoredProperty>
<psf:ScoredProperty name="psk:MediaSizeHeight"><psf:Value xsi:type="xsd:integer">{height}</psf:Value></psf:ScoredProperty>
</psf:Option></psf:Feature>
<psf:Feature name="psk:PageOrientation"><psf:Option name="psk:Portrait"/></psf:Feature>
<psf:Feature name="psk:PageScaling"><psf:Option name="psk:None"/></psf:Feature>
</psf:PrintTicket>"#
    )
}

/// A paper side a driver keeps in a `ParameterRef` is resolved through the `ParameterInit`
/// the ticket sets, and checked like a direct one: a referenced width that disagrees with the
/// sheet the request names is refused, one that agrees is taken.
#[trial]
fn a_referenced_paper_side_is_resolved_through_the_parameter_it_names() {
    let a4 = (210_000, 297_000);
    assert_eq!(verify_ticket(&validated_referenced(210_000, 297_000), Some(a4), false), Ok(a4));
    assert_eq!(verify_ticket(&validated_referenced(209_750, 297_000), Some(a4), false), Ok((209_750, 297_000)), "exactly the tolerance");
    // A referenced size that disagrees is refused, not rewritten from the request.
    let changed = verify_ticket(&validated_referenced(400_000, 297_000), Some(a4), false).unwrap_err();
    assert!(changed.to_string().contains("changed the paper size"), "{changed}");
    // A reference to a parameter the ticket does not set is a missing side, which falls back
    // to the requested paper like any unreadable one.
    let unset =
        validated_referenced(400_000, 297_000).replace(r#"name="psk:PageMediaSizeWidth"><psf:Value"#, r#"name="psk:OtherParameter"><psf:Value"#);
    assert_eq!(verify_ticket(&unset, Some(a4), false), Ok(a4), "a parameter the ticket does not set is a missing side");
}

/// A paper the driver keeps one side of (storytold/craft-libs#7230) is a candidate of last
/// resort only: no full entry matched, a Print Schema keyword entry, and its known side
/// agrees with the sheet.
#[cfg(windows)]
#[trial]
fn a_partially_known_paper_is_a_candidate_of_last_resort_only() {
    let paper = |width, height, name, keyword| Paper { width, height, name, keyword };
    let partial = paper(0, 297_000, Some("A4"), true);
    let alone = [partial.clone()];
    assert_eq!(matching_paper(&alone, 595.28, 841.89), Some(0), "no full entry: the partial one is taken");
    // A full candidate, however it spells its option, wins over the partial one.
    let with_full = [partial, paper(210_100, 297_000, Some("A4"), false)];
    assert_eq!(matching_paper(&with_full, 595.28, 841.89), Some(1), "the full entry wins");
    // A vendor's own entry is no partial candidate.
    assert_eq!(matching_paper(&[paper(0, 297_000, Some("A4"), false)], 595.28, 841.89), None);
    // A partial entry whose known side disagrees is no candidate.
    assert_eq!(matching_paper(&[paper(0, 279_400, Some("Letter"), true)], 595.28, 841.89), None);
    // Both sides gone: nothing known to agree with.
    assert_eq!(matching_paper(&[paper(0, 0, Some("A4"), true)], 595.28, 841.89), None);
}

/// One partial paper whose known side agrees is still taken, named like the requested
/// standard sheet or not. The known side agrees only to a quarter of a millimetre: exactly
/// the tolerance is taken, one micrometre past it is another paper.
#[cfg(windows)]
#[trial]
fn a_single_partially_known_paper_that_agrees_is_still_taken() {
    let paper = |width, height, name, keyword| Paper { width, height, name, keyword };
    assert_eq!(matching_paper(&[paper(215_900, 0, Some("Legal 215.9 x 355.6 mm"), true)], 612.0, 1008.0), Some(0), "named like the sheet");
    assert_eq!(matching_paper(&[paper(215_900, 0, None, true)], 612.0, 1008.0), Some(0), "or without a name at all");
    assert_eq!(matching_paper(&[paper(0, 297_250, None, true)], 595.28, 841.89), Some(0), "exactly the tolerance");
    assert_eq!(matching_paper(&[paper(0, 297_251, None, true)], 595.28, 841.89), None, "one µm past the tolerance");
}

/// storytold/craft-libs#1225 (Code Rabbit, second round): a single partial paper whose known
/// side agrees is still refused when its own name is known and contradicts the requested
/// standard sheet — the width the driver kept is shared by Letter and Legal, and a known
/// Letter name is evidence against choosing it for a Legal sheet (INV-4). "Extra" is a
/// modifier word, another paper: a known "Letter Extra" name contradicts a Letter sheet too.
/// Without a name there is nothing contradicting, and the agreeing entry is taken as before.
#[cfg(windows)]
#[trial]
fn a_single_partially_known_paper_whose_known_name_contradicts_the_sheet_is_refused() {
    let paper = |width, height, name, keyword| Paper { width, height, name, keyword };
    assert_eq!(
        matching_paper(&[paper(215_900, 0, Some("Letter 215.9 x 279.4 mm"), true)], 612.0, 1008.0),
        None,
        "a known Letter name is not a Legal sheet"
    );
    assert_eq!(
        matching_paper(&[paper(215_900, 0, Some("Letter Extra 215.9 x 431.8 mm"), true)], 612.0, 792.0),
        None,
        "a known \"Letter Extra\" name is not a Letter sheet"
    );
    assert_eq!(matching_paper(&[paper(215_900, 0, None, true)], 612.0, 1008.0), Some(0), "an unnamed twin stands, the agreement alone");
}

/// storytold/craft-libs#1225: two partial papers that both agree (Letter and Legal share a
/// width) are told apart by name, as full candidates are; when nothing tells them apart,
/// none is chosen, not the first one that agrees.
#[cfg(windows)]
#[trial]
fn two_partially_known_papers_that_agree_are_told_apart_by_their_names() {
    let paper = |width, height, name, keyword| Paper { width, height, name, keyword };
    let both = [paper(215_900, 0, Some("Letter"), true), paper(215_900, 0, Some("Legal"), true)];
    assert_eq!(matching_paper(&both, 612.0, 1008.0), Some(1), "the partial named like the sheet wins");
    assert_eq!(matching_paper(&both, 612.0, 792.0), Some(0), "and for a Letter sheet, the Letter one");
    let unnamed = [paper(215_900, 0, None, true), paper(215_900, 0, None, true)];
    assert_eq!(matching_paper(&unnamed, 612.0, 1008.0), None, "nothing to tell them apart by");
    let same_name = [paper(215_900, 0, Some("Legal"), true), paper(215_900, 0, Some("Legal"), true)];
    assert_eq!(matching_paper(&same_name, 612.0, 1008.0), None, "two of the same name are ambiguous too");
}
