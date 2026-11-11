//! `craft_platform::default_app`.

use crate::default_app::path_from_arg;

// file:///a%20b.pdf and file://localhost/x decode to paths; http://, file://host/x, plain paths
// and non-UTF-8 escapes pass through; file:///C:/x.pdf is C:/x.pdf on Windows.
#[test]
fn file_uris_become_paths() {
    assert_eq!(path_from_arg("file:///a%20b.pdf".into()), "/a b.pdf");
    assert_eq!(path_from_arg("file://localhost/x".into()), "/x");
    assert_eq!(path_from_arg("file:///caf%C3%A9.pdf".into()), "/café.pdf");
    assert_eq!(path_from_arg("file:///100%.pdf".into()), "/100%.pdf");
    for same in ["http://example.com/a.pdf", "file://host/x", "/plain/path.pdf", "file:///bad%FF.pdf", "mailto:a@b.c"] {
        assert_eq!(path_from_arg(same.into()), same);
    }
    let drive = path_from_arg("file:///C:/x.pdf".into());
    assert_eq!(drive, if cfg!(windows) { "C:/x.pdf" } else { "/C:/x.pdf" });
}
