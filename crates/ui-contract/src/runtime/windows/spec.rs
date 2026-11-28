//! How a window opens: its level, its spec and the frame it gets.

use super::shared;
use super::shell::{base_shell, os_of};
use craft_host::target::{Os, WindowChrome, WindowKind};
use craft_window::policy::{CaptionFamily, Captions, Frame, MacInset, tool_frame};
use egui::{Context, ViewportId};

/// Where a window stands in the stack of windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum WindowLevel {
    /// An ordinary window.
    #[default]
    Normal,
    /// A palette: above the app's other windows while the app is active, an ordinary window while an app-modal layer
    /// in another window blocks it, and an ordinary window (behind other apps' windows on Windows) once none of the
    /// app's windows has focus, so it stays shown without covering other apps or the modal. Hidden while the main
    /// window is minimized, except on macOS, where the app stays active and its palettes stay.
    Palette,
    /// Above every other window.
    AlwaysOnTop,
}

/// The app's state that its palettes follow, as its windows reported it in the last root frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct AppState {
    /// Every window of the app reported that it has no focus.
    pub away: bool,
    /// The window that holds an app-modal layer, which blocks the app's other windows.
    pub modal: Option<ViewportId>,
    /// The main window is minimized.
    pub minimized: bool,
}

/// How a window stands among the windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Standing {
    /// The level egui is given.
    pub level: egui::WindowLevel,
    /// Whether the window is shown.
    pub shown: bool,
    /// Whether the window stands as an ordinary window because an app-modal layer in another window blocks it.
    pub lowered: bool,
}

impl WindowLevel {
    /// How `viewport`'s window at this level stands while the app is in `app`, on `os`. An inactive app's palette goes
    /// behind other apps' windows: on Windows `Normal` puts a window in front of every window that is not topmost, the
    /// activated app's included, so the palette goes to the bottom. A palette blocked by a modal stands as an ordinary
    /// window, so the modal's window can be raised above it.
    pub(super) fn standing(self, viewport: ViewportId, app: AppState, os: Os) -> Standing {
        let palette = self == WindowLevel::Palette;
        let lowered = palette && !app.away && app.modal.is_some_and(|modal| modal != viewport);
        let level = match self {
            WindowLevel::Normal => egui::WindowLevel::Normal,
            WindowLevel::AlwaysOnTop => egui::WindowLevel::AlwaysOnTop,
            WindowLevel::Palette if app.away && os == Os::Windows => egui::WindowLevel::AlwaysOnBottom,
            WindowLevel::Palette if app.away || lowered => egui::WindowLevel::Normal,
            WindowLevel::Palette => egui::WindowLevel::AlwaysOnTop,
        };
        Standing { level, shown: !(palette && app.minimized && os != Os::Mac), lowered }
    }
}

/// How an OS window opens (21.2): what it is for, who draws its frame, where it stands and whether it has a taskbar
/// entry. Its size limits, position and resizability are the container's fallback `Overlay`'s (`size_limits`,
/// `placed`, `resizable`).
///
/// A chrome or caption layout the spec leaves unset is the installed decision's for the window's kind
/// (`InstallSpec::frame`); with no decision installed the window has the OS's frame.
#[derive(Clone, Debug, PartialEq)]
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
    /// Whether the window takes OS focus when it opens.
    pub takes_focus: bool,
    /// Whether pointer input passes through the window to the window below it.
    pub passes_pointer: bool,
    /// Whether an Escape nothing in the window takes is the Cancel of the window it was opened from (a dock window's):
    /// no component, layer, capture or app widget of the window, and no reader of its intents.
    pub hands_on_cancel: bool,
}

impl WindowSpec {
    /// A tool window (a native dock window): its own title row in place of the OS title bar
    /// (`craft_window::policy::tool_frame`), no caption buttons, a palette level, no taskbar entry. On Windows it has
    /// the [tool-window style](https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles), which
    /// also keeps it out of Alt-Tab, while it has no taskbar entry.
    pub fn tool() -> Self {
        WindowSpec { kind: WindowKind::Tool, level: WindowLevel::Palette, taskbar: false, ..Self::dialog() }
    }

    /// A popup window: a layer opened from a palette window (a flyout, menu or tooltip) in an OS window of its own. It
    /// has no frame, no taskbar entry and a palette level, and appears once its first pass ran, at the size and place
    /// its layer took in that pass.
    pub fn popup() -> Self {
        WindowSpec { chrome: Some(WindowChrome::Integrated), shown_after_first_paint: true, ..Self::tool() }
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
            takes_focus: true,
            passes_pointer: false,
            hands_on_cancel: false,
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

    /// Sets whether the window takes OS focus when it opens.
    #[must_use]
    pub fn takes_focus(mut self, takes_focus: bool) -> Self {
        self.takes_focus = takes_focus;
        self
    }

    /// Sets whether pointer input passes through the window to the window below it (a tooltip's).
    #[must_use]
    pub fn passes_pointer(mut self, passes_pointer: bool) -> Self {
        self.passes_pointer = passes_pointer;
        self
    }

    /// Sets whether an Escape nothing in the window takes is the Cancel of the window it was opened from.
    #[must_use]
    pub fn hands_on_cancel(mut self, hands_on_cancel: bool) -> Self {
        self.hands_on_cancel = hands_on_cancel;
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

/// The frame a window of `kind` gets: a tool window's own ([`tool_frame`]), whatever the main window has; any other
/// the installed decision's. `None` for those while no decision is installed.
fn installed_frame(ctx: &Context, kind: WindowKind) -> Option<Frame> {
    if kind == WindowKind::Tool {
        return Some(tool_frame(os_of(ctx), base_shell(ctx).runtime));
    }
    shared(ctx, |shared| shared.root_frame.clone())
}

/// The chrome and caption buttons `spec` opens with: its own, else the frame for its kind, else the OS's frame.
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
