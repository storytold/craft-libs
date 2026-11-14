//! The hosted form dialogs on a `DialogHost`: each opens there, its Cancel reaches the app, and it opens again after
//! closing; OK answers with the dialog's own result type.

use craft_behavior::PixelSize;
use craft_ui::dialog::{DialogHost, DialogResult, StackDialog};
use craft_ui::patterns::form_dialogs::{
    AboutInfo, AboutLink, CanvasSize, CanvasSizeValues, Credits, ExportOptions, ExportOptionsKind, FormatOptions, HostedAbout, HostedCanvasSize,
    HostedExportAs, HostedFormatOptions, JpegOptions,
};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::RefCell;
use std::fmt::Debug;
use std::path::PathBuf;
use std::rc::Rc;

const PICTURE: PixelSize = PixelSize::new(1600, 900);

fn about() -> HostedAbout {
    let info = AboutInfo::new("PhotoCraft", "An image editor.", "1.0").links([AboutLink::new("Website", "https://example.org")]);
    HostedAbout::new(info, Credits::new(Vec::new(), Vec::new(), 0))
}

#[allow(clippy::redundant_closure, reason = "the builder must be generic over the bind's lifetime, which the bare path is not")]
fn canvas_size() -> HostedCanvasSize {
    HostedCanvasSize::new(CanvasSizeValues::new(PICTURE), |values| CanvasSize::new(values))
}

/// A fixture that runs `host` every pass, in a room large enough for every form.
fn fixture(host: &Rc<RefCell<DialogHost>>) -> Fixture {
    let shown = Rc::clone(host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    fixture
}

/// Opens `make()` on a host, then Escape; twice, so a dialog that closed once opens again. `cancel` is what the app
/// takes after Escape.
fn opens_and_cancels<D: StackDialog>(make: fn() -> D, title: &str, cancel: DialogResult<D::Result>)
where
    D::Result: Clone + Debug + PartialEq,
{
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    for round in ["first", "second"] {
        host.borrow_mut().open(make());
        fixture.idle(10);
        assert!(fixture.by_label(title).is_some(), "{title} shows on the host ({round} open)");
        fixture.key(Key::Escape, Modifiers::NONE).idle(4);
        assert!(host.borrow().is_empty(), "Escape closes {title} ({round} open)");
        assert_eq!(host.borrow_mut().take::<D>(), Some(cancel.clone()), "{title}'s Escape reaches the app ({round} open)");
    }
}

#[trial]
fn each_hosted_form_opens_on_the_host_and_escape_reaches_the_app() {
    opens_and_cancels(canvas_size, "Canvas Size", DialogResult::Cancel);
    opens_and_cancels(|| HostedFormatOptions::new(ExportOptionsKind::Jpeg), "JPEG Options", DialogResult::Cancel);
    opens_and_cancels(|| HostedExportAs::new("Poster", PICTURE, ExportOptions::default()), "Export As", DialogResult::Cancel);
    // Escape closes About like OK: a notice has no Cancel.
    opens_and_cancels(about, "About PhotoCraft", DialogResult::Ok(()));
}

#[trial]
fn hosted_forms_answer_ok_with_their_values() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);

    host.borrow_mut().open(canvas_size());
    fixture.idle(10).key(Key::Enter, Modifiers::NONE).idle(4);
    let size = host.borrow_mut().take::<HostedCanvasSize>().and_then(DialogResult::ok).map(|result| result.size);
    assert_eq!(size, Some(PICTURE), "Enter answers Canvas Size with its size");

    host.borrow_mut().open(HostedFormatOptions::new(ExportOptionsKind::Jpeg).initial(FormatOptions::Jpeg(JpegOptions::new(10))));
    fixture.idle(10).key(Key::Enter, Modifiers::NONE).idle(4);
    let quality = match host.borrow_mut().take::<HostedFormatOptions>() {
        Some(DialogResult::Ok(FormatOptions::Jpeg(options))) => Some(options.quality),
        _ => None,
    };
    assert_eq!(quality, Some(10), "OK answers with the JPEG options it opened with");

    host.borrow_mut().open(about());
    fixture.idle(10);
    let website = fixture.by_label("Website").expect("About shows its link");
    fixture.click(&website).idle(4);
    let link = host.borrow_mut().get_mut::<HostedAbout>().and_then(HostedAbout::take_link);
    assert_eq!(link.as_deref(), Some("https://example.org"), "the link waits for the app");
}

#[trial]
fn hosted_export_as_finishes_when_the_app_answers_its_save_request() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let mut fixture = fixture(&host);
    host.borrow_mut().open(HostedExportAs::new("Poster", PICTURE, ExportOptions::default()));
    fixture.idle(10);
    assert!(host.borrow_mut().get_mut::<HostedExportAs>().and_then(HostedExportAs::wanted_preview).is_some(), "the dialog asks for a preview");

    let export = fixture.by_label("Export…").expect("the Export button");
    fixture.click(&export).idle(4);
    let request = host.borrow_mut().get_mut::<HostedExportAs>().and_then(HostedExportAs::pick_path).expect("Export asks for a path");
    assert!(host.borrow().is_open::<HostedExportAs>(), "the dialog waits for the path");

    let path = PathBuf::from("/tmp/poster.png");
    if let Some(dialog) = host.borrow_mut().get_mut::<HostedExportAs>() {
        dialog.answer_path(request.key.clone(), path.clone());
    }
    fixture.idle(6);
    let answered = host.borrow_mut().take::<HostedExportAs>().and_then(DialogResult::ok).map(|result| result.path);
    assert_eq!(answered, Some(path), "the answered path finishes the dialog with it");
}
