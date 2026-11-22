use super::*;
use craft_ui::testing::trial;

/// A portal answer falls back to another dialog only when the portal failed: a quick code 2 is a
/// dialog that never showed, a slow one is Esc, a queued cancel wins over a late answer, and a
/// failed or empty answer is unavailable.
#[trial]
fn a_portal_answer_falls_back_only_when_it_failed() {
    let nothing: HashMap<String, OwnedValue> = HashMap::new();
    let file = HashMap::from([("uris".to_owned(), Value::from(vec!["file:///s/a.txt".to_owned()]).try_to_owned().expect("an owned value"))]);
    let (quick, slow) = (Duration::ZERO, TOO_QUICK);
    assert_eq!(classify(2, quick, false, false, &nothing), Decision::Retry, "a dialog that closed at once never showed");
    assert_eq!(classify(2, slow, false, false, &nothing), Decision::Cancelled, "Esc in GNOME's dialog is a cancel");
    assert_eq!(classify(1, quick, false, false, &nothing), Decision::Cancelled, "Cancel is never asked again");
    assert!(matches!(classify(0, slow, false, false, &file), Decision::Saved(saved) if saved.path == *"/s/a.txt"), "a save names its file");
    assert_eq!(classify(0, slow, true, false, &file), Decision::Cancelled, "an answer after a queued cancel is a cancel");
    assert!(matches!(classify(0, slow, false, true, &nothing), Decision::Unavailable(_)), "the portal left the bus");
    assert!(matches!(classify(3, slow, false, false, &file), Decision::Unavailable(_)), "another code is a failure");
    assert!(matches!(classify(0, slow, false, false, &nothing), Decision::Unavailable(_)), "a save without a file");
}

#[trial]
fn file_uris_become_local_paths() {
    assert_eq!(file_uri_path("file:///home/me/Untitled-1.psd"), Some(PathBuf::from("/home/me/Untitled-1.psd")));
    assert_eq!(file_uri_path("file:///home/me/A%20b%C3%A9.png"), Some(PathBuf::from("/home/me/A bé.png")));
    assert_eq!(file_uri_path("file://localhost/tmp/x.png"), Some(PathBuf::from("/tmp/x.png")));
    assert_eq!(file_uri_path("sftp://host/x.png"), None);
    assert_eq!(file_uri_path("file://host/x.png"), None);
}

/// The name the Save dialog's answer ends up with: the picked type (`picked`, an index into `filters`) decides the
/// extension a bare name takes, and an extension the user typed wins unless it is the one the dialog opened with.
#[trial]
fn the_picked_file_type_gives_a_bare_name_its_extension_and_a_typed_extension_wins() {
    let two = [Filter::new("Photoshop", &["psd", "psb"]), Filter::new("PNG", &["png"])];
    let only = [Filter::new("Swatches", &["aco"])];
    // (path, opened with, filters, picked type, expected path)
    type Case<'a> = (&'a str, Option<&'a str>, &'a [Filter], Option<usize>, &'a str);
    let table: [Case; 8] = [
        ("/s/Untitled", Some("Untitled.psd"), &two, Some(1), "/s/Untitled.png"),
        ("/s/Untitled.psd", Some("Untitled.psd"), &two, Some(1), "/s/Untitled.png"),
        ("/s/art.png", Some("art.psd"), &two, Some(0), "/s/art.png"),
        ("/s/art.PSB", Some("art.psd"), &two, Some(0), "/s/art.PSB"),
        ("/s/scan.2024", None, &two, Some(1), "/s/scan.2024.png"),
        ("/s/Swatches", None, &only, None, "/s/Swatches.aco"),
        ("/s/Swatches.aco", None, &only, None, "/s/Swatches.aco"),
        ("/s/free", None, &two, None, "/s/free"),
    ];
    for (path, opened, filters, picked, expected) in table {
        let chosen: &[String] = match chosen_type(filters, picked) {
            Some(at) => &filters[at].extensions,
            None => &[],
        };
        assert_eq!(with_chosen_type(Path::new(path), opened, chosen, filters), PathBuf::from(expected), "{path} with type {picked:?}");
    }
}

// pc: apps/wordcraft/src/portal_save.rs:852 is_document_portal_detects_portal_fuse_paths
#[trial]
fn document_portal_paths_are_told_from_others() {
    assert!(is_document_portal(Path::new("/run/user/1000/doc/1234abcd/test.docx")));
    assert!(is_document_portal(Path::new("/var/run/user/1000/doc/by-app/app/test.docx")));
    assert!(is_document_portal(Path::new("/run/flatpak/doc/1234abcd/test.docx")));
    assert!(!is_document_portal(Path::new("/home/user/Documents/test.docx")));
    assert!(!is_document_portal(Path::new("/tmp/doc/test.docx")));
    assert!(!is_document_portal(Path::new("/run/flatpak/other/test.docx")));
}
