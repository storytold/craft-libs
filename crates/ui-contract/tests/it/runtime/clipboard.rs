//! The clipboard handle: a write is one offer of every flavour, and a late paste reads only in the frame it arrived in.

use craft_platform::clipboard::{Clipboard, EventClipboard, Flavor, HTML, MemoryClipboard, RTF, TEXT};
use craft_ui::runtime::commands::{ClipItem, ClipboardHandle};
use craft_ui::runtime::frame;
use craft_ui::testing::{Frames, install_spec, trial};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The text of the clipboard's `mime` flavour.
fn text_of(handle: &ClipboardHandle, mime: &str) -> Option<String> {
    handle.read(&[mime]).expect("the clipboard reads").map(|flavor| String::from_utf8(flavor.data).expect("the flavour is UTF-8"))
}

/// One frame that puts `items` (once, in its first pass) and reads every text flavour back in each pass.
fn put_frame(frames: &mut Frames, items: Vec<ClipItem>) -> Vec<[Option<String>; 3]> {
    let mut items = Some(items);
    let mut seen = Vec::new();
    frames.frame(Vec::new(), |ui| {
        let handle = ClipboardHandle::of(ui.ctx());
        if let Some(items) = items.take() {
            handle.put(&items);
        }
        seen.push([TEXT, HTML, RTF].map(|mime| text_of(&handle, mime)));
    });
    seen
}

/// One frame, counted as the host counts frames (once per `set_input`), and what each of its passes saw: whether a
/// late paste arrived and the clipboard's text. With `rerun` the frame reruns its first pass.
fn paste_frame(frames: &mut Frames, frame_nr: &AtomicU64, rerun: bool) -> Vec<(bool, Option<String>)> {
    frame_nr.fetch_add(1, Ordering::SeqCst);
    let mut passes = Vec::new();
    let mut reran = false;
    frames.frame(Vec::new(), |ui| {
        let handle = ClipboardHandle::of(ui.ctx());
        passes.push((handle.arrived(), handle.read_text().expect("the clipboard reads")));
        if rerun && !reran && frame::discardable(ui.ctx()) {
            reran = true;
            frame::discard_pass(ui.ctx(), "clipboard trial: a rerun pass");
        }
    });
    passes
}

#[trial]
fn a_clipboard_handle_writes_one_offer_and_a_late_paste_reads_only_in_its_frame() {
    let mut frames = Frames::uninstalled();
    craft_ui::install(&frames.ctx, install_spec().clipboard(Clipboard::new(MemoryClipboard::default()))).expect("the runtime installs once");
    let rich = [ClipItem::Text("plain".into()), ClipItem::Html("<b>rich</b>".into()), ClipItem::Rtf("{\\rtf1 rich}".into())];
    let seen = put_frame(&mut frames, rich.to_vec());
    let all = [Some("plain".to_owned()), Some("<b>rich</b>".to_owned()), Some("{\\rtf1 rich}".to_owned())];
    assert_eq!(seen.last(), Some(&all), "one put offers text, HTML and RTF together");
    let handle = ClipboardHandle::of(&frames.ctx);
    assert!(handle.has(&[RTF]), "the clipboard holds the RTF flavour of the offer");
    assert!(handle.holds_ours(), "the clipboard holds what the app offered");

    let seen = put_frame(&mut frames, vec![ClipItem::Text("again".into())]);
    assert_eq!(seen.last(), Some(&[Some("again".to_owned()), None, None]), "a later put replaces every flavour of the earlier one");

    let frame_nr = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&frame_nr);
    let event = EventClipboard::new(move || counter.load(Ordering::SeqCst), |_: &[Flavor]| Ok(0));
    let late = event.late_offers();
    let mut frames = Frames::uninstalled();
    craft_ui::install(&frames.ctx, install_spec().clipboard(Clipboard::new(event))).expect("the runtime installs once");
    assert_eq!(paste_frame(&mut frames, &frame_nr, false), [(false, None)], "nothing arrived before the offer");

    late.offer(vec![Flavor::text("late"), Flavor::new(HTML, b"<i>late</i>".to_vec())]);
    let passes = paste_frame(&mut frames, &frame_nr, true);
    assert_eq!(passes.len(), 2, "the frame reran its pass");
    assert!(
        passes.iter().all(|pass| *pass == (true, Some("late".to_owned()))),
        "the late paste arrived and reads in every pass of its frame, the rerun one included: {passes:?}"
    );

    assert_eq!(paste_frame(&mut frames, &frame_nr, false), [(false, None)], "the arrival and its flavours stay with the frame it came in");
}
