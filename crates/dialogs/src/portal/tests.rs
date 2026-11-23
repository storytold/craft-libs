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

/// An Open with a check box: the portal gets the box as a choice that starts off, under the
/// request's label, and the answer reads the box's state, the files and the codes as Save does.
#[trial]
fn an_open_check_box_is_a_portal_choice_and_its_answer_is_read() {
    let dialog = FileDialog::open_many().title("Open").filter("PNG", &["png"]).directory("/s").check_box("Image Sequence");
    let req = OpenRequest::new(&dialog, "x11:2a".into()).expect("an Open with a check box");
    assert!(req.multiple);
    assert_eq!(OpenRequest::new(&FileDialog::open(), String::new()), None, "no box asked for");
    assert_eq!(OpenRequest::new(&FileDialog::save().check_box("Image Sequence"), String::new()), None, "a Save takes no box");
    let options = open_options(&req, "t");
    let choices: Vec<(String, String, Vec<(String, String)>, String)> =
        options.get("choices").expect("the box is a choice").try_to_owned().expect("owned").try_into().expect("a(ssa(ss)s)");
    assert_eq!(choices, [(CHECK_BOX.to_owned(), "Image Sequence".to_owned(), Vec::new(), "false".to_owned())]);
    assert!(bool::try_from(options.get("multiple").expect("multiple").try_to_owned().expect("owned")).expect("a bool"));

    let owned = |v: Value<'_>| v.try_to_owned().expect("an owned value");
    let answer = |uris: Vec<&str>, ticked: Option<&str>| {
        let mut results = HashMap::from([("uris".to_owned(), owned(Value::from(uris.into_iter().map(str::to_owned).collect::<Vec<_>>())))]);
        if let Some(state) = ticked {
            results.insert(
                "choices".to_owned(),
                owned(Value::from(vec![("other".to_owned(), "true".to_owned()), (CHECK_BOX.to_owned(), state.to_owned())])),
            );
        }
        results
    };
    let slow = TOO_QUICK;
    let files = vec![PathBuf::from("/s/frame_001.png"), PathBuf::from("/s/a b.png")];
    let two = vec!["file:///s/frame_001.png", "file:///s/a%20b.png"];
    assert_eq!(classify_open(0, slow, false, false, &answer(two.clone(), Some("true"))), OpenOutcome::Opened { paths: files.clone(), checked: true });
    assert_eq!(
        classify_open(0, slow, false, false, &answer(two.clone(), Some("false"))),
        OpenOutcome::Opened { paths: files.clone(), checked: false }
    );
    assert_eq!(
        classify_open(0, slow, false, false, &answer(two.clone(), None)),
        OpenOutcome::Opened { paths: files, checked: false },
        "no state is off"
    );
    assert_eq!(classify_open(0, slow, false, false, &answer(Vec::new(), Some("true"))), OpenOutcome::Cancelled, "no file");
    assert!(matches!(classify_open(0, slow, false, false, &answer(vec!["sftp://h/x.png"], None)), OpenOutcome::UseRfd(_)), "not a local file");
    assert_eq!(classify_open(1, slow, false, false, &answer(two.clone(), Some("true"))), OpenOutcome::Cancelled);
    assert_eq!(classify_open(2, slow, false, false, &HashMap::new()), OpenOutcome::Cancelled, "Esc");
    assert!(matches!(classify_open(2, Duration::ZERO, false, false, &HashMap::new()), OpenOutcome::UseRfd(_)), "closed at once");
    assert_eq!(classify_open(0, slow, true, false, &answer(two, Some("true"))), OpenOutcome::Cancelled, "a queued cancel wins");
    assert!(matches!(classify_open(0, slow, false, true, &HashMap::new()), OpenOutcome::UseRfd(_)), "the portal left the bus");
}
