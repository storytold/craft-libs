//! `CompareTile`: one picture in a `PreviewPane` over a view the panes share, with its caption strip, the active ring
//! and the rejected veil.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::CanvasTransform;
use craft_ui::contract::{
    Bind, Common, Component, Container, ContainerProps, Cx, FooterRegion, Held, NoValue, Outcome, PaintUi, Props, Remember as _, Role, Show as _,
    ShowContainer as _, Sizing, clicked,
};
use craft_ui::patterns::form_dialogs::{PaneOverlay, PreviewPane, PreviewState, PreviewView};
use craft_ui::primitives::textures::{CachedTexture, Revision, TextureStore, TextureStoreOptions};
use craft_ui::primitives::{ContentPainterCx, PaintShape, StrokeWidth};
use craft_ui::runtime::access::Access;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::StyleCx;
use egui::load::SizedTexture;
use egui::{ColorImage, CornerRadius, Rect, Response, Ui, Vec2, vec2};

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
    /// A render is running; `overdue` once it has taken longer than the host's wait, which replaces the picture with
    /// the pane's pending line. Before that the last picture stays.
    /// True once the render has taken longer than the host's wait.
    Pending(bool),
    /// The render failed: the pane shows the message.
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

/// The pane's well in the last pass, for the marks painted over it.
#[derive(Clone, Default)]
struct Well(Option<Rect>);

/// One compare pane: a picture fitted into its well, zoomed and panned through a [`PreviewView`] the panes share,
/// under its caption strip, with the rejected veil and the active ring.
///
/// - **Picture.** The well is a `PreviewPane`: drag and the arrow keys pan, the wheel and pinch zoom. A double click
///   toggles Fit and 1:1 ([`CompareTile::toggle_zoom`]). `.survey(true)` shows the picture always fitted and adopts no
///   view change.
/// - **Render state.** `Pending { overdue: true }` and `Failed` replace the picture with the pane's pending line and
///   failed message; before the host's wait a pending render keeps the last picture.
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
}

impl<'a> CompareTile<'a> {
    /// A tile named `label` showing `image` through the shared `view`.
    pub fn new(label: impl Into<Label>, view: Bind<'a, PreviewView>, image: CompareImage<'a>) -> Self {
        let label = label.into();
        CompareTile {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill).footer(FooterRegion::new()),
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
        // The body runs in the measuring pass and again in the real one: it borrows what it shows and consumes
        // nothing. The overlay is called at most once, by the pane's paint, which only the real run reaches.
        let view = RefCell::new(self.view.take().map(Held::new));
        let overlay = self.overlay.take().map(|overlay| Rc::new(RefCell::new(Some(overlay.into_paint()))));
        let label = self.label.clone();
        let state = self.state.clone();
        let survey = self.survey;
        let shown = self.label(name).show_at(ui, rect, |ui, cx| {
            let mut view = view.borrow_mut();
            let view = view.as_mut()?;
            let held = if survey { PreviewView::fixed() } else { view.read(cx).unwrap_or_else(PreviewView::fitting) };
            let preview = match (&state, picture) {
                (RenderState::Failed(message), _) => PreviewState::Failed(Label::from(message.clone())),
                (RenderState::Pending(true), _) | (_, None) => PreviewState::Pending(None),
                (_, Some(texture)) => PreviewState::Ready(texture),
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
            let out = pane.show(ui);
            cx.memory::<Well>().0 = Some(out.response.rect);
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
        let Some(well) = cx.recall::<Well>().0 else { return };
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

impl Container for CompareTile<'_> {
    /// The caption strip under the picture.
    fn footer(&mut self, ui: &mut Ui, cx: &mut craft_ui::contract::BodyCx<'_>) {
        if let Some(caption) = &self.caption {
            caption.strip(ui, cx.rect.width());
        }
    }
}
