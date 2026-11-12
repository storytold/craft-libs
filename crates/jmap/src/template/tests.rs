use super::*;

fn base() -> Address {
    Address::parse("https://jmap.example.com/.well-known/jmap").unwrap()
}

const DOWNLOAD: &str = "https://jmap.example.com/download/{accountId}/{blobId}/{name}?accept={type}";

fn download(name: &str, media_type: &str) -> Result<Address, TemplateError> {
    AddressTemplate::parse(DOWNLOAD, &base()).unwrap().expand(&[("accountId", "A13824"), ("blobId", "Gabc-_1"), ("name", name), ("type", media_type)])
}

// RFC 8620 §2.1 example template and RFC 6570 §3.2.1: each value is percent-encoded.
#[test]
fn expands_download_template() {
    let address = download("report 2024.pdf", "application/pdf").unwrap();
    assert_eq!(address.as_str(), "https://jmap.example.com/download/A13824/Gabc-_1/report%202024.pdf?accept=application%2Fpdf");
    let address = download("a/b?c#d", "text/plain; charset=utf-8").unwrap();
    assert_eq!(address.as_str(), "https://jmap.example.com/download/A13824/Gabc-_1/a%2Fb%3Fc%23d?accept=text%2Fplain%3B%20charset%3Dutf-8");
}

// RFC 8620 §6.1: the upload template with its accountId.
#[test]
fn expands_upload_template() {
    let template = AddressTemplate::parse("https://jmap.example.com/upload/{accountId}/", &base()).unwrap();
    assert_eq!(template.expand(&[("accountId", "A1")]).unwrap().as_str(), "https://jmap.example.com/upload/A1/");
}

// Requirement 1.7: an empty, "." or ".." value in a path segment is refused; in the query it is fine.
#[test]
fn refuses_dot_values_in_path() {
    for name in ["", ".", ".."] {
        assert_eq!(download(name, "text/plain"), Err(TemplateError::UnsafePathValue), "{name:?}");
    }
    assert!(download("..pdf", "").is_ok());
    assert!(download("x", "").is_ok());
    let template = AddressTemplate::parse("/d/{a}{b}/x", &base()).unwrap();
    assert_eq!(template.expand(&[("a", "x."), ("b", ".")]), Err(TemplateError::UnsafePathValue));
    assert_eq!(template.expand(&[("a", "x."), ("b", "y.")]).unwrap().as_str(), "https://jmap.example.com/d/x.y./x");
    let template = AddressTemplate::parse("/d/{a}%2e/x", &base()).unwrap();
    assert_eq!(template.expand(&[("a", ".")]), Err(TemplateError::UnsafePathValue));
    let template = AddressTemplate::parse("/d/x/{a}", &base()).unwrap();
    assert_eq!(template.expand(&[("a", "..")]), Err(TemplateError::UnsafePathValue));
}

// RFC 3986 §5.2 applied to templates: an absolute-path template resolves against the session address.
#[test]
fn relative_templates_resolve_against_base() {
    let template = AddressTemplate::parse("/upload/{accountId}/", &base()).unwrap();
    assert_eq!(template.expand(&[("accountId", "A1")]).unwrap().as_str(), "https://jmap.example.com/upload/A1/");
    let template = AddressTemplate::parse("//files.example.net/u/{accountId}", &base()).unwrap();
    assert_eq!(template.expand(&[("accountId", "A1")]).unwrap().as_str(), "https://files.example.net/u/A1");
    assert_eq!(AddressTemplate::parse("upload/{accountId}", &base()), Err(TemplateError::RelativeTemplate));
}

// Requirement 1.7 and 1.2: the expansion must be usable under the transport rule.
#[test]
fn refuses_insecure_expansions() {
    let template = AddressTemplate::parse("http://jmap.example.com/upload/{accountId}", &base()).unwrap();
    assert_eq!(template.expand(&[("accountId", "A1")]), Err(TemplateError::Address(AddressError::InsecureTransport)));
    let template = AddressTemplate::parse("http://127.0.0.1/upload/{accountId}", &base()).unwrap();
    assert_eq!(template.expand(&[("accountId", "A1")]), Err(TemplateError::Address(AddressError::LocalFromRemote)));
    let local = Address::parse("http://localhost:8080/jmap").unwrap();
    let template = AddressTemplate::parse("http://localhost:8080/upload/{accountId}", &local).unwrap();
    assert!(template.expand(&[("accountId", "A1")]).is_ok());
}

// RFC 6570 §2.2: operators belong to levels 2 to 4 and are refused; §2.3 varname syntax.
#[test]
fn level_one_only() {
    for template in ["/d/{+path}", "/d/{#frag}", "/d{/seg}", "/d/{?q}", "/d/{a,b}", "/d/{a:3}", "/d/{a*}", "/d/{=x}", "/d/{.a}"] {
        assert_eq!(AddressTemplate::parse(template, &base()), Err(TemplateError::UnsupportedLevel), "{template}");
    }
    for template in ["/d/{}", "/d/{a", "/d/a}", "/d/{a-b}", "/d/{a..b}", "/d/%zz/{a}", "/d/ {a}", "/d/<{a}>"] {
        assert_eq!(AddressTemplate::parse(template, &base()), Err(TemplateError::Syntax), "{template}");
    }
    let template = AddressTemplate::parse("/d/{a.b}/{c%41}", &base()).unwrap();
    assert_eq!(template.variables().collect::<Vec<_>>(), ["a.b", "c%41"]);
}

// RFC 3986 §3 (components) and RFC 8620 §6.2: a value never chooses the scheme, host, port or fragment.
#[test]
fn expressions_only_in_path_and_query() {
    for template in ["https://{host}/d", "https://example.com:{port}/d", "https://a{x}.example.com/d", "//{h}/d", "/d#{a}"] {
        assert_eq!(AddressTemplate::parse(template, &base()), Err(TemplateError::ExpressionOutsidePathOrQuery), "{template}");
    }
    assert_eq!(AddressTemplate::parse("{x}://example.com/", &base()), Err(TemplateError::RelativeTemplate));
}

// RFC 6570 §3.2.1: an undefined variable would vanish; here it is an error instead.
#[test]
fn every_variable_needs_a_value() {
    let template = AddressTemplate::parse(DOWNLOAD, &base()).unwrap();
    assert_eq!(template.expand(&[("accountId", "A1")]), Err(TemplateError::MissingValue("blobId".into())));
    assert_eq!(template.check_variables(&["accountId", "blobId"], &["accountId", "blobId", "type", "name"]), Ok(()));
    assert_eq!(template.check_variables(&["ping"], &["ping"]), Err(TemplateError::MissingVariable("ping")));
    assert_eq!(template.check_variables(&[], &["accountId"]), Err(TemplateError::UnknownVariable("blobId".into())));
}

// Requirement 1.8: the debug form shows the origin and variable names only.
#[test]
fn debug_hides_path() {
    let template = AddressTemplate::parse("https://jmap.example.com/secret-path/{accountId}", &base()).unwrap();
    let shown = format!("{template:?}");
    assert!(!shown.contains("secret-path"));
    assert!(shown.contains("https://jmap.example.com"));
}
