use super::*;
use craft_ui::testing::trial;

fn filters() -> Vec<Filter> {
    vec![Filter::new("Document", &["doc"]), Filter::new("Drawing", &["ai"]), Filter::new("Picture", &["jpg", "jpeg"])]
}

/// The menu lists the types with an extension; "every file" types and ones without an extension stay out, and a
/// panel left with one type gets no menu.
#[trial]
fn the_menu_lists_types_with_extensions_and_needs_two() {
    assert_eq!(entries(&filters()), [0, 1, 2]);
    let mixed = [Filter::new("Document", &["doc"]), Filter::new("All files", &["*"]), Filter::new("Any", &[]), Filter::new("Drawing", &[".ai"])];
    assert_eq!(entries(&mixed), [0, 3], "the dot is not part of the extension");
    assert!(entries(&[Filter::new("Document", &["doc"]), Filter::new("All files", &["*"])]).is_empty());
    assert!(entries(&[]).is_empty());
}

/// Two types with one name both stay in the menu, under different titles; other titles are the names.
#[trial]
fn types_that_share_a_name_get_different_titles() {
    let twice = [Filter::new("Document", &["doc"]), Filter::new("Document", &["docx"]), Filter::new("Drawing", &["ai"])];
    let listed = entries(&twice);
    assert_eq!(listed, [0, 1, 2]);
    let shown = titles(&twice, &listed);
    assert_eq!(shown.len(), listed.len());
    assert!(shown.iter().enumerate().all(|(i, t)| !shown[..i].contains(t)), "{shown:?}");
    assert_eq!(shown[2], "Drawing");
    for set in [
        vec![Filter::new("Document", &["doc"]), Filter::new("Document", &["doc"])],
        vec![Filter::new("X (x) 2", &["a"]), Filter::new("X", &["x"]), Filter::new("X", &["x"])],
        vec![Filter::new("X", &["x"]), Filter::new("X", &["x"]), Filter::new("X (x) 2", &["a"]), Filter::new("X (x)", &["b"])],
    ] {
        let shown = titles(&set, &entries(&set));
        assert_eq!(shown.len(), set.len());
        assert!(shown.iter().enumerate().all(|(i, t)| !shown[..i].contains(t)), "{shown:?}");
    }
}

/// The answer reports the chosen type, unless the user typed another listed type's extension; a name the panel kept
/// when a choice could not rename it keeps the chosen type.
#[trial]
fn a_typed_extension_of_another_type_is_the_type_saved() {
    let all = filters();
    let listed = entries(&all);
    let (kept, opened) = (Some("Untitled-1.doc"), Some("Untitled-1.doc"));
    let answer = |returned, kept| answered_type(returned, kept, opened, &all, &listed, 1);
    assert_eq!(answer("Untitled-1.ai", None), Some(1));
    assert_eq!(answer("Untitled-1.doc", kept), Some(1), "the panel kept the opened name");
    assert_eq!(answer("Other.doc", kept), Some(0), "the user typed a new name after that");
    assert_eq!(answer("Untitled-1.doc", None), Some(0), "the user typed the first type's extension back");
    assert_eq!(answer("photo.JPEG", kept), Some(2), "the user typed another type's extension");
    assert_eq!(answer("Other.doc", Some("Other.doc")), Some(1), "a kept name with the opened extension");
    assert_eq!(answer("Other.jpg", Some("Other.jpg")), Some(2), "a kept name with another type's extension keeps that type");
    assert_eq!(answer("notes.v2", None), Some(1));
    assert_eq!(answer("art", None), Some(1));
    assert_eq!(answered_type("art.ai", None, opened, &all, &listed, 9), None, "a position the menu doesn't have");
}

#[trial]
fn the_suggested_name_chooses_its_type_and_any_other_the_first() {
    let all = filters();
    let listed = entries(&all);
    for (name, at) in [("Untitled-1.doc", 0), ("art.AI", 1), ("photo.jpeg", 2), ("Untitled-1", 0), ("notes.v2", 0), ("", 0)] {
        assert_eq!(initial(name, &all, &listed), at, "{name}");
    }
}

/// Choosing a type replaces the extension of another listed type, keeps one of its own and adds its own after any
/// other dot in the name.
#[trial]
fn choosing_a_type_gives_the_name_its_extension() {
    let all = filters();
    let drawing = &all[1];
    for (name, expected) in [
        ("Untitled-1.doc", "Untitled-1.ai"),
        ("Plan 1.DOC", "Plan 1.ai"),
        ("photo.jpeg", "photo.ai"),
        ("art.ai", "art.ai"),
        ("art.AI", "art.AI"),
        ("art", "art.ai"),
        ("art.v2", "art.v2.ai"),
        ("", ""),
    ] {
        assert_eq!(renamed(name, drawing, &all), expected, "{name}");
    }
    assert_eq!(renamed("photo.jpeg", &all[2], &all), "photo.jpeg", "a second extension of the chosen type stays");
}

/// A sequence of choices (A to B to C) ends with C's extension, and back to A gives A's.
#[trial]
fn each_choice_follows_the_previous_one() {
    let all = filters();
    let mut name = "Untitled-1.doc".to_owned();
    for (at, ext) in [(1, "ai"), (2, "jpg"), (0, "doc")] {
        name = renamed(&name, &all[at], &all);
        assert_eq!(Path::new(&name).extension().and_then(|e| e.to_str()), Some(ext));
        assert_eq!(Path::new(&name).file_stem().and_then(|e| e.to_str()), Some("Untitled-1"));
    }
}

/// The chosen type's extension comes first, so AppKit adds it to a bare name; the others stay allowed.
#[trial]
fn the_chosen_type_is_allowed_first() {
    let all = filters();
    let listed = entries(&all);
    assert_eq!(allowed(&all, &listed, 0), ["doc", "ai", "jpg", "jpeg"]);
    assert_eq!(allowed(&all, &listed, 1), ["ai", "doc", "jpg", "jpeg"]);
    assert_eq!(allowed(&all, &listed, 2), ["jpg", "jpeg", "doc", "ai"]);
}
