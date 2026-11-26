//! Opt-in Flat and Quiet palettes and their corner scale.
//!
//! Moved without changing the palette values from VectorCraft's theme proposal at
//! `f3b70e347a77324a5649b166ef16c1303696173a` (Taha-Mahmoodi, MIT OR Apache-2.0).
//! The original theme ids are retained. Applications choose, register and persist the schemes;
//! these definitions neither change the suite default nor supply application-specific metrics.

use crate::{AppSizes, ColorToken, ContrastIntent, EdgeSeed, ModePins, Pins, Rgba, Seeds, ShapeSeed, SizeValue, Status, ThemeDef, ThemeId, Variants};
use std::sync::{Arc, LazyLock};

/// An opt-in palette and corner scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    /// Separate flat chrome and panel surfaces.
    Flat,
    /// A shared chrome surface, inset fields and bevelled controls.
    Quiet,
}

/// The four palette tones; an application's saved brightness choice maps to one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brightness {
    /// The darkest tone.
    Dark,
    /// The medium dark tone.
    MediumDark,
    /// The medium light tone.
    MediumLight,
    /// The lightest tone.
    Light,
}

/// Stable definitions keep a host's root-token cache warm across frames.
pub fn definition(scheme: Scheme, brightness: Brightness) -> Arc<ThemeDef> {
    static FLAT_DARK: LazyLock<Arc<ThemeDef>> = LazyLock::new(|| build(Scheme::Flat, Brightness::Dark));
    static FLAT_MEDIUM_DARK: LazyLock<Arc<ThemeDef>> = LazyLock::new(|| build(Scheme::Flat, Brightness::MediumDark));
    static FLAT_MEDIUM_LIGHT: LazyLock<Arc<ThemeDef>> = LazyLock::new(|| build(Scheme::Flat, Brightness::MediumLight));
    static FLAT_LIGHT: LazyLock<Arc<ThemeDef>> = LazyLock::new(|| build(Scheme::Flat, Brightness::Light));
    static QUIET_DARK: LazyLock<Arc<ThemeDef>> = LazyLock::new(|| build(Scheme::Quiet, Brightness::Dark));
    static QUIET_MEDIUM_DARK: LazyLock<Arc<ThemeDef>> = LazyLock::new(|| build(Scheme::Quiet, Brightness::MediumDark));
    static QUIET_MEDIUM_LIGHT: LazyLock<Arc<ThemeDef>> = LazyLock::new(|| build(Scheme::Quiet, Brightness::MediumLight));
    static QUIET_LIGHT: LazyLock<Arc<ThemeDef>> = LazyLock::new(|| build(Scheme::Quiet, Brightness::Light));
    let definition = match (scheme, brightness) {
        (Scheme::Flat, Brightness::Dark) => &FLAT_DARK,
        (Scheme::Flat, Brightness::MediumDark) => &FLAT_MEDIUM_DARK,
        (Scheme::Flat, Brightness::MediumLight) => &FLAT_MEDIUM_LIGHT,
        (Scheme::Flat, Brightness::Light) => &FLAT_LIGHT,
        (Scheme::Quiet, Brightness::Dark) => &QUIET_DARK,
        (Scheme::Quiet, Brightness::MediumDark) => &QUIET_MEDIUM_DARK,
        (Scheme::Quiet, Brightness::MediumLight) => &QUIET_MEDIUM_LIGHT,
        (Scheme::Quiet, Brightness::Light) => &QUIET_LIGHT,
    };
    Arc::clone(definition)
}

/// Applies the shared corner scale to every density, preserving the host's other size overrides.
/// Call once while constructing the host's cached [`AppSizes`], not on every frame.
/// The resolver derives the small radius as half the control radius; docked panels stay square.
pub fn apply_corners(scheme: Scheme, sizes: &mut AppSizes) {
    let (control, floating) = match scheme {
        Scheme::Flat => (4.0, 8.0),
        Scheme::Quiet => (8.0, 14.0),
    };
    for map in [&mut sizes.all, &mut sizes.compact, &mut sizes.comfortable] {
        for (path, radius) in [
            ("radius.control", control),
            ("radius.button", control),
            ("radius.surface", floating),
            ("radius.window", floating),
            ("radius.panel", 0.0),
        ] {
            map.insert(path.into(), SizeValue::Number(radius));
        }
    }
}

#[derive(Clone, Copy)]
struct Palette {
    chrome: u32,
    panel: u32,
    darker: u32,
    canvas: u32,
    field: u32,
    field_edge: u32,
    border: u32,
    divider: u32,
    text: u32,
    muted: u32,
    disabled: u32,
    hover: u32,
    pressed: u32,
    selected: u32,
    button: u32,
    button_edge: u32,
    accent: u32,
    strong: u32,
    shade: u32,
}

fn flat(brightness: Brightness) -> Palette {
    let base = Palette {
        chrome: 0x1a1b1e,
        panel: 0x1f2023,
        darker: 0x1a1b1e,
        canvas: 0x2a2b2f,
        field: 0x28292d,
        field_edge: 0x313236,
        border: 0x34363b,
        divider: 0x2e3035,
        text: 0xe4e5e8,
        muted: 0x9ea2aa,
        disabled: 0x6c7079,
        hover: 0x2c2d32,
        pressed: 0x3a3c42,
        selected: 0x2a3448,
        button: 0x2d2e33,
        button_edge: 0x4a4d55,
        accent: 0x5b8def,
        strong: 0x4a7de6,
        shade: 0x313236,
    };
    match brightness {
        Brightness::Dark => Palette {
            chrome: 0x131416,
            panel: 0x18191b,
            darker: 0x131416,
            canvas: 0x222326,
            field: 0x212226,
            field_edge: 0x2a2b2f,
            border: 0x2d2f33,
            divider: 0x26282c,
            hover: 0x24252a,
            pressed: 0x33353a,
            selected: 0x242e41,
            button: 0x26272b,
            button_edge: 0x44474e,
            ..base
        },
        Brightness::MediumDark => base,
        Brightness::MediumLight => Palette {
            chrome: 0xdddfe2,
            panel: 0xe6e7ea,
            darker: 0xdddfe2,
            canvas: 0xd3d5d9,
            field: 0xdadcdf,
            field_edge: 0xc4c7cc,
            border: 0xcacdd2,
            divider: 0xcacdd2,
            text: 0x1b1d22,
            muted: 0x50555d,
            disabled: 0x80858d,
            hover: 0xdcdee2,
            pressed: 0xcacdd2,
            selected: 0xd0d8e9,
            button: 0xd5d7db,
            button_edge: 0x9a9ea6,
            accent: 0x2f6be0,
            strong: 0x2a60cc,
            ..base
        },
        Brightness::Light => Palette {
            chrome: 0xedeef0,
            panel: 0xf5f5f6,
            darker: 0xedeef0,
            canvas: 0xe4e5e8,
            field: 0xeaebed,
            field_edge: 0xd6d8dc,
            border: 0xdcdee2,
            divider: 0xdcdee2,
            text: 0x1e2025,
            muted: 0x5e636b,
            disabled: 0x8e939b,
            hover: 0xebecee,
            pressed: 0xdcdee2,
            selected: 0xdde4f3,
            button: 0xe4e5e8,
            button_edge: 0xa9adb4,
            accent: 0x2f6be0,
            strong: 0x2a60cc,
            ..base
        },
    }
}

fn quiet(brightness: Brightness) -> Palette {
    let base = Palette {
        chrome: 0x1d1e21,
        panel: 0x1d1e21,
        darker: 0x18191c,
        canvas: 0x2a2c30,
        field: 0x16171a,
        field_edge: 0x121315,
        border: 0x34363b,
        divider: 0x2b2d31,
        text: 0xe7e8ea,
        muted: 0xa0a4ab,
        disabled: 0x6f737b,
        hover: 0x27292d,
        pressed: 0x34363c,
        selected: 0x2c3447,
        button: 0x2c2e33,
        button_edge: 0x4a4d55,
        accent: 0x6e97f2,
        strong: 0x5b86e6,
        shade: 0x0e0f11,
    };
    match brightness {
        Brightness::Dark => Palette {
            chrome: 0x151618,
            panel: 0x151618,
            darker: 0x111214,
            canvas: 0x222327,
            field: 0x0f1012,
            field_edge: 0x0b0c0d,
            border: 0x2b2d31,
            divider: 0x232528,
            hover: 0x1f2023,
            pressed: 0x2b2d32,
            selected: 0x252d3f,
            button: 0x25272b,
            button_edge: 0x44474e,
            shade: 0x08090a,
            ..base
        },
        Brightness::MediumDark => base,
        Brightness::MediumLight => Palette {
            chrome: 0xe5e7ea,
            panel: 0xe5e7ea,
            darker: 0xdddfe3,
            canvas: 0xd8dade,
            field: 0xdadde1,
            field_edge: 0xd0d3d7,
            border: 0xd3d6da,
            divider: 0xd3d6da,
            text: 0x1b1e23,
            muted: 0x50555d,
            disabled: 0x80858d,
            hover: 0xdadce0,
            pressed: 0xf7f8f9,
            selected: 0xced6e8,
            button: 0xf7f8f9,
            button_edge: 0x9a9ea6,
            accent: 0x3d6fe0,
            strong: 0x3563cc,
            shade: 0xcbced2,
        },
        Brightness::Light => Palette {
            chrome: 0xf3f4f6,
            panel: 0xf3f4f6,
            darker: 0xecedf0,
            canvas: 0xe7e9ec,
            field: 0xe9ebee,
            field_edge: 0xe0e2e6,
            border: 0xe3e5e9,
            divider: 0xe3e5e9,
            text: 0x1f2328,
            muted: 0x646a73,
            disabled: 0x969ca5,
            hover: 0xe8eaed,
            pressed: 0xffffff,
            selected: 0xdde4f3,
            button: 0xffffff,
            button_edge: 0xa9adb4,
            accent: 0x3d6fe0,
            strong: 0x3563cc,
            shade: 0xd9dbdf,
        },
    }
}

fn rgb(hex: u32) -> Rgba {
    Rgba::rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn theme_id(id: &str) -> ThemeId {
    ThemeId::parse(id).unwrap_or_else(|_| ThemeId::suite_default())
}

fn build(scheme: Scheme, brightness: Brightness) -> Arc<ThemeDef> {
    use ColorToken as C;
    let is_quiet = scheme == Scheme::Quiet;
    let p = if is_quiet { quiet(brightness) } else { flat(brightness) };
    let dark = matches!(brightness, Brightness::Dark | Brightness::MediumDark);
    let id = match (is_quiet, brightness) {
        (false, Brightness::Dark) => "vectorcraft.flat-dark",
        (false, Brightness::MediumDark) => "vectorcraft.flat-medium-dark",
        (false, Brightness::MediumLight) => "vectorcraft.flat-medium-light",
        (false, Brightness::Light) => "vectorcraft.flat-light",
        (true, Brightness::Dark) => "vectorcraft.quiet-dark",
        (true, Brightness::MediumDark) => "vectorcraft.quiet-medium-dark",
        (true, Brightness::MediumLight) => "vectorcraft.quiet-medium-light",
        (true, Brightness::Light) => "vectorcraft.quiet-light",
    };
    let pins: Pins = [
        (C::SurfaceBackground, p.chrome),
        (C::SurfaceChrome, p.chrome),
        (C::SurfacePanel, p.panel),
        (C::SurfaceRaised, p.button),
        (C::SurfaceCanvas, p.canvas),
        (C::SurfacePage, p.canvas),
        (C::SurfaceSunken, p.field),
        (C::SurfaceControl, p.button),
        (C::TextNormal, p.text),
        (C::TextMuted, p.muted),
        (C::TextFaint, p.muted),
        (C::TextDisabled, p.disabled),
        (C::TextOnSelected, p.text),
        (C::LineBorder, p.border),
        (C::LineDivider, p.divider),
        (C::LineControlEdge, p.field_edge),
        (C::LineOutline, p.button_edge),
        (C::LineMark, p.button_edge),
        (C::LineFocus, p.accent),
        (C::AccentSolid, p.accent),
        (C::AccentStrong, p.strong),
        (C::AccentText, p.accent),
        (C::AccentSoft, p.selected),
        (C::AccentBorder, p.accent),
        (C::StateHover, p.hover),
        (C::StatePressed, p.pressed),
        (C::StateSelected, p.selected),
        (C::StateDisabledFill, p.darker),
        (C::EdgeLight, p.button_edge),
        (C::EdgeDark, p.shade),
    ]
    .into_iter()
    .map(|(token, hex)| (token, rgb(hex)))
    .collect();
    Arc::new(ThemeDef {
        id: theme_id(id),
        name: if is_quiet { "Quiet" } else { "Flat" }.into(),
        version: 1,
        seeds: Seeds {
            neutral: rgb(p.panel),
            accent: rgb(p.accent),
            status: Status { good: rgb(0x45af63), warn: rgb(0xf0a330), bad: rgb(0xe34850), info: rgb(p.accent) },
        },
        mode_seeds: Default::default(),
        shape: Some(ShapeSeed {
            radius: None,
            radius_offset: None,
            button_radius: None,
            edge: Some(if is_quiet { EdgeSeed::Bevel { width: 0.5 } } else { EdgeSeed::Flat }),
            knob: None,
            wells: None,
            grip: None,
            locator: None,
        }),
        variants: if dark { Variants::DarkOnly } else { Variants::LightOnly },
        pins: if dark { ModePins { dark: pins, ..Default::default() } } else { ModePins { light: pins, ..Default::default() } },
        high_contrast: Default::default(),
        contrast_intent: ContrastIntent::Standard,
        icon_pack: None,
        apps: Default::default(),
    })
}
