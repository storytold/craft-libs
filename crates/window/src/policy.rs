//! The frame policy: who draws the title bar, decided from the OS, desktop, backend, the user's
//! "Use system title bar" setting and the `<PREFIX>_TITLEBAR` override.
//!
//! The app's own bar is the title bar by default ([`Frame::Integrated`]); the setting turns that
//! off ([`Frame::Native`]).
//!
//! | Platform, desktop, backend | Default | With "Use system title bar" |
//! |---|---|---|
//! | macOS | `Inset` | `Native` |
//! | Windows | `Integrated(Windows)` | `Native` |
//! | GNOME, Wayland or XWayland | `Integrated(Adwaita)`, `button-layout` order | `Native` |
//! | KDE, Wayland or X11 | `Integrated(Breeze)`, KWin's order | `Native` |
//! | COSMIC, Xfce, Cinnamon, MATE, other floating desktops | `Integrated(Generic)`, or Adwaita or Breeze when the GTK theme is one of those | `Native` |
//! | Tiling compositors and window managers | `Native` | `Native` |
//! | Wayland without server-side decorations, not GNOME | `Integrated(Generic)` | `Native` |
//! | Web | `Native` (no frame code runs) | `Native` |
//!
//! ```ignore
//! let d = craft_window::policy::decide_for(Prefix::default(), Backend::Wayland, TitleBarPref::Integrated);
//! let viewport = craft_window::policy::apply(egui::ViewportBuilder::default(), &d.frame);
//! ```

pub use crate::desktop::{Backend, Desktop, DesktopKind};
use crate::{Layout, Prefix, layout};

/// Whether KDE, COSMIC, Xfce, Cinnamon, MATE and X11 window managers, which draw their own frame,
/// get the app's bar by default. Set to `false` for [`Frame::Native`] on those desktops instead;
/// nothing else changes.
const INTEGRATED_ON_SSD_DESKTOPS: bool = true;

/// The user setting "Use system title bar".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TitleBarPref {
    /// The app's bar is the title bar.
    #[default]
    Integrated,
    /// The OS, window manager or compositor draws it.
    System,
}

/// The operating system family. Linux stands for Linux and the BSDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
    Web,
}

impl Os {
    /// The OS this build runs on.
    pub const fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_arch = "wasm32") {
            Os::Web
        } else {
            Os::Linux
        }
    }
}

/// The look of the caption buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionFamily {
    Windows,
    Adwaita,
    Breeze,
    Generic,
}

impl CaptionFamily {
    /// A family by name (`windows`, `adwaita`, `breeze`, `generic`), case-insensitive.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "windows" => Some(Self::Windows),
            "adwaita" => Some(Self::Adwaita),
            "breeze" => Some(Self::Breeze),
            "generic" => Some(Self::Generic),
            _ => None,
        }
    }
}

/// Caption buttons the app draws: their look and which end of the bar they sit at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Captions {
    pub family: CaptionFamily,
    pub layout: Layout,
}

/// Where macOS puts the traffic lights over the app's bar, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MacInset {
    /// Width reserved at the leading edge for the lights.
    pub leading: f32,
    /// Distance from the window's top to the lights' vertical centre.
    pub centre_from_top: f32,
}

impl Default for MacInset {
    /// The standard lights' geometry.
    fn default() -> Self {
        Self { leading: 78.0, centre_from_top: 14.0 }
    }
}

/// Who draws the title bar.
#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
    /// The OS, window manager or compositor draws it; the app's strip sits below it.
    Native,
    /// macOS: full-size content view and hidden title; the traffic lights sit over the app's bar.
    Inset(MacInset),
    /// Decorations are off; the app's bar is the title bar, with these caption buttons.
    Integrated(Captions),
}

/// The decided frame and how it was decided.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub frame: Frame,
    /// `Some(true)` when `<PREFIX>_TITLEBAR` forced the app's bar, `Some(false)` when it forced the
    /// system's, `None` when the platform and setting decided.
    pub overridden: Option<bool>,
    /// The desktop the decision was made for, so an app can hide the "Use system title bar" row
    /// where it changes nothing (tiling compositors, web) without probing again.
    pub desktop: Desktop,
}

/// The decision, pure. `env` is the value of `<PREFIX>_TITLEBAR` (`app`, `system` or `desktop`);
/// it wins over `pref`. Caption layouts are each family's default; [`decide_for`] fills in the
/// desktop's own.
pub fn decide(os: Os, backend: Backend, desktop: &Desktop, pref: TitleBarPref, env: Option<&str>) -> Decision {
    decide_with(os, backend, desktop, pref, env, &layout::default_layout)
}

/// `layout` supplies the caption layout for a desktop kind; it is called only when the frame comes
/// out integrated.
fn decide_with(
    os: Os,
    backend: Backend,
    desktop: &Desktop,
    pref: TitleBarPref,
    env: Option<&str>,
    layout: &dyn Fn(DesktopKind) -> Layout,
) -> Decision {
    let overridden = crate::parse_override(env);
    let integrated = match (overridden, pref) {
        (Some(app), _) => app,
        (None, pref) => pref == TitleBarPref::Integrated,
    };
    let frame =
        if integrated { integrated_frame(os, backend, desktop, overridden.is_some(), INTEGRATED_ON_SSD_DESKTOPS, layout) } else { Frame::Native };
    Decision { frame, overridden, desktop: desktop.clone() }
}

/// The frame when the app's bar is wanted. `forced` is set when the override asked for it, which
/// also applies on tiling compositors.
/// `integrated_on_ssd` says whether desktops that draw their own frame get the app's bar.
fn integrated_frame(
    os: Os,
    backend: Backend,
    desktop: &Desktop,
    forced: bool,
    integrated_on_ssd: bool,
    layout: &dyn Fn(DesktopKind) -> Layout,
) -> Frame {
    let captions = |family: CaptionFamily| {
        let kind = if family == CaptionFamily::Windows { DesktopKind::Other } else { desktop.kind };
        Frame::Integrated(Captions { family, layout: layout(kind) })
    };
    match os {
        Os::Web => Frame::Native,
        Os::Mac => Frame::Inset(MacInset::default()),
        Os::Windows => captions(CaptionFamily::Windows),
        Os::Linux => {
            let no_ssd = backend == Backend::Wayland && desktop.ssd == Some(false);
            match desktop.kind {
                DesktopKind::Tiling if !forced => Frame::Native,
                DesktopKind::Tiling => captions(layout::family_from_gtk_theme(desktop.gtk_theme.as_deref())),
                DesktopKind::Gnome => captions(CaptionFamily::Adwaita),
                // Open decision 1: the line below is the only place that picks the app's bar on
                // desktops that draw their own frame. `false` gives `Native` there.
                _ if !no_ssd && !integrated_on_ssd && !forced => Frame::Native,
                DesktopKind::Kde => captions(CaptionFamily::Breeze),
                DesktopKind::Other if no_ssd => captions(CaptionFamily::Generic),
                DesktopKind::Other => captions(layout::family(DesktopKind::Other, desktop.gtk_theme.as_deref())),
            }
        }
    }
}

/// Read the platform once (500 ms at most, as for [`crate::Choice::probe`]) and decide.
/// `prefix` names the override variable `<PREFIX>_TITLEBAR`; when it is unset, `CRAFT_TITLEBAR`
/// applies. `backend` is the one the app opens its window on, [`Backend::from_env`]
/// unless the app forces X11.
pub fn decide_for(prefix: Prefix, backend: Backend, pref: TitleBarPref) -> Decision {
    let os = Os::current();
    let env = std::env::var(prefix.var_name()).or_else(|_| std::env::var(Prefix::default().var_name())).ok();
    let desktop =
        if os == Os::Linux { Desktop::detect(backend) } else { Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None } };
    decide_with(os, backend, &desktop, pref, env.as_deref(), &|kind| desktop_layout(os, kind))
}

/// The desktop's window-button layout, where it names one.
fn desktop_layout(os: Os, kind: DesktopKind) -> Layout {
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    if os == Os::Linux {
        return layout::read_layout(kind, &|k: &str| std::env::var(k).ok());
    }
    let _ = os;
    layout::default_layout(kind)
}

/// Apply a [`Frame`] to an [`egui::ViewportBuilder`] before the window is created: decorations off
/// for [`Frame::Integrated`], a full-size content view with hidden title and transparent title bar
/// for [`Frame::Inset`], decorations on for [`Frame::Native`].
#[cfg(feature = "egui")]
pub fn apply(builder: egui::ViewportBuilder, frame: &Frame) -> egui::ViewportBuilder {
    match frame {
        Frame::Native => builder.with_decorations(true),
        Frame::Inset(_) => builder.with_decorations(true).with_fullsize_content_view(true).with_title_shown(false).with_titlebar_shown(false),
        Frame::Integrated(_) => builder.with_decorations(false),
    }
}

impl From<&crate::Frame> for Frame {
    /// The old two-way frame as a new one: `Desktop` is `Native`, `App(layout)` is integrated
    /// with generic captions.
    fn from(old: &crate::Frame) -> Self {
        match old {
            crate::Frame::Desktop => Frame::Native,
            crate::Frame::App(layout) => Frame::Integrated(Captions { family: CaptionFamily::Generic, layout: layout.clone() }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CaptionFamily::{Adwaita, Breeze, Generic, Windows};
    use TitleBarPref::{Integrated as Int, System};

    fn desk(kind: DesktopKind, ssd: Option<bool>, theme: Option<&str>) -> Desktop {
        Desktop { kind, ssd, gtk_theme: theme.map(str::to_string), icon_theme: None }
    }

    fn family_of(d: &Decision) -> Option<CaptionFamily> {
        match &d.frame {
            Frame::Integrated(c) => Some(c.family),
            _ => None,
        }
    }

    #[test]
    fn every_row_of_the_table() {
        use DesktopKind::{Gnome, Kde, Other, Tiling};
        // (os, backend, desktop, expected family; None = Native)
        let rows: Vec<(Os, Backend, Desktop, Option<CaptionFamily>)> = vec![
            (Os::Windows, Backend::Wayland, desk(Other, None, None), Some(Windows)),
            (Os::Linux, Backend::Wayland, desk(Gnome, Some(false), None), Some(Adwaita)),
            (Os::Linux, Backend::X11, desk(Gnome, None, Some("Arc")), Some(Adwaita)),
            (Os::Linux, Backend::Wayland, desk(Kde, Some(true), None), Some(Breeze)),
            (Os::Linux, Backend::X11, desk(Kde, None, None), Some(Breeze)),
            (Os::Linux, Backend::Wayland, desk(Other, Some(true), Some("Adwaita")), Some(Adwaita)),
            (Os::Linux, Backend::Wayland, desk(Other, Some(true), Some("Breeze-Dark")), Some(Breeze)),
            (Os::Linux, Backend::X11, desk(Other, None, Some("Mint-Y")), Some(Generic)),
            (Os::Linux, Backend::Wayland, desk(Other, Some(true), None), Some(Generic)),
            (Os::Linux, Backend::Wayland, desk(Tiling, Some(true), None), None),
            (Os::Linux, Backend::X11, desk(Tiling, None, None), None),
            (Os::Linux, Backend::Wayland, desk(Other, Some(false), Some("Adwaita")), Some(Generic)),
            (Os::Web, Backend::X11, desk(Other, None, None), None),
        ];
        for (os, backend, d, want) in rows {
            let got = decide(os, backend, &d, Int, None);
            assert_eq!(family_of(&got), want, "{os:?} {backend:?} {d:?}");
            assert_eq!(got.overridden, None);
            if want.is_none() {
                assert_eq!(got.frame, Frame::Native);
            }
            let system = decide(os, backend, &d, System, None);
            assert_eq!(system.frame, Frame::Native, "system bar: {os:?} {d:?}");
        }
    }

    #[test]
    fn native_on_ssd_desktops_when_the_flag_is_off() {
        use DesktopKind::{Gnome, Kde, Other};
        let frame = |kind, backend, ssd, forced| integrated_frame(Os::Linux, backend, &desk(kind, ssd, None), forced, false, &layout::default_layout);
        assert_eq!(frame(Kde, Backend::Wayland, Some(true), false), Frame::Native);
        assert_eq!(frame(Other, Backend::X11, None, false), Frame::Native);
        assert!(matches!(frame(Kde, Backend::Wayland, Some(true), true), Frame::Integrated(_)), "the override still forces the bar");
        assert!(matches!(frame(Gnome, Backend::Wayland, Some(false), false), Frame::Integrated(_)));
        assert!(matches!(frame(Other, Backend::Wayland, Some(false), false), Frame::Integrated(_)), "no SSD: nothing else draws a frame");
    }

    #[test]
    fn the_decision_carries_the_desktop() {
        let d = desk(DesktopKind::Tiling, Some(true), None);
        assert_eq!(decide(Os::Linux, Backend::Wayland, &d, Int, None).desktop, d);
    }

    #[test]
    fn macos_gets_the_inset() {
        let d = decide(Os::Mac, Backend::X11, &desk(DesktopKind::Other, None, None), Int, None);
        assert_eq!(d.frame, Frame::Inset(MacInset::default()));
        assert_eq!(decide(Os::Mac, Backend::X11, &desk(DesktopKind::Other, None, None), System, None).frame, Frame::Native);
    }

    #[test]
    fn the_override_wins_over_the_setting_both_ways() {
        let kde = desk(DesktopKind::Kde, Some(true), None);
        let system = decide(Os::Linux, Backend::Wayland, &kde, Int, Some("system"));
        assert_eq!((system.frame, system.overridden), (Frame::Native, Some(false)));
        let desktop = decide(Os::Linux, Backend::Wayland, &kde, Int, Some("desktop"));
        assert_eq!((desktop.frame, desktop.overridden), (Frame::Native, Some(false)));
        let app = decide(Os::Linux, Backend::Wayland, &kde, System, Some("app"));
        assert_eq!(app.overridden, Some(true));
        assert_eq!(family_of(&app), Some(Breeze));
        // `app` also forces the bar on a tiling compositor, which would otherwise be native.
        let tiling = desk(DesktopKind::Tiling, Some(true), None);
        assert_eq!(family_of(&decide(Os::Linux, Backend::Wayland, &tiling, Int, Some("app"))), Some(Generic));
        // An unknown value is no override.
        let unknown = decide(Os::Linux, Backend::Wayland, &kde, System, Some("maybe"));
        assert_eq!((unknown.frame, unknown.overridden), (Frame::Native, None));
    }

    #[test]
    fn default_layouts_follow_the_family() {
        let gnome = decide(Os::Linux, Backend::Wayland, &desk(DesktopKind::Gnome, Some(false), None), Int, None);
        assert!(matches!(gnome.frame, Frame::Integrated(c) if c.layout.right == vec![crate::Button::Close]));
        let win = decide(Os::Windows, Backend::X11, &desk(DesktopKind::Other, None, None), Int, None);
        assert!(matches!(win.frame, Frame::Integrated(c) if c.layout == Layout::default()));
    }

    #[test]
    fn the_old_frame_converts() {
        assert_eq!(Frame::from(&crate::Frame::Desktop), Frame::Native);
        assert!(matches!(Frame::from(&crate::Frame::App(Layout::default())), Frame::Integrated(_)));
        assert_eq!(CaptionFamily::parse(" Breeze "), Some(Breeze));
    }
}
