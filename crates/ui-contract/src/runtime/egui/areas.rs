//! `Area` with its left-top pivot, `Order`, sublayers, transform layers and `set_layer_order`.

#[cfg(feature = "testing")]
use egui::Ui;
use egui::emath::TSTransform;
use egui::{Color32, Context, Id, LayerId, Order, Painter, Pos2, Rect, Sense, Vec2};

/// Shows an `egui::Area` named `name` and runs `body` on its `Ui`.
#[cfg(feature = "testing")]
pub(crate) fn show_area<R>(ctx: &Context, name: &'static str, body: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Area::new(crate::runtime::egui::memory::unique(name)).show(ctx, body).inner
}

/// Shows an interactable `egui::Area` at the screen's top left and runs `body` on its `Ui`.
#[cfg(feature = "testing")]
pub(crate) fn show_cover<R>(ctx: &Context, name: &'static str, body: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Area::new(crate::runtime::egui::memory::unique(name)).fixed_pos(Pos2::ZERO).interactable(true).show(ctx, body).inner
}

/// An egui layer: an `Area`, a panel or a transformed body.
pub(crate) type Layer = LayerId;

/// A layer's mapping from its own space to the screen's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LayerMap(TSTransform);

impl LayerMap {
    /// The position in the layer of a screen position.
    pub(crate) fn to_layer(self, point: Pos2) -> Pos2 {
        self.0.inverse() * point
    }

    /// The screen position of a position in the layer.
    pub(crate) fn to_screen(self, point: Pos2) -> Pos2 {
        self.0 * point
    }

    /// A screen-space movement as the layer sees it.
    pub(crate) fn delta_to_layer(self, delta: Vec2) -> Vec2 {
        delta / self.0.scaling
    }
}

/// The mapping of `layer` to the screen, `None` for a layer with no transform.
pub(crate) fn layer_map(ctx: &Context, layer: LayerId) -> Option<LayerMap> {
    ctx.layer_transform_to_global(layer).map(LayerMap)
}

/// How high `layer` stands in egui's layer order, which the overlay host sets each pass: a larger value lies over a
/// smaller one. A layer egui does not list stands lowest.
pub(crate) fn layer_position(ctx: &Context, layer: LayerId) -> usize {
    ctx.memory(|memory| memory.layer_ids().position(|listed| listed == layer).map_or(0, |at| at + 1))
}

/// The topmost layer under a screen position. Inside the canvas a modal layer leaves open (`runtime::sampler`) and
/// where nothing but the canvas lies over it, it answers the canvas's own layer as if the modal's gate and its scrim
/// were not there.
pub(crate) fn layer_at(ctx: &Context, point: Pos2) -> Option<LayerId> {
    crate::runtime::sampler::layer_at(ctx, point).or_else(|| ctx.layer_id_at(point))
}

/// The topmost layer under a screen position, as if `skipped` were not shown: the layer below a window that moves
/// with the pointer. A modal layer still holds the input, as in `layer_at`.
pub(crate) fn layer_at_without(ctx: &Context, point: Pos2, skipped: LayerId) -> Option<LayerId> {
    if let Some(layer) = crate::runtime::sampler::layer_at(ctx, point) {
        return Some(layer);
    }
    let under = layers_under(ctx, point, Some(skipped));
    let found = *under.first()?;
    ctx.memory(|memory| if memory.is_above_modal_layer(found) { Some(found) } else { memory.top_modal_layer() })
}

/// The shown, interactable layers with their screen rects, bottom first, without `skipped`.
fn interactive_layers(ctx: &Context, skipped: Option<LayerId>) -> Vec<(LayerId, Rect)> {
    let stack: Vec<LayerId> =
        ctx.memory(|memory| memory.layer_ids().filter(|layer| Some(*layer) != skipped && memory.areas().is_visible(layer)).collect());
    stack
        .into_iter()
        .filter_map(|layer| {
            let state = egui::AreaState::load(ctx, layer.id)?;
            let rect = ctx.layer_transform_to_global(layer).map_or(state.rect(), |to_global| to_global * state.rect());
            state.interactable.then_some((layer, rect))
        })
        .collect()
}

/// The shown, interactable layers whose area holds `point` (screen space), topmost first, without `skipped`.
fn layers_under(ctx: &Context, point: Pos2, skipped: Option<LayerId>) -> Vec<LayerId> {
    interactive_layers(ctx, skipped).into_iter().rev().filter(|(_, rect)| rect.contains(point)).map(|(layer, _)| layer).collect()
}

/// The screen rects of the layers that lie over `layer` in the layer order, without `layer`'s own sublayers and the
/// modal scrim `scrim`: what can hide a canvas from a pointer.
pub(crate) fn rects_over(ctx: &Context, layer: LayerId, scrim: Option<LayerId>) -> Vec<Rect> {
    let own = |other: LayerId| {
        other == layer || ctx.memory(|memory| memory.areas().parent_layer(other) == Some(layer) || memory.areas().parent_layer(layer) == Some(other))
    };
    // A layer the order does not list (a panel) stands lowest, as `layer_position` has it.
    let order: Vec<LayerId> = ctx.memory(|memory| memory.layer_ids().collect());
    let skip = order.iter().position(|other| own(*other)).map_or(0, |at| at + 1);
    let above: std::collections::HashSet<LayerId> = order.into_iter().skip(skip).collect();
    interactive_layers(ctx, None)
        .into_iter()
        .filter(|(other, _)| above.contains(other) && !own(*other) && Some(*other) != scrim)
        .map(|(_, rect)| rect)
        .collect()
}

/// Whether the topmost layer under `point` belongs to the top modal layer's family: the modal, its chrome and what
/// opened above it. egui's own `layer_id_at` answers the modal for a point below it, so the layers are read directly.
pub(crate) fn in_modal_family(ctx: &Context, point: Pos2) -> bool {
    let top = layers_under(ctx, point, None).first().copied();
    top.is_some_and(|top| ctx.memory(|memory| memory.is_above_modal_layer(top)))
}

/// The layer of a transformed body: a sublayer of `parent` in the parent's order, named by `root`.
pub(crate) fn body_layer(parent: LayerId, root: Id) -> LayerId {
    LayerId::new(parent.order, crate::runtime::memory::sub(root, "body_layer"))
}

/// What a transformed body's forwarding widget carries: the root's sense and interact rect, in screen space.
pub(crate) struct Forward {
    pub id: Id,
    pub sense: egui::Sense,
    pub rect: Rect,
}

/// A transformed body's layer: `layer` is shown through `transform`, a sublayer of `parent`, with its top left at
/// `at` in its own space and clipped to `clip` in screen space.
pub(crate) struct BodyLayer {
    pub parent: LayerId,
    pub layer: LayerId,
    pub transform: TSTransform,
    pub at: Pos2,
    pub clip: Rect,
    /// Registered on the layer first, so an allocating `Ui` on the layer cannot hide the root under egui's
    /// layer-cover rule.
    pub forward: Option<Forward>,
}

/// Shows a transformed body in its layer.
pub(crate) fn show_body_layer<R>(ctx: &Context, body_layer: BodyLayer, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let BodyLayer { parent, layer, transform, at, clip, forward } = body_layer;
    ctx.set_transform_layer(layer, transform);
    ctx.set_sublayer(parent, layer);
    egui::Area::new(layer.id)
        .order(layer.order)
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(at)
        .constrain(false)
        .sense(egui::Sense::hover())
        .show(ctx, |ui| {
            if let Some(forward) = forward {
                ui.interact(transform.inverse() * forward.rect, forward.id, forward.sense);
                super::access::hide(ctx, forward.id);
            }
            ui.set_clip_rect(transform.inverse() * clip);
            ui.expand_to_include_rect(transform.inverse() * clip);
            body(ui)
        })
        .inner
}

/// The transform of a body zoomed by `scale` about its top left `at` and moved by `offset`.
pub(crate) fn body_transform(at: Pos2, scale: f32, offset: Vec2) -> TSTransform {
    TSTransform::new(at.to_vec2() * (1.0 - scale) + offset, scale)
}

/// The mapping between the screen and a body zoomed by `scale` about `at` and moved by `offset`.
pub(crate) fn body_map(at: Pos2, scale: f32, offset: Vec2) -> LayerMap {
    LayerMap(body_transform(at, scale, offset))
}

/// An egui layer, as the runtime's shape records name it.
pub type LayerKey = egui::LayerId;

/// How an overlay layer ranks among egui's orders: the one place the runtime chooses an `Order`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tier {
    /// Floating windows and panels.
    Window,
    /// Toasts, popups, modals and their chrome.
    Foreground,
    /// Tooltips and the ghost layer.
    Tooltip,
}

impl Tier {
    fn order(self) -> Order {
        match self {
            Tier::Window => Order::Middle,
            Tier::Foreground => Order::Foreground,
            Tier::Tooltip => Order::Tooltip,
        }
    }
}

/// What the overlay host asks of an `Area`.
pub(crate) struct LayerSpec {
    pub id: Id,
    pub tier: Tier,
    /// The physical left-top of the layer.
    pub at: Pos2,
    /// Whether the layer takes input; a tooltip and the ghost layer never do.
    pub interactable: bool,
}

/// What `show_layer` reports.
pub(crate) struct LayerShown {
    /// The layer's rect in screen space.
    pub rect: Rect,
    /// The layer's egui id, as `Context::layer_id_at` names it.
    pub layer: LayerId,
}

/// Shows an overlay layer: an `Area` with its left-top pivot at `spec.at`, never egui's default pivot.
pub(crate) fn show_layer(ctx: &Context, spec: &LayerSpec, body: impl FnOnce(&mut egui::Ui)) -> LayerShown {
    let shown = egui::Area::new(spec.id)
        .order(spec.tier.order())
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(spec.at)
        .constrain(false)
        .interactable(spec.interactable)
        .show(ctx, body);
    LayerShown { rect: shown.response.rect, layer: shown.response.layer_id }
}

/// What [`show_cover_layer`] shows: the layer's id and tier, the `screen` it covers, its `scrim`, the `hole` left
/// unpainted, the `through` canvas left open, and whether it `absorbs` pointer hits.
pub(crate) struct CoverLayer {
    pub id: Id,
    pub tier: Tier,
    pub screen: Rect,
    pub scrim: Color32,
    pub hole: Option<Rect>,
    pub through: Option<Rect>,
    pub absorbs: bool,
}

/// Shows a layer that covers `screen` and paints `scrim` on it, around `hole` when there is one. While `absorbs` it
/// takes every pointer hit on it, so nothing below it is hit, except inside `through`, the canvas a modal leaves open
/// (`runtime::sampler`): that rect is left unpainted and unabsorbed, and the four bands around it absorb. `hole` is
/// left unpainted as well.
pub(crate) fn show_cover_layer(ctx: &Context, cover: CoverLayer) {
    let CoverLayer { id, tier, screen, scrim, hole, through, absorbs } = cover;
    let scrim = Some(scrim);
    let clear = through.map(|through| through.intersect(screen)).filter(|through| through.is_positive());
    let holes: Vec<Rect> = [clear, hole].into_iter().flatten().map(|hole| hole.intersect(screen)).filter(|hole| hole.is_positive()).collect();
    egui::Area::new(id)
        .order(tier.order())
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(screen.min)
        .constrain(false)
        .interactable(absorbs)
        .sense(Sense::hover())
        .show(ctx, |ui| {
            if let Some(scrim) = scrim {
                let painter = ui.painter();
                let mut painted = vec![screen];
                for hole in &holes {
                    painted = painted
                        .into_iter()
                        .flat_map(|rect| if rect.intersects(*hole) { bands(rect, hole.intersect(rect)).to_vec() } else { vec![rect] })
                        .collect();
                }
                for band in painted.into_iter().filter(|band| band.is_positive()) {
                    painter.rect_filled(band, 0.0, scrim);
                }
            }
            let absorbed = match clear {
                Some(clear) => bands(screen, clear).into_iter().filter(|band| band.is_positive()).collect(),
                None => vec![screen],
            };
            for rect in absorbed {
                let absorber = ui.allocate_rect(rect, Sense::click_and_drag());
                crate::runtime::egui::widgets::note_widget(ui.ctx(), absorber.id);
                crate::runtime::egui::access::hide(ui.ctx(), absorber.id);
            }
        });
}

/// The four rects of `screen` around `hole`: above, below, left of and right of it.
fn bands(screen: Rect, hole: Rect) -> [Rect; 4] {
    [
        Rect::from_min_max(screen.min, Pos2::new(screen.right(), hole.top())),
        Rect::from_min_max(Pos2::new(screen.left(), hole.bottom()), screen.max),
        Rect::from_min_max(Pos2::new(screen.left(), hole.top()), Pos2::new(hole.left(), hole.bottom())),
        Rect::from_min_max(Pos2::new(hole.right(), hole.top()), Pos2::new(screen.right(), hole.bottom())),
    ]
}

/// The rect an overlay layer had when it was last shown, in screen space.
pub(crate) fn layer_rect(ctx: &Context, id: Id) -> Option<Rect> {
    ctx.memory(|memory| memory.area_rect(id))
}

/// The viewport's safe rect: the room overlays must stay inside.
pub(crate) fn safe_rect(ctx: &Context) -> Rect {
    ctx.content_rect()
}

/// Hands egui the whole layer order of the running pass, bottom first. egui consumes it by the end of the pass.
pub(crate) fn set_layer_order(ctx: &Context, order: &[LayerId]) {
    ctx.set_layer_order(order);
}

/// Makes `layer` the modal layer of the running pass: layers below it take no input.
pub(crate) fn set_modal_layer(ctx: &Context, layer: LayerId) {
    ctx.memory_mut(|memory| memory.set_modal_layer(layer));
}

/// The layer id an overlay layer named `id` has in `tier`.
pub(crate) fn layer_of(tier: Tier, id: Id) -> LayerId {
    LayerId::new(tier.order(), id)
}

/// The mapping from `layer`'s space to the screen's; the identity for a layer with no transform.
pub(crate) fn layer_to_screen(ctx: &Context, layer: LayerId) -> TSTransform {
    ctx.layer_transform_to_global(layer).unwrap_or(TSTransform::IDENTITY)
}

/// Limits `ui` to `max` and at least `min_width` across: the room the placement leaves the layer's body.
pub(crate) fn bound(ui: &mut egui::Ui, max: Vec2, min_width: f32) {
    ui.set_max_size(max);
    ui.set_min_width(min_width);
}

/// Fades everything painted in `ui` to `opacity`.
pub(crate) fn fade(ui: &mut egui::Ui, opacity: f32) {
    ui.set_opacity(opacity);
}

/// The painter of the ghost layer, over the whole screen and above every popup. The layer takes no input.
pub(crate) fn ghost_painter(ctx: &Context) -> Painter {
    ctx.layer_painter(layer_of(Tier::Tooltip, crate::runtime::egui::memory::unique("craft_ui.ghost")))
}

/// The pointer position of the running pass, in screen space.
pub(crate) fn pointer_pos(ctx: &Context) -> Option<Pos2> {
    ctx.input(|input| input.pointer.latest_pos())
}

/// The layer order after `Context::set_layer_order`.
#[cfg(feature = "testing")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LayerOrder {
    /// The listed layer is the topmost of the layers shown.
    pub listed_on_top: bool,
    /// The unlisted layers keep their relative order.
    pub unlisted_keep_order: bool,
    /// A listed id that is no layer does not become one.
    pub unknown_ignored: bool,
}

/// Two frames over three areas; the second lists the first area and an unknown layer.
#[cfg(feature = "testing")]
pub fn layer_order() -> LayerOrder {
    let ctx = super::fixture::bare_context();
    let ids =
        [super::memory::unique("craft_ui.surface.la"), super::memory::unique("craft_ui.surface.lb"), super::memory::unique("craft_ui.surface.lc")];
    let layers = ids.map(|id| LayerId::new(Order::Middle, id));
    let ghost = LayerId::new(Order::Middle, super::memory::unique("craft_ui.surface.ghost"));
    let frame = |time: f64, list: &[LayerId]| {
        let mut output = ctx.run_ui(super::surface::raw(time, Vec::new()), |ui| {
            let ctx = ui.ctx().clone();
            for id in ids {
                egui::Area::new(id).show(&ctx, |ui| {
                    ui.label("layer");
                });
            }
            if !list.is_empty() {
                ctx.set_layer_order(list);
            }
        });
        output.textures_delta.clear();
        ctx.memory(|memory| memory.layer_ids().filter(|layer| layers.contains(layer) || *layer == ghost).collect::<Vec<_>>())
    };
    let before = frame(1.0, &[]);
    let after = frame(2.0, &[layers[0], ghost]);
    let rest = |order: &[LayerId]| order.iter().copied().filter(|layer| *layer != layers[0]).collect::<Vec<_>>();
    LayerOrder {
        listed_on_top: after.last() == Some(&layers[0]),
        unlisted_keep_order: rest(&before) == rest(&after),
        unknown_ignored: !after.contains(&ghost),
    }
}

/// Gives a container layer's area a size before its first pass, so egui runs no hidden sizing pass for it. That pass
/// shows no body, so nothing would be measured in it; the layer's first pass is a real one that records the sizes
/// the discard pass places by.
pub(crate) fn seed_area(ctx: &Context, layer: LayerId, at: Pos2, room: Vec2) {
    let state =
        egui::AreaState { pivot_pos: Some(at), pivot: egui::Align2::LEFT_TOP, size: Some(room), interactable: false, last_became_visible_at: None };
    ctx.memory_mut(|memory| memory.areas_mut().set_state(layer, state));
}

/// The width of the widest layer above the background, 0 when there is none.
#[cfg(feature = "testing")]
pub(crate) fn widest_layer(ctx: &Context) -> f32 {
    ctx.memory(|memory| {
        memory
            .layer_ids()
            .filter(|layer| layer.order != Order::Background)
            .filter_map(|layer| memory.area_rect(layer.id))
            .map(|rect| rect.width())
            .fold(0.0, f32::max)
    })
}
