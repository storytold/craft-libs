//! `PagePreview`: a page shown large in a modal dialog with previous and next, as PdfCraft's Combine Files preview
//! and VectorCraft's Place PDF page choice show it: the page as large as fits, "Page 3 of 12", the arrow and Page
//! keys to step, Escape or the close action to leave.

#[cfg(feature = "screen-testing")]
mod stories;

use super::view::PageSource;
use craft_behavior::IconId;
use craft_behavior::{Input, Metrics, NavMove, Sense};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{
    Common, Component, Cx, Focus as _, MeasureCx, NoValue, Outcome, PaintUi, Props as _, Role, Show as _, ShowContainer as _, clicked,
};
use craft_ui::dialog::Dialog;
use craft_ui::layout_components::Row;
use craft_ui::primitives::textures::{
    MAX_TEXTURE, Priority, TextureState, TextureStoreOptions, ThumbPaint, ThumbRequest, ThumbnailOptions, Thumbnails,
};
use craft_ui::primitives::{PaintShape, TextRun};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use craft_ui::runtime::memory::Remember as _;
use craft_ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx, TypeRole};
use egui::{CornerRadius, Rect, Response, Ui, Vec2};
use std::cell::Cell;

craft_ui::words! {
    /// The preview's words (PdfCraft's strings).
    pub struct PagePreviewWords {
        page_of: template "pagePreview.pageOf" = "Page {p} of {n}";
        previous: "pagePreview.previous" = "Previous page";
        next: "pagePreview.next" = "Next page";
        unavailable: "pagePreview.unavailable" = "Preview unavailable";
    }
}

/// What the preview did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct PagePreviewOutput {
    /// The dialog's response.
    pub response: Response,
    /// The page shown changed (the app may sync its grid's selection).
    pub stepped: bool,
    /// Escape or the close action.
    pub closed: bool,
}

impl Outcome for PagePreviewOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The preview over the window, shown each pass while the app keeps it open.
///
/// - A modal [`Dialog`] titled "Page {p} of {n}"; the page's picture as large as fits through `Thumbnails<usize>`
///   over the [`PageSource`] (side from `ThumbRequest::for_cell`, at most `MAX_TEXTURE`; the last picture stays as
///   the stand-in until the new one lands), on `paper` while it loads. A page that fails to render shows [`PagePreviewWords::unavailable`].
/// - In the footer: previous and next, disabled at the ends. The pages stepped through are `pages` (the pages a
///   range selects), in order.
/// - Keys (with the picture focused): Left and Page Up previous, Right and Page Down next (reversed in a
///   right-to-left interface), Escape closes.
#[derive(craft_ui::contract::Pattern)]
pub struct PagePreview<'a> {
    source: &'a dyn PageSource,
    pages: &'a [usize],
    at: &'a mut usize,
    paper: DataColor,
}

impl<'a> PagePreview<'a> {
    /// Previews `pages` (0-based) on the document's `paper`, showing `pages[*at]`.
    pub fn new(source: &'a dyn PageSource, pages: &'a [usize], at: &'a mut usize, paper: DataColor) -> Self {
        PagePreview { source, pages, at, paper }
    }

    /// Shows the preview over the window and reports page changes and close requests.
    pub fn show(self, ui: &mut Ui) -> PagePreviewOutput {
        let PagePreview { source, pages, at, paper } = self;
        let Some(last) = pages.len().checked_sub(1) else {
            return PagePreviewOutput { response: ui.response(), stepped: false, closed: true };
        };
        *at = (*at).min(last);
        let env = Env::of(ui);
        let words = env.words::<PagePreviewWords>();
        let format = env.format();
        let page = pages[*at];
        let title = words.page_of.fill(&[("p", source.page_label(page).as_str()), ("n", format.integer(source.page_count() as u64).as_str())]);
        let step = Cell::new(0_isize);
        let shown = *at;
        let dialog = Dialog::new(title)
            .footer(|ui, _| {
                Row::new().show(ui, |ui, _| {
                    if clicked(&IconButton::new(IconId::new("chevron_left")).label(words.previous.clone()).enabled(shown > 0).show(ui)) {
                        step.set(step.get() - 1);
                    }
                    if clicked(&IconButton::new(IconId::new("chevron_right")).label(words.next.clone()).enabled(shown < last).show(ui)) {
                        step.set(step.get() + 1);
                    }
                });
            })
            .show(ui, |ui, _| {
                let picture = PagePicture::new(source, page, words.unavailable.as_str(), paper).label(source.page_label(page)).show(ui);
                step.set(step.get() + picture.step);
            });
        let target = (*at as isize + step.get()).clamp(0, last as isize) as usize;
        let stepped = target != *at;
        *at = target;
        PagePreviewOutput { response: dialog.output.response, stepped, closed: dialog.output.closed.is_some() }
    }
}

/// The style of the preview picture: the message of a page that failed to render.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PagePictureStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The longest side the picture asks for room, from the tokens' floating window height.
    ideal: f32,
    /// The failure message.
    message: StyleColor,
}

/// What the preview picture reports: the pages the keys asked to step, negative for back.
#[derive(Debug)]
struct PagePictureOutput {
    response: Response,
    step: isize,
}

impl Outcome for PagePictureOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The page picture of the preview: one Tab stop that turns the page with the arrow and Page keys.
#[derive(craft_ui::contract::Component)]
#[component(content_painter)]
struct PagePicture<'a> {
    #[common]
    common: Common,
    source: &'a dyn PageSource,
    page: usize,
    unavailable: &'a str,
    paper: DataColor,
    step: isize,
}

impl<'a> PagePicture<'a> {
    fn new(source: &'a dyn PageSource, page: usize, unavailable: &'a str, paper: DataColor) -> Self {
        PagePicture { common: Common::new(), source, page, unavailable, paper, step: 0 }
    }

    /// The part of the room the page fills, whole page and no larger than the ideal.
    fn fitted(&self, room: Vec2) -> Vec2 {
        let size = self.source.page_size(self.page);
        if size.width > 0.0 && size.height > 0.0 {
            let scale = (f64::from(room.x) / size.width).min(f64::from(room.y) / size.height) as f32;
            Vec2::new(size.width as f32 * scale, size.height as f32 * scale)
        } else {
            Vec2::splat(room.min_elem())
        }
    }
}

impl Component for PagePicture<'_> {
    const ROLE: Role = Role::IMAGE;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither).with_moves();
    type Style = PagePictureStyle;
    type Output = PagePictureOutput;

    fn style(cx: &StyleCx<'_>, _: PagePictureStyleKey) -> PagePictureStyle {
        PagePictureStyle { metrics: Metrics::new(), ideal: cx.tokens.size.float.height, message: cx.color(cx.tokens.color.text.muted) }
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let ideal = cx.style.ideal;
        let room = Vec2::new(cx.room.unwrap_or(f32::INFINITY), ideal);
        self.fitted(room)
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let side = if cx.flow.is_rtl() { -1 } else { 1 };
        while let Some((mv, _)) = cx.intents().take_move() {
            match mv {
                NavMove::Back => self.step -= side,
                NavMove::Forward => self.step += side,
                NavMove::PageUp => self.step -= 1,
                NavMove::PageDown => self.step += 1,
                _ => {}
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let painter = ui.content_painter(cx);
        let ctx = painter.painter.ctx().clone();
        let thumbs = Thumbnails::<usize>::in_context(
            &ctx,
            cx.key("pictures"),
            ThumbnailOptions::new().max_side(MAX_TEXTURE).store(TextureStoreOptions::pages()),
        );
        let area = cx.rect;
        let shape = self.fitted(area.size());
        let rect = Rect::from_center_size(area.center(), shape);
        let request = ThumbRequest::for_cell(shape.max_elem(), painter.pixels_per_point());
        let state = thumbs.state(&ctx, self.source, &self.page, &request, Priority::VISIBLE);
        // The paper shows under a page that has no picture yet; under a picture the thumbnail paints it.
        ui.draw().data_fill(&PaintShape::Rect(rect, CornerRadius::ZERO), self.paper);
        thumbs.paint(&painter, rect, &state, &ThumbPaint::default().paper(self.paper));
        if matches!(state, TextureState::Failed { .. }) {
            let run = TextRun::new(self.unavailable).role(TypeRole::Caption).color(cx.style.message);
            ui.draw().text(rect, &run);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> PagePictureOutput {
        PagePictureOutput { response: cx.response.clone(), step: self.step }
    }
}
