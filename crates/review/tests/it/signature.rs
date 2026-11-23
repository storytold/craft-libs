//! The signature pad: stroke undo, image import by Browse… and by a drop, and the stored form of the settings.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use craft_host::{AppInfo, Host, InlineJobs, Jobs, ManualJobs, MemorySections, Picked, ScriptedFiles, Section};
use craft_review::signature::{MAX_IMAGE_BYTES, SavedSignature, SignatureDraft, SignatureImage, SignatureTab};
use craft_review::ui::{ScanEvent, SignaturePadDialog, SignatureSettings};
use craft_ui::contract::Role;
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::popover::DropVeil;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, Frames, trial};
use egui::{Key, Modifiers, Pos2, Rect, Ui, vec2};

/// Five pixels of a photographed signature, straight RGBA: white paper, off-white paper, ink, a grey edge and
/// half-transparent ink.
const PHOTO: [u8; 20] = [255, 255, 255, 255, 250, 248, 252, 255, 10, 20, 30, 255, 230, 235, 240, 255, 0, 0, 0, 128];

/// Asserts `rgba` is [`PHOTO`] with its paper made transparent: both papers clear, the ink opaque, the grey edge
/// faded between clear and opaque, the half-transparent ink not raised, and no colour changed.
fn assert_paper_gone(rgba: &[u8]) {
    assert_eq!(rgba.len(), PHOTO.len(), "the image keeps its size");
    for (was, now) in PHOTO.chunks(4).zip(rgba.chunks(4)) {
        assert_eq!(was[..3], now[..3], "the colours stay");
    }
    let alpha = |pixel: usize| rgba[pixel * 4 + 3];
    assert_eq!(alpha(0), 0, "white paper goes clear");
    assert_eq!(alpha(1), 0, "off-white paper goes clear");
    assert_eq!(alpha(2), 255, "ink stays opaque");
    assert!(alpha(3) > 0 && alpha(3) < 255, "the grey edge fades: {}", alpha(3));
    assert!(alpha(4) > 0 && alpha(4) <= 128, "half-transparent ink is not raised: {}", alpha(4));
}

/// The dialog under test and the context it last ran in.
struct Pad {
    dialog: SignaturePadDialog,
    stack: DialogStack,
    ctx: Option<egui::Context>,
}

fn pad(draft: SignatureDraft) -> Rc<RefCell<Pad>> {
    let dialog = SignaturePadDialog::new(draft);
    let mut stack = DialogStack::default();
    stack.push(dialog.id(), None);
    Rc::new(RefCell::new(Pad { dialog, stack, ctx: None }))
}

/// Clear must also reach scan-owned state so a later scan pass cannot restore the cleared image.
#[trial]
fn clearing_a_scan_does_not_let_the_scan_restore_the_image() {
    let pad = pad(SignatureDraft { tab: SignatureTab::Scan, ..SignatureDraft::default() });
    let mut pending = true;
    pad.borrow_mut().dialog =
        SignaturePadDialog::new(SignatureDraft { tab: SignatureTab::Scan, ..SignatureDraft::default() }).scan(move |_, draft, event| match event {
            ScanEvent::Clear => pending = false,
            ScanEvent::Show(_) if pending => draft.image = Some(SignatureImage::new(5, 1, PHOTO.to_vec())),
            ScanEvent::Show(_) => {}
        });
    let shown = pad.clone();
    let mut fx = Fixture::new(move |ui| show(&shown, ui));
    fx.room(vec2(900.0, 900.0)).settle();
    let clear = fx.by_label("Clear").expect("Clear is shown");
    fx.click(&clear).settle();
    assert!(saved(&pad).is_none(), "the cleared scan must not restore a pending result");
}

/// Shows the dialog in `ui`.
fn show(pad: &RefCell<Pad>, ui: &mut Ui) {
    let mut pad = pad.borrow_mut();
    let Pad { dialog, stack, ctx } = &mut *pad;
    *ctx = Some(ui.ctx().clone());
    let _ = dialog.show(ui, stack);
}

/// What Apply would save now; `None` while Apply is disabled.
fn saved(pad: &RefCell<Pad>) -> Option<SavedSignature> {
    let mut pad = pad.borrow_mut();
    let ctx = pad.ctx.clone().expect("the dialog has run");
    pad.dialog.confirm(&ctx).map(|result| result.signature)
}

/// The strokes Apply would save on the Draw tab.
fn strokes(pad: &RefCell<Pad>) -> Vec<Vec<[f32; 2]>> {
    match saved(pad) {
        Some(SavedSignature::Drawn(strokes)) => strokes,
        other => panic!("the Draw tab saves strokes, not {other:?}"),
    }
}

fn host(files: ScriptedFiles, jobs: Arc<dyn Jobs>) -> Arc<Host> {
    Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).files(Arc::new(files)).jobs(jobs).build())
}

/// `rgba` (`width` × `height`) as a PNG file.
fn png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    use image::ImageEncoder as _;
    let mut out = Vec::new();
    let encoded = image::codecs::png::PngEncoder::new(&mut out).write_image(rgba, width, height, image::ExtendedColorType::Rgba8);
    encoded.expect("the trial's image encodes");
    out
}

/// A drag of a few points from `from` by `by`: a visible stroke.
fn stroke(fx: &mut Fixture, from: Pos2, by: egui::Vec2) {
    fx.drag_at(from, by);
}

/// A press that moves past the drag slop in one step and is released where it stopped: a stroke of one point, a bare
/// dot.
fn dot(fx: &mut Fixture, at: Pos2) {
    let to = at + vec2(12.0, 0.0);
    fx.frame(vec![moved(at)]).frame(vec![press(at)]).frame(vec![moved(to)]).frame(vec![release(to)]).frame(Vec::new());
}

/// Undo takes back the last visible stroke and the bare dots after it, from the Undo button and from Cmd+Z on the
/// pad alike; the earlier stroke stays.
#[trial]
fn undo_takes_back_one_stroke_and_skips_bare_dots() {
    let pad = pad(SignatureDraft { tab: SignatureTab::Draw, ..SignatureDraft::default() });
    let shown = Rc::clone(&pad);
    let mut fx = Fixture::new(move |ui| show(&shown, ui));
    fx.room(vec2(900.0, 900.0)).settle();
    let area: Rect = fx
        .by_role(Role::IMAGE)
        .into_iter()
        .map(|target| target.rect)
        .max_by(|a, b| a.area().total_cmp(&b.area()))
        .expect("the Draw tab shows the pad");
    let at = |x: f32, y: f32| Pos2::new(area.left() + area.width() * x, area.top() + area.height() * y);
    let along = vec2(area.width() * 0.6, area.height() * 0.1);

    stroke(&mut fx, at(0.1, 0.2), along);
    let first = strokes(&pad);
    assert_eq!(first.len(), 1, "a drag draws one stroke: {first:?}");
    assert!(first[0].len() > 1, "the drag's stroke is visible: {first:?}");
    stroke(&mut fx, at(0.1, 0.5), along);
    dot(&mut fx, at(0.2, 0.8));
    let drawn = strokes(&pad);
    assert_eq!(drawn.iter().map(Vec::len).filter(|&len| len == 1).count(), 1, "the dot is one point: {drawn:?}");
    assert_eq!(drawn.len(), 3, "two strokes and a dot: {drawn:?}");

    let undo = fx.by_label("Undo").expect("the Draw tab has Undo");
    fx.click(&undo).settle();
    assert_eq!(strokes(&pad), first, "the Undo button takes back the dot and the second stroke");

    stroke(&mut fx, at(0.15, 0.55), along);
    dot(&mut fx, at(0.4, 0.85));
    assert_eq!(strokes(&pad).len(), 3, "a stroke and a dot are drawn again");
    fx.key(Key::Z, Modifiers::COMMAND).settle();
    assert_eq!(strokes(&pad), first, "Cmd+Z on the pad takes back the dot and the stroke before it");
}

/// An image picked with Browse… comes back with its paper transparent and its ink opaque.
#[trial]
fn an_imported_image_loses_its_paper_and_keeps_its_ink() {
    let files = ScriptedFiles::answering([Picked::Paths(vec!["ink.png".into()])]).with_file("ink.png", png(&PHOTO, 5, 1));
    let pad = pad(SignatureDraft { tab: SignatureTab::Image, ..SignatureDraft::default() });
    let shown = Rc::clone(&pad);
    let mut fx = Fixture::new(move |ui| show(&shown, ui));
    fx.host(host(files, Arc::new(InlineJobs))).room(vec2(900.0, 900.0)).settle();
    let browse = fx.by_label("Browse…").expect("the Image tab has Browse…");
    fx.click(&browse).settle();
    let Some(SavedSignature::Image(image)) = saved(&pad) else { panic!("the picked image is the draft's: {:?}", saved(&pad)) };
    assert_eq!((image.width(), image.height()), (5, 1));
    assert_paper_gone(image.rgba());
}

/// A file dropped while the Image tab shows is imported by the pad, once, and the window's own veil gets nothing.
/// Every frame runs two passes: the second has no input of its own, so a drop taken again would import twice.
#[trial]
fn a_file_dropped_on_the_pad_is_imported_and_the_window_gets_none() {
    let jobs = Arc::new(ManualJobs::new());
    let files = ScriptedFiles::new().with_file("photo.png", png(&PHOTO, 5, 1));
    let pad = pad(SignatureDraft { tab: SignatureTab::Image, ..SignatureDraft::default() });
    let window_drops = Rc::new(Cell::new(0_usize));
    let (shown, seen) = (Rc::clone(&pad), Rc::clone(&window_drops));
    let mut frame = move |ui: &mut Ui| {
        let window = DropVeil::new("Open files").show_with_files(ui, |_, _| {});
        if window.output.dropped.is_some() {
            seen.set(seen.get() + 1);
        }
        show(&shown, ui);
        if ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the trial reruns the pass");
        }
    };
    let mut frames = Frames::new();
    frames.screen(vec2(900.0, 900.0));
    frames.host(host(files, jobs.clone()));
    frames.idle(3, &mut frame);

    let over = Pos2::new(450.0, 450.0);
    frames.hover_files(&["photo.png"]);
    frames.frame(vec![moved(over)], &mut frame);
    frames.idle(1, &mut frame);
    frames.drop_files(&["photo.png"]);
    frames.frame(vec![moved(over)], &mut frame);
    frames.idle(1, &mut frame);
    assert_eq!(jobs.run_all(), 1, "the dropped file is read once");
    frames.idle(2, &mut frame);

    assert_eq!(window_drops.get(), 0, "the window's veil gets no drop while the pad takes it");
    let Some(SavedSignature::Image(image)) = saved(&pad) else { panic!("the dropped image is the draft's: {:?}", saved(&pad)) };
    assert_paper_gone(image.rgba());
}

/// A drop reaches a veil's body in the pass that receives it and in no other: the frame's second pass has no input of
/// its own, and later frames have no drop.
#[trial]
fn a_drop_reaches_the_veils_body_in_the_pass_that_receives_it() {
    let seen = Rc::new(RefCell::new(Vec::<usize>::new()));
    let log = Rc::clone(&seen);
    let mut frame = move |ui: &mut Ui| {
        let pass = ui.ctx().current_pass_index();
        DropVeil::new("Open files").show_with_files(ui, |_, dropped| {
            if dropped.is_some() {
                log.borrow_mut().push(pass);
            }
        });
        if pass == 0 {
            ui.ctx().request_discard("the trial reruns the pass");
        }
    };
    let mut frames = Frames::new();
    frames.screen(vec2(900.0, 900.0));
    frames.idle(3, &mut frame);
    assert!(seen.borrow().is_empty(), "no body sees a drop before one happens");

    let over = Pos2::new(450.0, 450.0);
    frames.hover_files(&["photo.png"]);
    frames.frame(vec![moved(over)], &mut frame);
    frames.idle(1, &mut frame);
    assert!(seen.borrow().is_empty(), "hovering files are not a drop");
    frames.drop_files(&["photo.png"]);
    frames.frame(vec![moved(over)], &mut frame);
    let at_drop = seen.borrow().clone();
    assert!(!at_drop.is_empty(), "the body sees the drop in the pass that receives it");
    assert!(at_drop.iter().all(|&pass| pass == 0), "the rerun pass sees no drop: {at_drop:?}");
    frames.idle(3, &mut frame);
    assert_eq!(*seen.borrow(), at_drop, "later frames see no drop");
}

/// What a settings file can hold under the signatures key: the form [`SignatureSettings`] writes, with any text in it.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Stored {
    signature: Option<StoredSlot>,
    initials: Option<StoredSlot>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum StoredSlot {
    Typed(String),
    Image { png: String },
}

impl Section for Stored {
    const KEY: &'static str = <SignatureSettings as Section>::KEY;
}

/// A saved image is stored as base64 PNG text and reads back equal; text over the cap, text that is not a PNG and
/// text that is not base64 each leave the signature `None` while the initials still load.
#[trial]
fn a_saved_image_round_trips_as_png_and_an_oversize_one_is_refused() {
    let host = Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).settings(Arc::new(MemorySections::default())).build();
    let initials = SavedSignature::Typed("AR".into());
    let settings = SignatureSettings {
        signature: Some(SavedSignature::Image(SignatureImage::new(5, 1, PHOTO.to_vec()))),
        initials: Some(initials.clone()),
        ..SignatureSettings::default()
    };
    host.set_setting(&settings);

    let stored = host.setting::<Stored>();
    assert!(matches!(&stored.initials, Some(StoredSlot::Typed(text)) if text == "AR"), "the initials are stored as text");
    let Some(StoredSlot::Image { png }) = stored.signature else { panic!("the image is stored as {{ png }}") };
    let bytes = STANDARD.decode(png).expect("the stored text is base64");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "the stored bytes are a PNG");
    assert_eq!(host.setting::<SignatureSettings>(), settings, "the settings read back as written");

    let over_cap = "A".repeat(MAX_IMAGE_BYTES.div_ceil(3) * 4 + 4);
    let cases = [
        ("text over the cap", over_cap),
        ("a corrupt PNG", STANDARD.encode(b"\x89PNG\r\n\x1a\nnot a picture")),
        ("not base64", "*** no ***".to_owned()),
    ];
    for (case, png) in cases {
        host.set_setting(&Stored { signature: Some(StoredSlot::Image { png }), initials: Some(StoredSlot::Typed("AR".into())) });
        let read = host.setting::<SignatureSettings>();
        assert_eq!(read.signature, None, "{case} is refused");
        assert_eq!(read.initials, Some(initials.clone()), "{case} leaves the initials loaded");
    }
}
