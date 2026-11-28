//! Stories of the signature pad.

use super::{DrawPad, ScanEvent, SignaturePadDialog};
use crate::signature::{SignatureDraft, SignatureImage, SignatureTab};
use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, Validity, clicked};
use craft_ui::dialog::StackDialog as _;
use craft_ui::display::Text;
use craft_ui::popover::DropVeil;
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::Ui;

#[story(DrawPad<'static>, covers = [Default], drive = Drive::Drag(Grip::new(super::PAD)))]
fn draw_pad_with_strokes(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (strokes, strokes_at) = data.value_at("strokes", vec![vec![[0.1, 0.5], [0.3, 0.2], [0.5, 0.7], [0.8, 0.4]]]);
    DrawPad::new("Draw", strokes_at.bind(strokes)).show(ui).into()
}

#[story(DrawPad<'static>, covers = [Invalid], drive = Drive::Never)]
fn draw_pad_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_many =
        |strokes: &Vec<Vec<[f32; 2]>>| if strokes.len() > 3 { Validity::Refuse(Message::new("Too many strokes")) } else { Validity::Ok };
    let (strokes, strokes_at) = data.value_at("strokes", vec![vec![[0.1, 0.2], [0.3, 0.4]]; 4]);
    DrawPad::new("Draw", strokes_at.bind(strokes).check(refuse_many)).show(ui).into()
}

/// A dialog kept across passes.
#[derive(Clone)]
struct Held(std::sync::Arc<std::sync::Mutex<SignaturePadDialog>>);

impl PartialEq for Held {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Held(..)")
    }
}

/// What a story's dialog starts from.
struct Start {
    draft: fn() -> SignatureDraft,
    /// Whether the dialog has a Scan tab, with [`scan_stub`] as the app's content.
    scan: bool,
}

impl Start {
    fn dialog(&self) -> Held {
        let pad = SignaturePadDialog::new((self.draft)());
        let pad = if self.scan { pad.scan(scan_stub) } else { pad };
        Held(std::sync::Arc::new(std::sync::Mutex::new(pad)))
    }
}

/// The dialog `start` makes, closed until its opener button is pressed. Apply or Cancel closes it, and the opener then
/// starts a fresh one.
fn dialog(ui: &mut Ui, data: &mut StoryData, start: &Start) -> StoryOutput {
    if !*data.value("open", false) {
        if clicked(&Button::new("Open signature pad").show(ui)) {
            *data.value("pad", None::<Held>) = Some(start.dialog());
            *data.value("open", false) = true;
        }
        return StoryOutput::default();
    }
    let Some(held) = data.value("pad", None::<Held>).clone() else {
        *data.value("open", false) = false;
        return StoryOutput::default();
    };
    let mut stack = craft_ui::dialog::DialogStack::default();
    let mut pad = held.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    stack.push(pad.id(), None);
    if pad.show(ui, &stack).is_some() {
        *data.value("open", false) = false;
    }
    StoryOutput::default()
}

/// The app's Scan tab as a stub: a line of text, and a dropped photo stands in for a cleaned scan.
fn scan_stub(ui: &mut Ui, draft: &mut SignatureDraft, event: ScanEvent) {
    if let ScanEvent::Show(dropped) = event {
        Text::new("The app's scan view: drop a photo of a signature on paper.").show(ui);
        if dropped.is_some() {
            draft.image = Some(ink_sample());
        }
    }
}

/// A picked image: a dark diagonal stroke on clear ground, 120 × 40 pixels.
fn ink_sample() -> SignatureImage {
    let (width, height) = (120_u32, 40_u32);
    let mut rgba = vec![0_u8; (width * height * 4) as usize];
    for x in 0..width {
        let y = (x * (height - 1)) / (width - 1);
        for dy in 0..3 {
            let row = (y + dy).min(height - 1);
            let at = ((row * width + x) * 4) as usize;
            rgba[at..at + 4].copy_from_slice(&[20, 30, 60, 255]);
        }
    }
    SignatureImage::new(width, height, rgba)
}

#[story(SignaturePadDialog, covers = [], drive = Drive::Never)]
fn signature_pad_type(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dialog(ui, data, &Start { draft: || SignatureDraft::new(false, "Ana Reyes"), scan: false })
}

/// A name too wide for the pad at full height shrinks to its width and stays on one line.
#[story(SignaturePadDialog, covers = [], drive = Drive::Never)]
fn signature_pad_typed_long_name(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let draft = || SignatureDraft::new(false, "María de los Ángeles Concepción Fernández-Villaseñor");
    dialog(ui, data, &Start { draft, scan: false })
}

#[story(SignaturePadDialog, covers = [], drive = Drive::Never)]
fn signature_pad_draw(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let draft = || SignatureDraft {
        tab: SignatureTab::Draw,
        strokes: vec![vec![[0.1, 0.6], [0.2, 0.3], [0.3, 0.7], [0.45, 0.35], [0.6, 0.65]], vec![[0.65, 0.5], [0.9, 0.45]]],
        ..SignatureDraft::default()
    };
    dialog(ui, data, &Start { draft, scan: false })
}

#[story(SignaturePadDialog, covers = [], drive = Drive::Never)]
fn signature_pad_image(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let draft = || SignatureDraft { tab: SignatureTab::Image, image: Some(ink_sample()), ..SignatureDraft::default() };
    dialog(ui, data, &Start { draft, scan: false })
}

#[story(SignaturePadDialog, covers = [], drive = Drive::Never)]
fn signature_pad_scan(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let draft = || SignatureDraft { tab: SignatureTab::Scan, ..SignatureDraft::default() };
    dialog(ui, data, &Start { draft, scan: true })
}

/// The Image tab over a window that takes files too: a file dragged in shows the pad's veil, and the pad, not the
/// window, imports it.
#[story(SignaturePadDialog, covers = [], drive = Drive::Never)]
fn signature_pad_drop(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let window = DropVeil::new("Open files").show_with_files(ui, |ui, _| {
        Text::new("Drag a PNG or JPEG over the window.").show(ui);
    });
    if window.output.dropped.is_some() {
        *data.value("window_took", false) = true;
    }
    let draft = || SignatureDraft { tab: SignatureTab::Image, ..SignatureDraft::default() };
    let shown = dialog(ui, data, &Start { draft, scan: false });
    let readout = if *data.value("window_took", false) { "The window took a drop" } else { "The window took no drop" };
    shown.readout(readout)
}
