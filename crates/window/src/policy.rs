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
//! A tool window (a palette, a native dock window) draws its own title row whatever the main window has
//! ([`tool_frame`]): `Inset` with the traffic lights hidden on macOS, `Integrated` without caption buttons elsewhere.
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
    /// AppKit lays the lights out from the window's right edge (`windowTitlebarLayoutDirection` is right to left: a
    /// right-to-left system language, whatever the app's own language). `leading` is then reserved at the right end.
    pub lights_right: bool,
}

impl MacInset {
    /// The inset with the lights at the right end when `right`.
    pub fn with_lights_right(mut self, right: bool) -> Self {
        self.lights_right = right;
        self
    }
}

impl Default for MacInset {
    /// The standard lights' geometry, for a window that does not exist yet and a macOS version
    /// that is not known; [`crate::mac::default_inset`] knows the versions.
    fn default() -> Self {
        Self { leading: 78.0, centre_from_top: 14.0, lights_right: false }
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
/// the desktop's own. A browser draws no frame code ([`Frame::Native`]). A tool window's frame is
/// [`tool_frame`], whatever `pref` and `env` say.
pub fn decide(os: Os, runtime: Runtime, backend: Backend, desktop: &Desktop, kind: WindowKind, pref: TitleBarPref, env: Option<&str>) -> Decision {
    decide_with(&Inputs { os, runtime, backend, desktop, kind, pref, env, caption_style: None, caption_layout: None }, &layout::default_layout)
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
    /// The caption family override (`<PREFIX>_CAPTION_STYLE`).
    caption_style: Option<&'a str>,
    /// The caption layout override (`<PREFIX>_CAPTION_LAYOUT`).
    caption_layout: Option<&'a str>,
}

/// `layout` supplies the caption layout for a desktop kind; it is called only when the frame comes
/// out integrated.
fn decide_with(inputs: &Inputs<'_>, layout: &dyn Fn(DesktopKind) -> Layout) -> Decision {
    let Inputs { os, runtime, backend, desktop, kind, pref, env, caption_style, caption_layout } = *inputs;
    let overridden = crate::parse_override(env);
    if kind == WindowKind::Tool {
        return Decision { frame: tool_frame(os, runtime), overridden, desktop: desktop.clone() };
    }
    let integrated = match (overridden, pref) {
        (Some(app), _) => app,
        (None, pref) => pref == TitleBarPref::Integrated,
    };
    let mut frame = if integrated && runtime == Runtime::Native {
        integrated_frame(os, backend, desktop, overridden.is_some(), INTEGRATED_ON_SSD_DESKTOPS, layout)
    } else {
        Frame::Native
    };
    // The overrides pick the look and order of the app's own caption buttons; an unknown family and a layout with no
    // usable button leave what the platform decided.
    if let Frame::Integrated(captions) = &mut frame {
        if let Some(family) = caption_style.and_then(CaptionFamily::parse) {
            captions.family = family;
        }
        if let Some(parsed) = caption_layout.map(Layout::parse).filter(Layout::is_usable) {
            captions.layout = parsed;
        }
    }
    Decision { frame, overridden, desktop: desktop.clone() }
}

/// The frame of a tool window on `os`: its own title row replaces the OS title bar, whatever frame the main window
/// has. macOS keeps the OS frame with the title bar hidden (`Inset`; a tool window shows no traffic lights), so AppKit
/// still resizes it from its edges and draws its rounded corners and shadow; elsewhere decorations are off, with no
/// caption buttons. A browser draws no frame code.
pub fn tool_frame(os: Os, runtime: Runtime) -> Frame {
    match (runtime, os) {
        (Runtime::Web, _) => Frame::Native,
        (_, Os::Mac) => Frame::Inset(MacInset::default()),
        (_, Os::Windows) => Frame::Integrated(Captions { family: CaptionFamily::Windows, layout: Layout { left: Vec::new(), right: Vec::new() } }),
        _ => Frame::Integrated(Captions { family: CaptionFamily::Generic, layout: Layout { left: Vec::new(), right: Vec::new() } }),
    }
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
    let read = |own: String, shared: String| std::env::var(own).or_else(|_| std::env::var(shared)).ok();
    let shared = Prefix::default();
    let env = read(prefix.var_name(), shared.var_name());
    let caption_style = read(prefix.caption_style_var(), shared.caption_style_var());
    let caption_layout = read(prefix.caption_layout_var(), shared.caption_layout_var());
    let desktop =
        if os == Os::Linux { Desktop::detect(backend) } else { Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None } };
    let inputs = Inputs {
        os,
        runtime,
        backend,
        desktop: &desktop,
        kind,
        pref,
        env: env.as_deref(),
        caption_style: caption_style.as_deref(),
        caption_layout: caption_layout.as_deref(),
    };
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

/// The commands that move an open window from the frame `from` to `to`, the runtime twin of [`apply`]: `Native` to
/// `Integrated` gives `Decorations(false)`, the reverse `Decorations(true)`, two `Integrated` or two equal frames
/// `[]`. A change to or from `Inset` gives `None`, as egui has no runtime command for its builder-only settings.
#[cfg(feature = "ui")]
pub fn reframe(from: &Frame, to: &Frame) -> Option<Vec<egui::ViewportCommand>> {
    match (from, to) {
        // The full-size content view, hidden title and hidden title bar are set only through the builder.
        (Frame::Inset(_), _) | (_, Frame::Inset(_)) => (from == to).then(Vec::new),
        (Frame::Native, Frame::Integrated(_)) => Some(vec![egui::ViewportCommand::Decorations(false)]),
        (Frame::Integrated(_), Frame::Native) => Some(vec![egui::ViewportCommand::Decorations(true)]),
        // The caption buttons are the app's own drawing: no window command changes them.
        (Frame::Native, Frame::Native) | (Frame::Integrated(_), Frame::Integrated(_)) => Some(Vec::new()),
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

    fn decided(pref: TitleBarPref, style: Option<&str>, layout: Option<&str>) -> Frame {
        let desktop = Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None };
        let inputs = Inputs {
            os: Os::Windows,
            runtime: Runtime::Native,
            backend: Backend::Wayland,
            desktop: &desktop,
            kind: WindowKind::Main,
            pref,
            env: None,
            caption_style: style,
            caption_layout: layout,
        };
        decide_with(&inputs, &layout::default_layout).frame
    }

    fn captions(frame: Frame) -> Captions {
        match frame {
            Frame::Integrated(captions) => captions,
            other => panic!("an integrated frame was expected, got {other:?}"),
        }
    }

    #[test]
    fn a_caption_override_replaces_an_integrated_frames_family_and_usable_layout() {
        let base = captions(decided(TitleBarPref::Integrated, None, None));
        assert_eq!(base.family, CaptionFamily::Windows);

        let styled = captions(decided(TitleBarPref::Integrated, Some("Breeze"), None));
        assert_eq!(styled.family, CaptionFamily::Breeze, "the family variable picks the family");
        assert_eq!(styled.layout, base.layout, "the layout stays as decided");

        let unknown = captions(decided(TitleBarPref::Integrated, Some("bogus"), Some("appmenu:")));
        assert_eq!(unknown, base, "an unknown family and an unusable layout change nothing");

        let laid = captions(decided(TitleBarPref::Integrated, None, Some("close:")));
        assert_eq!(laid.layout, Layout::parse("close:"), "a usable layout replaces the decided one");
        assert_ne!(laid.layout, base.layout);

        assert_eq!(decided(TitleBarPref::System, Some("breeze"), Some("close:")), Frame::Native, "a native frame has no captions to change");
    }

    #[cfg(feature = "ui")]
    #[test]
    fn reframe_moves_an_open_window_between_native_and_integrated_and_refuses_inset() {
        use egui::ViewportCommand::Decorations;
        let integrated = decided(TitleBarPref::Integrated, None, None);
        let restyled = decided(TitleBarPref::Integrated, Some("breeze"), Some("close:"));
        let inset = Frame::Inset(MacInset::default());

        assert_eq!(reframe(&Frame::Native, &integrated), Some(vec![Decorations(false)]), "the app's bar turns the OS frame off");
        assert_eq!(reframe(&integrated, &Frame::Native), Some(vec![Decorations(true)]), "the system's bar turns it back on");
        assert_eq!(reframe(&integrated, &restyled), Some(Vec::new()), "the caption buttons are the app's drawing");
        assert_eq!(reframe(&Frame::Native, &Frame::Native), Some(Vec::new()));
        assert_eq!(reframe(&inset, &inset), Some(Vec::new()), "an unchanged inset frame needs no command");

        assert_eq!(reframe(&Frame::Native, &inset), None, "the inset frame is set only before the window opens");
        assert_eq!(reframe(&inset, &Frame::Native), None);
        assert_eq!(reframe(&inset, &integrated), None);
        assert_eq!(reframe(&integrated, &inset), None);
    }
}
