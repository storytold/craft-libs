//! `CompareTile`: one picture in a `PreviewPane` over a view the panes share, with its caption strip, the active ring
//! and the rejected veil.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::{CanvasTransform, Proposal};
use craft_ui::contract::{
    Bind, Common, Component, Container, ContainerProps, Cx, Held, NoValue, Outcome, PaintUi, Props, Role, Show as _, ShowContainer as _, Sizing,
    clicked,
};
use craft_ui::patterns::form_dialogs::{PaneOverlay, PreviewPane, PreviewState, PreviewView};
use craft_ui::primitives::textures::{CachedTexture, Revision, TextureStore, TextureStoreOptions};
use craft_ui::primitives::{ContentPainterCx, PaintShape, StrokeWidth};
use craft_ui::runtime::access::Access;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::StyleCx;
use egui::load::SizedTexture;
use egui::{ColorImage, CornerRadius, Rect, Response, Ui, Vec2, vec2};

use super::pill::RenderPill;
use super::{CompareCaption, CompareTileStyle, CompareWords};

/// The picture a compare tile shows.
#[non_exhaustive]
pub enum CompareImage<'a> {
    /// A texture from the app's own `TextureStore`, with the image's full size in pixels (for Fit and 1:1).
    Texture(CachedTexture, [u32; 2]),
    /// A picture the tile uploads into its own `TextureStore` (keyed by `key` and `revision`; `make` runs once per
    /// revision).
    Picture(u64, Revision, [u32; 2], Box<dyn FnOnce() -> ColorImage + 'a>),
    /// Nothing yet: the pane's placeholder.
    None,
}

/// The host's render state for the tile.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderState {
    /// The picture is current.
    Ready,
    /// A render is running: true once it has taken longer than the host's wait, which shows the "Rendering…" pill over
    /// the picture.
    Pending(bool),
    /// The render failed: the "Render failed" pill over the picture, with the message as its tooltip.
    Failed(String),
}

/// What [`CompareTile::place`] returns.
#[derive(Debug)]
#[non_exhaustive]
pub struct CompareOutput {
    /// The tile.
    pub response: Response,
    /// A click: the host makes the pane active.
    pub clicked: bool,
    /// A double click: the tile has toggled Fit and 1:1 in `view`.
    pub double_clicked: bool,
    /// The view after a pan, a zoom or a double click of this pass; `None` when it did not change. The tile also
    /// writes it to the binding it was given.
    pub view: Option<PreviewView>,
}

impl Outcome for CompareOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What the body hands back.
#[derive(Default)]
struct Shown {
    clicked: bool,
    double_clicked: bool,
    view: Option<PreviewView>,
}

/// One compare pane: a picture fitted into its well, zoomed and panned through a [`PreviewView`] the panes share,
/// with its caption strip under it, the rejected veil and the active ring.
///
/// - **Layout.** The well has the picture's aspect, as large as the tile's rect allows with the caption strip under
///   it; well and strip are centred in the rect together, so the ring and the caption follow the picture. Without a
///   picture the well takes the whole room.
/// - **Picture.** The well is a `PreviewPane`: drag and the arrow keys pan, the wheel and pinch zoom. A double click
///   toggles Fit and 1:1 ([`CompareTile::toggle_zoom`]). `.survey(true)` shows the picture always fitted and adopts no
///   view change.
/// - **Render state.** The picture stays. An overdue render shows a "Rendering…" pill (scrim, spinner) and a failed
///   one a "Render failed" pill (`status.bad.solid`) with the message as its tooltip, centred at the top of the visible
///   picture. Before the host's wait a pending render shows nothing more.
/// - **Marks.** The rejected veil (`surface.scrim`) and the active ring (`accent.solid`, with its halo) are painted
///   over the well from the tile's style.
/// - **Access.** An image named by the caption read as text, with the pending or failed text as its description.
#[must_use = "show the tile with `show`"]
#[derive(craft_ui::contract::Component, Props)]
#[component(container)]
pub struct CompareTile<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    label: Label,
    view: Option<Bind<'a, PreviewView>>,
    image: Option<CompareImage<'a>>,
    /// The caption strip.
    #[prop]
    caption: Option<CompareCaption>,
    /// The render state (default `Ready`).
    #[prop]
    state: RenderState,
    /// The active pane: the ring.
    #[prop]
    active: bool,
    /// The rejected veil.
    #[prop]
    rejected: bool,
    /// Survey: fitted, no zoom or pan.
    #[prop]
    survey: bool,
    /// The host's drawing over the picture (a window render): the content painter, the transform the picture is
    /// drawn with and the picture's rect.
    #[prop(func = "FnOnce(&ContentPainterCx, CanvasTransform, Rect)")]
    overlay: Option<PaneOverlay<'a>>,
    description: Option<String>,
    /// The well `place` laid out, for the marks painted over it.
    well: Option<Rect>,
}

impl<'a> CompareTile<'a> {
    /// A tile named `label` showing `image` through the shared `view`.
    pub fn new(label: impl Into<Label>, view: Bind<'a, PreviewView>, image: CompareImage<'a>) -> Self {
        let label = label.into();
        CompareTile {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill),
            label,
            view: Some(view),
            image: Some(image),
            caption: None,
            state: RenderState::Ready,
            active: false,
            rejected: false,
            survey: false,
            overlay: None,
            description: None,
            well: None,
        }
    }

    /// The 1:1 view: one device pixel per image pixel, about the well's centre.
    pub const ONE_TO_ONE: CanvasTransform = CanvasTransform::new(1.0, Vec2::ZERO);

    /// `view` toggled between Fit and 1:1 (one device pixel per image pixel, about the well's centre): from Fit to
    /// 1:1, from anything else to Fit.
    pub fn toggle_zoom(view: &PreviewView) -> PreviewView {
        if view.fit {
            PreviewView::new(Self::ONE_TO_ONE).zoom_range(view.range.clone())
        } else {
            PreviewView::fitting().zoom_range(view.range.clone())
        }
    }

    /// Shows the tile at `rect`.
    pub fn place(mut self, ui: &mut Ui, rect: Rect) -> CompareOutput {
        let env = Env::of(ui);
        let words = env.words::<CompareWords>();
        let format = env.view().format();
        let name = match &self.caption {
            Some(caption) => caption.accessible_name(&words, format),
            None => self.label.as_str().to_owned(),
        };
        self.description = match &self.state {
            RenderState::Pending(true) => Some(words.rendering.as_str().to_owned()),
            RenderState::Failed(message) if message.is_empty() => Some(words.render_failed.as_str().to_owned()),
            RenderState::Failed(message) => Some(message.clone()),
            RenderState::Ready | RenderState::Pending(false) => None,
        };
        let ppp = ui.pixels_per_point();
        let picture = match self.image.take() {
            Some(CompareImage::Texture(texture, size)) => Some(SizedTexture::new(texture.id, points(size, ppp))),
            Some(CompareImage::Picture(key, revision, size, make)) => {
                let store = TextureStore::<u64>::in_context(ui.ctx(), ("compare-textures", self.label.as_str()), TextureStoreOptions::frames());
                let texture = store.get_or_put(ui.ctx(), key, revision, make);
                Some(SizedTexture::new(texture.id, points(size, ppp)))
            }
            Some(CompareImage::None) | None => None,
        };
        let tokens = &env.tokens;
        let caption = self.caption.clone();
        let strip_height = caption.as_ref().map_or(0.0, |caption| caption.height(ui));
        let (well, strip) = Self::layout(
            rect,
            picture.map(|texture| texture.size),
            strip_height,
            tokens.space.gap.row,
            tokens.stroke.thick,
            env.view().direction().is_rtl(),
        );
        self.well = Some(well);
        // The body runs in the measuring pass and again in the real one: it borrows what it shows and consumes
        // nothing. The overlay is called at most once, by the pane's paint, which only the real run reaches.
        let view = RefCell::new(self.view.take().map(Held::new));
        let overlay = self.overlay.take().map(|overlay| Rc::new(RefCell::new(Some(overlay.into_paint()))));
        let label = self.label.clone();
        let state = self.state.clone();
        let survey = self.survey;
        let (rendering, render_failed) = (words.rendering.clone(), words.render_failed.clone());
        let shown = self.label(name).show_at(ui, rect, |ui, cx| {
            let mut view = view.borrow_mut();
            let view = view.as_mut()?;
            let held = if survey { PreviewView::fixed() } else { view.read(cx).unwrap_or_else(PreviewView::fitting) };
            let preview = match (&state, picture) {
                (_, Some(texture)) => PreviewState::Ready(texture),
                (RenderState::Failed(_), None) => PreviewState::Failed(Label::Empty),
                (_, None) => PreviewState::Pending(None),
            };
            let mut pane = PreviewPane::state(label.clone(), preview, &held);
            if let Some(overlay) = &overlay {
                let overlay = Rc::clone(overlay);
                pane = pane.overlay(move |painter: &ContentPainterCx, view, picture| {
                    if let Some(paint) = overlay.borrow_mut().take() {
                        paint(painter, view, picture);
                    }
                });
            }
            let out = pane.show_at(ui, well);
            if let Some(caption) = &caption {
                caption.strip(ui, strip);
            }
            let pill = match &state {
                RenderState::Pending(true) => Some(RenderPill::pending(rendering.clone())),
                RenderState::Failed(message) => Some(RenderPill::failed(render_failed.clone(), Some(message.clone()).filter(|m| !m.is_empty()))),
                RenderState::Ready | RenderState::Pending(false) => None,
            };
            if let Some(mut pill) = pill {
                let transform = out.shown.transform;
                let drawn = picture.map_or(well, |texture| Rect::from_center_size(well.center() + transform.offset, texture.size * transform.scale));
                let visible = drawn.intersect(well);
                let visible = if visible.is_positive() { visible } else { well };
                let size = pill.measure_in(ui, Proposal::Ideal);
                let top = visible.top() + Env::of(ui).tokens.space.gap.row;
                pill.show_at(ui, Rect::from_min_size(egui::pos2(visible.center().x - size.x / 2.0, top), size));
            }
            let double_clicked = !survey && out.double_clicked;
            let next = if survey {
                None
            } else if double_clicked {
                Some(Self::toggle_zoom(&out.shown))
            } else {
                out.view.clone()
            };
            if let Some(next) = &next {
                view.write(cx, &held, next.clone(), "Zoom");
            }
            Some(Shown { clicked: clicked(&out.response), double_clicked, view: next })
        });
        let Shown { clicked, double_clicked, view } = shown.inner.flatten().unwrap_or_default();
        CompareOutput { response: shown.output, clicked, double_clicked, view }
    }
}

impl CompareTile<'_> {
    /// The well and the caption strip in `rect`, for a picture of `picture` points, a strip `strip` high and `gap`
    /// under the well, and a ring `ring` wide round the well. The strip runs from the well's reading start to the
    /// rect's end, as wide as the caption may need.
    fn layout(rect: Rect, picture: Option<Vec2>, strip: f32, gap: f32, ring: f32, rtl: bool) -> (Rect, Rect) {
        let inner = rect.shrink(ring);
        let under = if strip > 0.0 { gap + strip } else { 0.0 };
        let room = vec2(inner.width(), (inner.height() - under).max(0.0));
        let size = match picture.filter(|picture| picture.x > 0.0 && picture.y > 0.0) {
            Some(picture) => picture * (room.x / picture.x).min(room.y / picture.y),
            None => room,
        };
        let top = inner.top() + (inner.height() - size.y - under) / 2.0;
        let well = Rect::from_min_size(egui::pos2(inner.center().x - size.x / 2.0, top), size);
        let (start, end) = if rtl { (inner.left(), well.right()) } else { (well.left(), inner.right()) };
        let strip = Rect::from_min_max(egui::pos2(start, well.bottom() + gap), egui::pos2(end, well.bottom() + under));
        (well, strip)
    }
}

/// An image of `size` pixels in points: at 1.0 zoom one device pixel per image pixel.
fn points(size: [u32; 2], pixels_per_point: f32) -> Vec2 {
    vec2(size[0].max(1) as f32, size[1].max(1) as f32) / pixels_per_point
}

impl Component for CompareTile<'_> {
    const ROLE: Role = Role::IMAGE;
    type Style = CompareTileStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: CompareTileStyleKey) -> CompareTileStyle {
        Self::tile_style(cx)
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        if let Some(description) = &self.description {
            access.description(description.clone());
        }
    }

    /// The rejected veil and the active ring over the well.
    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(well) = self.well else { return };
        let style = cx.style;
        let draw = ui.draw();
        if self.rejected {
            draw.fill(&PaintShape::Rect(well, CornerRadius::ZERO), style.veil);
        }
        if self.active {
            draw.stroke(&PaintShape::Rect(well, CornerRadius::ZERO), StrokeWidth::Thick, style.halo);
            draw.stroke(&PaintShape::Rect(well, CornerRadius::ZERO), StrokeWidth::Medium, style.ring);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for CompareTile<'_> {}
