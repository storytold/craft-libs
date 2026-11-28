//! The clipping warning buttons ([`ClipWarning`]) and the [`ClippingOverlay`] over a preview: one nearest-sampled
//! texture the size of the mask.

use craft_behavior::kurbo::Rect as DocRect;
use craft_behavior::{Metrics, Sense};
use craft_ui::canvas::canvas_space;
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Input, MeasureCx, PaintUi, Pointer as _, Props as _, Role,
};
use craft_ui::primitives::{PaintShape, Revision, StrokeWidth, TextureStore, TextureStoreOptions};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, DataColor, FocusLook, StyleColor, StyleCx};
use egui::epaint::Vertex;
use egui::{Color32, ColorImage, Mesh, Response, Shape, TextureOptions, Vec2, pos2};

use crate::scopes::{ChannelMask, ClippingMask};

/// The share of a warning button's side that its triangle is inset from each edge.
const INSET_SHARE: f32 = 1.0 / 6.0;

/// The clipped channels the warning buttons show: their fill is these channels' data colour
/// (`data.red`, `data.green`, `data.blue` mixed by the mask), black when none clip.
/// The host folds its histogram's end bins or [`ClippingMask::any`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ClippingFrame {
    /// Channels clipped at 0 anywhere.
    pub shadows: ChannelMask,
    /// Channels clipped at 1 anywhere.
    pub highlights: ChannelMask,
}

/// The clipping warnings that are on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ClippingWarnings {
    /// The shadow warning (U): shadow-clipped pixels in `data.blue`.
    pub shadows: bool,
    /// The highlight warning (O): highlight-clipped pixels in `data.red`.
    pub highlights: bool,
}

/// An Alt-drag on a tone control that shows the per-channel diagnostics instead of the warnings;
/// the host knows which control is dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ClippingDiagnostic {
    /// Blacks or Shadows: each pixel in the colour of the channels that do not clip at 0.
    Shadows,
    /// Highlights, Whites or Exposure: each pixel in the colour of the channels that clip at 1.
    Highlights,
}

/// What the clipping overlay draws ([`ScopeSettings::clipping_mode`](super::ScopeSettings::clipping_mode)).
/// Warnings and diagnostics are separate modes, so both warnings never read as a diagnostic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ClippingMode {
    /// Nothing.
    #[default]
    None,
    /// The warnings that are on: highlight-clipped pixels in `data.red` (they win), else
    /// shadow-clipped ones in `data.blue`, else transparent.
    Warnings(ClippingWarnings),
    /// Shadow diagnostics: each pixel in the data colour of the channels *not* clipped at 0
    /// (`RGB ^ shadows`).
    ShadowChannels,
    /// Highlight diagnostics: each pixel in the data colour of its highlight-clipped channels,
    /// `data.scope` where none clip.
    HighlightChannels,
}

/// The data colours of the channels: `data.red`, `data.green`, `data.blue` and `data.scope`, as the style hooks read
/// them from the tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ChannelInks([[u8; 4]; 4]);

impl ClippingFrame {
    /// The clipped channels: `shadows` at 0, `highlights` at 1.
    pub const fn new(shadows: ChannelMask, highlights: ChannelMask) -> Self {
        ClippingFrame { shadows, highlights }
    }
}

impl ChannelInks {
    fn new(cx: &StyleCx<'_>) -> Self {
        let data = &cx.tokens.color.data;
        ChannelInks([data.red.0, data.green.0, data.blue.0, data.scope.0])
    }

    /// The colour of the channels in `mask`: `data.red`, `data.green` and `data.blue` added (saturating), `data.scope`
    /// for none (PhotoCraft's `histogram_color`).
    fn color(&self, mask: ChannelMask) -> Color32 {
        if mask.is_empty() {
            let [r, g, b, a] = self.0[3];
            return Color32::from_rgba_unmultiplied(r, g, b, a);
        }
        let mut sum = [0u8; 3];
        for (bit, [r, g, b, _]) in [ChannelMask::R, ChannelMask::G, ChannelMask::B].into_iter().zip(self.0) {
            if mask.contains(bit) {
                for (total, part) in sum.iter_mut().zip([r, g, b]) {
                    *total = total.saturating_add(part);
                }
            }
        }
        Color32::from_rgb(sum[0], sum[1], sum[2])
    }
}

/// The style of a clipping warning button.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ClipWarningStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    inks: ChannelInks,
    /// The triangle's outline while the warning is on.
    on: StyleColor,
    /// The triangle's outline while it is off.
    off: StyleColor,
    side: f32,
}

/// One clipping warning button: a triangle filled with the data colour of the clipped channels (`data.scope` when none
/// clip), outlined in `text.normal` while on and `text.faint` while off; a click, Space or Enter toggles it. The
/// panel's `ScopeCommand::ShadowWarning` and `HighlightWarning` keys toggle the same settings. AccessKit: a check box
/// named by the label ("Shadow Clipping Warning (U)"); its tooltip is the label. Plots never mirror, so it does not.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct ClipWarning<'a> {
    #[common]
    common: Common,
    value: Bind<'a, bool>,
    clipped: ChannelMask,
}

impl<'a> ClipWarning<'a> {
    /// A warning button named `label`, on while `value` is true, filled with the colour of the `clipped` channels.
    pub fn new(label: impl Into<Label>, value: Bind<'a, bool>, clipped: ChannelMask) -> Self {
        ClipWarning { common: Common::new(), value, clipped }.label(label)
    }
}

impl Component for ClipWarning<'_> {
    const ROLE: Role = Role::CHECK_BOX;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = ClipWarningStyle;
    type Output = EditResponse<bool>;

    fn style(cx: &StyleCx<'_>, _: ClipWarningStyleKey) -> ClipWarningStyle {
        let colors = &cx.tokens.color;
        let side = cx.control().icon;
        ClipWarningStyle {
            metrics: Metrics::new().min_size(Vec2::splat(side)),
            focus: cx.focus_look(),
            inks: ChannelInks::new(cx),
            on: cx.color(colors.text.normal),
            off: cx.color(colors.text.faint),
            side,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.side)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn bind(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if cx.activation().is_some() {
            let on = cx.value().copied().unwrap_or(false);
            cx.set(!on);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let inset = cx.style.side * INSET_SHARE;
        let triangle = cx.rect.shrink(inset);
        let shape = PaintShape::Polygon(vec![pos2(triangle.center().x, triangle.top()), triangle.left_bottom(), triangle.right_bottom()]);
        let on = cx.value().copied().unwrap_or(false);
        let draw = ui.draw();
        draw.data_fill(&shape, DataColor::new(cx.style.inks.color(self.clipped)));
        draw.stroke(&shape, StrokeWidth::Thin, if on { cx.style.on } else { cx.style.off });
    }

    fn access(&self, _: &mut Access, _: &Cx<'_, Self>) {}

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<bool> {
        cx.edit_response()
    }
}

/// The style of the clipping overlay: the data colours it paints with.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ClippingOverlayStyle {
    #[style(metrics)]
    metrics: Metrics,
    inks: ChannelInks,
}

/// The clipping overlay over a preview, a canvas control: one texture of the mask's size, nearest-sampled, drawn over
/// the document through the canvas's view. The texture is rebuilt only when `revision`, the mode or the theme's data
/// colours change. Transparent and non-finite pixels are transparent in every mode. Decoration: no pointer, no key,
/// no node of its own.
///
/// ```ignore
/// ClippingOverlay::new("Clipping", &mask, revision, mode).show_at(ui, canvas_rect);
/// ```
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct ClippingOverlay<'a> {
    #[common]
    common: Common,
    mask: &'a ClippingMask,
    revision: Revision,
    mode: ClippingMode,
}

impl<'a> ClippingOverlay<'a> {
    /// An overlay named `label` of `mask` in `mode`. `revision` is the host's revision of the image the mask was
    /// counted from.
    pub fn new(label: impl Into<Label>, mask: &'a ClippingMask, revision: Revision, mode: ClippingMode) -> Self {
        ClippingOverlay { common: Common::new(), mask, revision, mode }.label(label)
    }

    /// The overlay picture of `mask` in `mode`: one pixel per mask pixel, colours from `inks`.
    fn image(mask: &ClippingMask, mode: ClippingMode, inks: &ChannelInks) -> ColorImage {
        let pixels = (0..mask.size[1])
            .flat_map(|y| (0..mask.size[0]).map(move |x| (x, y)))
            .map(|(x, y)| {
                let clipped = mask.at(x, y);
                match mode {
                    ClippingMode::Warnings(warnings) => {
                        if warnings.highlights && !clipped.highlights.is_empty() {
                            inks.color(ChannelMask::R)
                        } else if warnings.shadows && !clipped.shadows.is_empty() {
                            inks.color(ChannelMask::B)
                        } else {
                            Color32::TRANSPARENT
                        }
                    }
                    ClippingMode::ShadowChannels => inks.color(ChannelMask(ChannelMask::RGB.bits() ^ clipped.shadows.bits())),
                    ClippingMode::HighlightChannels => inks.color(clipped.highlights),
                    _ => Color32::TRANSPARENT,
                }
            })
            .collect();
        ColorImage::new(mask.size, pixels)
    }
}

impl Component for ClippingOverlay<'_> {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = ClippingOverlayStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: ClippingOverlayStyleKey) -> ClippingOverlayStyle {
        ClippingOverlayStyle { metrics: Metrics::new(), inks: ChannelInks::new(cx) }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(space), true) = (canvas_space(&cx.env), self.mode != ClippingMode::None) else { return };
        let [width, height] = self.mask.size;
        if width == 0 || height == 0 {
            return;
        }
        let painter = ui.content_painter(cx);
        let ctx = painter.painter.ctx();
        let store = TextureStore::<u64>::in_context(ctx, ("imaging.clipping", cx.id), TextureStoreOptions::frames().filter(TextureOptions::NEAREST));
        let inks = &cx.style.inks;
        let texture = store.get_or_put(ctx, 0, Revision::of(&(self.revision, self.mode, inks)), || Self::image(self.mask, self.mode, inks));
        let bounds = DocRect::new(0.0, 0.0, width as f64, height as f64);
        let uvs = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        let mut mesh = Mesh::with_texture(texture.sized().id);
        for (pos, uv) in space.view.quad(bounds).into_iter().zip(uvs) {
            mesh.vertices.push(Vertex { pos, uv, color: Color32::from_gray(u8::MAX) });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        painter.painter.add(Shape::mesh(mesh));
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
