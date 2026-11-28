//! `PaintSlot`: a place in a layer's paint list, reserved while a component paints and filled or read later by code
//! that holds no painter.

use crate::runtime::egui::areas::Layer;
use egui::layers::ShapeIdx;
use egui::{Context, Painter, Rect, Shape};

/// An empty shape reserved in a layer's paint list. Shapes added to it later paint at its place in the paint order;
/// shapes painted on the layer after it can be taken out to move them elsewhere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintSlot {
    layer: Layer,
    idx: ShapeIdx,
}

impl PaintSlot {
    /// Reserves an empty shape at the end of `painter`'s layer, clipped like the painter.
    pub fn reserve(painter: &Painter) -> Self {
        PaintSlot { layer: painter.layer_id(), idx: painter.add(Shape::Noop) }
    }

    /// Reserves an empty shape at the end of `layer`, clipped to `clip`.
    pub(crate) fn reserve_on(ctx: &Context, layer: Layer, clip: Rect) -> Self {
        PaintSlot { layer, idx: ctx.graphics_mut(|graphics| graphics.entry(layer).add(clip, Shape::Noop)) }
    }

    /// Adds `shapes` to the slot, after the shapes it holds, keeping the slot's clip.
    pub fn extend(self, ctx: &Context, shapes: Vec<Shape>) {
        if shapes.is_empty() {
            return;
        }
        ctx.graphics_mut(|graphics| {
            graphics.entry(self.layer).mutate_shape(self.idx, |clipped| {
                if let Shape::Vec(list) = &mut clipped.shape {
                    list.extend(shapes);
                    return;
                }
                let reserved = std::mem::replace(&mut clipped.shape, Shape::Noop);
                let list = if matches!(reserved, Shape::Noop) { shapes } else { std::iter::once(reserved).chain(shapes).collect() };
                clipped.shape = Shape::Vec(list);
            });
        });
    }

    /// Takes the shapes painted on the slot's layer after it, in paint order, leaving empty shapes in their places.
    pub fn take_after(self, ctx: &Context) -> Vec<Shape> {
        ctx.graphics_mut(|graphics| {
            let list = graphics.entry(self.layer);
            (self.idx.0 + 1..list.next_idx().0)
                .map(|at| {
                    let mut taken = Shape::Noop;
                    list.mutate_shape(ShapeIdx(at), |clipped| std::mem::swap(&mut clipped.shape, &mut taken));
                    taken
                })
                .collect()
        })
    }
}
