//! `RowDress` and `SelectPicture`: what a picker adds to an option's row and face.

use crate::list::ListRow;
use crate::primitives::ContentPainterCx;
use egui::Rect;

/// What a picker adds to an option's row: the host's content for an option (a detail line, a trailing shortcut text, a
/// thumbnail painter). `Select::decorate` and `ComboBox::decorate` take the closure.
pub struct RowDress<'a, T>(Box<DressFn<'a, T>>);

type DressFn<'a, T> = dyn Fn(&T, ListRow<'a>) -> ListRow<'a> + 'a;

impl<'a, T> RowDress<'a, T> {
    /// The dress that runs `dress` over the row of each option, given the option's value.
    pub fn new(dress: impl Fn(&T, ListRow<'a>) -> ListRow<'a> + 'a) -> Self {
        RowDress(Box::new(dress))
    }

    /// The row of `value` as dressed.
    pub(super) fn apply(&self, value: &T, row: ListRow<'a>) -> ListRow<'a> {
        (self.0)(value, row)
    }
}

/// The picture of an option (an arrowhead, a line style): one painter draws it in the option's list row and on the
/// closed face. `Select::picture` takes the closure.
pub struct SelectPicture<'a, T>(Box<PictureFn<'a, T>>);

type PictureFn<'a, T> = dyn Fn(&T, &ContentPainterCx, Rect) + 'a;

impl<'a, T> SelectPicture<'a, T> {
    /// The picture `paint` draws for an option, given its value, into the rect the row or face gives it.
    pub fn new(paint: impl Fn(&T, &ContentPainterCx, Rect) + 'a) -> Self {
        SelectPicture(Box::new(paint))
    }

    /// Paints the picture of `value` into `rect`.
    pub(super) fn paint(&self, value: &T, cx: &ContentPainterCx, rect: Rect) {
        (self.0)(value, cx, rect)
    }
}
