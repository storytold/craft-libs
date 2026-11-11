//! The canvas reach under a modal layer, as a control asks for it (15.5, 8.3); the runtime side is
//! `runtime::sampler`.

use super::{BodyCx, Component, Cx};
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
/// implemented for `Cx`, `BodyCx` and so for `FormCx`, which derefs to `BodyCx`. The registration holds for this pass
/// and the next and belongs to the modal layer whose body calls it: a modal opened over that layer blocks the canvas.
pub trait Sampler: sealed::Host {
    /// Arms a sampler for `canvas` (the canvas component's id): while the modal layer is open, primary presses on the
    /// canvas pass through it to the canvas, and so does the view navigation (`keep_view_open`). The arming control
    /// calls it in every pass while it is armed.
    fn arm_sampler(&mut self, canvas: Id) {
        sampler::register(self.egui(), canvas, Reach::Sample);
    }

    /// Keeps the view of `canvas` (the canvas component's id) usable under the modal layer: the wheel scrolls and
    /// zooms it, and a middle press pans it, as in Photoshop's Color Picker and Levels dialogs; Space held over the
    /// canvas adds the primary press for the pan. Its other input stays blocked. A dialog body calls it in every pass
    /// while the dialog is open.
    fn keep_view_open(&mut self, canvas: Id) {
        sampler::register(self.egui(), canvas, Reach::View);
    }
}

impl<C: Component> Sampler for Cx<'_, C> {}

impl Sampler for BodyCx<'_> {}
