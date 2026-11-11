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
//!
//! [`LayoutWatch`] keeps the caption layout current while the app runs: GNOME's value from the
//! settings portal, KWin's and GTK's files re-read when the window regains focus.

pub use crate::desktop::{Backend, Desktop, DesktopKind};
pub use crate::layout::LayoutWatch;
use crate::{Layout, Prefix, layout};
pub use craft_host::target::{Os, Runtime, WindowKind};

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

/// The look of the caption buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionFamily {
    #[allow(missing_docs)]
    Windows,
    #[allow(missing_docs)]
    Adwaita,
    #[allow(missing_docs)]
    Breeze,
    #[allow(missing_docs)]
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
    /// The buttons' look.
    pub family: CaptionFamily,
    /// Which buttons go on which end.
    pub layout: Layout,
}

impl Captions {
    /// Whether no button is shown.
    pub fn is_empty(&self) -> bool {
        self.layout.left.is_empty() && self.layout.right.is_empty()
    }
}

/// Where macOS puts the traffic lights over the app's bar, in points. [`crate::mac`] measures the
/// real lights and has the per-version defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MacInset {
    /// Width reserved at the leading edge for the lights.
    pub leading: f32,
    /// Distance from the window's top to the lights' vertical centre.
    pub centre_from_top: f32,
}

impl Default for MacInset {
    /// The standard lights' geometry, for a window that does not exist yet and a macOS version
    /// that is not known; [`crate::mac::default_inset`] knows the versions.
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
    /// Who draws the window frame.
    pub frame: Frame,
    /// `Some(true)` when `<PREFIX>_TITLEBAR` forced the app's bar, `Some(false)` when it forced the
    /// system's, `None` when the platform and setting decided.
    pub overridden: Option<bool>,
    /// The desktop the decision was made for, so an app can hide the "Use system title bar" row
    /// where it changes nothing (tiling compositors, web) without probing again.
    pub desktop: Desktop,
}

/// The decision for a window of `kind`, pure. `env` is the value of `<PREFIX>_TITLEBAR` (`app`, `system`
/// or `desktop`); it wins over `pref`. Caption layouts are each family's default; [`decide_for`] fills in
/// the desktop's own. A browser draws no frame code ([`Frame::Native`]). A tool window has the frame the
/// others have but no caption buttons: its captions have an empty layout.
pub fn decide(os: Os, runtime: Runtime, backend: Backend, desktop: &Desktop, kind: WindowKind, pref: TitleBarPref, env: Option<&str>) -> Decision {
    decide_with(&Inputs { os, runtime, backend, desktop, kind, pref, env }, &layout::default_layout)
}

/// What a decision reads.
struct Inputs<'a> {
    os: Os,
    runtime: Runtime,
    backend: Backend,
    desktop: &'a Desktop,
    kind: WindowKind,
    pref: TitleBarPref,
    env: Option<&'a str>,
}

/// `layout` supplies the caption layout for a desktop kind; it is called only when the frame comes
/// out integrated.
fn decide_with(inputs: &Inputs<'_>, layout: &dyn Fn(DesktopKind) -> Layout) -> Decision {
    let Inputs { os, runtime, backend, desktop, kind, pref, env } = *inputs;
    let overridden = crate::parse_override(env);
    let integrated = match (overridden, pref) {
        (Some(app), _) => app,
        (None, pref) => pref == TitleBarPref::Integrated,
    };
    let mut frame = if integrated && runtime == Runtime::Native {
        integrated_frame(os, backend, desktop, overridden.is_some(), INTEGRATED_ON_SSD_DESKTOPS, layout)
    } else {
        Frame::Native
    };
    if kind == WindowKind::Tool
        && let Frame::Integrated(captions) = &mut frame
    {
        captions.layout = Layout { left: Vec::new(), right: Vec::new() };
    }
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
        Os::Mac => Frame::Inset(MacInset::default()),
        Os::Windows => captions(CaptionFamily::Windows),
        // Linux and every other system.
        _ => {
            let no_ssd = backend == Backend::Wayland && desktop.ssd == Some(false);
            match desktop.kind {
                DesktopKind::Tiling if !forced => Frame::Native,
                DesktopKind::Tiling => captions(layout::family_from_gtk_theme(desktop.gtk_theme.as_deref())),
                DesktopKind::Gnome => captions(CaptionFamily::Adwaita),
                // Open decision 1: the line below is the only place that picks the app's bar on
                // desktops that draw their own frame. `false` gives `Native` there.
                _ if !no_ssd && !integrated_on_ssd && !forced => Frame::Native,
                DesktopKind::Kde => captions(CaptionFamily::Breeze),
                _ if no_ssd => captions(CaptionFamily::Generic),
                _ => captions(layout::family(DesktopKind::Other, desktop.gtk_theme.as_deref())),
            }
        }
    }
}

/// Read the platform once (500 ms at most, as for [`crate::Choice::probe`]) and decide.
/// `prefix` names the override variable `<PREFIX>_TITLEBAR`; when it is unset, `CRAFT_TITLEBAR`
/// applies. `backend` is the one the app opens its window on, [`Backend::from_env`]
/// unless the app forces X11.
pub fn decide_for(prefix: Prefix, backend: Backend, pref: TitleBarPref) -> Decision {
    decide_for_kind(prefix, backend, WindowKind::Main, pref)
}

/// [`decide_for`] for a window of `kind`.
pub fn decide_for_kind(prefix: Prefix, backend: Backend, kind: WindowKind, pref: TitleBarPref) -> Decision {
    let os = Os::current();
    let runtime = Runtime::current();
    let env = std::env::var(prefix.var_name()).or_else(|_| std::env::var(Prefix::default().var_name())).ok();
    let desktop =
        if os == Os::Linux { Desktop::detect(backend) } else { Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None } };
    let inputs = Inputs { os, runtime, backend, desktop: &desktop, kind, pref, env: env.as_deref() };
    decide_with(&inputs, &|kind| desktop_layout(os, kind))
}

/// The desktop's window-button layout, where it names one.
pub(crate) fn desktop_layout(os: Os, kind: DesktopKind) -> Layout {
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
#[cfg(feature = "ui")]
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
