//! The signature pad dialog and saved signatures.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use craft_behavior::{EditCommand, Input, Metrics, PartKey, PointerButton, PressPhase, Sense};
use craft_host::{Host, PickId, PickRequest, Picked};
use craft_theme::variant::Tone;
use craft_ui::buttons::Button;
use craft_ui::choice::Segmented;
use craft_ui::contract::{
    Bind, BindMut, Common, Component, Cx, Edit as _, Entry, Focus as _, MeasureCx, PaintUi, PartsCx, Pointer as _, Props as _, Role, Show as _,
    ShowContainer as _, clicked,
};
use craft_ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::display::{Picture, PictureSize, Text};
use craft_ui::fonts::FontRef;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::ThumbSize;
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::primitives::{DocRun, DocText, Draw, HAlign, PaintShape, StrokeWidth, TextureCache};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::focus::{ActivateKeys, EditSet, KeyUse};
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::style::{ComponentStyle, DataColor, FocusLook, StyleColor, StyleCx, SurfaceKind};
use craft_ui::text::TextField;
use egui::{ColorImage, Pos2, Rect, Vec2};

use crate::signature::{MAX_IMAGE_BYTES, MAX_IMAGE_SIDE, MAX_SIGNATURE_CHARS, SavedSignature, SignatureDraft, SignatureImage, SignatureTab};

#[cfg(feature = "screen-testing")]
mod stories;

/// The pad's width over its height.
const PAD_ASPECT: f32 = 460.0 / 150.0;
/// Pad points closer than this (in pad units, both axes together) to the last one are dropped.
const MIN_STEP: f32 = 0.002;
/// The share of a box's height that strokes fill.
const FILL: f32 = 0.9;
/// The share of a box's height that typed text's font size takes.
const TYPED_HEIGHT_SHARE: f32 = 0.9;
/// The share of a box's width that typed text may take.
const TYPED_WIDTH_SHARE: f32 = 0.95;

craft_ui::words! {
    /// The pad's words, English by default.
    #[non_exhaustive]
    pub struct SignatureWords {
        create: template "signature.create" = "Create {what}";
        change: template "signature.change" = "Change {what}";
        signature: "signature.signature" = "signature";
        initials: "signature.initials" = "initials";
        type_tab: "signature.typeTab" = "Type";
        draw_tab: "signature.drawTab" = "Draw";
        image_tab: "signature.imageTab" = "Image";
        scan_tab: "signature.scanTab" = "Scan";
        type_hint: "signature.typeHint" = "Type text";
        image_hint: "signature.imageHint" = "Choose a PNG or JPEG image (up to 4 MiB). Transparency is preserved.";
        browse: "signature.browse" = "Browse…";
        browse_title: "signature.browseTitle" = "Choose an image";
        image_filter: "signature.imageFilter" = "Image";
        undo: "signature.undo" = "Undo";
        clear: "signature.clear" = "Clear";
        apply: "signature.apply" = "Apply";
        cancel: "signature.cancel" = "Cancel";
        image_failed: "signature.imageFailed" = "That image could not be read.";
        image_too_large: "signature.imageTooLarge" = "That image is too large: up to 4 MiB and 8192 × 8192 pixels.";
    }
}

/// The saved signature and initials, one settings section shared by every document.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct SignatureSettings {
    /// The saved signature, if any.
    pub signature: Option<SavedSignature>,
    /// The saved initials, if any.
    pub initials: Option<SavedSignature>,
}

impl craft_host::Section for SignatureSettings {
    const KEY: &'static str = "signatures";
}

/// Draws a saved signature into `rect` in `color`: strokes scaled from pad units and fitted keeping the pad's shape
/// (at the theme's medium stroke, whatever the rect's size), typed text in the proportional face centred in the
/// rect, an image fitted keeping its aspect. `ctx` holds the image's texture, cached under the image's key.
pub fn paint_signature(ctx: &egui::Context, draw: &Draw<'_>, rect: Rect, signature: &SavedSignature, color: DataColor) {
    match signature {
        SavedSignature::Drawn(strokes) => paint_strokes(
            rect,
            strokes,
            true,
            |dot| draw.data_fill(&PaintShape::Circle(dot, 1.0), color),
            |line| draw.stroke(&PaintShape::Path(line), StrokeWidth::Medium, color),
        ),
        SavedSignature::Typed(label) => paint_typed(draw, rect, label, color),
        SavedSignature::Image(image) => paint_image(ctx, draw, rect, image),
    }
}

/// Strokes in pad units: with `fit`, scaled to fill `rect` keeping the pad's shape; else laid over `rect` as the pad's
/// own box. A one-point stroke goes to `dot`, a longer one to `line`.
fn paint_strokes(rect: Rect, strokes: &[Vec<[f32; 2]>], fit: bool, dot: impl Fn(Pos2), line: impl Fn(Vec<Pos2>)) {
    let points = || strokes.iter().flatten().filter(|p| p.iter().all(|v| v.is_finite()));
    let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for p in points() {
        let at = Vec2::new(p[0] * PAD_ASPECT, p[1]);
        min = min.min(at);
        max = max.max(at);
    }
    if min.x > max.x {
        return;
    }
    let extent = (max - min).max(Vec2::splat(f32::EPSILON));
    let scale = (rect.width() / extent.x).min(rect.height() / extent.y) * FILL;
    let middle = (min + max) / 2.0;
    let place = |p: &[f32; 2]| {
        if fit {
            rect.center() + (Vec2::new(p[0] * PAD_ASPECT, p[1]) - middle) * scale
        } else {
            rect.min + Vec2::new(p[0] * rect.width(), p[1] * rect.height())
        }
    };
    for stroke in strokes {
        let shown: Vec<Pos2> = stroke.iter().filter(|p| p.iter().all(|v| v.is_finite())).map(place).collect();
        match shown.as_slice() {
            [] => {}
            [point] => dot(*point),
            _ => line(shown),
        }
    }
}

/// `label` in the proportional face, as large as fits `rect`, centred.
fn paint_typed(draw: &Draw<'_>, rect: Rect, label: &str, color: DataColor) {
    if label.trim().is_empty() {
        return;
    }
    let size = rect.height() * TYPED_HEIGHT_SHARE;
    let room = Vec2::new(rect.width() * TYPED_WIDTH_SHARE, size);
    let run = [DocRun::new(label.to_owned(), FontRef::proportional(), size, color)];
    let mut text = DocText::new(&run);
    text.align = HAlign::Center;
    draw.doc(Rect::from_center_size(rect.center(), room), &text);
}

/// `image` fitted into `rect` keeping its aspect.
fn paint_image(ctx: &egui::Context, draw: &Draw<'_>, rect: Rect, image: &SignatureImage) {
    let size = Vec2::new(image.width() as f32, image.height() as f32);
    if size.min_elem() < 1.0 {
        return;
    }
    let texture = TextureCache::texture(ctx, ("signature-image", image.key()), || pixels(image));
    let fitted = size * (rect.width() / size.x).min(rect.height() / size.y);
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    draw.texture(Rect::from_center_size(rect.center(), fitted), texture, uv);
}

/// Why a picked image was not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickError {
    /// Unreadable, or not a PNG or JPEG.
    Unreadable,
    /// More than [`MAX_IMAGE_BYTES`] of file or [`MAX_IMAGE_SIDE`] pixels on a side.
    TooLarge,
}

type Decoded = Result<SignatureImage, PickError>;

/// Where a decode job gets its bytes.
enum Source {
    Bytes(Vec<u8>),
    Path(PathBuf),
}

/// Decodes a PNG or JPEG of at most [`MAX_IMAGE_BYTES`] and [`MAX_IMAGE_SIDE`] pixels a side into straight RGBA. The
/// size limits apply while decoding, so an oversize image is refused before its pixels are allocated.
fn decode_image(bytes: &[u8]) -> Decoded {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(PickError::TooLarge);
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|_| PickError::Unreadable)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| if matches!(error, image::ImageError::Limits(_)) { PickError::TooLarge } else { PickError::Unreadable })?
        .into_rgba8();
    Ok(SignatureImage::new(decoded.width(), decoded.height(), decoded.into_raw()))
}

/// The pixels of `image` for a `Picture`, empty when its buffer does not match its size.
fn pixels(image: &SignatureImage) -> ColorImage {
    let size = [image.width() as usize, image.height() as usize];
    if size[0].checked_mul(size[1]).and_then(|n| n.checked_mul(4)) == Some(image.rgba().len()) {
        ColorImage::from_rgba_unmultiplied(size, image.rgba())
    } else {
        ColorImage::new([1, 1], vec![egui::Color32::TRANSPARENT])
    }
}

/// The style of the pad.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct DrawPadStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    ink: StyleColor,
}

const PAD: PartKey = PartKey::new("pad");

/// The Draw tab's pad: a paper box that records strokes in pad units from the pointer or a pen. The Type tab makes the
/// same signature from the keyboard.
#[derive(craft_ui::contract::Component)]
pub struct DrawPad<'a> {
    #[common]
    common: Common,
    strokes: Bind<'a, Vec<Vec<[f32; 2]>>>,
}

impl<'a> DrawPad<'a> {
    /// A pad named `label` that edits `strokes`.
    pub fn new(label: impl Into<craft_ui::runtime::env::Label>, strokes: Bind<'a, Vec<Vec<[f32; 2]>>>) -> Self {
        DrawPad { common: Common::new(), strokes }.label(label)
    }
}

impl Component for DrawPad<'_> {
    const ROLE: Role = Role::IMAGE;
    const OVERLAYS: bool = false;
    const SINGLE_VALUE: bool = true;
    /// The pad is drawn with the pointer: it takes no activation key, and Undo removes the last stroke.
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_activate(ActivateKeys::Neither).with_edits(EditSet::NONE.with(EditCommand::Undo));
    type Style = DrawPadStyle;
    type Output = craft_ui::contract::EditResponse<Vec<Vec<[f32; 2]>>>;

    fn style(cx: &StyleCx<'_>, _: DrawPadStyleKey) -> DrawPadStyle {
        let width = cx.tokens.size.plot.width;
        DrawPadStyle {
            metrics: Metrics::new().min_size(Vec2::new(width, width / PAD_ASPECT)),
            focus: cx.focus_look(),
            ink: cx.color(cx.tokens.color.text.normal),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        cx.style.metrics.min_size
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS
    }

    fn bind(&mut self) -> Option<BindMut<'_, Vec<Vec<[f32; 2]>>>> {
        Some(self.strokes.reborrow())
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(PAD, rect).sense(Sense::DRAG | Sense::TAKE_FOCUS).pointer_only("the Type tab makes the same signature from the keyboard");
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let rect = cx.rect;
        let normalise = |p: Pos2| {
            let p = p.clamp(rect.min, rect.max);
            [(p.x - rect.left()) / rect.width(), (p.y - rect.top()) / rect.height()]
        };
        let mut strokes = cx.value().cloned().unwrap_or_default();
        while let Some(command) = cx.intents().take_edit() {
            if command == EditCommand::Undo && pop_stroke(&mut strokes) {
                cx.set(strokes.clone());
            }
        }
        // The pointer belongs to the pad part: the component itself senses focus only.
        let press = cx.part(PAD).press;
        if !press.phase.start().is_some_and(|start| start.button == PointerButton::Primary) {
            return;
        }
        match press.phase {
            PressPhase::DragStart { pos, .. } => {
                strokes.push(vec![normalise(pos)]);
                cx.propose(strokes);
            }
            PressPhase::Drag { pos, .. } if cx.gesture().is_some() => {
                if extend(&mut strokes, normalise(pos)) {
                    cx.propose(strokes);
                }
            }
            PressPhase::Release { pos, click: false, .. } if cx.gesture().is_some() => {
                if extend(&mut strokes, normalise(pos)) {
                    cx.propose(strokes);
                }
                cx.commit();
            }
            _ => {}
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        draw.surface(cx.rect, SurfaceKind::Well, craft_ui::style::Corners::All);
        let strokes = cx.value().cloned().unwrap_or_default();
        let ink = cx.style.ink;
        paint_strokes(
            cx.rect,
            &strokes,
            false,
            |dot| draw.fill(&PaintShape::Circle(dot, 1.0), ink),
            |line| draw.stroke(&PaintShape::Path(line), StrokeWidth::Medium, ink),
        );
    }

    fn finish(self, cx: Cx<'_, Self>) -> Self::Output {
        cx.edit_response()
    }
}

/// Adds `point` to the last stroke unless it is within [`MIN_STEP`] of the stroke's last point; `true` when it was added.
fn extend(strokes: &mut [Vec<[f32; 2]>], point: [f32; 2]) -> bool {
    let Some(stroke) = strokes.last_mut() else { return false };
    if stroke.last().is_none_or(|last| (last[0] - point[0]).abs() + (last[1] - point[1]).abs() > MIN_STEP) {
        stroke.push(point);
        true
    } else {
        false
    }
}

/// Removes the last visible stroke (one of two points or more) with any bare dots after it; `true` when one went.
fn pop_stroke(strokes: &mut Vec<Vec<[f32; 2]>>) -> bool {
    while let Some(stroke) = strokes.pop() {
        if stroke.len() > 1 {
            return true;
        }
    }
    false
}

/// The Scan tab's content, the app's.
pub type ScanTab = Box<dyn FnMut(&mut egui::Ui, &mut SignatureDraft) + Send>;

/// Acrobat's signature pad on the dialog host: "Create signature" or "Change initials" with the tabs Type (a field),
/// Draw (a pad of strokes, Undo), Image (Browse… a PNG or JPEG up to [`MAX_IMAGE_BYTES`] through `host.files()`) and
/// Scan (shown when the app gives a scan hook); Clear; Apply (enabled when [`SignatureDraft::ready`]) and Cancel.
/// Browse… reads and decodes the file on `host.jobs()`; a pick that fails or is too large shows an error under the
/// button.
#[derive(craft_ui::contract::Pattern)]
pub struct SignaturePadDialog {
    draft: SignatureDraft,
    scan: Option<ScanTab>,
    open: bool,
    /// The Browse… pick waiting for its answer.
    pick: Option<PickId>,
    /// The decode job of a pick, until its result arrives.
    decoding: Option<Receiver<Decoded>>,
    /// The last pick's failure, until a pick succeeds or Clear.
    error: Option<PickError>,
}

/// What [`SignaturePadDialog`] answers with.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SignaturePadResult {
    /// What the user drew, typed or picked.
    pub signature: SavedSignature,
    /// Whether it is initials (else the signature).
    pub initials: bool,
}

impl SignaturePadDialog {
    /// A pad that starts from `draft`.
    pub fn new(draft: SignatureDraft) -> Self {
        Self { draft, scan: None, open: true, pick: None, decoding: None, error: None }
    }

    /// The Scan tab's content: it fills `draft.image`.
    #[must_use]
    pub fn scan(mut self, tab: impl FnMut(&mut egui::Ui, &mut SignatureDraft) + Send + 'static) -> Self {
        self.scan = Some(Box::new(tab));
        self
    }

    /// Starts the decode of a Browse… answer when it has come, and takes a finished decode.
    fn take_picked(&mut self, host: &Host) {
        if let Some(ticket) = self.pick
            && let Some(picked) = host.files().take_answer(ticket)
        {
            self.pick = None;
            let source = match picked {
                Picked::Cancelled | Picked::Unavailable(_) => None,
                Picked::Contents(files) => files.into_iter().next().map(|(_, bytes)| Source::Bytes(bytes)),
                Picked::Paths(paths) => paths.into_iter().next().map(Source::Path),
            };
            if let Some(source) = source {
                let (send, receive) = channel();
                let files = host.clone();
                host.jobs().spawn(
                    "signature-image",
                    Box::new(move || {
                        let decoded = match source {
                            Source::Bytes(bytes) => decode_image(&bytes),
                            Source::Path(path) => files.files().read(&path).map_err(|_| PickError::Unreadable).and_then(|bytes| decode_image(&bytes)),
                        };
                        // The dialog may have closed; then nobody reads the answer.
                        let _ = send.send(decoded);
                    }),
                );
                self.decoding = Some(receive);
            }
        }
        let Some(receive) = &self.decoding else { return };
        match receive.try_recv() {
            Ok(Ok(image)) => {
                self.draft.image = Some(image);
                self.error = None;
                self.decoding = None;
            }
            Ok(Err(error)) => {
                self.error = Some(error);
                self.decoding = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.error = Some(PickError::Unreadable);
                self.decoding = None;
            }
        }
    }

    fn forget_pick(&mut self, host: Option<&Host>) {
        self.decoding = None;
        if let (Some(ticket), Some(host)) = (self.pick.take(), host) {
            host.files().forget(ticket);
        }
    }
}

impl StackDialog for SignaturePadDialog {
    type Result = SignaturePadResult;

    fn id(&self) -> DialogId {
        dialog_id("signature-pad")
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let env = Env::of(ui);
        let words = env.words::<SignatureWords>();
        let host = env.view().host().cloned();
        if let Some(host) = &host {
            self.take_picked(host);
        }
        let what = if self.draft.initials { words.initials.as_str() } else { words.signature.as_str() };
        let title = if self.draft.editing { words.change.fill(&[("what", what)]) } else { words.create.fill(&[("what", what)]) };
        let ready = self.draft.ready();
        let buttons = vec![
            FooterButton::new(words.apply.clone(), DialogRole::Accept).enabled(ready),
            FooterButton::new(words.cancel.clone(), DialogRole::Cancel),
        ];
        let mut browse = false;
        let SignaturePadDialog { draft, scan, open, decoding, error, .. } = self;
        let decoding = decoding.is_some();
        let shown = Dialog::new(title.clone()).opened(Bind::bound(open)).buttons(buttons).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                let mut tabs = vec![
                    Entry::new(SignatureTab::Type, words.type_tab.clone()),
                    Entry::new(SignatureTab::Draw, words.draw_tab.clone()),
                    Entry::new(SignatureTab::Image, words.image_tab.clone()),
                ];
                if scan.is_some() {
                    tabs.push(Entry::new(SignatureTab::Scan, words.scan_tab.clone()));
                }
                Segmented::new(title.clone(), Bind::bound(&mut draft.tab), &tabs).show(ui);
                match draft.tab {
                    SignatureTab::Type => {
                        TextField::new(words.type_hint.clone(), Bind::bound(&mut draft.text))
                            .placeholder(words.type_hint.clone())
                            .limit(MAX_SIGNATURE_CHARS)
                            .show(ui);
                    }
                    SignatureTab::Draw => {
                        DrawPad::new(words.draw_tab.clone(), Bind::bound(&mut draft.strokes)).show(ui);
                    }
                    SignatureTab::Image => {
                        Text::new(words.image_hint.clone()).show(ui);
                        if clicked(&Button::new(words.browse.clone()).icon(craft_behavior::IconId::new("folder")).loading(decoding).show(ui)) {
                            browse = true;
                        }
                        if let Some(error) = *error {
                            let word = match error {
                                PickError::Unreadable => &words.image_failed,
                                PickError::TooLarge => &words.image_too_large,
                            };
                            Notice::new(Tone::Bad, word.clone()).layout(NoticeLayout::Inline).show(ui);
                        }
                        if let Some(image) = &draft.image {
                            let make = || pixels(image);
                            Picture::new(words.image_tab.clone(), image.key(), &make).extent(PictureSize::Thumb(ThumbSize::Small)).show(ui);
                        }
                    }
                    SignatureTab::Scan => {
                        if let Some(tab) = scan.as_mut() {
                            tab(ui, draft);
                        }
                    }
                }
                Row::new().show(ui, |ui, _| {
                    if draft.tab == SignatureTab::Draw && clicked(&Button::new(words.undo.clone()).enabled(!draft.strokes.is_empty()).show(ui)) {
                        draft.undo_stroke();
                    }
                    if clicked(&Button::new(words.clear.clone()).show(ui)) {
                        draft.clear();
                        *error = None;
                    }
                });
            });
        });
        if browse && let Some(host) = &host {
            self.forget_pick(Some(host));
            let request =
                PickRequest::open("signature-image").title(words.browse_title.as_str()).filter(words.image_filter.as_str(), &["png", "jpg", "jpeg"]);
            self.pick = Some(host.files().pick(request));
        }
        let output = shown.output;
        match output.picked {
            Some(0) => {
                let saved = self.draft.saved()?;
                self.forget_pick(host.as_deref());
                Some(DialogResult::Ok(SignaturePadResult { signature: saved, initials: self.draft.initials }))
            }
            _ if output.closed.is_some() => {
                self.forget_pick(host.as_deref());
                Some(DialogResult::Cancel)
            }
            _ => None,
        }
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<Self::Result> {
        self.draft.saved().map(|signature| SignaturePadResult { signature, initials: self.draft.initials })
    }

    fn cancelled(&mut self, _: &egui::Context) {
        self.decoding = None;
        self.pick = None;
    }
}
