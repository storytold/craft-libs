//! `PagePreview`: a page shown as large as the window over a scrim, with previous and next, as PdfCraft's Combine
//! Files preview and VectorCraft's Place PDF page choice show it: the page as large as fits, "Page 3 of 12" under it,
//! the arrow and Page keys to step from anywhere in the preview, Escape or Close to leave.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::view::PageSource;
use craft_behavior::{Input, Metrics, PartKey, Proposal, Sense};
use craft_ui::contract::{
    Bind, CommandSet, Common, Component, Cx, DisabledLook, MeasureCx, NoValue, Outcome, PaintUi, PartsCx, Props as _, Role, RowAlign, Show as _,
    ShowContainer,
};
use craft_ui::dialog::Dialog;
use craft_ui::layout_components::Row;
use craft_ui::patterns::form_dialogs::{PagerStep, PanePager};
use craft_ui::primitives::textures::{
    MAX_TEXTURE, Priority, TextureState, TextureStoreOptions, TextureWords, ThumbPaint, ThumbRequest, ThumbnailOptions, Thumbnails,
};
use craft_ui::primitives::{PaintShape, TextRun};
use craft_ui::runtime::commands::Commands as _;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx, TypeRole};
use egui::{CornerRadius, Rect, Response, Ui, Vec2};
use std::cell::Cell;

craft_ui::words! {
    /// The preview's words (PdfCraft's strings).
    pub struct PagePreviewWords {
        close: "pagePreview.close" = "Close";
        unavailable: "pagePreview.unavailable" = "Preview unavailable";
    }
}

/// The keys of the page preview: Page Up and Page Down step back and forward, and the arrows step in reading order
/// (Left goes back in a left-to-right interface, forward in a right-to-left one). They belong to the preview, so they
/// work wherever the focus is in it; a widget that acts on its own moves keeps them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "pagePreview", scope = "pagePreview")]
#[non_exhaustive]
pub enum PagePreviewCommands {
    /// The previous page.
    #[command(modal = "in_dialogs", label = "Previous Page", default = "PageUp")]
    Previous,
    /// The next page.
    #[command(modal = "in_dialogs", label = "Next Page", default = "PageDown")]
    Next,
    /// The page to the left: the previous one in a left-to-right interface.
    #[command(modal = "in_dialogs", label = "Page to the Left", default = "Left")]
    Left,
    /// The page to the right: the next one in a left-to-right interface.
    #[command(modal = "in_dialogs", label = "Page to the Right", default = "Right")]
    Right,
}

/// What the preview did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct PagePreviewOutput {
    /// The preview's response.
    pub response: Response,
    /// The page shown changed (the app may sync its grid's selection).
    pub stepped: bool,
    /// Escape or Close.
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
/// - Over the whole window on the scrim, with dialog padding around it: the page's picture as large as the window
///   leaves, through `Thumbnails<usize>` over the [`PageSource`] (side from `ThumbRequest::for_cell`, at most
///   `MAX_TEXTURE`; the last picture stays as the stand-in until the new one lands), on `paper` while it loads. A page
///   that fails to render shows [`PagePreviewWords::unavailable`].
/// - Under the page, one centred row: previous, "Page {p} of {n}", next, Close. Previous and next are disabled at the
///   ends. The pages stepped through are `pages` (the pages a range selects), in order.
/// - A modal dialog named "Page {p} of {n}" that traps the focus: Close holds it when the preview opens, and it goes
///   back where it was when the preview closes.
/// - Keys ([`PagePreviewCommands`], wherever the focus is in the preview): Left and Page Up previous, Right and Page
///   Down next (the arrows reversed in a right-to-left interface), Escape closes.
#[derive(craft_ui::contract::Pattern)]
pub struct PagePreview<'a> {
    source: &'a dyn PageSource,
    pages: &'a [usize],
    at: &'a mut usize,
    paper: DataColor,
    title: Option<Label>,
}

impl<'a> PagePreview<'a> {
    /// Previews `pages` (0-based) on the document's `paper`, showing `pages[*at]`.
    pub fn new(source: &'a dyn PageSource, pages: &'a [usize], at: &'a mut usize, paper: DataColor) -> Self {
        PagePreview { source, pages, at, paper, title: None }
    }

    /// The name the frame's header shows (the file's), with the page's place after it.
    #[must_use]
    pub fn title(mut self, title: impl Into<Label>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Shows the preview over the window and reports page changes and close requests.
    pub fn show(self, ui: &mut Ui) -> PagePreviewOutput {
        let PagePreview { source, pages, at, paper, title } = self;
        let Some(last) = pages.len().checked_sub(1) else {
            return PagePreviewOutput { response: ui.response(), stepped: false, closed: true };
        };
        *at = (*at).min(last);
        let env = Env::of(ui);
        let words = env.words::<PagePreviewWords>();
        let page = pages[*at];
        // The pager counts the document's pages, as the caption names the page shown, though it steps through `pages`.
        let pager = PanePager {
            shown: page + 1,
            count: source.page_count(),
            shown_label: Some(Label::from(source.page_label(page))),
            back: *at > 0,
            next: *at < last,
            ..PanePager::default()
        };
        let caption = pager.caption(&env);
        let step = Cell::new(0_isize);
        let side = if env.direction.is_rtl() { -1 } else { 1 };
        let mut open = true;
        // The frame: a titled dialog (the file's name, with the page's place after it) and a close x in its header,
        // the page as large as the window leaves, and the arrows under it.
        let room = ui.ctx().content_rect().size();
        let room = Vec2::new((room.x * 0.8).max(120.0), (room.y * 0.72).max(90.0));
        let mut dialog = Dialog::new(title.clone().unwrap_or_else(|| caption.clone()))
            .opened(Bind::bound(&mut open))
            .command_scope(PagePreviewCommands::SCOPE)
            .footer(|ui, _| {
                let mut close = None;
                Row::new().fill_width().align(RowAlign::Center).show(ui, |ui, _| match pager.clone().show(ui) {
                    Some(PagerStep::Back) => close = Some(-1),
                    Some(PagerStep::Next) => close = Some(1),
                    None => {}
                });
                if let Some(by) = close {
                    step.set(step.get() + by);
                }
            });
        if title.is_some() {
            dialog = dialog.title_note(caption);
        }
        let shown = ShowContainer::show(dialog, ui, |ui, cx| {
            while let Some((command, _)) = cx.take_command::<PagePreviewCommands>() {
                let by = match command {
                    PagePreviewCommands::Previous => -1,
                    PagePreviewCommands::Next => 1,
                    PagePreviewCommands::Left => -side,
                    PagePreviewCommands::Right => side,
                };
                step.set(step.get() + by);
            }
            // design: allow[contract/allocate](the dialog sizes from its content: the page claims a box sized from the window, as large as it leaves)
            let (rect, _) = ui.allocate_exact_size(room, egui::Sense::hover());
            PagePicture::new(source, page, words.unavailable.as_str(), paper).label(source.page_label(page)).show_at(ui, rect);
        });
        let target = (*at as isize + step.get()).clamp(0, last as isize) as usize;
        let stepped = target != *at;
        *at = target;
        PagePreviewOutput { response: shown.output.response, stepped, closed: shown.output.closed.is_some() || !open }
    }
}

/// The style of the preview picture: the message of a page that failed to render.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PagePictureStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The failure message.
    message: StyleColor,
}

/// What the preview picture reports.
#[derive(Debug)]
struct PagePictureOutput {
    response: Response,
}

impl Outcome for PagePictureOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The page picture of the preview.
#[derive(craft_ui::contract::Component)]
#[component(content_painter)]
struct PagePicture<'a> {
    #[common]
    common: Common,
    source: &'a dyn PageSource,
    page: usize,
    unavailable: &'a str,
    paper: DataColor,
    /// The page's picture as step 7 asked for it, which `paint` shows.
    state: Option<TextureState>,
}

/// The page's picture; a page that failed to render gives the reason as its tooltip.
const PICTURE: PartKey = PartKey::new("picture");

impl<'a> PagePicture<'a> {
    fn new(source: &'a dyn PageSource, page: usize, unavailable: &'a str, paper: DataColor) -> Self {
        PagePicture { common: Common::new(), source, page, unavailable, paper, state: None }
    }

    /// The page pictures of the component `id`.
    fn thumbnails(ctx: &egui::Context, id: egui::Id) -> Thumbnails<usize> {
        Thumbnails::<usize>::in_context(ctx, id.with("pictures"), ThumbnailOptions::new().max_side(MAX_TEXTURE).store(TextureStoreOptions::pages()))
    }

    /// The part of the room the whole page fills.
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
    const DISABLED: DisabledLook = DisabledLook::Faded;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer);
    type Style = PagePictureStyle;
    type Output = PagePictureOutput;

    fn style(cx: &StyleCx<'_>, _: PagePictureStyleKey) -> PagePictureStyle {
        PagePictureStyle { metrics: Metrics::new(), message: cx.color(cx.tokens.color.text.muted) }
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    /// The page fills the room it is shown at; measured in a flow, it takes the room's width as its height too.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        match cx.proposal {
            Proposal::Size(room) => self.fitted(room),
            _ => cx.room.map_or(Vec2::ZERO, |width| self.fitted(Vec2::splat(width))),
        }
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let ctx = cx.ui.ctx().clone();
        let thumbs = Self::thumbnails(&ctx, cx.id);
        let shape = self.fitted(rect.size());
        let request = ThumbRequest::for_cell(shape.max_elem(), ctx.pixels_per_point());
        let state = thumbs.state(&ctx, self.source, &self.page, &request, Priority::VISIBLE);
        if let TextureState::Failed { failure, .. } = &state {
            let reason = failure.text(&cx.env.words::<TextureWords>()).to_owned();
            cx.part(PICTURE, Rect::from_center_size(rect.center(), shape)).tooltip(reason);
        }
        self.state = Some(state);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(state) = self.state.take() else { return };
        let painter = ui.content_painter(cx);
        let ctx = painter.painter.ctx().clone();
        let thumbs = Self::thumbnails(&ctx, cx.id);
        let area = cx.rect;
        let rect = Rect::from_center_size(area.center(), self.fitted(area.size()));
        // The paper shows under a page that has no picture yet; under a picture the thumbnail paints it.
        ui.draw().data_fill(&PaintShape::Rect(rect, CornerRadius::ZERO), self.paper);
        thumbs.paint(&painter, rect, &state, &ThumbPaint::default().paper(self.paper));
        if matches!(state, TextureState::Failed { .. }) {
            let run = TextRun::new(self.unavailable).role(TypeRole::Caption).color(cx.style.message);
            ui.draw().text(rect, &run);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> PagePictureOutput {
        PagePictureOutput { response: cx.response.clone() }
    }
}
