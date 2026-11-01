//! Helpers shared by unit tests.

use super::ns;

/// A reply envelope with the given action header, optional `RelatesTo` and body fragment.
pub fn reply(action: &str, relates: Option<&str>, body: &str) -> String {
    let rel = relates.map(|r| format!("<wsa:RelatesTo>urn:uuid:{r}</wsa:RelatesTo>")).unwrap_or_default();
    format!(
        "<s:Envelope xmlns:s=\"{}\" xmlns:wsa=\"{}\" xmlns:wprt=\"{}\"><s:Header><wsa:Action>{action}</wsa:Action>{rel}</s:Header><s:Body>{body}</s:Body></s:Envelope>",
        ns::SOAP,
        ns::WSA,
        ns::WPRT
    )
}
