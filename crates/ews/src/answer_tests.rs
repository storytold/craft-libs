//! Answers and requests, from the tests of the SOAP layer of the source.

use proptest::prelude::*;

use super::*;
use crate::codes::{DEFAULT_BACK_OFF_MS, MAX_BACK_OFF_MS, MIN_BACK_OFF_MS};
use crate::soap::{Element, Version, envelope, escape};

fn reply_with(code: &str, extra: &str) -> String {
    format!(
        "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><m:GetFolderResponse xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\">\
         <m:ResponseMessages><m:GetFolderResponseMessage ResponseClass=\"Error\"><m:ResponseCode>{code}</m:ResponseCode>{extra}</m:GetFolderResponseMessage></m:ResponseMessages>\
         </m:GetFolderResponse></s:Body></s:Envelope>"
    )
}

/// The error the first message of a `GetFolder` answer names.
fn folder_error(body: &str) -> Error {
    let answer = Answer::parse(body.as_bytes()).unwrap();
    answer.messages("GetFolder").unwrap()[0].ok().unwrap_err()
}

#[test]
fn a_busy_server_says_how_long_and_is_believed_only_so_far() {
    assert_eq!(
        folder_error(&reply_with("ErrorServerBusy", "<m:BackOffMilliseconds>soon</m:BackOffMilliseconds>")),
        Error::Busy { back_off_ms: DEFAULT_BACK_OFF_MS }
    );
    assert_eq!(
        folder_error(&reply_with("ErrorServerBusy", "<m:BackOffMilliseconds>7000</m:BackOffMilliseconds>")),
        Error::Busy { back_off_ms: 7_000 }
    );
    assert_eq!(
        folder_error(&reply_with("ErrorServerBusy", "<m:BackOffMilliseconds>9999999999999999999999</m:BackOffMilliseconds>")),
        Error::Busy { back_off_ms: MAX_BACK_OFF_MS }
    );
    assert_eq!(folder_error(&reply_with("ErrorServerBusy", "")), Error::Busy { back_off_ms: DEFAULT_BACK_OFF_MS });
    // A wait of nothing is a wait.
    assert_eq!(
        folder_error(&reply_with("ErrorServerBusy", "<m:BackOffMilliseconds>0</m:BackOffMilliseconds>")),
        Error::Busy { back_off_ms: MIN_BACK_OFF_MS }
    );
    // The whole answer says so.
    let answer = Answer::parse(reply_with("ErrorServerBusy", "<m:BackOffMilliseconds>7000</m:BackOffMilliseconds>").as_bytes()).unwrap();
    assert_eq!(answer.busy(), Some(Error::Busy { back_off_ms: 7_000 }));
    assert_eq!(Answer::parse(reply_with("ErrorAccessDenied", "").as_bytes()).unwrap().busy(), None);
}

#[test]
fn a_fault_is_the_error_it_names() {
    let fault = |code: &str| {
        format!(
            "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><s:Fault><faultcode>s:Client</faultcode><detail>\
             <e:ResponseCode xmlns:e=\"http://schemas.microsoft.com/exchange/services/2006/errors\">{code}</e:ResponseCode></detail></s:Fault></s:Body></s:Envelope>"
        )
    };
    assert_eq!(Answer::parse(fault("ErrorSchemaValidation").as_bytes()).unwrap_err(), Error::Server(ResponseCode::SchemaValidation));
    assert_eq!(Answer::parse(fault("ErrorInvalidRequest").as_bytes()).unwrap_err(), Error::Server(ResponseCode::InvalidRequest));
    assert_eq!(Answer::parse(fault("not a code").as_bytes()).unwrap_err(), Error::Server(ResponseCode::Other(String::new())));
}

#[test]
fn prefixes_are_the_documents_own_choice() {
    let body = "<a:Envelope xmlns:a=\"http://schemas.xmlsoap.org/soap/envelope/\"><a:Body>\
        <x:GetFolderResponse xmlns:x=\"http://schemas.microsoft.com/exchange/services/2006/messages\" xmlns:y=\"http://schemas.microsoft.com/exchange/services/2006/types\"><x:ResponseMessages>\
        <x:GetFolderResponseMessage ResponseClass=\"Success\"><x:ResponseCode>NoError</x:ResponseCode><x:Folders><y:Folder><y:FolderId Id=\"AbC+/=-_\"/><y:DisplayName>Inbox</y:DisplayName></y:Folder></x:Folders>\
        </x:GetFolderResponseMessage></x:ResponseMessages></x:GetFolderResponse></a:Body></a:Envelope>";
    let answer = Answer::parse(body.as_bytes()).unwrap();
    let messages = answer.messages("GetFolder").unwrap();
    assert_eq!((messages[0].class(), messages[0].code()), (ResponseClass::Success, &ResponseCode::NoError));
    let folder = messages[0].ok().unwrap().path(&["m:Folders", "t:Folder"]).unwrap();
    assert_eq!(folder.child_attr("t:FolderId", "Id"), Some("AbC+/=-_"));
    assert_eq!(folder.child_text("t:DisplayName").as_deref(), Some("Inbox"));
}

#[test]
fn an_element_in_the_wrong_namespace_is_not_the_one_asked_for() {
    let body = reply_with("NoError", "").replace("services/2006/messages", "services/2006/elsewhere");
    assert_eq!(Answer::parse(body.as_bytes()).unwrap().messages("GetFolder").unwrap_err(), Error::Unexpected);
    // Nor is another operation's answer.
    assert_eq!(Answer::parse(reply_with("NoError", "").as_bytes()).unwrap().messages("GetItem").unwrap_err(), Error::Unexpected);
}

#[test]
fn unknown_elements_are_skipped() {
    let extra = "<m:Surprise><m:Deeper>1</m:Deeper></m:Surprise><x:Other xmlns:x=\"urn:other\"/>";
    assert_eq!(folder_error(&reply_with("ErrorAccessDenied", extra)), Error::Server(ResponseCode::AccessDenied));
}

#[test]
fn response_codes_are_kept_only_as_short_identifiers() {
    assert_eq!(ResponseCode::from_name(" ErrorServerBusy "), ResponseCode::ServerBusy);
    assert_eq!(ResponseCode::from_name("ErrorSomethingNew"), ResponseCode::Other("ErrorSomethingNew".into()));
    assert_eq!(ResponseCode::from_name("bad code\r\nX: y"), ResponseCode::Other(String::new()));
    assert_eq!(ResponseCode::from_name(&"A".repeat(500)), ResponseCode::Other(String::new()));
    assert_eq!(ResponseCode::from_name("bad\r\nX: y").to_string(), "an unnamed error");
}

#[test]
fn a_message_without_a_class_or_a_code_is_unexpected() {
    let no_class = reply_with("NoError", "").replace(" ResponseClass=\"Error\"", "");
    assert_eq!(Answer::parse(no_class.as_bytes()).unwrap().messages("GetFolder").unwrap_err(), Error::Unexpected);
    let odd_class = reply_with("NoError", "").replace("\"Error\"", "\"Maybe\"");
    assert_eq!(Answer::parse(odd_class.as_bytes()).unwrap().messages("GetFolder").unwrap_err(), Error::Unexpected);
    let no_code = reply_with("NoError", "").replace("<m:ResponseCode>NoError</m:ResponseCode>", "");
    assert_eq!(Answer::parse(no_code.as_bytes()).unwrap().messages("GetFolder").unwrap_err(), Error::Unexpected);
}

#[test]
fn hostile_answers_are_errors_never_panics() {
    for body in [
        "<!DOCTYPE x [<!ENTITY a 'b'>]><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body/></s:Envelope>".to_string(),
        "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body>".to_string(),
        "not xml at all".to_string(),
        format!(
            "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body>{}{}</s:Body></s:Envelope>",
            "<x>".repeat(MAX_DEPTH),
            "</x>".repeat(MAX_DEPTH)
        ),
        format!("<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body>{}</s:Body></s:Envelope>", "<x/>".repeat(MAX_NODES)),
    ] {
        assert_eq!(Answer::parse(body.as_bytes()).unwrap_err(), Error::Malformed, "{}", &body[..body.len().min(80)]);
    }
    assert_eq!(Answer::parse(&vec![b' '; MAX_BYTES + 1]).unwrap_err(), Error::Malformed);
    assert_eq!(Answer::parse(b"<html><body>Sign in</body></html>").unwrap_err(), Error::Unexpected);
    assert_eq!(Answer::parse(b"<s:Envelope xmlns:s='http://schemas.xmlsoap.org/soap/envelope/'/>").unwrap_err(), Error::Unexpected);
    // An entity XML does not define is left out, and the rest is read.
    let body = reply_with("Error&nonsense;AccessDenied", "");
    assert_eq!(folder_error(&body), Error::Server(ResponseCode::AccessDenied));
}

#[test]
fn a_busy_fault_with_its_wait_in_the_types_namespace_is_read() {
    let fault = |wait: &str| {
        format!(
            "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><s:Fault><faultcode>s:Server</faultcode><detail>\
             <e:ResponseCode xmlns:e=\"http://schemas.microsoft.com/exchange/services/2006/errors\">ErrorServerBusy</e:ResponseCode>\
             <t:MessageXml xmlns:t=\"http://schemas.microsoft.com/exchange/services/2006/types\"><t:Value Name=\"BackOffMilliseconds\">{wait}</t:Value></t:MessageXml>\
             </detail></s:Fault></s:Body></s:Envelope>"
        )
    };
    assert_eq!(Answer::parse(fault("297749").as_bytes()).unwrap_err(), Error::Busy { back_off_ms: 297_749 });
    assert_eq!(Answer::parse(fault("9999999").as_bytes()).unwrap_err(), Error::Busy { back_off_ms: MAX_BACK_OFF_MS });
}

#[test]
fn a_busy_server_inside_an_internal_server_error_is_busy() {
    let inner = "<m:MessageXml><t:Value xmlns:t=\"x\" Name=\"InnerErrorResponseCode\">ErrorServerBusy</t:Value><t:Value xmlns:t=\"x\" Name=\"BackOffMilliseconds\">4000</t:Value></m:MessageXml>";
    assert_eq!(folder_error(&reply_with("ErrorInternalServerError", inner)), Error::Busy { back_off_ms: 4_000 });
    // Without the inner error it is the server's own error, and not a reason to wait.
    let body =
        reply_with("ErrorInternalServerError", "<m:MessageXml><t:Value xmlns:t=\"x\" Name=\"BackOffMilliseconds\">4000</t:Value></m:MessageXml>");
    assert_eq!(folder_error(&body), Error::Server(ResponseCode::InternalServerError));
    // The same in a fault.
    let fault = "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><s:Fault><faultcode>s:Server</faultcode><detail>\
        <e:ResponseCode xmlns:e=\"http://schemas.microsoft.com/exchange/services/2006/errors\">ErrorInternalServerError</e:ResponseCode>\
        <e:MessageXml xmlns:e=\"http://schemas.microsoft.com/exchange/services/2006/errors\"><e:Value Name=\"InnerErrorResponseCode\">ErrorServerBusy</e:Value><e:Value Name=\"BackOffMilliseconds\">2500</e:Value></e:MessageXml></detail></s:Fault></s:Body></s:Envelope>";
    assert_eq!(Answer::parse(fault.as_bytes()).unwrap_err(), Error::Busy { back_off_ms: 2_500 });
}

#[test]
fn debug_output_shows_nothing_a_server_chose() {
    let body = "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><m:GetFolderResponse xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\" xmlns:t=\"http://schemas.microsoft.com/exchange/services/2006/types\"><m:ResponseMessages>\
        <m:GetFolderResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:Folders><t:Folder><t:FolderId Id=\"SecretId\" ChangeKey=\"SecretKey\"/><t:DisplayName>Secret Name</t:DisplayName></t:Folder></m:Folders></m:GetFolderResponseMessage>\
        </m:ResponseMessages></m:GetFolderResponse></s:Body></s:Envelope>";
    let answer = Answer::parse(body.as_bytes()).unwrap();
    let messages = answer.messages("GetFolder").unwrap();
    let shown = format!("{answer:?} {messages:?} {:?}", Element::new("t:DisplayName").attr("Id", "SecretId").text("Secret Name"));
    assert!(!shown.contains("Secret") && !shown.contains("Folders"), "{shown}");
}

// ---- the request ----

#[test]
fn the_request_is_a_soap_envelope_with_the_version_and_one_operation() {
    let operation = Element::new("m:GetFolder").child(Element::new("m:FolderIds").child(Element::new("t:DistinguishedFolderId").attr("Id", "inbox")));
    let text = envelope(Version::Exchange2016, &operation);
    let tree = parse_xml(text.as_bytes()).unwrap();
    assert_eq!(tree.name, "s:Envelope");
    assert_eq!(tree.path(&["s:Header", "t:RequestServerVersion"]).and_then(|v| v.attr("Version")), Some("Exchange2016"));
    let body = tree.child("s:Body").unwrap();
    assert_eq!(body.elements().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["m:GetFolder"]);
    assert!(envelope(Version::Exchange2010Sp2, &operation).contains("Exchange2010_SP2"));
    assert_eq!(Version::NAMES.len(), 9);
}

#[test]
fn nothing_written_can_break_out_of_its_place() {
    let nasty = "a\r\nX-Injected: 1\0</t:BaseShape><m:Evil/>\"'&";
    let element = Element::new("m:Op\r\n<x").attr("Na\nme", nasty).text(nasty).child(Element::new("t:Child").text(nasty));
    let text = element.write();
    // (A line feed in a text stays; a carriage return is a reference; an attribute has neither.)
    assert!(!text.contains(['\r', '\0']), "{text:?}");
    let attribute = text.split_once("Name=\"").and_then(|(_, rest)| rest.split_once('"')).map(|(value, _)| value).unwrap_or_default();
    assert!(!attribute.is_empty() && !attribute.contains(['\r', '\n', '\0']), "{text:?}");
    assert!(!text.contains("<m:Evil/>") && !text.contains("<x"));
    // What is written reads back as the one element, holding the text as text.
    let tree = parse_xml(envelope(Version::default(), &element).as_bytes()).unwrap();
    let op = tree.child("s:Body").unwrap().elements().next().unwrap();
    assert_eq!(op.name, "m:Opx");
    assert!(op.text().contains("</t:BaseShape><m:Evil/>"));
    assert_eq!(op.elements().count(), 1);
}

#[test]
fn a_text_keeps_its_lines() {
    let text = "first\r\nsecond\tindented\nthird\0\u{1}\u{FFFE}<&>";
    let written = Element::new("t:Body").text(text).write();
    assert_eq!(written, "<t:Body>first&#13;\nsecond\tindented\nthird&lt;&amp;&gt;</t:Body>");
    assert!(!written.contains(['\r', '\0']));
    let tree = parse_xml(envelope(Version::default(), &Element::new("m:Op").text(text)).as_bytes()).unwrap();
    let op = tree.child("s:Body").unwrap().elements().next().unwrap();
    assert_eq!(op.text(), "first\r\nsecond\tindented\nthird<&>");
    // An attribute, or a header, keeps none of it.
    assert_eq!(escape("a\r\nb\tc\0"), "abc");
}

proptest! {
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        if let Ok(answer) = Answer::parse(&bytes) {
            let _ = (answer.busy(), answer.messages("GetFolder").map(|m| m.len()));
        }
    }

    #[test]
    fn any_text_written_reads_back_as_itself(text in "\\PC{0,200}", name in "[a-zA-Z][a-zA-Z0-9]{0,10}") {
        let tree = parse_xml(envelope(Version::default(), &Element::new(&format!("m:{name}")).attr("A", &text).text(&text)).as_bytes()).unwrap();
        let op = tree.child("s:Body").unwrap().elements().next().unwrap();
        let kept: String = text.chars().filter(|c| !c.is_control() && *c != '\u{FFFE}' && *c != '\u{FFFF}').collect();
        prop_assert_eq!(op.attr("A"), Some(kept.as_str()));
        prop_assert_eq!(op.text().into_owned(), kept);
    }
}
