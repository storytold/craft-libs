//! The canvas reach under a modal layer, as a control asks for it (15.5, 8.3); the runtime side is
//! `runtime::sampler`.

use super::{BodyCx, Component, Cx};
pub use crate::runtime::sampler::CanvasGesture;
use crate::runtime::sampler::{self, Reach};
use egui::{Context, Id};

mod sealed {
    use super::{BodyCx, Component, Context, Cx};

    /// What the chapter needs of a context: the egui context the registration is made in.
    pub trait Host {
        fn egui(&self) -> &Context;
    }

    impl<C: Component> Host for Cx<'_, C> {
        fn egui(&self) -> &Context {
            &self.core.ctx
        }
    }

    impl Host for BodyCx<'_> {
        fn egui(&self) -> &Context {
            self.memory.ctx()
        }
    }
}

/// The canvas reach under a modal layer (15.5, 8.3): a control of a dialog keeps the canvas below it usable. It is
/// implemented for `Cx`, `BodyCx` and so for `FormCx`, which derefs to `BodyCx`. A registration holds for this pass
/// and the next and belongs to the modal layer whose body calls it: a modal opened over that layer blocks the canvas.
/// The registrations of one pass add up.
pub trait Sampler: sealed::Host {
    /// Lets `gesture` on `canvas` (the canvas component's id) through the modal layer, and the view navigation with it
    /// (`keep_view_open`): an armed eyedropper's press ([`CanvasGesture::PRIMARY`]), a dialog's drag on the image, a
    /// ⌘-click (`CanvasGesture::PRIMARY.with(Mods::CMD)`). A press that starts no named gesture stays blocked. The
    /// app runs the dialog's own tool on the canvas meanwhile. The control calls it in every pass while it takes the
    /// gesture.
    fn pass_canvas_gesture(&mut self, canvas: Id, gesture: CanvasGesture) {
        sampler::register(self.egui(), canvas, Reach::gesture(gesture));
    }

    /// Keeps the view of `canvas` (the canvas component's id) usable under the modal layer: the wheel scrolls and
    /// zooms it, and a middle press pans it, as in Photoshop's Color Picker and Levels dialogs; Space held over the
    /// canvas adds the primary press for the pan. Its other input stays blocked. A dialog body calls it in every pass
    /// while the dialog is open.
    fn keep_view_open(&mut self, canvas: Id) {
        sampler::register(self.egui(), canvas, Reach::view());
    }
}

impl<C: Component> Sampler for Cx<'_, C> {}

impl Sampler for BodyCx<'_> {}
