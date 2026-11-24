//! The panes of Save for Web: the view tabs, the animation row, the zoom row and the preview panes with their captions.

use super::{
    Animation, Live, SaveForWebOptions, SaveForWebWords, WEB_SPEEDS, WebFormat, WebPictures, WebSlice, WebVariant, WebView, ZOOM_RANGE,
    download_seconds, four_up_variants,
};
use crate::buttons::Button;
use crate::choice::Segmented;
use crate::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, TooltipSpec, clicked};
use crate::display::{Shade, Text};
use crate::layout_components::{Row, Stack};
use crate::patterns::form_dialogs::{PreviewPane, PreviewState, PreviewView, PreviewZoom};
use crate::pickers::Select;
use crate::primitives::{ContentPainterCx, StrokeWidth, TextRun};
use crate::runtime::env::{Label, Units};
use crate::style::TypeRole;
use craft_behavior::{CanvasTransform, PixelSize};
use egui::{Pos2, Rect, Ui, Vec2};

/// Whether the zoom row has Preview in Browser.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum BrowserPreview {
    /// No button.
    #[default]
    Hidden,
    /// The button works.
    Available,
    /// The button is disabled and the label is its reason.
    Unavailable(Label),
}

/// A pane of the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WebPane {
    /// The document as it is.
    Original,
    /// The encode with the settings (index 0) or one of the 4-Up variants (1 and 2).
    Encoded(usize),
}

/// What the panes read besides the dialog's live state.
pub(super) struct Panes<'p> {
    pub(super) words: &'p SaveForWebWords,
    pub(super) units: &'p Units,
    pub(super) name: &'p str,
    pub(super) size: PixelSize,
    pub(super) pictures: &'p WebPictures,
    pub(super) animation: Option<Animation>,
    pub(super) hint: &'p Label,
    pub(super) options: &'p SaveForWebOptions,
    /// The document's slices, in the order they are drawn: a later one lies over an earlier one.
    pub(super) slices: &'p [WebSlice],
    /// The numbers of the selected slices.
    pub(super) chosen: &'p [u32],
    /// The formats the host can write.
    pub(super) formats: &'p [WebFormat],
    /// Whether the zoom row has Preview in Browser.
    pub(super) browser: &'p BrowserPreview,
    /// The Download speed of the captions, in Kbps.
    pub(super) speed: f64,
}

/// A click on a slice: its number and whether Shift was held (add to or remove from the selection).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SliceClick {
    pub(super) number: u32,
    pub(super) add: bool,
}

/// What the panes of one pass report.
#[derive(Default)]
struct Reported {
    moved: Option<PreviewView>,
    shown: Option<PreviewView>,
    clicked: Option<WebPane>,
    slice: Option<SliceClick>,
}

/// The view tabs, the play and zoom rows and the panes; a click on a slice goes to `slice` and a press of Preview in
/// Browser sets `browser`.
pub(super) fn show(ui: &mut Ui, panes: &Panes<'_>, live: &mut Live, slice: &mut Option<SliceClick>, browser: &mut bool) {
    let words = panes.words;
    let tabs = [
        Entry::new(WebView::Original, words.original.clone()),
        Entry::new(WebView::Optimized, words.optimized.clone()),
        Entry::new(WebView::TwoUp, words.two_up.clone()),
        Entry::new(WebView::FourUp, words.four_up.clone()),
    ];
    if let Some(view) = Segmented::new(words.view.clone(), Bind::given(live.view), &tabs).show(ui).value().cloned() {
        live.view = view;
    }
    play_row(ui, panes, live);
    zoom_row(ui, panes, live, browser);
    pane_grid(ui, panes, live, slice);
}

/// Play or Pause and the frame readout, for an animation of two frames or more.
fn play_row(ui: &mut Ui, panes: &Panes<'_>, live: &mut Live) {
    let Some(Animation { frames, .. }) = panes.animation.filter(|animation| animation.frames >= 2) else { return };
    let words = panes.words;
    Row::new().id_salt("play").show(ui, |ui, _| {
        let label = if live.playing { words.pause.clone() } else { words.play.clone() };
        if clicked(&Button::new(label).id_salt("play").show(ui)) {
            live.playing = !live.playing;
        }
        let readout = words.frame.fill(&[("frame", (live.frame + 1).to_string().as_str()), ("total", frames.to_string().as_str())]);
        Text::new(Label::from(readout)).show(ui);
    });
}

/// The word of the download speed `kbps`: megabits from 1024 Kbps up.
fn speed_label(words: &SaveForWebWords, units: &Units, kbps: f64) -> String {
    let (word, n) = if kbps >= 1024.0 { (&words.speed_mbps, kbps / 1024.0) } else { (&words.speed_kbps, kbps) };
    word.fill(&[("n", units.number(n, 0..=1).as_str())])
}

/// Fit, the zoom steps, 100%, the Download speed, the hint and Preview in Browser, over the one view the panes share.
fn zoom_row(ui: &mut Ui, panes: &Panes<'_>, live: &mut Live, pressed: &mut bool) {
    let words = panes.words;
    Row::new().wrap(true).id_salt("zoom").show(ui, |ui, _| {
        if clicked(&Button::new(words.fit.clone()).id_salt("fit").show(ui)) {
            live.shared = PreviewView::fitting().zoom_range(ZOOM_RANGE);
        }
        if let Some(view) = PreviewZoom::new(&live.shared).show(ui).view {
            live.shared = view;
        }
        if clicked(&Button::new(words.actual.clone()).id_salt("actual").show(ui)) {
            live.shared = PreviewView::new(CanvasTransform::new(1.0, Vec2::ZERO)).zoom_range(ZOOM_RANGE);
        }
        let speeds: Vec<Entry<usize>> =
            WEB_SPEEDS.iter().enumerate().map(|(at, kbps)| Entry::new(at, Label::from(speed_label(words, panes.units, *kbps)))).collect();
        let current = WEB_SPEEDS.iter().position(|kbps| *kbps == live.speed).unwrap_or(0);
        if let Some(at) = Select::new(words.download_speed.clone(), Bind::given(current), &speeds).id_salt("speed").show(ui).value().copied()
            && let Some(kbps) = WEB_SPEEDS.get(at)
        {
            live.speed = *kbps;
        }
        Text::new(panes.hint.clone()).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
        if !matches!(panes.browser, BrowserPreview::Hidden) {
            let mut button = Button::new(words.preview_browser.clone()).id_salt("browser");
            if let BrowserPreview::Unavailable(reason) = panes.browser {
                button = button.enabled(false).tooltip(TooltipSpec::new(words.preview_browser.clone()).disabled_text(reason.clone()));
            }
            if clicked(&button.show(ui)) {
                *pressed = true;
            }
        }
    });
}

/// The panes of the view, two to a row, over the one shared view: the first view any pane reports is applied to all.
fn pane_grid(ui: &mut Ui, panes: &Panes<'_>, live: &mut Live, slice: &mut Option<SliceClick>) {
    let list = live.view.panes();
    let mut reported = Reported::default();
    Stack::new().id_salt("panes").show(ui, |ui, _| {
        for (line, row) in list.chunks(2).enumerate() {
            Row::new().id_salt(line).show(ui, |ui, _| {
                for (at, pane) in row.iter().enumerate() {
                    one_pane(ui, panes, live, *pane, line * 2 + at, &mut reported);
                }
            });
        }
    });
    if let Some(pane) = reported.clicked {
        live.selected = pane;
    }
    if reported.slice.is_some() {
        *slice = reported.slice;
    }
    if let Some(view) = reported.moved.or(reported.shown) {
        live.shared = view;
    }
}

/// One pane with its two caption lines.
fn one_pane(ui: &mut Ui, panes: &Panes<'_>, live: &Live, pane: WebPane, at: usize, reported: &mut Reported) {
    let (first, second) = captions(panes, pane);
    let name = Label::from(panes.words.pane_name.fill(&[
        ("pane", pane_title(panes.words, pane).as_str()),
        ("first", first.as_str()),
        ("second", second.as_str()),
    ]));
    let state = state_of(panes, pane);
    let shade = if live.selected == pane { Shade::Normal } else { Shade::Muted };
    Stack::new().id_salt(at).show(ui, |ui, _| {
        let mut preview = PreviewPane::state(name.clone(), state.clone(), &live.shared).click_centres(false).id_salt("pane");
        if !panes.slices.is_empty() {
            let (slices, chosen) = (panes.slices, panes.chosen);
            preview = preview.overlay(move |painter: &ContentPainterCx, view: CanvasTransform, picture: Rect| {
                paint_slices(painter, view, picture, slices, chosen);
            });
        }
        let shown = preview.show(ui);
        if clicked(&shown.response) {
            reported.clicked = Some(pane);
        }
        if let Some(point) = shown.clicked
            && let Some(number) = slice_at(panes.slices, point)
        {
            reported.slice = Some(SliceClick { number, add: crate::runtime::pointer::mods(ui.ctx()).shift });
        }
        if reported.moved.is_none() {
            reported.moved = shown.view;
        }
        reported.shown.get_or_insert(shown.shown);
        Text::new(Label::from(first.clone())).role(TypeRole::Caption).shade(shade).id_salt("first").show(ui);
        Text::new(Label::from(second.clone())).role(TypeRole::Caption).shade(shade).id_salt("second").show(ui);
    });
}

/// The number of the slice a click at `point` (in document pixels) hit: the one drawn last of those that hold it.
fn slice_at(slices: &[WebSlice], point: Pos2) -> Option<u32> {
    slices.iter().rev().find(|slice| slice.rect.contains(point)).map(|slice| slice.number)
}

/// Outlines each slice over the picture with its number; a selected slice has the thicker outline. `picture` is the
/// picture's rectangle on screen and `view` its scale, so the outlines follow the pane's own zoom and pan.
fn paint_slices(painter: &ContentPainterCx<'_>, view: CanvasTransform, picture: Rect, slices: &[WebSlice], chosen: &[u32]) {
    let ink = painter.ink();
    let (thin, thick) = (painter.stroke_width(StrokeWidth::Thin), painter.stroke_width(StrokeWidth::Thick));
    for slice in slices {
        let rect = Rect::from_min_max(picture.min + slice.rect.min.to_vec2() * view.scale, picture.min + slice.rect.max.to_vec2() * view.scale);
        let width = if chosen.contains(&slice.number) { thick } else { thin };
        let corners = vec![rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
        painter.painter.add(egui::Shape::closed_line(corners, egui::Stroke::new(width, ink)));
        let number = slice.number.to_string();
        let inset = painter.style_cx().tokens.space.gap.tight;
        painter.with_draw(|draw| {
            let run = TextRun::new(number.as_str()).role(TypeRole::Caption);
            let corner = rect.left_top() + Vec2::splat(inset);
            let layout = draw.layout(&run, f32::INFINITY);
            draw.galley(corner, &layout);
        });
    }
}

/// The pane's name in the accessible tree.
fn pane_title(words: &SaveForWebWords, pane: WebPane) -> String {
    match pane {
        WebPane::Original => words.original.as_str().to_owned(),
        WebPane::Encoded(0) => words.optimized.as_str().to_owned(),
        WebPane::Encoded(index) => words.optimized_variant.fill(&[("name", words.optimized.as_str()), ("number", (index + 1).to_string().as_str())]),
    }
}

/// What the pane's well shows.
fn state_of(panes: &Panes<'_>, pane: WebPane) -> PreviewState {
    match pane {
        WebPane::Original => panes.pictures.original.map_or(PreviewState::Pending(None), PreviewState::Ready),
        WebPane::Encoded(index) => match panes.pictures.variants.get(index).and_then(Option::as_ref) {
            Some(WebVariant { picture: Ok(texture), .. }) => PreviewState::Ready(*texture),
            Some(WebVariant { picture: Err(message), .. }) => PreviewState::Failed(message.clone()),
            None => PreviewState::Pending(None),
        },
    }
}

/// The pane's two caption lines: format, size and proxy mark; download time and colour count.
fn captions(panes: &Panes<'_>, pane: WebPane) -> (String, String) {
    let words = panes.words;
    match pane {
        WebPane::Original => {
            let second = words.original_caption.fill(&[
                ("name", panes.name),
                ("width", panes.size.width.to_string().as_str()),
                ("height", panes.size.height.to_string().as_str()),
            ]);
            (words.original.as_str().to_owned(), second)
        }
        WebPane::Encoded(index) => {
            let Some(variant) = panes.pictures.variants.get(index).and_then(Option::as_ref) else {
                return (words.pane_pending.as_str().to_owned(), String::new());
            };
            let format = pane_format(panes.options, panes.formats, index);
            let mut first = words
                .encoded_caption
                .fill(&[("format", format.extension().to_uppercase().as_str()), ("size", panes.units.file_size(variant.bytes).as_str())]);
            if variant.proxy {
                first.push_str(words.proxy.as_str());
            }
            let mut second = words.download.fill(&[
                ("seconds", download_seconds(variant.bytes, panes.speed).to_string().as_str()),
                ("speed", speed_label(words, panes.units, panes.speed).as_str()),
            ]);
            if let (Some(colors), true) = (variant.colors, format.indexed()) {
                second.push_str(&words.color_count.fill(&[("count", colors.to_string().as_str())]));
            }
            (first, second)
        }
    }
}

/// The format the encoded pane `index` is encoded in.
fn pane_format(options: &SaveForWebOptions, formats: &[WebFormat], index: usize) -> WebFormat {
    match index {
        0 => options.format,
        _ => four_up_variants(options, formats)[(index - 1).min(1)].format,
    }
}
