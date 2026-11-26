//! The leaves of the page view: `PageMetrics`, which carries the view's spacing from the style, and `PagePictures`,
//! which paints the page pictures and their detail regions over the page frames.

use super::{PageSource, PlacedPage};
use crate::layout::PageSpacing;
use crate::ui::access::{AccessDescriber, describe};
use craft_behavior::{Input, Metrics, PixelSize, Sense};
use craft_theme::ComponentSet;
use craft_ui::canvas::DetailPlan;
use craft_ui::canvas::PixelRect;
use craft_ui::contract::{Access, Common, Component, Cx, DisabledLook, MeasureCx, NoValue, Outcome, PaintUi, Role};
use craft_ui::primitives::textures::{
    Priority, StateLook, TextureState, TextureStoreOptions, ThumbPaint, ThumbRequest, ThumbnailOptions, Thumbnails, texture_side,
};
use craft_ui::style::{ComponentStyle, DataColor, StyleCx};
use egui::{Context, Id, Rect, Response, Vec2, pos2};

/// The options of the page view's picture stores: textures up to the context's side, kept as pages are.
pub(super) fn picture_options(ctx: &Context) -> ThumbnailOptions {
    ThumbnailOptions::new().max_side(texture_side(ctx)).store(TextureStoreOptions::pages())
}

/// The page view's two picture stores, the one place their keys are made: the page store, of each page's own
/// picture, and the detail store, of the part in view of a page larger than the texture side (a store of its own, so
/// a page's two regions never share a key). `owner` is the page view's body; the pass asks the page store for the
/// pages' states and `PagePictures` paints from both.
pub(super) fn page_stores(ctx: &Context, owner: Id, options: ThumbnailOptions) -> (Thumbnails<usize>, Thumbnails<usize>) {
    (Thumbnails::in_context(ctx, owner.with("pictures"), options.clone()), Thumbnails::in_context(ctx, owner.with("detail"), options))
}

/// Productivity inter-page gap as a multiple of `space.gap.section`: 18 pt in Compact (Word's gap) and 24 pt in
/// Comfortable.
const PRODUCTIVITY_GAP_RATIO: f32 = 1.5;

/// The style of the page view's spacing: the gap between pages, the margin, the side column, Hide White Space's pad
/// and its seam, from the tokens and the scope's [`ComponentSet`].
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PageMetricsStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// Between pages, and between rows.
    gap: f32,
    /// Above the first and below the last row.
    margin: f32,
    /// Kept clear at each side.
    side: f32,
    /// What Hide White Space leaves above and below a page's text area, at 100 % zoom.
    white_space_pad: f32,
    /// The seam Hide White Space draws between pages.
    seam: f32,
}

/// What the metrics report: the spacing the layout uses.
#[derive(Debug)]
pub(super) struct PageMetricsOutput {
    response: Response,
    pub(super) spacing: PageSpacing,
}

impl Outcome for PageMetricsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A leaf of no size that gives the page view its spacing from the resolved style, so the layout never reads tokens.
#[derive(craft_ui::contract::Component)]
pub(super) struct PageMetrics {
    #[common]
    common: Common,
}

impl PageMetrics {
    pub(super) fn new() -> Self {
        PageMetrics { common: Common::new() }
    }
}

impl Component for PageMetrics {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: Input = Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = PageMetricsStyle;
    type Output = PageMetricsOutput;

    fn style(cx: &StyleCx<'_>, _: PageMetricsStyleKey) -> PageMetricsStyle {
        let tokens = cx.tokens;
        match cx.set() {
            // With `PageInsets::Theme` (PdfCraft's page view) the creative set's spacing is the theme's page insets.
            ComponentSet::Creative if cx.env::<super::PageInsets>() == super::PageInsets::Theme => {
                let inset = tokens.space.inset;
                PageMetricsStyle {
                    metrics: Metrics::new(),
                    gap: inset.page_gap,
                    margin: inset.page_margin,
                    side: inset.page_side,
                    white_space_pad: tokens.space.gap.inline,
                    seam: tokens.stroke.thin,
                }
            }
            ComponentSet::Creative => PageMetricsStyle {
                metrics: Metrics::new(),
                gap: tokens.space.gap.section,
                margin: tokens.space.pad.roomy,
                side: tokens.size.rail.width,
                white_space_pad: tokens.space.gap.inline,
                seam: tokens.stroke.thin,
            },
            ComponentSet::Productivity => {
                let gap = tokens.space.gap.section * PRODUCTIVITY_GAP_RATIO;
                PageMetricsStyle {
                    metrics: Metrics::new(),
                    gap,
                    margin: gap,
                    side: gap,
                    white_space_pad: tokens.space.gap.stack,
                    seam: tokens.stroke.thin,
                }
            }
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> PageMetricsOutput {
        let style = &cx.style;
        let spacing = PageSpacing {
            gap: f64::from(style.gap),
            margin: f64::from(style.margin),
            side: f64::from(style.side),
            white_space_pad: f64::from(style.white_space_pad),
            seam: f64::from(style.seam),
        };
        PageMetricsOutput { response: cx.response.clone(), spacing }
    }
}

/// The style of the page pictures: they paint the pictures the source renders and nothing of their own.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PagePicturesStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The pictures of the pages on screen, over their frames: each page's own picture, in the state the pass found it
/// (`PlacedPage::picture`, captioned while it renders or when it failed), and, for a page larger than the texture
/// side, a second picture of the part in view. Paint only; the describer adds the text under the pages to the
/// accessibility tree.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control, content_painter)]
pub(super) struct PagePictures<'a> {
    #[common]
    common: Common,
    source: &'a dyn PageSource,
    pages: &'a [PlacedPage],
    /// The page view's body, whose picture stores these are (`page_stores`).
    owner: Id,
    /// The document's paper under each picture.
    paper: DataColor,
    /// Sharp renders of the part in view wait: the view zooms or pans.
    // The page store's hold is set where its requests are, in the view's pass, so only the detail hold is a prop.
    #[prop]
    hold_detail: bool,
    /// The description of the text under the pages.
    #[prop]
    describer: Option<&'a dyn AccessDescriber>,
}

impl<'a> PagePictures<'a> {
    pub(super) fn new(source: &'a dyn PageSource, pages: &'a [PlacedPage], owner: Id, paper: DataColor) -> Self {
        PagePictures { common: Common::new(), source, pages, owner, paper, hold_detail: false, describer: None }
    }
}

impl Component for PagePictures<'_> {
    const ROLE: Role = Role::GROUP;
    const DISABLED: DisabledLook = DisabledLook::Faded;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: Input = Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = PagePicturesStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PagePicturesStyleKey) -> PagePicturesStyle {
        PagePicturesStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let mut painter = ui.content_painter(cx);
        let ctx = painter.painter.ctx().clone();
        let base = painter.painter.clone();
        let inside = base.clip_rect();
        let side = texture_side(&ctx);
        let (thumbs, details) = page_stores(&ctx, self.owner, picture_options(&ctx));
        details.hold(self.hold_detail);
        let ppp = painter.pixels_per_point();
        let paint = ThumbPaint::default().paper(self.paper).states(StateLook::Captioned);
        for placed in self.pages {
            let (top, bottom) = placed.band;
            let band = Rect::from_min_max(pos2(placed.rect.min.x, placed.rect.min.y + top), pos2(placed.rect.max.x, placed.rect.min.y + bottom))
                .intersect(inside);
            painter.painter = base.with_clip_rect(band);
            thumbs.paint(&painter, placed.rect, &placed.picture, &paint);
            if let Some(detail) = Detail::of(placed.rect, band, side, ppp) {
                let state = details.state(&ctx, self.source, &placed.page, &detail.request, Priority::VISIBLE);
                // Only a picture of this region: another region's stand-in would sit in the wrong place.
                if matches!(state, TextureState::Ready(_)) {
                    details.paint(&painter, detail.rect, &state, &paint);
                }
            }
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        // The text nodes hang under this component's node; without a describer it stands for none.
        match self.describer {
            Some(describer) => describe(access, self.pages, describer),
            None => {
                access.role(Role::None);
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The picture of the part of a page in view, for a page larger than the texture side.
struct Detail {
    request: ThumbRequest,
    /// Where the part goes on screen.
    rect: Rect,
}

impl Detail {
    /// The detail for `page` (its screen rectangle) of which `shown` is in view, with textures of at most `side`
    /// pixels: [`DetailPlan`] over the page's size in device pixels, `None` when the page's own picture is as fine as
    /// the screen.
    fn of(page: Rect, shown: Rect, side: u32, ppp: f32) -> Option<Detail> {
        let seen = page.intersect(shown);
        if !(seen.width() > 0.0 && seen.height() > 0.0 && ppp > 0.0) {
            return None;
        }
        let size = PixelSize::new((page.width() * ppp).round() as u32, (page.height() * ppp).round() as u32);
        let px = |v: f32, from: f32, up: bool| {
            let n = (v - from) * ppp;
            (if up { n.ceil() } else { n.floor() }) as i32
        };
        let visible = PixelRect::new(
            px(seen.min.x, page.min.x, false),
            px(seen.min.y, page.min.y, false),
            px(seen.max.x, page.min.x, true),
            px(seen.max.y, page.min.y, true),
        );
        // One page pixel is one device pixel, so the plan's zoom is 1.
        let plan = DetailPlan::of(PixelRect::of_size(size), side, 1.0, visible)?;
        let r = plan.region;
        let (w, h) = (size.width as f32, size.height as f32);
        let part = Rect::from_min_max(pos2(r.x0 as f32 / w, r.y0 as f32 / h), pos2(r.x1 as f32 / w, r.y1 as f32 / h));
        let longest = r.width().max(r.height()).unsigned_abs().div_ceil(plan.factor);
        let request = ThumbRequest::for_cell(longest as f32 / ppp, ppp).region(part);
        let at = |f: egui::Pos2| pos2(page.min.x + f.x * page.width(), page.min.y + f.y * page.height());
        Some(Detail { request, rect: Rect::from_min_max(at(part.min), at(part.max)) })
    }
}
