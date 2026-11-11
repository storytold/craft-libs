//! Integration tests of craft-docpath: docpath.

use std::path::Path;

use craft_docpath::{PathKind, Refused, classify, is_local, resolve};

fn check(kind: PathKind, cases: &[&str]) {
    for raw in cases {
        assert_eq!(classify(raw), kind, "{raw:?}");
        assert_eq!(is_local(raw), kind == PathKind::Local, "{raw:?}");
    }
}

#[test]
fn network_paths() {
    check(
        PathKind::Network,
        &[
            r"\\h\s",
            r"\\h\s\x.png",
            "//h/s/x.png",
            r"\/h\s",
            r"/\h\s",
            r"\\h",
            r"\\",
            r"\\\h\s",
            r"\\?\UNC\h\s\x",
            r"\\?\unc\h\s",
            "//?/UNC/h/s/x",
            r"\\.\UNC\h\s\x",
            r"\\h@SSL\s\x",
            r"\\h@80\s\x",
            r"\\h@SSL@443\DavWWWRoot\x",
            "file://h/s/x.png",
            "file:////h/s/x.png",
            r"file:\\h\s\x.png",
            "file:%5C%5Ch%5Cs%5Cx.png",
            "file:///%5C%5Ch/s/x.png",
            "file://%5C%5Ch/s",
            "file:%2F%2Fh/s/x",
        ],
    );
}

#[test]
fn invalid_paths() {
    check(
        PathKind::Invalid,
        &[
            "",
            "a\0b",
            "a\nb",
            "a\tb",
            "a\u{7f}",
            "a\u{85}b",
            "file:",
            "file:///C:/a%00b",
            "file:///C:/a%0Ab",
            r"C:\..\x",
            r"C:\a\..\..\x",
            "/..",
            r"\\?\C:\a\..\b",
            r"\\?\C:\a\.\b",
            r"\\?\C:\a/b",
            r"\\?\C:\a\\b",
            r"C:\a\.. \b",
            r"C:\a\...\b",
            "a/. /b",
            "file://",
            "file://localhost",
            "file://?x",
            "file:#x",
            "file:///C:/%81",
            "a\u{FF3C}b",
            "\\\u{FF3C}h\\s",
            "a\u{FF0F}b",
            "a\u{2215}b",
            "a\u{29F5}b",
            "file:///C:/a%EF%BC%BCb",
        ],
    );
}

#[test]
fn resolve_allows_only_the_document_share() {
    let doc = Path::new(r"\\Server\Share\docs\a.doc");
    assert_eq!(resolve(r"img\x.png", Some(doc)).unwrap(), Path::new(r"\\Server\Share\docs\img\x.png"));
    assert_eq!(resolve(r"\x.png", Some(doc)).unwrap(), Path::new(r"\\Server\Share\x.png"));
    assert_eq!(resolve(r"\\server\share\img\x.png", Some(doc)).unwrap(), Path::new(r"\\server\share\img\x.png"));
    assert!(resolve("//SERVER/SHARE/img/x.png", Some(doc)).is_ok());
    assert!(resolve(r"\\?\UNC\server\share\x.png", Some(doc)).is_ok());
    assert!(resolve("file://localhost//server/share/x.png", Some(doc)).is_ok());

    assert_eq!(resolve(r"\\other\share\x.png", Some(doc)), Err(Refused::Network));
    assert_eq!(resolve(r"\\server\other\x.png", Some(doc)), Err(Refused::Network));
    assert_eq!(resolve(r"\\server\share\..\other\x.png", Some(doc)), Err(Refused::Invalid));
    assert_eq!(resolve(r"..\..\..\x.png", Some(doc)), Err(Refused::Invalid));
    assert_eq!(resolve("file://server/share/x.png", Some(doc)), Err(Refused::Network));
    assert_eq!(resolve(r"\\server\share\nul", Some(doc)), Err(Refused::Device));

    let verbatim = Path::new(r"\\?\UNC\server\share\docs\a.doc");
    assert_eq!(resolve("img/x.png", Some(verbatim)).unwrap(), Path::new(r"\\?\UNC\server\share\docs\img\x.png"));
    assert!(resolve(r"\\server\share\x.png", Some(verbatim)).is_ok());

    let local = Path::new(r"C:\docs\a.doc");
    assert_eq!(resolve(r"\\server\share\x.png", Some(local)), Err(Refused::Network));
    assert_eq!(resolve(r"\\server\share\x.png", None), Err(Refused::Network));
}

#[cfg(unix)]
#[test]
fn a_non_unicode_document_path_keeps_the_namespace_rules() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let doc = Path::new(OsStr::from_bytes(b"/home/u/\xff/a.doc"));
    assert_eq!(resolve("img/x.png", Some(doc)).unwrap(), Path::new(OsStr::from_bytes(b"/home/u/\xff/img/x.png")));
    let device = Path::new(OsStr::from_bytes(b"/??/\xff/a.doc"));
    assert_eq!(resolve("x.png", Some(device)), Err(Refused::Device));
    assert_eq!(resolve("x.png", Some(Path::new("/??/u/a.doc"))), Err(Refused::Device));
    for doc in [&b"\\??\\C:\\\xff\\a.doc"[..], b"\\\\.\\COM1\\\xff\\a.doc", b"//./COM1/\xff/a.doc"] {
        assert_eq!(resolve("x.png", Some(Path::new(OsStr::from_bytes(doc)))), Err(Refused::Device), "{doc:?}");
    }
    // A share is the link's boundary, as for a Unicode document path.
    let share = Path::new(OsStr::from_bytes(b"//server/share/\xff/a.doc"));
    assert_eq!(resolve("../x.png", Some(share)).unwrap(), Path::new("//server/share/x.png"));
    assert_eq!(resolve("../../x.png", Some(share)), Err(Refused::Invalid));
    // A `\\` inside a native name splits the Unicode path but not the native one: a folder that
    // the two agree on is kept, a document name they split differently is refused.
    let folder = Path::new(OsStr::from_bytes(b"/home/u/a\\b/\xff.doc"));
    assert_eq!(resolve("x.png", Some(folder)).unwrap(), Path::new(OsStr::from_bytes(b"/home/u/a\\b/x.png")));
    let name = Path::new(OsStr::from_bytes(b"/home/u/a\xff\\b.doc"));
    assert_eq!(resolve("x.png", Some(name)), Err(Refused::Invalid));
    // `.` and `..` in the document's folder are folded as for a Unicode document path.
    let dotted = Path::new(OsStr::from_bytes(b"/home/./x/../u/\xff/a.doc"));
    assert_eq!(resolve("../y/x.png", Some(dotted)).unwrap(), Path::new("/home/u/y/x.png"));
    assert_eq!(resolve("../../../../x.png", Some(dotted)), Err(Refused::Invalid));
    let relative = Path::new(OsStr::from_bytes(b"\xff/a.doc"));
    assert_eq!(resolve("../../x.png", Some(relative)).unwrap(), Path::new("../x.png"));
    // A `..` that climbs out of a name holding a `\` on Unix does not leave the share unnoticed.
    let climb = Path::new(OsStr::from_bytes(b"//server/share/a\\b/../../\xff/a.doc"));
    assert_eq!(resolve("x.png", Some(climb)), Err(Refused::Invalid));
}
