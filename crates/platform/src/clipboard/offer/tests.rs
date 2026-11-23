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

/// Ported from WordCraft (#7317). An X11 property read is taken only when it came whole and within the cap: over the
/// cap, or with bytes still left to read, it is refused rather than pasted truncated.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn property_reads_must_be_whole_and_capped() {
    use super::unix::complete;
    assert!(complete(10, 0, 10));
    assert!(!complete(11, 0, 10), "over the cap");
    assert!(!complete(8, 4, 10), "more left to read");
    assert!(complete(0, 0, 0));
}
