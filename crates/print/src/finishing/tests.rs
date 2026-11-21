use super::print_schema::{self, Chosen};
use super::*;
use crate::Job;
use crate::ticket::{NS_PSF, NS_PSK};

fn job(finishing: Finishing) -> Job {
    Job { finishing, ..Job::default() }
}

fn staple(s: Staple) -> Finishing {
    Finishing { staple: Some(s), ..Finishing::default() }
}

fn punch(edge: Edge, holes: Option<u8>) -> Finishing {
    Finishing { punch: Some(Punch { edge, holes }), ..Finishing::default() }
}

fn bin(name: &str) -> Finishing {
    Finishing { output_bin: Some(name.into()), ..Finishing::default() }
}

#[test]
fn staples_and_punches_have_their_registry_values() {
    let staples = [
        (Staple::TopLeft, 20),
        (Staple::BottomLeft, 21),
        (Staple::TopRight, 22),
        (Staple::BottomRight, 23),
        (Staple::DualLeft, 28),
        (Staple::DualTop, 29),
        (Staple::DualRight, 30),
        (Staple::DualBottom, 31),
        (Staple::Saddle, 8),
    ];
    for (s, value) in staples {
        assert_eq!(s.ipp(), value, "{s:?}");
    }
    let punches = [
        (Edge::Left, None, 74),
        (Edge::Top, Some(2), 75),
        (Edge::Right, Some(3), 80),
        (Edge::Bottom, Some(3), 81),
        (Edge::Left, Some(4), 82),
        (Edge::Bottom, Some(4), 85),
    ];
    for (edge, holes, value) in punches {
        assert_eq!(Punch { edge, holes }.ipp(), value, "{edge:?} {holes:?}");
    }
}

#[test]
fn lp_carries_the_finishing_as_ipp_options() {
    let f =
        Finishing { staple: Some(Staple::TopLeft), punch: Some(Punch { edge: Edge::Left, holes: Some(3) }), output_bin: Some("face-down".into()) };
    let args = crate::lp_args(&job(f));
    let options: Vec<&str> = args.windows(2).filter(|w| w[0] == "-o").map(|w| w[1].as_str()).collect();
    assert!(options.contains(&"finishings=20,78"), "{args:?}");
    assert!(options.contains(&"output-bin=face-down"), "{args:?}");
    let plain = crate::lp_args(&Job::default());
    assert!(!plain.iter().any(|a| a.starts_with("finishings=") || a.starts_with("output-bin=")), "{plain:?}");
}

#[test]
fn a_hole_count_or_bin_name_that_cannot_be_sent_refuses_the_job() {
    for holes in [0, 1, 5, 255] {
        assert!(punch(Edge::Left, Some(holes)).check().is_err(), "{holes} holes");
    }
    for holes in [None, Some(2), Some(3), Some(4)] {
        assert_eq!(punch(Edge::Left, holes).check(), Ok(()), "{holes:?}");
    }
    let long = "a".repeat(MAX_BIN + 1);
    for name in ["", "face down", "bin=2", "tray,1", "bin\"", long.as_str()] {
        assert!(bin(name).check().is_err(), "{name:?}");
        let args = crate::lp_args(&job(bin(name)));
        assert!(!args.iter().any(|a| a.starts_with("output-bin=")), "{name:?}: {args:?}");
    }
    assert_eq!(bin("Stacker_1.upper-face").check(), Ok(()));
}

#[test]
fn lp_is_not_started_for_a_finishing_it_cannot_send() {
    // A spooler that doesn't exist: the job must be refused before it is asked for.
    let lp = std::process::Command::new("craft-print-test-no-such-spooler");
    let error = crate::submit_via(lp, b"%PDF-1.7", &job(bin("face down")), &crate::Cancel::new()).unwrap_err();
    assert!(error.to_string().contains("output bin"), "{error}");
}

/// The IPP attributes of `request` in order: tag, name (empty for a set's further values), value.
fn ipp_attributes(request: &[u8]) -> Vec<(u8, String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 8;
    while let Some(&tag) = request.get(at) {
        at += 1;
        if tag == 0x03 {
            break;
        }
        if tag < 0x10 {
            continue;
        }
        let len = |at: usize| usize::from(u16::from_be_bytes([request[at], request[at + 1]]));
        let name_len = len(at);
        let name = String::from_utf8(request[at + 2..at + 2 + name_len].to_vec()).unwrap();
        at += 2 + name_len;
        let value_len = len(at);
        out.push((tag, name, request[at + 2..at + 2 + value_len].to_vec()));
        at += 2 + value_len;
    }
    out
}

#[test]
fn an_ipp_job_carries_finishings_as_a_set_of_enums_and_the_output_bin() {
    let f = Finishing { staple: Some(Staple::DualLeft), punch: Some(Punch { edge: Edge::Left, holes: None }), output_bin: Some("stacker-1".into()) };
    let attributes = ipp_attributes(&crate::cups::print_job_request("Office", b"%PDF", &job(f)));
    let at = attributes.iter().position(|(_, name, _)| name == "finishings").unwrap();
    assert_eq!(attributes[at], (0x23, "finishings".into(), 28i32.to_be_bytes().to_vec()));
    assert_eq!(attributes[at + 1], (0x23, String::new(), 74i32.to_be_bytes().to_vec()));
    assert!(attributes.contains(&(0x44, "output-bin".into(), b"stacker-1".to_vec())), "{attributes:?}");
    let plain = ipp_attributes(&crate::cups::print_job_request("Office", b"%PDF", &Job::default()));
    assert!(!plain.iter().any(|(_, name, _)| name == "finishings" || name == "output-bin"), "{plain:?}");
}

#[test]
fn an_ipp_job_with_a_finishing_it_cannot_send_is_refused_before_connecting() {
    let error = crate::cups::print_job_at(&[], b"%PDF", &job(punch(Edge::Top, Some(7)))).unwrap_err();
    assert!(error.to_string().contains("7 holes"), "{error}");
}

/// PrintCapabilities listing `features`: (feature, [(namespace, option)]).
fn capabilities(features: &[(&str, &[(&str, &str)])]) -> String {
    let mut body = String::new();
    for (feature, options) in features {
        body.push_str(&format!("<psf:Feature name=\"psk:{feature}\">"));
        for (namespace, option) in *options {
            body.push_str(&format!("<psf:Option xmlns:o=\"{namespace}\" name=\"o:{option}\"/>"));
        }
        body.push_str("</psf:Feature>");
    }
    format!("<psf:PrintCapabilities xmlns:psf=\"{NS_PSF}\" xmlns:psk=\"{NS_PSK}\" version=\"1\">{body}</psf:PrintCapabilities>")
}

const VENDOR: &str = "http://example.com/printers/finisher";

fn chosen(f: &Finishing, caps: &str) -> Result<Vec<(&'static str, String, String)>, crate::Error> {
    print_schema::chosen(f, caps, "Office").map(|c| c.into_iter().map(|c| (c.feature, c.namespace, c.option)).collect())
}

#[test]
fn a_staple_goes_in_the_feature_the_driver_lists_it_in() {
    let job_wide = capabilities(&[("JobStapleAllDocuments", &[(NS_PSK, "None"), (NS_PSK, "StapleTopLeft")])]);
    assert_eq!(chosen(&staple(Staple::TopLeft), &job_wide).unwrap(), [("JobStapleAllDocuments", NS_PSK.into(), "StapleTopLeft".into())]);
    let document_wide = capabilities(&[("JobStapleAllDocuments", &[(NS_PSK, "None")]), ("DocumentStaple", &[(NS_PSK, "SaddleStitch")])]);
    assert_eq!(chosen(&staple(Staple::Saddle), &document_wide).unwrap(), [("DocumentStaple", NS_PSK.into(), "SaddleStitch".into())]);
    let punches = capabilities(&[("DocumentHolePunch", &[(NS_PSK, "LeftEdge"), (NS_PSK, "TopEdge")])]);
    assert_eq!(chosen(&punch(Edge::Top, None), &punches).unwrap(), [("DocumentHolePunch", NS_PSK.into(), "TopEdge".into())]);
}

#[test]
fn what_the_driver_does_not_list_refuses_the_job() {
    let caps = capabilities(&[("JobStapleAllDocuments", &[(NS_PSK, "StapleTopLeft")]), ("JobHolePunch", &[(NS_PSK, "LeftEdge")])]);
    let refused = [staple(Staple::BottomRight), punch(Edge::Right, None), punch(Edge::Left, Some(2)), bin("Tray1")];
    for f in refused {
        assert!(chosen(&f, &caps).is_err(), "{f:?}");
    }
    // A vendor's option spelled like a public keyword is not the keyword.
    let vendor = capabilities(&[("JobStapleAllDocuments", &[(VENDOR, "StapleTopLeft")])]);
    assert!(chosen(&staple(Staple::TopLeft), &vendor).is_err());
    assert!(chosen(&staple(Staple::TopLeft), "<not-capabilities/>").is_err());
    assert_eq!(chosen(&Finishing::default(), "not even XML").unwrap(), []);
}

#[test]
fn an_output_bin_is_the_drivers_own_option_matched_without_case() {
    let caps = capabilities(&[("JobOutputBin", &[(VENDOR, "Upper"), (VENDOR, "Stacker1")]), ("PageOutputBin", &[(NS_PSK, "AutoSelect")])]);
    assert_eq!(chosen(&bin("stacker1"), &caps).unwrap(), [("JobOutputBin", VENDOR.into(), "Stacker1".into())]);
    assert_eq!(chosen(&bin("autoselect"), &caps).unwrap(), [("PageOutputBin", NS_PSK.into(), "AutoSelect".into())]);
}

#[test]
fn the_validated_ticket_must_keep_every_chosen_option() {
    let f = Finishing { staple: Some(Staple::DualTop), punch: Some(Punch { edge: Edge::Left, holes: None }), output_bin: Some("upper".into()) };
    let caps = capabilities(&[
        ("JobStapleAllDocuments", &[(NS_PSK, "StapleDualTop")]),
        ("JobHolePunch", &[(NS_PSK, "LeftEdge")]),
        ("DocumentOutputBin", &[(VENDOR, "Upper")]),
    ]);
    let picked: Vec<Chosen> = print_schema::chosen(&f, &caps, "Office").unwrap();
    let ticket = print_schema::ticket_xml(&picked);
    // The ticket names each option in its own namespace, and is what `verify` reads back.
    assert_eq!(print_schema::verify(&ticket, &picked), Ok(()));
    assert!(ticket.contains(VENDOR), "{ticket}");
    for dropped in ["StapleDualTop", "LeftEdge", "Upper"] {
        let without = ticket.replace(&format!(":{dropped}\""), ":None\"");
        assert!(print_schema::verify(&without, &picked).is_err(), "{dropped} dropped:\n{without}");
    }
    // The same local name in another namespace is another option.
    let elsewhere = ticket.replace(VENDOR, "http://example.com/other");
    assert!(print_schema::verify(&elsewhere, &picked).is_err());
}

#[test]
fn the_job_finishing_replaces_the_driver_dialogs_and_leaves_it_otherwise() {
    let dialog = format!(
        r#"<psf:PrintTicket xmlns:psf="{NS_PSF}" xmlns:psk="{NS_PSK}" version="1">
 <psf:Feature name="psk:JobStapleAllDocuments"><psf:Option name="psk:StapleTopLeft"/></psf:Feature>
 <psf:Feature name="psk:DocumentHolePunch"><psf:Option name="psk:LeftEdge"/></psf:Feature>
 <psf:Feature name="psk:PageOutputBin"><psf:Option name="psk:AutoSelect"/></psf:Feature>
</psf:PrintTicket>"#
    );
    let kept = crate::ticket::driver_settings_xml(&dialog, &Job::default()).unwrap();
    for feature in ["JobStapleAllDocuments", "DocumentHolePunch", "PageOutputBin"] {
        assert!(kept.contains(feature), "{feature} kept:\n{kept}");
    }
    let cases = [(staple(Staple::Saddle), "JobStapleAllDocuments"), (punch(Edge::Top, None), "DocumentHolePunch"), (bin("upper"), "PageOutputBin")];
    for (f, feature) in cases {
        let left = crate::ticket::driver_settings_xml(&dialog, &job(f)).unwrap();
        assert!(!left.contains(feature), "{feature} left in:\n{left}");
        assert_eq!(left.matches("psf:Feature name").count(), 2, "{left}");
    }
}

#[test]
fn the_windows_preview_names_the_finishing() {
    let f = Finishing { staple: Some(Staple::TopLeft), punch: Some(Punch { edge: Edge::Left, holes: Some(2) }), output_bin: Some("upper".into()) };
    let preview = crate::windows_preview(&job(f), None);
    for word in ["staple=TopLeft", "punch=Left (2 holes)", "output-bin=upper"] {
        assert!(preview.iter().any(|p| p == word), "{word}: {preview:?}");
    }
    assert!(
        !crate::windows_preview(&Job::default(), None)
            .iter()
            .any(|p| p.starts_with("staple") || p.starts_with("punch") || p.starts_with("output-bin"))
    );
}
