//! Rules a declarative spec must keep (task u2). Each case feeds an input that only the named rule
//! refuses, and checks where the error points and what it says.

use super::spec_support;

use serde_json::json;
use spec_support::{parse, queue, refused, set};

#[test]
fn a_path_cannot_leave_the_base() {
    let path = |p: &str| set(queue(), "/flow/submit/call/path", json!(p));
    let at = "flow.submit.call.path";
    refused(path("images"), at, "starts with /");
    refused(path("https://evil.example/x"), at, "another host");
    refused(path("//evil.example/x"), at, "another host");
    refused(path("/a/../b"), at, "'..'");
    refused(path("/.."), at, "'..'");
    refused(path("/a//b"), at, "'//'");
    refused(path("/a?x=1"), at, "no ? or #");
    refused(path("/a#x"), at, "no ? or #");
    refused(path("/a%2e%2e/b"), at, "no backslash or % escape");
    refused(path("/a\\b"), at, "no backslash or % escape");
    refused(path("/a b"), at, "plain visible");
    refused(path("/a\nb"), at, "plain visible");
    refused(path(&format!("/{}", "a".repeat(256))), at, "too long");
    assert!(parse(&path(&format!("/{}", "a".repeat(255)))).is_ok());
}
