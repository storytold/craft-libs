//! Write-through bindings to fields of a `Tracked` value.

use craft_behavior::{EditEvent, Mixed};
use craft_ui::contract::{PlainData, Tracked, bind_tracked};
use craft_ui::testing::trial;

#[derive(PlainData)]
struct Doc {
    title: String,
    style: Style,
}

#[derive(PlainData)]
struct Style {
    size: f32,
}

/// Passes that bind two fields of one model and read them leave the revision, and so does an edit to the value a
/// field already holds; an edit that changes one field moves the revision once, at the write.
#[trial]
fn tracked_bindings_move_the_revision_only_when_an_edit_changes_a_field() {
    let mut doc = Tracked::new(Doc { title: "a".into(), style: Style { size: 12.0 } });
    let start = doc.revision();
    for _ in 0..3 {
        let (mut title, mut size) = bind_tracked!(doc => title, style.size);
        assert_eq!(title.reborrow().get(), Some(Mixed::Same("a".to_owned())));
        assert_eq!(size.reborrow().get(), Some(Mixed::Same(12.0)));
    }
    assert_eq!(doc.revision(), start, "two bindings made and read every pass, and written by neither, leave the revision");

    let set = |after: f32| EditEvent::set(Mixed::Same(12.0), after, "Size".into(), None);
    {
        let (_title, mut size) = bind_tracked!(doc => title, style.size);
        size.reborrow().apply(&set(12.0));
    }
    assert_eq!(doc.revision(), start, "an edit to the value the field holds leaves the revision");

    {
        let (_title, mut size) = bind_tracked!(doc => title, style.size);
        size.reborrow().apply(&set(14.0));
    }
    assert_eq!(doc.revision().count, start.count + 1, "a changing edit to one field moves it once");
    assert_eq!((doc.style.size, doc.title.as_str()), (14.0, "a"));
}
