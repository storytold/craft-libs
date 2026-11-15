use super::*;
use craft_ui::testing::trial;

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
    let table: [(&str, Option<&str>, &[Filter], Option<usize>, &str); 8] = [
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
