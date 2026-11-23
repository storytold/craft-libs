//! The offer's format boundary: the CF_HTML header.

use super::wrap_cf_html;

fn field(data: &[u8], name: &str) -> usize {
    let at = data.windows(name.len()).position(|window| window == name.as_bytes()).expect("field") + name.len();
    std::str::from_utf8(&data[at..at + 10]).expect("offset digits").parse().expect("offset")
}

#[test]
fn cf_html_offsets_delimit_the_fragment_bytes() {
    let fragment = "<p>Olá, 世界 — café</p>";
    let data = wrap_cf_html(fragment);

    let start_html = field(&data, "StartHTML:");
    let end_html = field(&data, "EndHTML:");
    let start_fragment = field(&data, "StartFragment:");
    let end_fragment = field(&data, "EndFragment:");

    assert!(start_html < start_fragment, "HTML starts before its fragment");
    assert!(start_fragment < end_fragment, "fragment has a positive length");
    assert!(end_fragment < end_html, "fragment ends inside the document");
    assert_eq!(&data[start_html..start_html + "<html>".len()], b"<html>");
    assert_eq!(&data[start_fragment..end_fragment], fragment.as_bytes());
    assert_eq!(end_fragment - start_fragment, fragment.len(), "offsets count UTF-8 bytes");
    assert_eq!(&data[end_html..], b"\0", "EndHTML excludes the trailing NUL");
    assert_eq!(end_html, data.len() - 1);
}

/// Ported from WordCraft (#7317). When multiple RTF targets are offered, text/rtf is preferred over application/rtf and text/richtext.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn rtf_target_prefers_text_rtf() {
    use super::unix::rtf_target;
    let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(rtf_target(&names(&["TARGETS", "text/richtext", "application/rtf", "text/rtf"])), Some("text/rtf"));
    assert_eq!(rtf_target(&names(&["text/richtext", "UTF8_STRING"])), Some("text/richtext"));
    assert_eq!(rtf_target(&names(&["text/html", "text/rtf;charset=x"])), None);
    assert_eq!(rtf_target(&[]), None);
}
