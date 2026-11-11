//! `PaintUi` (5.3): what `paint` and `paint_over` get instead of a `Ui`. It lives in the contract (layer 4)
//! because `child` and `face` show components; `Draw` and the painters stay in `primitives`.

use super::SideLabel;
use super::label::{self, LeafLabel};
use super::run::{self, FaceSource, ShowAt};
use super::{ContentPainter, Cx, Show, TipFn};
use crate::primitives::sensing::PartResults;
use crate::primitives::{ContentPainterCx, Draw, DrawEnv, PaintRecord};
use crate::runtime::access;
use crate::runtime::egui::{alloc, child};
use crate::runtime::env::{Env, Label};
use crate::runtime::layout_memory;
use crate::runtime::memory::sub;
use crate::style::{Flags, Ink, Stateful, StyleColor, StyleCx};
use craft_behavior::PartRef;
use craft_theme::{ControlSize, Variant};
use egui::{Id, Rect, Response, Ui};
use std::cell::RefCell;
use std::fmt::Debug;
use std::hash::Hash;

/// What `paint` and `paint_over` get instead of a `Ui`.
pub struct PaintUi<'a> {
    pub(crate) ui: &'a mut Ui,
    #[doc(hidden)]
    pub env: &'a Env,
    /// The ink of the component's own surface, which `draw` paints text and rules in.
    pub(crate) ink: Ink,
    pub(crate) id: Id,
    pub(crate) rect: Rect,
    pub(crate) record: &'a RefCell<PaintRecord>,
    /// The part results of this pass; faces read their part's state from here.
    pub(crate) results: &'a PartResults,
    pub(crate) response: &'a Response,
    pub(crate) active: Option<(PartRef, bool)>,
    /// The composite's text colour, which its faces take as their host ink.
    pub(crate) text: Option<&'a Stateful<StyleColor>>,
    /// The composite's muted text colour (its second text look), which its faces take as their host's muted ink.
    pub(crate) muted: Option<&'a Stateful<StyleColor>>,
    /// The composite's validity flags, which its faces carry.
    pub(crate) flags: Flags,
    /// The composite's resolved enabled state and size, which the children it builds inherit.
    pub(crate) enabled: bool,
    pub(crate) size: ControlSize,
    /// The children's `Ui`, built on the first child under `sub(id, "children")`.
    pub(crate) kids: Option<Ui>,
}

impl PaintUi<'_> {
    /// The content-paint API: text, rules, surfaces and shapes, recorded for the later steps.
    pub fn draw(&self) -> Draw<'_> {
        Draw::new(self.ui.painter(), self.record, DrawEnv { tokens: &self.env.tokens, direction: self.env.direction, ink: self.ink })
            .text_direction(self.env.text_direction)
    }

    /// The content-paint API for painting `part`: as [`draw`](Self::draw), and when the part was declared
    /// [`inactive`](super::PartBuilder::inactive) its data fills and strokes use the theme's disabled fill.
    pub fn draw_part(&self, part: impl Into<PartRef>) -> Draw<'_> {
        let part = part.into();
        let inactive = self.results.get(&(part.key, part.index)).is_some_and(|result| result.inactive);
        self.draw().for_part(inactive)
    }

    /// Runs `f` with the style context of this component's env, so app hooks called from `paint` pick colours and
    /// sizes through tokens as `style` does.
    pub fn with_style<R>(&self, variant: Variant, f: impl FnOnce(&StyleCx<'_>) -> R) -> R {
        let env = self.env;
        let cx = StyleCx::new(
            self.ui.ctx(),
            &env.tokens,
            &env.values,
            self.size,
            variant,
            env.direction,
            env.scale,
            env.os,
            env.active,
            env.key,
            env.line_ratio,
        );
        f(&cx)
    }

    /// Shows a child component at a physical rect, with id `cx.key(key)`. The child `Ui` is pinned left to
    /// right: this site is unmirrored.
    pub fn child<C: Show>(&mut self, key: impl Hash + Debug, rect: Rect, child: C) -> C::Output {
        self.child_with(self.id.with(key), rect, child, None)
    }

    /// Shows `child` as the owner's own control, such as a picker the owner draws its field with. The child's node
    /// folds into the owner's: the owner's node stands for the child's focus stop, carries the child's value text when
    /// the owner's `access` sets none, and takes the actions and requests aimed at it, which reach the child. The
    /// owner's name names the control.
    pub fn inner<C: Show>(&mut self, key: impl Hash + Debug, rect: Rect, child: C) -> C::Output {
        let id = self.id.with(key);
        access::fold::request(self.ui.ctx(), id, self.id);
        self.child_with(id, rect, child, None)
    }

    /// Shows a child with rich tooltip content.
    pub fn child_with_tip<C: Show>(&mut self, key: impl Hash + Debug, rect: Rect, child: C, tip: impl FnOnce(&mut Ui)) -> C::Output {
        self.child_with(self.id.with(key), rect, child, Some(Box::new(tip)))
    }

    /// Runs the label step (B2) for one leaf child at `rect`: the label's placement beside or above the child, the
    /// label scope, scrub and the reset dot. The child gets the width the label leaves and its own height at the top
    /// of that rect, as in a labelled container's body, so the label lines up with its baseline on either path.
    pub fn labelled<C: Show>(&mut self, key: impl Hash + Debug, rect: Rect, label: Label, side: SideLabel, child: C) -> C::Output {
        let id = self.id.with(key);
        let env = Env::of(self.ui);
        let mut child = child;
        let at =
            LeafLabel { id, owner: self.id, parent: Some(self.id), row: rect, text: &label, side: &side, enabled: self.enabled, size: self.size };
        let ctx = self.ui.ctx().clone();
        layout_memory::boundary(&ctx, true, || {
            let placed = crate::layout::ColumnGroup::barrier(&mut *self.ui, |ui| label::leaf(ui, &env, &at, &mut child));
            let mut control = placed.control;
            if child.fills() != Some(craft_behavior::Axis::Y) {
                let ideal = run::measure_under(&mut child, &*self.ui, Some(id), craft_behavior::Proposal::Ideal, Some((self.enabled, self.size)));
                control.max.y = control.max.y.min(control.min.y + ideal.y);
            }
            let output = self.child_with(id, control, child, None);
            placed.finish(&ctx);
            output
        })
    }

    fn child_with<C: Show>(&mut self, id: Id, rect: Rect, child: C, tip: Option<TipFn<'_>>) -> C::Output {
        let scope = sub(self.id, "children");
        let parent = &mut *self.ui;
        let kids = self.kids.get_or_insert_with(|| {
            let builder = egui::UiBuilder::new().layout(child::pinned_layout()).scope_id(scope).max_rect(self.rect);
            alloc::child(parent, access::with_parent(builder, self.id))
        });
        let ctx = kids.ctx().clone();
        let env = super::cx::child_env(&Env::of(kids), self.enabled, self.size);
        let installed = Env::install_for(kids, env);
        // The child sits in a rect its owner chose, and the owner's own measure answers for it: no body's transcript
        // holds it.
        let output = layout_memory::boundary(&ctx, true, || run::show_leaf(super::Props::id(child, id), kids, ShowAt::Rect(rect), tip));
        Env::restore(&ctx, installed);
        output
    }

    /// Paints `face` as the look of `part`: its style, surface and state fill from the part's state merged with the
    /// instance's `flags()` and the host's validity flags, then its own `paint`. The face reads the host's text
    /// colour through `StyleCx::host_ink` and its muted colour through `StyleCx::host_muted_ink`. It registers no id, senses nothing, opens no memory stage and
    /// fires nothing.
    pub fn face<C: Show>(&mut self, part: impl Into<PartRef>, face: C) {
        self.face_with(part.into(), face, self.text);
    }

    /// `face`, for a face on a fill the composite draws for that part alone (a plate over a picture): the face reads
    /// `ink` through `StyleCx::host_ink` instead of the composite's text colour.
    pub fn face_in<C: Show>(&mut self, part: impl Into<PartRef>, face: C, ink: &Stateful<StyleColor>) {
        self.face_with(part.into(), face, Some(ink));
    }

    fn face_with<C: Show>(&mut self, part: PartRef, face: C, ink: Option<&Stateful<StyleColor>>) {
        let Some(result) = self.results.get(&(part.key, part.index)) else {
            debug_assert!(false, "face for a part `parts` did not declare");
            return;
        };
        let focus = self.active.filter(|(active, _)| *active == part).map(|(_, visible)| visible);
        let source =
            FaceSource { id: self.id, result, focus, response: self.response, ink, muted: self.muted, flags: self.flags, enabled: self.enabled };
        let record = run::paint_face(self.ui, &source, part, face);
        let mut own = self.record.borrow_mut();
        if let Some(baseline) = record.baseline {
            own.baseline.get_or_insert(baseline);
        }
        own.faces.push(record);
    }

    /// The raw painter for an allowlisted content painter, clipped to the component's own rect.
    pub fn content_painter<C: ContentPainter>(&self, cx: &Cx<'_, C>) -> ContentPainterCx<'_> {
        ContentPainterCx::new(self.ui.painter().with_clip_rect(cx.core.rect.intersect(self.ui.clip_rect())), self.env, self.ink)
    }
}
