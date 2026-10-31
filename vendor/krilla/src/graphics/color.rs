//! Dealing with colors and color spaces.
//!
//! # Color spaces
//!
//! krilla currently supports five color models:
//! - RGB
//! - Luma
//! - CMYK
//! - Separation (also known as Spot)
//! - DeviceN (several process and spot colorants at once)
//!
//! Each color space is associated with its specific color type, which you can use to create new
//! instances of a specific color in that color space.
//!
//! # Representation of colors
//!
//! When specifying colors in the process color spaces RGB, Luma, and CMYK, it is important
//! to understand the distinction between device-dependent and decide-independent color
//! specification. What follows is only a very brief explanation, if you want to dive into
//! more details, please look for appropriate resources on the web.
//!
//! When specifying colors in a *device-dependent way*, if I instruct the program to draw
//! the RGB color (145, 120, 45), then the program will use these literal values to activate
//! the R/G/B lights to achieve displaying a certain color. The problem is that specifying
//! colors in such a way can lead to slightly different results when actually displaying it,
//! depending on the screen that is used, since each screen is calibrated differently and
//! based on different display technologies. This is especially critical for printers, where
//! different values for CMYK colors might result in different-looking colors when being printed.
//!
//! This is why there is also the option to specify colors in a *device-independent* way,
//! which basically means that the color value (145, 120, 45) is represented in a well-specified
//! color space, and each device can then convert the colors to their native color space
//! so that they match the representation in the given color space as closely as possible.
//! This should lead to a more accurate color representation across different screens.
//!
//! In 90% of the cases, it is totally fine to just use a device-dependent colorspace, and it's
//! what krilla does by default. However, if you do care about that, then you can set the
//! `no_device_cs` property of [`SerializeSettings`] to true, in which case krilla will embed an ICC profile for the
//! sGrey and sRGB color spaces (for Luma and RGB colors, respectively). If a CMYK profile
//! was provided to the serialize settings, this will be used for CMYK colors. Otherwise,
//! it will fall back to device CMYK.
//!
//! # Separations
//!
//! An alternative way to achieve exact color reproduction in print are Separation colors.
//! Using these colors, you can request a specific colorant through its well-known name
//! (e.g. through a color registry such as PANTONE or RAL). Your production shop can then
//! supply that exact pigment.
//!
//! It is important to recognize that, when viewed on a computer or printed on a home printer,
//! the requested colorant won't be available. For that reason, you will need to supply a fallback
//! process color.
//!
//! Pay attention to remain consistent: Both in your use of colorant names and fallback colors. A
//! given colorant should not be referred to by multiple, slightly different names. Just the same
//! for fallback colors: For a given colorant, there should be only one fallback color. Using multiple
//! Separation color spaces with the same colorant and distinct fallback colors is considered a bad
//! practice and forbidden in PDF/A-2 and later. DeviceN spot colorants count too: a spot's
//! CMYK alternate in a DeviceN color space must match its Separation fallback.
//!
//! # DeviceN
//!
//! A [`device_n::DeviceNSpace`] paints several colorants with one color, for example
//! 60% Cyan and 40% Black without touching Magenta and Yellow, or a process ink mixed
//! with a spot. Its alternate space is DeviceCMYK, or the `cmyk_profile` ICC space with
//! `no_device_cs`. The tint transform is a PostScript calculator (Type 4) function, which
//! some export formats forbid.
//!
//! [`SerializeSettings`]: crate::SerializeSettings

use std::fmt::Debug;
use std::hash::Hash;

use crate::configure::ValidationError;
use crate::graphics::icc::ICCBasedColorSpace;
use crate::serialize::SerializeContext;

/// The PDF name for the device RGB color space.
pub(crate) const DEVICE_RGB: &str = "DeviceRGB";
/// The PDF name for the device gray color space.
pub(crate) const DEVICE_GRAY: &str = "DeviceGray";
/// The PDF name for the device CMYK color space.
pub(crate) const DEVICE_CMYK: &str = "DeviceCMYK";

/// A wrapper for storing colors from different color spaces.
#[derive(Debug, Hash, Eq, PartialEq, Clone)]
pub enum Color {
    /// A device or CIE-based color.
    Regular(RegularColor),
    /// A special color space.
    Special(SpecialColor),
}

/// A device or CIE-based color.
#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
pub enum RegularColor {
    /// An RGB-based color.
    Rgb(rgb::Color),
    /// A luma-based color.
    Luma(luma::Color),
    /// A device CMYK color.
    Cmyk(cmyk::Color),
}

/// A special color space color.
#[derive(Debug, Hash, Eq, PartialEq, Clone)]
pub enum SpecialColor {
    /// A separation color.
    Separation(separation::Color),
    /// A DeviceN color.
    DeviceN(device_n::Color),
}

impl Color {
    pub(crate) fn to_pdf_color(&self) -> Vec<f32> {
        match self {
            Color::Regular(RegularColor::Rgb(rgb)) => rgb.to_pdf_color().to_vec(),
            Color::Regular(RegularColor::Luma(l)) => vec![l.to_pdf_color()],
            Color::Regular(RegularColor::Cmyk(cmyk)) => cmyk.to_pdf_color().to_vec(),
            Color::Special(SpecialColor::Separation(spot)) => vec![spot.to_pdf_color()],
            Color::Special(SpecialColor::DeviceN(c)) => c.to_pdf_color(),
        }
    }

    pub(crate) fn color_space(&self, sc: &mut SerializeContext) -> ColorSpace {
        match self {
            Color::Regular(c) => c.color_space(sc).into(),
            Color::Special(c) => c.color_space().into(),
        }
    }

    /// Convert a color to a regular color for use with constructs like tags or
    /// annotations that don't support special color spaces
    pub(crate) fn to_regular(&self) -> RegularColor {
        match self {
            Color::Regular(c) => *c,
            Color::Special(SpecialColor::Separation(c)) => c.space.fallback,
            Color::Special(SpecialColor::DeviceN(c)) => c.alternate().into(),
        }
    }
}

impl RegularColor {
    pub(crate) fn color_space(&self, sc: &mut SerializeContext) -> RegularColorSpace {
        match self {
            Self::Rgb(r) => r.color_space(sc.serialize_settings().no_device_cs),
            Self::Luma(_) => luma::color_space(sc.serialize_settings().no_device_cs),
            Self::Cmyk(_) => match cmyk::color_space(&sc.serialize_settings()) {
                None => {
                    sc.register_validation_error(ValidationError::MissingCMYKProfile);
                    DeviceColorSpace::Cmyk.into()
                }
                Some(cs) => cs,
            },
        }
    }

    /// Return the current color as RGB for use with colored glyphs (SVG and
    /// COLR).
    pub(crate) fn as_rgb(self) -> Option<rgb::Color> {
        Some(match self {
            Self::Rgb(r) => r,
            Self::Luma(l) => rgb::Color::new(l.0, l.0, l.0),
            Self::Cmyk(_) => return None,
        })
    }

    /// Returns true if this is a subtractive color space (CMYK), false otherwise (RGB, Luma).
    /// Used for determining the correct tint transform behavior in Separation color spaces.
    pub(crate) fn is_subtractive(self) -> bool {
        matches!(self, Self::Cmyk(_))
    }
}

impl SpecialColor {
    pub(crate) fn color_space(&self) -> SpecialColorSpace {
        match self {
            Self::Separation(spot) => spot.color_space().into(),
            Self::DeviceN(c) => SpecialColorSpace::DeviceN(c.space.clone()),
        }
    }
}

/// Gray-scale colors.
pub mod luma {
    use crate::color::{CieBasedColorSpace, DeviceColorSpace, RegularColor, RegularColorSpace};

    /// A luma color.
    #[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
    pub struct Color(pub(crate) u8);

    impl Color {
        /// Create a new luma color.
        pub fn new(lightness: u8) -> Color {
            Color(lightness)
        }

        /// Create a black luma color.
        pub fn black() -> Self {
            Self::new(0)
        }

        /// Create a white RGB color.
        pub fn white() -> Self {
            Self::new(255)
        }

        pub(crate) fn to_pdf_color(self) -> f32 {
            self.0 as f32 / 255.0
        }
    }

    impl From<Color> for super::RegularColor {
        fn from(val: Color) -> Self {
            super::RegularColor::Luma(val)
        }
    }

    impl From<Color> for super::Color {
        fn from(val: Color) -> Self {
            RegularColor::from(val).into()
        }
    }

    impl Default for Color {
        fn default() -> Self {
            Color::new(0)
        }
    }

    pub(crate) fn color_space(no_device_cs: bool) -> RegularColorSpace {
        if no_device_cs {
            CieBasedColorSpace::Luma.into()
        } else {
            DeviceColorSpace::Gray.into()
        }
    }
}

impl From<RegularColor> for Color {
    fn from(value: RegularColor) -> Self {
        Self::Regular(value)
    }
}

impl From<SpecialColor> for Color {
    fn from(value: SpecialColor) -> Self {
        Self::Special(value)
    }
}

/// CMYK colors.
pub mod cmyk {
    use crate::color::{CieBasedColorSpace, DeviceColorSpace, RegularColorSpace};
    use crate::graphics::icc::ICCBasedColorSpace;
    use crate::SerializeSettings;

    /// A CMYK color.
    #[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
    pub struct Color(pub(crate) u8, pub(crate) u8, pub(crate) u8, pub(crate) u8);

    impl Color {
        /// Create a new CMYK color.
        pub fn new(cyan: u8, magenta: u8, yellow: u8, black: u8) -> Color {
            Color(cyan, magenta, yellow, black)
        }

        pub(crate) fn to_pdf_color(self) -> [f32; 4] {
            [
                self.0 as f32 / 255.0,
                self.1 as f32 / 255.0,
                self.2 as f32 / 255.0,
                self.3 as f32 / 255.0,
            ]
        }
    }

    impl From<Color> for super::RegularColor {
        fn from(val: Color) -> Self {
            super::RegularColor::Cmyk(val)
        }
    }

    impl From<Color> for super::Color {
        fn from(val: Color) -> Self {
            super::RegularColor::from(val).into()
        }
    }

    impl Default for Color {
        fn default() -> Self {
            Color::new(0, 0, 0, 255)
        }
    }

    pub(crate) fn color_space(ss: &SerializeSettings) -> Option<RegularColorSpace> {
        if ss.no_device_cs {
            ss.clone()
                .cmyk_profile
                .map(|p| CieBasedColorSpace::Cmyk(ICCBasedColorSpace::<4>(p.clone())).into())
        } else {
            Some(DeviceColorSpace::Cmyk.into())
        }
    }
}

/// RGB colors.
pub mod rgb {
    use crate::color::{CieBasedColorSpace, DeviceColorSpace, RegularColorSpace};

    /// An RGB color.
    #[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
    pub struct Color(pub(crate) u8, pub(crate) u8, pub(crate) u8);

    impl Default for Color {
        fn default() -> Self {
            Color::black()
        }
    }

    impl Color {
        /// Create a new RGB color.
        pub fn new(red: u8, green: u8, blue: u8) -> Self {
            Color(red, green, blue)
        }

        /// Create a new linear RGB color.
        pub fn new_linear(red: u8, green: u8, blue: u8) -> Self {
            Color(red, green, blue)
        }

        /// Create a black RGB color.
        pub fn black() -> Self {
            Self::new(0, 0, 0)
        }

        /// Create a white RGB color.
        pub fn white() -> Self {
            Self::new(255, 255, 255)
        }

        /// The `red` component of the color.
        pub fn red(&self) -> u8 {
            self.0
        }

        /// The `green` component of the color.
        pub fn green(&self) -> u8 {
            self.1
        }

        /// The `blue` component of the color.
        pub fn blue(&self) -> u8 {
            self.2
        }

        pub(crate) fn to_pdf_color(self) -> [f32; 3] {
            [
                self.0 as f32 / 255.0,
                self.1 as f32 / 255.0,
                self.2 as f32 / 255.0,
            ]
        }

        pub(super) fn color_space(&self, no_device_cs: bool) -> RegularColorSpace {
            color_space(no_device_cs)
        }
    }

    impl From<Color> for super::RegularColor {
        fn from(val: Color) -> Self {
            super::RegularColor::Rgb(val)
        }
    }

    impl From<Color> for super::Color {
        fn from(val: Color) -> Self {
            super::RegularColor::from(val).into()
        }
    }

    pub(crate) fn color_space(no_device_cs: bool) -> RegularColorSpace {
        if no_device_cs {
            CieBasedColorSpace::Srgb.into()
        } else {
            DeviceColorSpace::Rgb.into()
        }
    }
}

/// Separation (spot) colors.
pub mod separation {
    use crate::color::RegularColor;

    /// A spot color.
    #[derive(Debug, Hash, Eq, PartialEq, Clone)]
    pub struct Color {
        pub(crate) tint: u8,
        pub(crate) space: SeparationSpace,
    }

    impl Color {
        /// Create a new spot color.
        pub fn new(tint: u8, space: SeparationSpace) -> Self {
            Self { tint, space }
        }

        pub(crate) fn to_pdf_color(&self) -> f32 {
            self.tint as f32 / 255.0
        }

        pub(crate) fn color_space(&self) -> SeparationSpace {
            self.space.clone()
        }
    }

    impl Default for Color {
        fn default() -> Self {
            Color::new(0, SeparationSpace::default())
        }
    }

    impl From<Color> for super::SpecialColor {
        fn from(val: Color) -> Self {
            super::SpecialColor::Separation(val)
        }
    }

    impl From<Color> for super::Color {
        fn from(val: Color) -> Self {
            super::SpecialColor::from(val).into()
        }
    }

    /// A Separation color space (also known as spot colors).
    ///
    /// Separation color spaces use a single, subtractive colorant and allow
    /// achieving exact color reproduction in print.
    ///
    /// Krilla automatically linearly scales the fallback color with the separation
    /// tint.
    #[derive(Debug, Eq, PartialEq, Hash, Clone)]
    pub struct SeparationSpace {
        pub(crate) colorant: SeparationColorant,
        pub(crate) fallback: RegularColor,
    }

    impl SeparationSpace {
        /// Create a new Separation space.
        ///
        /// To export PDF/A-2 and later, make sure that a single Separation Colorant
        /// is always used with the same fallback color.
        pub fn new(colorant: SeparationColorant, fallback: RegularColor) -> Self {
            Self { colorant, fallback }
        }
    }

    impl From<SeparationSpace> for super::SpecialColorSpace {
        fn from(value: SeparationSpace) -> Self {
            Self::Separation(value)
        }
    }

    impl Default for SeparationSpace {
        fn default() -> Self {
            Self {
                colorant: SeparationColorant::default(),
                fallback: super::rgb::Color::default().into(),
            }
        }
    }

    /// What colorant to use for colors in this space.
    #[derive(Debug, Eq, PartialEq, Hash, Clone, Default)]
    pub enum SeparationColorant {
        /// Don't apply colorant at all. Sometimes used to indicate other production
        /// info, such as cuts.
        #[default]
        NoColorant,
        /// Apply the same amount of each available colorants.
        AllColorants,
        /// Specify a colorant name.
        Custom(String),
    }

    impl SeparationColorant {
        pub(crate) fn to_pdf<'a>(&'a self) -> pdf_writer::Name<'a> {
            match self {
                Self::AllColorants => pdf_writer::Name(b"All"),
                Self::NoColorant => pdf_writer::Name(b"None"),
                Self::Custom(s) => pdf_writer::Name(s.as_bytes()),
            }
        }
    }
}

/// DeviceN colors: several process and spot colorants painted by one color.
pub mod device_n {
    use std::sync::Arc;

    use crate::color::cmyk;

    /// A process colorant.
    #[derive(Debug, Eq, PartialEq, Hash, Clone, Copy)]
    pub enum Process {
        /// Cyan.
        Cyan,
        /// Magenta.
        Magenta,
        /// Yellow.
        Yellow,
        /// Black.
        Black,
    }

    impl Process {
        /// The process colorants in DeviceCMYK order.
        pub const ALL: [Process; 4] = [Self::Cyan, Self::Magenta, Self::Yellow, Self::Black];

        /// The PDF colorant name.
        pub fn name(self) -> &'static str {
            match self {
                Self::Cyan => "Cyan",
                Self::Magenta => "Magenta",
                Self::Yellow => "Yellow",
                Self::Black => "Black",
            }
        }

        fn index(self) -> usize {
            self as usize
        }
    }

    /// One colorant of a DeviceN color space.
    #[derive(Debug, Eq, PartialEq, Hash, Clone)]
    pub enum Colorant {
        /// A process colorant, painted on its own CMYK plate.
        Process(Process),
        /// A spot colorant with the CMYK color that stands in for it at 100% tint.
        ///
        /// To export PDF/A-2 and later, use the same alternate wherever the name appears,
        /// including as a Separation fallback.
        Spot {
            /// The colorant name.
            name: String,
            /// The CMYK color of a 100% tint.
            alternate: cmyk::Color,
        },
    }

    impl Colorant {
        /// The PDF colorant name.
        pub fn name(&self) -> &str {
            match self {
                Self::Process(p) => p.name(),
                Self::Spot { name, .. } => name,
            }
        }

        /// The CMYK components (0–1) a 100% tint of this colorant contributes.
        pub(crate) fn alternate(&self) -> [f32; 4] {
            match self {
                Self::Process(p) => {
                    let mut out = [0.0; 4];
                    out[p.index()] = 1.0;
                    out
                }
                Self::Spot { alternate, .. } => alternate.to_pdf_color(),
            }
        }
    }

    /// A DeviceN color space: an ordered list of colorants.
    ///
    /// A color's tints map to the alternate CMYK space component by component: each
    /// colorant contributes its tint times its CMYK alternate, and the contributions
    /// combine as `1 - (1 - a)(1 - b)…`. Process colorants contribute only to their own
    /// component, so a space of process colorants alone maps tints directly.
    #[derive(Debug, Eq, PartialEq, Hash, Clone)]
    pub struct DeviceNSpace {
        pub(crate) colorants: Arc<[Colorant]>,
    }

    impl DeviceNSpace {
        /// The largest number of colorants PDF allows in a DeviceN color space.
        pub const MAX_COLORANTS: usize = 32;

        /// Create a DeviceN color space with the colorants in this order.
        ///
        /// Returns `None` without colorants, with more than [`Self::MAX_COLORANTS`], with
        /// a name used twice, or with a spot named like a process colorant, `All`, `None`
        /// or the empty string.
        pub fn new(colorants: impl IntoIterator<Item = Colorant>) -> Option<Self> {
            let colorants: Arc<[Colorant]> = colorants.into_iter().collect();
            if colorants.is_empty() || colorants.len() > Self::MAX_COLORANTS {
                return None;
            }
            for (i, colorant) in colorants.iter().enumerate() {
                let name = colorant.name();
                if let Colorant::Spot { .. } = colorant {
                    let reserved = ["", "All", "None"].contains(&name)
                        || Process::ALL.iter().any(|p| p.name() == name);
                    if reserved {
                        return None;
                    }
                }
                if colorants[..i].iter().any(|c| c.name() == name) {
                    return None;
                }
            }
            Some(Self { colorants })
        }

        /// The colorants in color component order.
        pub fn colorants(&self) -> &[Colorant] {
            &self.colorants
        }

        pub(crate) fn has_process(&self) -> bool {
            self.colorants
                .iter()
                .any(|c| matches!(c, Colorant::Process(_)))
        }

        pub(crate) fn spots(&self) -> impl Iterator<Item = (&str, cmyk::Color)> {
            self.colorants.iter().filter_map(|c| match c {
                Colorant::Spot { name, alternate } => Some((name.as_str(), *alternate)),
                Colorant::Process(_) => None,
            })
        }

        /// The alternate CMYK components (0–1) for tints (0–1), as the tint transform
        /// computes them.
        pub(crate) fn alternate(&self, tints: &[f32]) -> [f32; 4] {
            let mut out = [0.0; 4];
            for (component, value) in out.iter_mut().enumerate() {
                let blank: f32 = self
                    .colorants
                    .iter()
                    .zip(tints)
                    .map(|(c, t)| 1.0 - t * c.alternate()[component])
                    .product();
                *value = 1.0 - blank;
            }
            out
        }
    }

    impl From<DeviceNSpace> for super::SpecialColorSpace {
        fn from(value: DeviceNSpace) -> Self {
            Self::DeviceN(value)
        }
    }

    /// A DeviceN color: one tint per colorant of its space.
    #[derive(Debug, Hash, Eq, PartialEq, Clone)]
    pub struct Color {
        pub(crate) tints: Vec<u8>,
        pub(crate) space: DeviceNSpace,
    }

    impl Color {
        /// Create a DeviceN color, with the tints (0 = no ink, 255 = full ink) in the
        /// order of the space's colorants.
        ///
        /// Returns `None` unless there is one tint per colorant.
        pub fn new(tints: Vec<u8>, space: DeviceNSpace) -> Option<Self> {
            (tints.len() == space.colorants.len()).then_some(Self { tints, space })
        }

        pub(crate) fn to_pdf_color(&self) -> Vec<f32> {
            self.tints.iter().map(|t| *t as f32 / 255.0).collect()
        }

        /// The color in the alternate CMYK space.
        pub(crate) fn alternate(&self) -> cmyk::Color {
            let [c, m, y, k] = self
                .space
                .alternate(&self.to_pdf_color())
                .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8);
            cmyk::Color::new(c, m, y, k)
        }
    }

    impl From<Color> for super::SpecialColor {
        fn from(val: Color) -> Self {
            super::SpecialColor::DeviceN(val)
        }
    }

    impl From<Color> for super::Color {
        fn from(val: Color) -> Self {
            super::SpecialColor::from(val).into()
        }
    }
}

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub(crate) enum ColorSpace {
    Device(DeviceColorSpace),
    CieBased(CieBasedColorSpace),
    Special(SpecialColorSpace),
}

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub(crate) enum RegularColorSpace {
    Device(DeviceColorSpace),
    CieBased(CieBasedColorSpace),
}

impl From<RegularColorSpace> for ColorSpace {
    fn from(value: RegularColorSpace) -> Self {
        match value {
            RegularColorSpace::Device(s) => Self::Device(s),
            RegularColorSpace::CieBased(s) => Self::CieBased(s),
        }
    }
}

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub(crate) enum DeviceColorSpace {
    Rgb,
    Gray,
    Cmyk,
}

impl From<DeviceColorSpace> for ColorSpace {
    fn from(value: DeviceColorSpace) -> Self {
        Self::Device(value)
    }
}

impl From<DeviceColorSpace> for RegularColorSpace {
    fn from(value: DeviceColorSpace) -> Self {
        Self::Device(value)
    }
}

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub(crate) enum CieBasedColorSpace {
    Srgb,
    Luma,
    Cmyk(ICCBasedColorSpace<4>),
}

impl From<CieBasedColorSpace> for ColorSpace {
    fn from(value: CieBasedColorSpace) -> Self {
        Self::CieBased(value)
    }
}

impl From<CieBasedColorSpace> for RegularColorSpace {
    fn from(value: CieBasedColorSpace) -> Self {
        Self::CieBased(value)
    }
}

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub(crate) enum SpecialColorSpace {
    /// A Separation color space with its colorant and fallback.
    Separation(separation::SeparationSpace),
    /// A DeviceN color space with its colorants.
    DeviceN(device_n::DeviceNSpace),
}

impl From<SpecialColorSpace> for ColorSpace {
    fn from(value: SpecialColorSpace) -> Self {
        Self::Special(value)
    }
}
