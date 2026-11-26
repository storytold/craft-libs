//! The annotations of a node graph: backdrops, notes and markers, which are document data the app stores.

use super::NodeGraph;
use super::kind::NodeId;
use crate::contract::{Bind, Cx, PaintUi, PartsCx};
use crate::style::DataColor;
use craft_behavior::{CanvasTransform, EditEvent, Selection};
use egui::{Pos2, Rect, Vec2};

/// Which annotation: the kind and the app's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AnnotationKey {
    /// The backdrop with this id.
    Backdrop(u64),
    /// The note with this id.
    Note(u64),
    /// The marker with this id.
    Marker(u64),
}

/// A titled, tinted rectangle under the nodes, with a grip at its end-bottom corner to resize it.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Backdrop<'a> {
    /// The app's id.
    pub id: u64,
    /// The backdrop's rect, in document units.
    pub rect: Rect,
    /// The title above the rect.
    pub title: Option<&'a str>,
    /// The description inside the rect.
    pub description: &'a str,
    /// The fill, document data.
    pub fill: DataColor,
}

impl<'a> Backdrop<'a> {
    /// Backdrop `id` over `rect` filled with `fill`, with no title and no description.
    pub const fn new(id: u64, rect: Rect, fill: DataColor) -> Self {
        Backdrop { id, rect, title: None, description: "", fill }
    }

    /// The title above the rect.
    #[must_use]
    pub fn title(self, title: &'a str) -> Self {
        let _ = (self, title);
        todo!("ge.C28")
    }

    /// The description inside the rect.
    #[must_use]
    pub fn description(self, description: &'a str) -> Self {
        let _ = (self, description);
        todo!("ge.C28")
    }
}

/// A note over the nodes: text that wraps to its rect's width.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Note<'a> {
    /// The app's id.
    pub id: u64,
    /// The note's rect, in document units.
    pub rect: Rect,
    /// The note's text.
    pub text: &'a str,
}

impl<'a> Note<'a> {
    /// Note `id` over `rect` saying `text`.
    pub const fn new(id: u64, rect: Rect, text: &'a str) -> Self {
        Note { id, rect, text }
    }
}

/// A marker over the notes: the pin icon with its label.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Marker<'a> {
    /// The app's id.
    pub id: u64,
    /// Where the pin points, in document units.
    pub at: Pos2,
    /// The label beside the pin.
    pub label: &'a str,
}

impl<'a> Marker<'a> {
    /// Marker `id` at `at` labelled `label`.
    pub const fn new(id: u64, at: Pos2, label: &'a str) -> Self {
        Marker { id, at, label }
    }
}

/// What the annotations report.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum AnnotationEvent {
    /// The selected annotation changed.
    Selected(Option<AnnotationKey>),
    /// An annotation was moved; the offset is in document units.
    #[non_exhaustive]
    Move {
        /// Which annotation.
        key: AnnotationKey,
        /// The edit of its offset.
        edit: EditEvent<Vec2>,
    },
    /// A backdrop was resized; its rect is in document units.
    #[non_exhaustive]
    Resize {
        /// Which annotation.
        key: AnnotationKey,
        /// The edit of its rect.
        edit: EditEvent<Rect>,
    },
    /// The context menu was asked for on an annotation, at a screen point.
    #[non_exhaustive]
    ContextMenu {
        /// Which annotation.
        key: AnnotationKey,
        /// Where the menu opens.
        at: Pos2,
    },
}

/// The seam with `NodeGraph`: the annotation layers it draws and lets the pointer reach.
#[allow(dead_code)] // P3(graph-w5): ge.C29
pub(super) struct Layers<'a> {
    pub(super) backdrops: &'a [Backdrop<'a>],
    pub(super) notes: &'a [Note<'a>],
    pub(super) markers: &'a [Marker<'a>],
}

#[allow(dead_code)] // P3(graph-w5): ge.C29
impl Layers<'_> {
    /// The parts of the layers: `BACKDROP`, `BACKDROP_GRIP`, `NOTE` and `MARKER`, each at the annotation's id.
    pub(super) fn parts<E: 'static>(&self, zoom: CanvasTransform, origin: Pos2, cx: &mut PartsCx<'_, NodeGraph<'_, E>>) {
        let _ = (zoom, origin, cx);
        todo!("ge.C29")
    }

    /// Paints the backdrops, under the items.
    pub(super) fn paint_under<E: 'static>(&self, zoom: CanvasTransform, origin: Pos2, ui: &mut PaintUi<'_>, cx: &Cx<'_, NodeGraph<'_, E>>) {
        let _ = (zoom, origin, ui, cx);
        todo!("ge.C29")
    }

    /// Paints the notes and the markers, over the parts.
    pub(super) fn paint_over<E: 'static>(&self, zoom: CanvasTransform, origin: Pos2, ui: &mut PaintUi<'_>, cx: &Cx<'_, NodeGraph<'_, E>>) {
        let _ = (zoom, origin, ui, cx);
        todo!("ge.C29")
    }

    /// Reads the pointer on the layers and returns what it did. A press on an annotation writes `annotations` and
    /// empties `nodes`; a press on a node or on empty space writes `None`.
    pub(super) fn update<E: 'static>(
        &self,
        zoom: CanvasTransform,
        origin: Pos2,
        cx: &mut Cx<'_, NodeGraph<'_, E>>,
        annotations: Option<&mut Bind<'_, Option<AnnotationKey>>>,
        nodes: Option<&mut Bind<'_, Selection<NodeId>>>,
    ) -> Vec<AnnotationEvent> {
        let _ = (zoom, origin, cx, annotations, nodes);
        todo!("ge.C29")
    }
}
