//! `craft_platform::default_app`.

use crate::default_app::{path_from_arg, rejoin_split_paths};

// XDG_DATA_HOME (else ~/.local/share) comes first, then each XDG_DATA_DIRS entry (else
// /usr/local/share:/usr/share); relative and empty entries are skipped, and no home is fine.
// Absolute here means a Unix path, so the trial runs on Unix.
#[cfg(unix)]
#[test]
fn application_dirs_follow_the_xdg_spec() {
    use crate::default_app::application_dirs;
    let p = |s: &str| std::path::PathBuf::from(s);
    assert_eq!(
        application_dirs(None, None, Some("/home/a")),
        [p("/home/a/.local/share/applications"), p("/usr/local/share/applications"), p("/usr/share/applications")]
    );
    assert_eq!(
        application_dirs(Some("/data"), Some("/opt/share::relative:/usr/share"), Some("/home/a")),
        [p("/data/applications"), p("/opt/share/applications"), p("/usr/share/applications")]
    );
    assert_eq!(application_dirs(Some(""), Some(""), None), [p("/usr/local/share/applications"), p("/usr/share/applications")]);
    assert_eq!(application_dirs(Some("rel"), None, Some("also-rel")), [p("/usr/local/share/applications"), p("/usr/share/applications")]);
}

// file:///a%20b.pdf and file://localhost/x decode to paths; truncated and invalid escapes are
// kept as written; http://, file://host/x, a bare file://, plain paths and non-UTF-8 escapes pass
// through; file:///C:/x.pdf is C:/x.pdf on Windows.
#[test]
fn file_uris_become_paths() {
    assert_eq!(path_from_arg("file:///a%20b.pdf".into()), "/a b.pdf");
    assert_eq!(path_from_arg("file://localhost/x".into()), "/x");
    assert_eq!(path_from_arg("file:///caf%C3%A9.pdf".into()), "/café.pdf");
    assert_eq!(path_from_arg("file:///100%.pdf".into()), "/100%.pdf");
    assert_eq!(path_from_arg("file:///tmp/a%2".into()), "/tmp/a%2");
    assert_eq!(path_from_arg("file:///tmp/%zz%".into()), "/tmp/%zz%");
    for same in ["http://example.com/a.pdf", "file://host/x", "file://", "report.pdf", "/plain/path.pdf", "file:///bad%FF.pdf", "mailto:a@b.c"] {
        assert_eq!(path_from_arg(same.into()), same);
    }
    let drive = path_from_arg("file:///C:/x.pdf".into());
    assert_eq!(drive, if cfg!(windows) { "C:/x.pdf" } else { "/C:/x.pdf" });
}

// A launcher that passes a path with spaces unquoted splits it; the pieces are joined back into
// the longest run that names a file, files that exist on their own stay apart, and arguments
// that join into nothing are kept as given so a missing file is still reported by its name.
#[test]
fn split_paths_with_spaces_are_rejoined() {
    let have = [r"P:\Archive\00042A26 Contract.PDF", r"C:\a b c.pdf", r"C:\a.pdf", r"C:\x y.pdf"];
    let exists = |p: &str| have.contains(&p);
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(rejoin_split_paths(args(&[r"P:\Archive\00042A26", "Contract.PDF"]), exists), args(&[r"P:\Archive\00042A26 Contract.PDF"]));
    assert_eq!(rejoin_split_paths(args(&[r"C:\a", "b", "c.pdf", r"C:\a.pdf"]), exists), args(&[r"C:\a b c.pdf", r"C:\a.pdf"]));
    assert_eq!(rejoin_split_paths(args(&[r"C:\a.pdf", r"C:\x", "y.pdf"]), exists), args(&[r"C:\a.pdf", r"C:\x y.pdf"]));
    assert_eq!(rejoin_split_paths(args(&["missing.pdf", "other"]), exists), args(&["missing.pdf", "other"]));
    assert!(rejoin_split_paths(Vec::new(), exists).is_empty());
}
