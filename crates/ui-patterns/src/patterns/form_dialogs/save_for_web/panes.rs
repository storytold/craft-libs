//! The panes of Save for Web: the view tabs, the animation row, the zoom row and the preview panes with their captions.

use super::{
    Animation, Live, MODEM_KBPS, SaveForWebOptions, SaveForWebWords, WebFormat, WebPictures, WebVariant, WebView, ZOOM_RANGE, download_seconds,
    four_up_variants,
};
use crate::buttons::Button;
use crate::choice::Segmented;
use crate::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, clicked};
use crate::display::{Shade, Text};
use crate::layout_components::{Row, Stack};
use crate::patterns::form_dialogs::{PreviewPane, PreviewState, PreviewView, PreviewZoom};
use crate::runtime::env::{Label, Units};
use crate::style::TypeRole;
use craft_behavior::{CanvasTransform, PixelSize};
use egui::{Ui, Vec2};

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
}

/// What the panes of one pass report.
#[derive(Default)]
struct Reported {
    moved: Option<PreviewView>,
    shown: Option<PreviewView>,
    clicked: Option<WebPane>,
}

/// The view tabs, the play and zoom rows and the panes.
pub(super) fn show(ui: &mut Ui, panes: &Panes<'_>, live: &mut Live) {
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
    zoom_row(ui, words, panes.hint, live);
    pane_grid(ui, panes, live);
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

/// Fit, the zoom steps, 100% and the hint, over the one view the panes share.
fn zoom_row(ui: &mut Ui, words: &SaveForWebWords, hint: &Label, live: &mut Live) {
    Row::new().id_salt("zoom").show(ui, |ui, _| {
        if clicked(&Button::new(words.fit.clone()).id_salt("fit").show(ui)) {
            live.shared = PreviewView::fitting().zoom_range(ZOOM_RANGE);
        }
        if let Some(view) = PreviewZoom::new(&live.shared).show(ui).view {
            live.shared = view;
        }
        if clicked(&Button::new(words.actual.clone()).id_salt("actual").show(ui)) {
            live.shared = PreviewView::new(CanvasTransform::new(1.0, Vec2::ZERO)).zoom_range(ZOOM_RANGE);
        }
        Text::new(hint.clone()).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
    });
}

/// The panes of the view, two to a row, over the one shared view: the first view any pane reports is applied to all.
fn pane_grid(ui: &mut Ui, panes: &Panes<'_>, live: &mut Live) {
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
        let shown = PreviewPane::state(name.clone(), state.clone(), &live.shared).click_centres(false).id_salt("pane").show(ui);
        if clicked(&shown.response) {
            reported.clicked = Some(pane);
        }
        if reported.moved.is_none() {
            reported.moved = shown.view;
        }
        reported.shown.get_or_insert(shown.shown);
        Text::new(Label::from(first.clone())).role(TypeRole::Caption).shade(shade).id_salt("first").show(ui);
        Text::new(Label::from(second.clone())).role(TypeRole::Caption).shade(shade).id_salt("second").show(ui);
    });
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
            let format = pane_format(panes.options, index);
            let mut first = words
                .encoded_caption
                .fill(&[("format", format.extension().to_uppercase().as_str()), ("size", panes.units.file_size(variant.bytes).as_str())]);
            if variant.proxy {
                first.push_str(words.proxy.as_str());
            }
            let mut second = words.download.fill(&[("seconds", download_seconds(variant.bytes, MODEM_KBPS).to_string().as_str())]);
            if let (Some(colors), true) = (variant.colors, format.indexed()) {
                second.push_str(&words.color_count.fill(&[("count", colors.to_string().as_str())]));
            }
            (first, second)
        }
    }
}

/// The format the encoded pane `index` is encoded in.
fn pane_format(options: &SaveForWebOptions, index: usize) -> WebFormat {
    match index {
        0 => options.format,
        _ => four_up_variants(options)[(index - 1).min(1)].format,
    }
}
