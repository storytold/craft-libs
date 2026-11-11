//! The moving pixels of a transform drawn through a quad, a mesh or a map ([`TransformPreview`]).
//!
//! The app reads its layer's pixels once per session into [`PreviewPixels`] and uploads them as
//! [`PreviewTextures`] (a mip chain built on demand). Every pass, [`TransformPreview`] draws them
//! through the session's map under the guides and the box: a [quad](PreviewMap::quad) for Free
//! Transform, a [warp mesh](PreviewMap::mesh), a [pin mesh](PreviewMap::pins) for Puppet Warp, or
//! the app's [function](PreviewMap::function) for Perspective Warp. A layer with a style is
//! composited by the app's [`StyledSource`] through [`StyledPreview`] instead, within a time
//! budget. The preview never changes the document.
//!
//! ```no_run
//! use craft_imaging::ui::transform_preview::{PreviewMap, PreviewPixels, PreviewTextures, TransformPreview};
//! use craft_ui::contract::Show as _;
//! # fn demo(ui: &mut egui::Ui, canvas_rect: egui::Rect, image: egui::ColorImage, quad: [kurbo::Point; 4]) {
//! // Once per session:
//! let textures = PreviewTextures::new("ft-preview", PreviewPixels::new(image, [1.0, 1.0]));
//! // Every pass, in the canvas body under the guides and the box:
//! TransformPreview::new("Moving layer", &textures).map(PreviewMap::quad(quad)).opacity(0.8).clip([800, 600]).show_at(ui, canvas_rect);
//! # }
//! ```

mod maps;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;
mod styled;
mod textures;

use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, Input, Metrics};
use craft_ui::canvas::canvas_space;
use craft_ui::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role};
use craft_ui::primitives::ContentPainterCx;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::epaint::Vertex;
use egui::{Color32, Mesh, Response, Shape, TextureId};

use maps::clip_to_canvas;
pub use maps::{PreviewMap, PreviewMesh};
pub use styled::{StylePlan, StyledBudget, StyledContent, StyledFrame, StyledPreview, StyledSource, resample};
pub use textures::{PreviewDisplay, PreviewMask, PreviewPixels, PreviewTextures, TextureChoice};

/// The preview's algorithm constants (LC23, LC24): choices of how finely to sample, not look, so
/// not tokens. Values are PhotoCraft's.
#[non_exhaustive]
pub struct PreviewQuality;

impl PreviewQuality {
    /// Cells per side of a quad's preview mesh: keeps perspective previews straight
    /// (PhotoCraft `transform_tool/draw.rs:114`).
    pub const QUAD_CELLS: usize = 24;
    /// Cells per side of a function map's preview mesh (PhotoCraft `perspective_ui.rs:57`).
    pub const PLANE_CELLS: usize = 40;
    /// Target screen points per segment of a warp mesh's preview (PhotoCraft `warp_preview.rs:11`).
    pub const SEGMENT_PX: f64 = 6.0;
    /// Fewest and most segments one warp patch gets along an axis.
    pub const MIN_PER_CELL: usize = 4;
    /// Most segments one warp patch gets along an axis.
    pub const MAX_PER_CELL: usize = 96;
    /// Most samples along either axis of a warp mesh.
    pub const MAX_PER_AXIS: usize = 240;
    /// Deepest mip level: 4096 times smaller (PhotoCraft `transform_tex.rs:75`).
    pub const MIP_LEVELS: u32 = 12;
    /// A source pixel shown at this many screen pixels or more counts as zoomed in.
    pub const CRISP_ZOOM: f32 = 2.0;
    /// At most this enlargement by the transform keeps the zoomed-in pixels crisp.
    pub const CRISP_SCALE: f32 = 1.05;
    /// Below this many screen pixels per source pixel a mip level is used.
    pub const MIP_BELOW: f32 = 0.5;
}

/// The style of the transform preview: it has no look of its own.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct PreviewStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The moving pixels of a transform drawn through a map, as image content under the guides and
/// the box (PhotoCraft draws them before the guides so a guide stays visible across the layer).
/// Show it at the canvas's rect in the canvas body; the host shows a `RasterDisplay` of the layers
/// above the moving one right after it (show order is paint order).
///
/// - The texture level and filter follow the on-screen size of a source pixel
///   ([`PreviewTextures::choose`]): a mip level when shrinking below ½, level 0 bilinear around
///   1:1, level 0 nearest at zoom ≥ 2 with the transform enlarging by at most 1.05, and always
///   level 0 nearest with [`nearest`](Self::nearest).
/// - The mesh is clipped to the canvas in document space ([`clip`](Self::clip)), also under a
///   turned view; quad cells with a corner past the horizon (a Distort dragged behind the eye)
///   are left out; triangles are drawn from the most displaced to the least, so a fold keeps its
///   layers whole.
/// - [`opacity`](Self::opacity) tints every vertex white with that alpha (layer opacity × fill).
/// - With [`styled`](Self::styled) and a plan that fits, the styled composite is drawn in place of
///   the mesh (quad maps only); otherwise the plain mesh.
/// - No pointer, key or accessibility node: image content.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control, content_painter)]
pub struct TransformPreview<'a> {
    #[common]
    common: Common,
    textures: &'a PreviewTextures,
    /// Where the source goes; nothing is drawn without a map.
    #[prop]
    map: Option<PreviewMap<'a>>,
    /// The layer's opacity times its fill, 0 to 1; 1 by default.
    #[prop(quantity = "fraction")]
    opacity: f32,
    /// Nearest Neighbor interpolation: level 0 with nearest filtering at every zoom. The app passes the draw
    /// session's `Interpolation::NearestNeighbor` as a bool: the imaging library cannot name the drawing library's
    /// type.
    #[prop]
    nearest: bool,
    /// The document's size in pixels: nothing is drawn past its edges.
    #[prop]
    clip: Option<[u32; 2]>,
    /// The monitor conversion of the displayed upload; the mip levels stay raw.
    #[prop]
    display: Option<&'a PreviewDisplay>,
    styled: Option<(&'a StyledPreview, &'a dyn StyledSource)>,
    /// The pointer is down: the styled composite uses the drag budget.
    #[prop]
    dragging: bool,
    /// How long a styled rebuild may take; PhotoCraft's times by default.
    #[prop]
    styled_budget: StyledBudget,
}

impl<'a> TransformPreview<'a> {
    /// The preview of `textures`, named `label`; nothing is drawn until a [`map`](Self::map) is set.
    pub fn new(label: impl Into<Label>, textures: &'a PreviewTextures) -> Self {
        TransformPreview {
            common: Common::new(),
            textures,
            map: None,
            opacity: 1.0,
            nearest: false,
            clip: None,
            display: None,
            styled: None,
            dragging: false,
            styled_budget: StyledBudget::default(),
        }
        .label(label)
    }

    /// A styled layer: composite through the app's `source` and keep the result in `preview`.
    #[must_use]
    pub fn styled(mut self, preview: &'a StyledPreview, source: &'a dyn StyledSource) -> Self {
        self.styled = Some((preview, source));
        self
    }
}

impl Component for TransformPreview<'_> {
    const ROLE: Role = Role::GRAPHICS_OBJECT;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    const INPUT: Input = Input::Display;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = PreviewStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: TransformPreviewStyleKey) -> PreviewStyle {
        PreviewStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        // pc: crates/ui-egui/src/tools/transform_tool/draw.rs:113 (the quad mesh), :118 (texture
        // pick), :83 (`front_vertices`), :395 (`clip_to_canvas`);
        // crates/ui-egui/src/tools/perspective_ui.rs:263 (`draw_content`, function map);
        // crates/ui-egui/src/tools/puppet_ui.rs:348 (`draw_content`, pin mesh).
        let (Some(map), Some(space)) = (self.map, canvas_space(&cx.env)) else { return };
        let view = space.view;
        let painter = ui.content_painter(cx);
        let ctx = painter.painter.ctx().clone();
        let device = space.pixels_per_point;
        let zoom = view.zoom as f32 * device;
        let scale = map.source_scale(self.textures.size());
        let tint = Color32::from_white_alpha((self.opacity.clamp(0.0, 1.0) * 255.0).round() as u8);

        let styled = match (&self.styled, &map, self.clip) {
            (Some((preview, source)), PreviewMap::Quad(quad), Some(canvas)) => {
                let mut frame = StyledFrame::new(self.textures, *quad, canvas, zoom);
                frame.quad_scale = scale;
                frame.nearest = self.nearest;
                frame.dragging = self.dragging;
                frame.budget = self.styled_budget;
                preview.composite(&ctx, &frame, *source, self.display)
            }
            _ => None,
        };
        if let Some((texture, rect)) = styled {
            let corners = [(rect.x0, rect.y0, 0.0, 0.0), (rect.x1, rect.y0, 1.0, 0.0), (rect.x1, rect.y1, 1.0, 1.0), (rect.x0, rect.y1, 0.0, 1.0)];
            let mesh = PreviewMesh {
                points: corners.iter().map(|c| Point::new(c.0, c.1)).collect(),
                rest: corners.iter().map(|c| Point::new(c.0, c.1)).collect(),
                uv: corners.iter().map(|c| [c.2, c.3]).collect(),
                indices: vec![0, 1, 2, 0, 2, 3],
            };
            add_mesh(&painter, mesh, texture, Color32::from_gray(u8::MAX), self.clip, &view);
        } else {
            let choice = PreviewTextures::choose(zoom, scale, self.nearest);
            let Some((texture, uv)) = self.textures.pick(&ctx, choice, self.display) else { return };
            let mesh = map.build(view.zoom * f64::from(device), uv);
            add_mesh(&painter, mesh, texture, tint, self.clip, &view);
        }
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// `mesh` clipped to the canvas in document space, mapped through `view` and added to the painter.
fn add_mesh(painter: &ContentPainterCx<'_>, mesh: PreviewMesh, texture: TextureId, tint: Color32, canvas: Option<[u32; 2]>, view: &CanvasView) {
    let mesh = match canvas {
        Some(canvas) => clip_to_canvas(mesh, canvas),
        None => mesh,
    };
    let mut out = Mesh::with_texture(texture);
    out.vertices =
        mesh.points.iter().zip(&mesh.uv).map(|(p, uv)| Vertex { pos: view.to_screen(*p), uv: egui::pos2(uv[0], uv[1]), color: tint }).collect();
    out.indices = mesh.indices;
    painter.painter.add(Shape::mesh(out));
}
