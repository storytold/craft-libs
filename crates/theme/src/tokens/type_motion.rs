//! Type roles, elevation and motion tokens.

use crate::scale::{Easing, Elevation};

/// The font a type role uses. `craft_ui::fonts` maps each slot to a face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontSlot {
    /// The interface face, regular weight.
    #[default]
    Ui,
    /// The interface face, semibold.
    UiSemibold,
    /// The monospace face.
    Mono,
}

/// Size, line height and font of one type role.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TypeStyle {
    /// Font size in points.
    pub size: f32,
    /// Line height in points.
    pub line_height: f32,
    /// Font slot (family and weight).
    pub font: FontSlot,
}

/// Names one type role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeRole {
    /// Small print, badges.
    Caption,
    /// Control labels.
    Label,
    /// Body text.
    Body,
    /// Emphasised body text.
    BodyStrong,
    /// Panel and dialog titles.
    Title,
    /// Section headings.
    Heading,
    /// Large regular text: a prominent text field.
    BodyLarge,
    /// Code, numbers in tables.
    Mono,
}

/// Every type role.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TypeTokens {
    /// Small print, badges.
    pub caption: TypeStyle,
    /// Control labels.
    pub label: TypeStyle,
    /// Body text.
    pub body: TypeStyle,
    /// Emphasised body text.
    pub body_strong: TypeStyle,
    /// Panel and dialog titles.
    pub title: TypeStyle,
    /// Section headings.
    pub heading: TypeStyle,
    /// Large regular text: a prominent text field.
    pub body_large: TypeStyle,
    /// Code, numbers in tables.
    pub mono: TypeStyle,
}

impl TypeTokens {
    /// The style of `role`.
    pub fn get(&self, role: TypeRole) -> TypeStyle {
        match role {
            TypeRole::Caption => self.caption,
            TypeRole::Label => self.label,
            TypeRole::Body => self.body,
            TypeRole::BodyStrong => self.body_strong,
            TypeRole::Title => self.title,
            TypeRole::Heading => self.heading,
            TypeRole::BodyLarge => self.body_large,
            TypeRole::Mono => self.mono,
        }
    }

    /// Every size and line height multiplied by `factor` and rounded to `step` points.
    pub(crate) fn scaled(&self, factor: f32, step: f32) -> TypeTokens {
        let round = |v: f32| (v * factor / step).round() * step;
        let s = |t: TypeStyle| TypeStyle { size: round(t.size), line_height: round(t.line_height), font: t.font };
        TypeTokens {
            caption: s(self.caption),
            label: s(self.label),
            body: s(self.body),
            body_strong: s(self.body_strong),
            title: s(self.title),
            heading: s(self.heading),
            body_large: s(self.body_large),
            mono: s(self.mono),
        }
    }
}

/// Shadows of floating surfaces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElevationTokens {
    /// Popups, menus and dialogs (PhotoCraft's dialogs are popup frames).
    pub popup: Elevation,
    /// Floating windows, panels and toolbars: PhotoCraft's `window_shadow`.
    pub floating: Elevation,
    /// Knobs on a track (`surface.knob`); level 0 in bevel themes, whose bevel draws the relief.
    pub knob: Elevation,
}

/// One animation: how long and along which curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Motion {
    /// Duration in milliseconds; 0 with reduced motion.
    pub millis: u32,
    /// Easing curve.
    pub easing: Easing,
}

/// Animations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MotionTokens {
    /// Hover and press feedback.
    pub hover: Motion,
    /// Popups, menus and panels opening.
    pub open: Motion,
    /// Elements moving within the screen.
    pub r#move: Motion,
    /// A text caret gliding to its new place (`motion.caret`).
    pub caret: Motion,
    /// A selection outline gliding to its new cells (`motion.outline`).
    pub outline: Motion,
    /// The appearance asks for reduced motion: every duration above is zero.
    pub reduced: bool,
}
