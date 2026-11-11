//! How a window opens: its level, its spec and the frame it gets.

use super::shared;
use craft_host::target::{Os, WindowChrome, WindowKind};
use craft_window::policy::{CaptionFamily, Captions, Frame, MacInset};
use egui::Context;

/// Where a window stands in the stack of windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum WindowLevel {
    /// An ordinary window.
    #[default]
    Normal,
    /// A palette: above the app's other windows while the app is active, and an ordinary window (behind other
    /// apps' windows on Windows) once none of the app's windows has focus, so it stays shown without covering
    /// other apps.
    Palette,
    /// Above every other window.
    AlwaysOnTop,
}

impl WindowLevel {
    /// The level egui is given while the app is `active` or not, on `os`. An inactive app's palette goes behind
    /// other apps' windows: on Windows `Normal` puts a window in front of every window that is not topmost, the
    /// activated app's included, so the palette goes to the bottom.
    pub(super) fn for_egui(self, active: bool, os: Os) -> egui::WindowLevel {
        match self {
            WindowLevel::Normal => egui::WindowLevel::Normal,
            WindowLevel::AlwaysOnTop => egui::WindowLevel::AlwaysOnTop,
            WindowLevel::Palette if active => egui::WindowLevel::AlwaysOnTop,
            WindowLevel::Palette if os == Os::Windows => egui::WindowLevel::AlwaysOnBottom,
            WindowLevel::Palette => egui::WindowLevel::Normal,
        }
    }
}

/// How an OS window opens (21.2): what it is for, who draws its frame, where it stands and whether it has a taskbar
/// entry. Its size limits, position and resizability are the container's fallback `Overlay`'s (`size_limits`,
/// `placed`, `resizable`).
///
/// A chrome or caption layout the spec leaves unset is the installed decision's for the window's kind
/// (`InstallSpec::frame`); with no decision installed the window has the OS's frame.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct WindowSpec {
    /// What the window is for.
    pub kind: WindowKind,
    /// Who draws the frame; unset takes the installed decision's.
    pub chrome: Option<WindowChrome>,
    /// The caption buttons the app draws when the chrome is `Integrated`. Unset takes the installed decision's when
    /// `chrome` is unset too, and none otherwise.
    pub captions: Option<Captions>,
    /// Where the window stands among the windows.
    pub level: WindowLevel,
    /// Whether the window has a taskbar or Dock entry.
    pub taskbar: bool,
    /// Whether the window opens hidden and appears once its first pass has been laid out and painted.
    pub shown_after_first_paint: bool,
}

impl WindowSpec {
    /// A tool window (a native dock window): the installed decision's frame without caption buttons, a palette level,
    /// no taskbar entry.
    pub fn tool() -> Self {
        WindowSpec { kind: WindowKind::Tool, level: WindowLevel::Palette, taskbar: false, ..Self::dialog() }
    }

    /// A document window with the installed decision's frame.
    pub fn document() -> Self {
        WindowSpec { kind: WindowKind::Document, ..Self::dialog() }
    }

    /// A dialog window with the installed decision's frame.
    pub fn dialog() -> Self {
        WindowSpec {
            kind: WindowKind::Dialog,
            chrome: None,
            captions: None,
            level: WindowLevel::Normal,
            taskbar: true,
            shown_after_first_paint: false,
        }
    }

    /// Sets what the window is for.
    #[must_use]
    pub fn kind(mut self, kind: WindowKind) -> Self {
        self.kind = kind;
        self
    }

    /// Sets who draws the frame, whatever the installed decision says.
    #[must_use]
    pub fn chrome(mut self, chrome: WindowChrome) -> Self {
        self.chrome = Some(chrome);
        self
    }

    /// Sets the caption buttons.
    #[must_use]
    pub fn captions(mut self, captions: Option<Captions>) -> Self {
        self.captions = captions;
        self
    }

    /// Takes the chrome and the captions from a decided `frame`; a frame without buttons has no captions.
    #[must_use]
    pub fn frame(mut self, frame: &Frame) -> Self {
        self.chrome = Some(chrome_of(frame));
        self.captions = match frame {
            Frame::Integrated(captions) if !captions.is_empty() => Some(captions.clone()),
            _ => None,
        };
        self
    }

    /// Sets the level.
    #[must_use]
    pub fn level(mut self, level: WindowLevel) -> Self {
        self.level = level;
        self
    }

    /// Sets whether the window has a taskbar or Dock entry.
    #[must_use]
    pub fn taskbar(mut self, taskbar: bool) -> Self {
        self.taskbar = taskbar;
        self
    }

    /// Sets whether the window opens hidden and appears after its first paint, so a window that shows a capture or
    /// other content that must be ready does not flash an empty frame.
    #[must_use]
    pub fn shown_after_first_paint(mut self, hidden_until_painted: bool) -> Self {
        self.shown_after_first_paint = hidden_until_painted;
        self
    }
}

/// The frame the installed decision gives a window of `kind`: a tool window shows no caption buttons. `None` while
/// no decision is installed.
fn installed_frame(ctx: &Context, kind: WindowKind) -> Option<Frame> {
    let frame = shared(ctx, |shared| shared.root_frame.clone())?;
    Some(match frame {
        Frame::Integrated(captions) if kind == WindowKind::Tool => {
            Frame::Integrated(Captions { family: captions.family, layout: craft_window::Layout { left: Vec::new(), right: Vec::new() } })
        }
        frame => frame,
    })
}

/// The chrome and caption buttons `spec` opens with: its own, else the installed decision's for its kind, else the
/// OS's frame.
pub(super) fn frame_of_spec(ctx: &Context, spec: &WindowSpec) -> (WindowChrome, Option<Captions>) {
    let decided = installed_frame(ctx, spec.kind);
    let chrome = spec.chrome.or_else(|| decided.as_ref().map(chrome_of)).unwrap_or(WindowChrome::Native);
    let captions = spec.captions.clone().or_else(|| match decided {
        Some(Frame::Integrated(captions)) if spec.chrome.is_none() && !captions.is_empty() => Some(captions),
        _ => None,
    });
    (chrome, captions)
}

/// Who draws the frame `frame` describes.
pub(super) fn chrome_of(frame: &Frame) -> WindowChrome {
    match frame {
        Frame::Native => WindowChrome::Native,
        Frame::Inset(_) => WindowChrome::Inset,
        Frame::Integrated(_) => WindowChrome::Integrated,
    }
}

/// The frame of a window drawn as `chrome` with `captions`; an integrated frame without captions has no buttons.
pub(super) fn frame_of(chrome: WindowChrome, captions: Option<Captions>) -> Frame {
    match chrome {
        WindowChrome::Inset => Frame::Inset(MacInset::default()),
        WindowChrome::Integrated => Frame::Integrated(
            captions
                .unwrap_or_else(|| Captions { family: CaptionFamily::Generic, layout: craft_window::Layout { left: Vec::new(), right: Vec::new() } }),
        ),
        _ => Frame::Native,
    }
}
