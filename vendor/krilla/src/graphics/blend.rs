//! Blending and overprint.

/// How to blend source and backdrop.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[allow(missing_docs)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    pub(crate) fn to_pdf(self) -> pdf_writer::types::BlendMode {
        match self {
            BlendMode::Normal => pdf_writer::types::BlendMode::Normal,
            BlendMode::Multiply => pdf_writer::types::BlendMode::Multiply,
            BlendMode::Screen => pdf_writer::types::BlendMode::Screen,
            BlendMode::Overlay => pdf_writer::types::BlendMode::Overlay,
            BlendMode::Darken => pdf_writer::types::BlendMode::Darken,
            BlendMode::Lighten => pdf_writer::types::BlendMode::Lighten,
            BlendMode::ColorDodge => pdf_writer::types::BlendMode::ColorDodge,
            BlendMode::ColorBurn => pdf_writer::types::BlendMode::ColorBurn,
            BlendMode::HardLight => pdf_writer::types::BlendMode::HardLight,
            BlendMode::SoftLight => pdf_writer::types::BlendMode::SoftLight,
            BlendMode::Difference => pdf_writer::types::BlendMode::Difference,
            BlendMode::Exclusion => pdf_writer::types::BlendMode::Exclusion,
            BlendMode::Hue => pdf_writer::types::BlendMode::Hue,
            BlendMode::Saturation => pdf_writer::types::BlendMode::Saturation,
            BlendMode::Color => pdf_writer::types::BlendMode::Color,
            BlendMode::Luminosity => pdf_writer::types::BlendMode::Luminosity,
        }
    }
}

/// Overprint settings for the graphics state (`/op`, `/OP`, `/OPM`).
///
/// With overprint on, painting in one colorant leaves the other colorants of the backdrop
/// in place on separated output instead of knocking them out. Viewers that don't simulate
/// overprint ignore it.
#[derive(Debug, Copy, Clone, Default, Eq, PartialEq, Hash)]
pub struct Overprint {
    /// Overprint for fills, including filled text (`/op`).
    pub fill: bool,
    /// Overprint for strokes, including stroked text (`/OP`).
    pub stroke: bool,
    /// Nonzero overprint mode (`/OPM 1`): a DeviceCMYK component of 0 leaves that
    /// colorant of the backdrop unchanged. With `false` (`/OPM 0`), every DeviceCMYK
    /// component paints, zero included.
    ///
    /// ISO 32000-1 (8.6.7) applies the mode to DeviceCMYK only. With
    /// [`SerializeSettings::no_device_cs`] all CMYK is ICCBased, so a PDF 1.7 reader paints
    /// zero components as with `false`, and PDF/A-2 and later reject the combination
    /// ([`ValidationError::OverprintModeIccCmyk`]).
    ///
    /// [`SerializeSettings::no_device_cs`]: crate::SerializeSettings::no_device_cs
    /// [`ValidationError::OverprintModeIccCmyk`]: crate::configure::ValidationError::OverprintModeIccCmyk
    pub nonzero: bool,
}

impl Overprint {
    pub(crate) fn to_pdf_mode(self) -> pdf_writer::types::OverprintMode {
        if self.nonzero {
            pdf_writer::types::OverprintMode::IgnoreZeroChannel
        } else {
            pdf_writer::types::OverprintMode::OverrideAllColorants
        }
    }
}

/// Attributes of a transparency group pushed with [`Surface::push_group`].
///
/// A group with neither attribute set is drawn as a plain form XObject: composited with
/// the Normal blend mode at full opacity, it looks the same as its contents drawn directly.
///
/// [`Surface::push_group`]: crate::surface::Surface::push_group
#[derive(Debug, Copy, Clone, Default, Eq, PartialEq, Hash)]
pub struct GroupOptions {
    /// Isolated group (`/I true`): its contents composite onto a fully transparent
    /// backdrop instead of the group's backdrop, so blend modes inside it don't reach
    /// what lies beneath.
    pub isolated: bool,
    /// Knockout group (`/K true`): each object inside composites with the group's
    /// initial backdrop instead of the objects before it, so where objects overlap only
    /// the topmost shows.
    pub knockout: bool,
}
