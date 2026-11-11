//! The Layer Style value model, without egui: the effects of a layer ([`LayerStyle`]), Photoshop's
//! ten effect tables ([`EffectSpec::photoshop`]) with their defaults, the Blending Options
//! ([`Blending`], [`BlendIf`]), contours ([`Contour`]), and the dialog's draft logic
//! ([`LayerStyleDraft`]: the page, adding and removing instances, the shared light angle, the
//! shadow drag).
//!
//! Everything here is plain data and pure functions, testable without a window. The dialog that
//! edits a draft is `craft_imaging::ui::layer_style` (the `ui` feature); the engine that renders
//! the style is the app's, which converts its own effects to and from [`LayerStyle`] by
//! [`EffectKind::key`] and the parameter keys of the tables. Colours are [`craft_theme::Srgba`];
//! contours are point lists, which the UI turns into a curve for its editor.
//!
//! ```no_run
//! use craft_imaging::layer_style::{EffectKind, EffectSpec, LayerStyle, LayerStyleDraft, LayerStylePage};
//!
//! // Layer › Layer Style › Drop Shadow…: the first Drop Shadow, turned on, or a new one.
//! let mut draft = LayerStyleDraft::new(LayerStyle::default());
//! draft.open_on(EffectKind::DropShadow);
//! assert!(matches!(draft.page, LayerStylePage::Effect(_)));
//!
//! // A drag on the image while the shadow's page shows: whole degrees and pixels.
//! let defaults = EffectSpec::photoshop(EffectKind::DropShadow).defaults;
//! draft.drag_shadow(&defaults, true, [0.0, 0.0]);
//! draft.drag_shadow(&defaults, false, [4.0, 3.0]);
//! ```

mod contour;
mod draft;
mod model;
mod shadow;
mod spec;
#[cfg(test)]
mod tests;

pub use contour::{Contour, ContourPoint};
pub use draft::LayerStyleDraft;
pub use model::{
    BlendIf, BlendIfRange, Blending, EffectId, EffectInstance, EffectKind, Knockout, LayerStyle, LayerStylePage, StyleParams, StyleValue,
};
pub use shadow::ShadowDrag;
pub use spec::{ChoiceOption, EffectField, EffectSpec, FieldKind, FieldUnit};
