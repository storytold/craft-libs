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

// Copied files reach the system clipboard as a file list holding the same paths, spaces and
// non-ASCII names included; a URI that names no local file is never written as one, and a list
// with no file in it leaves the write to the next flavour.
#[test]
fn a_file_list_is_written_as_files() {
    use super::{Plan, Primary};
    use crate::clipboard::types::{Flavor, URI_LIST};

    let paths = vec![std::env::temp_dir().join("a b#1.pdf"), std::env::temp_dir().join("café 100%.png")];
    let flavors = [Flavor::files(&paths), Flavor::text("names")];
    let plan = Plan::new(&flavors, None);
    assert_eq!(plan.primary(), Some(Primary::Files));
    assert_eq!(plan.files.as_ref().map(|files| (files.index, files.paths.clone())), Some((0, paths.clone())));

    let mut mixed = b"# copied\r\nhttps://example.com/a.pdf\r\n".to_vec();
    mixed.extend(Flavor::files(&paths[..1]).data);
    let flavors = [Flavor::new(URI_LIST, mixed)];
    let plan = Plan::new(&flavors, None);
    assert_eq!(plan.primary(), Some(Primary::Files));
    assert_eq!(plan.files.as_ref().map(|files| files.paths.clone()), Some(paths[..1].to_vec()));

    let flavors = [Flavor::new(URI_LIST, b"https://example.com/a.pdf\r\n".to_vec()), Flavor::text("names")];
    let plan = Plan::new(&flavors, None);
    assert!(plan.files.is_none());
    assert_eq!(plan.primary(), Some(Primary::Rich));
    assert_eq!(plan.text.as_ref().map(|text| text.index), Some(1));

    // Text put first stays the write's shape even with files beside it.
    let flavors = [Flavor::text("names"), Flavor::files(&paths)];
    assert_eq!(Plan::new(&flavors, None).primary(), Some(Primary::Rich));
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
