//! `AppPaint`: a picture the app paints itself inside a framework control, which places it and draws its own parts
//! around it.

use crate::primitives::ContentPainterCx;

type AppPaintFn<'a, A> = Box<dyn FnMut(&ContentPainterCx<'_>, A) + 'a>;

/// A closure that paints the app's picture where a control places it: the control calls it from its paint with its
/// content painter and the placement `A` (the [`Navigator`](super::Navigator)'s fitted rect, the
/// [`RasterDisplay`](super::RasterDisplay)'s canvas view). The app adds what it likes to the painter, a GPU paint
/// callback included; the control's own parts paint before and after it.
pub struct AppPaint<'a, A>(AppPaintFn<'a, A>);

impl<'a, A> AppPaint<'a, A> {
    /// A painter that draws the picture with `paint`, given the content painter and the placement.
    pub fn new(paint: impl FnMut(&ContentPainterCx<'_>, A) + 'a) -> Self {
        AppPaint(Box::new(paint))
    }

    /// The same painter lent for one run of a body that may run more than once per pass.
    pub fn reborrowed(&mut self) -> AppPaint<'_, A> {
        let paint = &mut self.0;
        AppPaint::new(move |painter, at| paint(painter, at))
    }

    /// Paints the picture with `painter` at `at`.
    pub fn paint(&mut self, painter: &ContentPainterCx<'_>, at: A) {
        (self.0)(painter, at);
    }
}
