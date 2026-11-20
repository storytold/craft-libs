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

/// A `current_filter` of another shape than `(sa(us))` (a structure with fewer, other or more fields, a string)
/// gives the path alone; `uris` of another shape is an error.
#[trial]
fn an_answer_of_another_shape_is_not_read() {
    let answer_with = |uris: Value<'_>, filter: Option<Value<'_>>| {
        let mut r = HashMap::new();
        r.insert("uris".to_string(), OwnedValue::try_from(uris).unwrap());
        if let Some(filter) = filter {
            r.insert("current_filter".to_string(), OwnedValue::try_from(filter).unwrap());
        }
        answer(0, &r)
    };
    let card = || Value::from(vec!["file:///tmp/card".to_string()]);
    let malformed = [
        Value::from(("PNG".to_string(),)),
        Value::from(("PNG".to_string(), vec![(0u32,)])),
        Value::from(("PNG".to_string(), vec![("*.png".to_string(), 0u32)])),
        Value::from((1u32, vec![(0u32, "*.png".to_string())])),
        Value::from(("PNG".to_string(), vec![(0u32, "*.png".to_string())], 1u32)),
        Value::from("PNG"),
    ];
    for filter in malformed {
        let shape = filter.value_signature().to_string();
        assert_eq!(answer_with(card(), Some(filter)), Ok(Some(Saved { path: "/tmp/card".into(), filter: None })), "{shape}");
    }
    // The shape the portal defines is still read, and so is the one `options` sends.
    let png = Value::from(("PNG".to_string(), vec![(0u32, "*.png".to_string())]));
    assert_eq!(answer_with(card(), Some(png)), Ok(Some(Saved { path: "/tmp/card".into(), filter: Some("PNG".into()) })));
    let sent = Value::from(wire(&Filter::new("Photoshop", &["psd", "psb"])));
    assert_eq!(filter_label(&sent), Some("Photoshop".into()));
    let malformed = [
        Value::from(vec![1u32]),
        Value::from(vec![("file:///tmp/card".to_string(),)]),
        Value::from(Vec::<String>::new()),
        Value::from("file:///tmp/card"),
    ];
    for uris in malformed {
        let shape = uris.value_signature().to_string();
        assert!(answer_with(uris, None).is_err(), "{shape}");
    }
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
