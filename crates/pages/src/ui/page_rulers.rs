//! The rulers around one page of a [`PageView`](super::PageView): [`VerticalRuler`] with table row grips, and
//! [`PageRulers`], which joins a [`TextRuler`](craft_text::ui::ruler::TextRuler), a [`VerticalRuler`] and
//! [`MarginHandles`](super::MarginHandles) around the page.

use std::marker::PhantomData;

use craft_behavior::GesturePhase;
use craft_keymap::Mods;
use craft_text::ui::DocUnit;
use craft_text::ui::ruler::{RulerValues, TabAlign, TextRulerOutput};
use craft_ui::contract::Bind;
use craft_ui::runtime::env::Label;
use egui::{Rangef, Rect, Response, Ui};

use super::margins::{MarginHandlesOutput, PageEdges};
use super::view::PlacedPage;

/// A table row grip dragged on a vertical ruler: one gesture per drag, reported with its phase.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RowDrag {
    /// Where the gesture stands.
    pub phase: GesturePhase,
    /// The row whose bottom border the grip moves.
    pub index: usize,
    /// The position the pointer gives the border, points from the page's top.
    pub position: f32,
    /// The drag pass's modifiers.
    pub mods: Mods,
}

/// What a [`VerticalRuler`] did this pass.
#[non_exhaustive]
pub struct VerticalRulerOutput {
    /// The ruler's response.
    pub response: Response,
    /// A table row grip dragged ([`VerticalRuler::rows`]).
    pub row: Option<RowDrag>,
}

/// The vertical ruler of one page, with grips on the bottom borders of the caret row's table rows: the page's span is
/// shaded as margin and the text range filled, and a drag of a grip is one gesture.
#[must_use = "show the ruler with `show_at`"]
pub struct VerticalRuler<'a> {
    stub: PhantomData<&'a PlacedPage>,
}

impl<'a> VerticalRuler<'a> {
    /// The ruler named `label` along `page`.
    pub fn new(label: impl Into<Label>, page: &'a PlacedPage) -> Self {
        let _ = (label.into(), page);
        todo!("cv.K21")
    }

    /// The range the text occupies, points from the page's top.
    pub fn text(self, text: Rangef) -> Self {
        let _ = (self, text);
        todo!("cv.K21")
    }

    /// The unit the ticks and numbers show.
    pub fn unit(self, unit: DocUnit) -> Self {
        let _ = (self, unit);
        todo!("cv.K21")
    }

    /// The table row bottoms, points from the page's top, each a grip.
    pub fn rows(self, rows: &'a [f32]) -> Self {
        let _ = (self, rows);
        todo!("cv.K21")
    }

    /// Shows the ruler over `canvas_rect`.
    pub fn show_at(self, ui: &mut Ui, canvas_rect: Rect) -> VerticalRulerOutput {
        let _ = (self, ui, canvas_rect);
        todo!("cv.K21")
    }
}

/// What [`PageRulers`] did this pass.
#[non_exhaustive]
pub struct PageRulersOutput {
    /// The horizontal ruler's output.
    pub ruler: TextRulerOutput,
    /// The margin handles' output.
    pub margins: MarginHandlesOutput,
    /// A table row grip dragged on the vertical ruler.
    pub row: Option<RowDrag>,
}

/// The rulers around a page: the text ruler over the top band, the vertical ruler and the margin handles last, so
/// their edges win. It shows no guides, no zero point and no units menu; `Rulers` is unchanged.
#[must_use = "show the rulers with `show_at`"]
pub struct PageRulers<'a> {
    stub: PhantomData<(&'a PlacedPage, &'a mut PageEdges, &'a mut RulerValues)>,
}

impl<'a> PageRulers<'a> {
    /// The rulers named `label` around `page`, editing its margin `edges` and the paragraph's `ruler` values.
    pub fn new(label: impl Into<Label>, page: &'a PlacedPage, edges: Bind<'a, PageEdges>, ruler: Bind<'a, RulerValues>) -> Self {
        let _ = (label.into(), page, edges, ruler);
        todo!("cv.K22")
    }

    /// The text column's start and end, points from the page's left.
    pub fn column(self, column: (f32, f32)) -> Self {
        let _ = (self, column);
        todo!("cv.K22")
    }

    /// The unit the ticks and numbers show.
    pub fn unit(self, unit: DocUnit) -> Self {
        let _ = (self, unit);
        todo!("cv.K22")
    }

    /// Shows the tab selector in the corner, with the kind a click sets.
    pub fn kind(self, kind: Option<TabAlign>) -> Self {
        let _ = (self, kind);
        todo!("cv.K22")
    }

    /// The paragraph's direction is right to left (`true`) or left to right (`false`).
    pub fn right_to_left(self, right_to_left: Option<bool>) -> Self {
        let _ = (self, right_to_left);
        todo!("cv.K22")
    }

    /// The caret row's table column borders, points from the column's start edge.
    pub fn columns(self, columns: &'a [f32]) -> Self {
        let _ = (self, columns);
        todo!("cv.K22")
    }

    /// The text column gaps as (start, end) points.
    pub fn gaps(self, gaps: &'a [(f32, f32)]) -> Self {
        let _ = (self, gaps);
        todo!("cv.K22")
    }

    /// The table row bottoms, points from the page's top.
    pub fn rows(self, rows: &'a [f32]) -> Self {
        let _ = (self, rows);
        todo!("cv.K22")
    }

    /// Binding room added to the inside margin, points.
    pub fn gutter(self, gutter: f64) -> Self {
        let _ = (self, gutter);
        todo!("cv.K22")
    }

    /// The page is an even page of mirrored margins.
    pub fn mirrored_even(self, mirrored_even: bool) -> Self {
        let _ = (self, mirrored_even);
        todo!("cv.K22")
    }

    /// Shows the rulers over the canvas at `canvas_rect`.
    pub fn show_at(self, ui: &mut Ui, canvas_rect: Rect) -> PageRulersOutput {
        let _ = (self, ui, canvas_rect);
        todo!("cv.K22")
    }
}
