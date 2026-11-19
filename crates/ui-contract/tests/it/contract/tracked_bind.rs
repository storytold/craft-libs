//! A write-through binding to a field of a `Tracked` value.

use craft_behavior::{EditEvent, Mixed};
use craft_ui::contract::{Bind, PlainData, Tracked, field};
use craft_ui::testing::trial;

#[derive(PlainData)]
struct Doc {
    title: String,
    size: f32,
}

/// Making and reading the binding every pass leaves the revision, and so does an edit to the value the field already
/// holds; an edit that changes the field moves it once and lands in the field.
#[trial]
fn a_tracked_binding_moves_the_revision_only_when_it_writes() {
    let mut doc = Tracked::new(Doc { title: "a".into(), size: 12.0 });
    let start = doc.revision();
    for _ in 0..3 {
        let mut bind = Bind::tracked(&mut doc, field!(size));
        assert_eq!(bind.reborrow().get(), Some(Mixed::Same(12.0)));
    }
    assert_eq!(doc.revision(), start, "bindings made and read every pass leave the revision");

    let set = |after: f32| EditEvent::set(Mixed::Same(12.0), after, "Size".into(), None);
    Bind::tracked(&mut doc, field!(size)).reborrow().apply(&set(12.0));
    assert_eq!(doc.revision(), start, "an edit to the value the field holds leaves the revision");

    Bind::tracked(&mut doc, field!(size)).reborrow().apply(&set(14.0));
    assert_eq!(doc.revision().count, start.count + 1, "a changing edit moves it once");
    assert_eq!((doc.size, doc.title.as_str()), (14.0, "a"));
}
