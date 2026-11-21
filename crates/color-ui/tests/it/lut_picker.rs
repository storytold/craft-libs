//! `LutPicker`'s Load 3D LUT…: the picked file comes back parsed and as the bytes it was read from.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use craft_color_ui::ui::{LutChoice, LutPicker};
use craft_host::{AppInfo, Host, InlineJobs, Picked, ScriptedFiles};
use craft_ui::contract::{Bind, Props as _, Role, Show as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};

/// A 2³ identity `.cube` whose input range line the parse folds into the domain.
const WIDE: &str = "TITLE \"Wide\"\nLUT_3D_SIZE 2\nLUT_3D_INPUT_RANGE 0 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";

/// What a load reports: whether the parse succeeded, and the bytes.
type Reported = (Option<bool>, Option<Vec<u8>>);

/// What one Load 3D LUT… through `files` reports.
fn load(files: ScriptedFiles) -> Reported {
    let files = Arc::new(files);
    let seen: Rc<RefCell<Reported>> = Rc::default();
    let out = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let shown = LutPicker::new(Bind::given(LutChoice::None)).label("3D LUT File").show(ui);
        if let Some(loaded) = &shown.loaded {
            *out.borrow_mut() = (Some(loaded.is_ok()), shown.loaded_bytes.as_deref().cloned());
        }
    });
    let host = Host::builder(AppInfo { id: "test", name: "Test", version: "1.0" }).files(files.clone()).jobs(Arc::new(InlineJobs)).build();
    fixture.host(Arc::new(host));
    fixture.settle();
    // Open the field and go up from None to Load 3D LUT…, as the keyboard does.
    let field = *fixture.by_role(Role::COMBO_BOX).first().expect("the picker's field");
    fixture.click(&field);
    fixture.key(Key::ArrowUp, Modifiers::NONE);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(10);
    assert_eq!(files.requests().len(), 1, "Load 3D LUT… asked for one file");
    seen.take()
}

/// The app gets the file as it was read, from a browser's contents and from a path, so it can hand its own
/// reader the input range line instead of a re-written domain. A file the parse refuses still comes back as read (the
/// app's reader may take it); a file that cannot be read has no bytes.
#[trial]
fn a_loaded_lut_comes_back_with_the_bytes_it_was_read_from() {
    let contents = ScriptedFiles::answering([Picked::Contents(vec![("wide.cube".into(), WIDE.as_bytes().to_vec())])]);
    assert_eq!(load(contents), (Some(true), Some(WIDE.as_bytes().to_vec())), "a browser's contents");
    let path = PathBuf::from("/luts/wide.cube");
    let on_disk = ScriptedFiles::answering([Picked::Paths(vec![path.clone()])]).with_file(&path, WIDE.as_bytes().to_vec());
    assert_eq!(load(on_disk), (Some(true), Some(WIDE.as_bytes().to_vec())), "a file on disk");
    let broken = b"LUT_3D_SIZE 2\n0 0 0\n".to_vec();
    let unparsed = ScriptedFiles::answering([Picked::Contents(vec![("broken.cube".into(), broken.clone())])]);
    assert_eq!(load(unparsed), (Some(false), Some(broken)), "a file that reads but does not parse still comes back as read");
    let missing = ScriptedFiles::answering([Picked::Paths(vec![PathBuf::from("/luts/gone.cube")])]);
    assert_eq!(load(missing), (Some(false), None), "an unreadable file: an error and no bytes");
}
