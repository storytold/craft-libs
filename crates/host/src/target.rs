//! The target platform, egui-free so layers below the UI read it.

/// The operating system of the host, also in a browser: shortcut labels in a browser on a Mac read "Mac".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Os {
    /// macOS.
    Mac,
    /// Windows.
    Windows,
    /// Linux and every other system.
    #[default]
    Linux,
}

impl Os {
    /// The system this build runs on. Web builds count as Linux; with the `ui` feature,
    /// [`Os::of_egui`] gives the browser's operating system.
    pub const fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }

    /// Whether this is macOS.
    pub const fn is_mac(self) -> bool {
        matches!(self, Os::Mac)
    }

    /// The system egui reports: iOS counts as Mac; Android and unknown systems count as Linux.
    #[cfg(feature = "ui")]
    pub fn of_egui(os: egui::os::OperatingSystem) -> Os {
        use egui::os::OperatingSystem;
        match os {
            OperatingSystem::Mac | OperatingSystem::IOS => Os::Mac,
            OperatingSystem::Windows => Os::Windows,
            OperatingSystem::Nix | OperatingSystem::Android | OperatingSystem::Unknown => Os::Linux,
        }
    }
}

/// What runs the app: a native process or a browser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Runtime {
    /// A native process.
    #[default]
    Native,
    /// A browser (wasm).
    Web,
}

impl Runtime {
    /// The runtime this build runs in.
    pub const fn current() -> Runtime {
        if cfg!(target_arch = "wasm32") { Runtime::Web } else { Runtime::Native }
    }
}

/// The windowing backend a native Linux window opens on; elsewhere the platform's own, which counts as `X11`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Backend {
    /// A Wayland compositor.
    Wayland,
    /// X11, or any platform without the choice.
    #[default]
    X11,
}

impl Backend {
    /// The backend winit opens by default: Wayland when `WAYLAND_DISPLAY` or `WAYLAND_SOCKET` is set, else X11. An app
    /// that forces X11 (XWayland) passes [`Backend::X11`] instead.
    pub fn from_env() -> Backend {
        let set = |k| std::env::var_os(k).is_some_and(|v| !v.is_empty());
        if set("WAYLAND_DISPLAY") || set("WAYLAND_SOCKET") { Backend::Wayland } else { Backend::X11 }
    }
}

/// The kind of desktop, as far as window chrome cares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DesktopKind {
    /// GNOME and its relatives (Budgie).
    Gnome,
    /// KDE Plasma.
    Kde,
    /// A tiling compositor or window manager.
    Tiling,
    /// Any other desktop, and the systems with none.
    #[default]
    Other,
}

/// What a double click on a title bar does (macOS: System Settings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DoubleClick {
    /// Maximize or restore.
    #[default]
    Zoom,
    /// Minimize.
    Minimize,
    /// Nothing.
    Nothing,
}

/// Where the menu bar lives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MenuPlacement {
    /// Inside each window.
    #[default]
    InWindow,
    /// At the top of the screen, owned by the OS (macOS).
    ScreenTop,
}

impl MenuPlacement {
    /// Where a window's menu bar lives: at the top of the screen when the OS has such a bar and the OS draws the
    /// window's frame around it, else in the window (an app-drawn frame carries the bar itself). The one rule the
    /// shell and every host or fixture derive the placement from.
    pub fn of(caps: Caps, chrome: WindowChrome) -> MenuPlacement {
        if caps.contains(Caps::NATIVE_MENU_BAR) && chrome != WindowChrome::Integrated { MenuPlacement::ScreenTop } else { MenuPlacement::InWindow }
    }
}

bitflags::bitflags! {
    /// What the host can do. Capabilities are data: a pattern hides an action whose capability is missing.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Caps: u16 {
        /// More than one OS window.
        const VIEWPORTS = 1;
        /// Window commands (move, resize, minimize, maximize).
        const WINDOW_COMMANDS = 1 << 1;
        /// A file system.
        const FILE_SYSTEM = 1 << 2;
        /// Threads.
        const THREADS = 1 << 3;
        /// Screen capture.
        const SCREEN_CAPTURE = 1 << 4;
        /// The app sets the system cursor.
        const SYSTEM_CURSOR = 1 << 5;
        /// A native menu bar.
        const NATIVE_MENU_BAR = 1 << 6;
        /// The OS resizes undecorated windows from their edges.
        const OS_RESIZES_UNDECORATED = 1 << 7;
    }
}

impl Caps {
    /// The capabilities of a native desktop build on `os`.
    pub const fn native(os: Os) -> Caps {
        let base = Caps::VIEWPORTS.union(Caps::WINDOW_COMMANDS).union(Caps::FILE_SYSTEM).union(Caps::THREADS).union(Caps::SYSTEM_CURSOR);
        match os {
            Os::Mac => base.union(Caps::NATIVE_MENU_BAR).union(Caps::SCREEN_CAPTURE),
            Os::Windows => base.union(Caps::OS_RESIZES_UNDECORATED).union(Caps::SCREEN_CAPTURE),
            Os::Linux => base,
        }
    }

    /// The capabilities of a browser build.
    pub const fn web() -> Caps {
        Caps::SYSTEM_CURSOR
    }
}

/// What a window is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WindowKind {
    /// The app's main window.
    #[default]
    Main,
    /// A document window.
    Document,
    /// A tool window (a native dock window).
    Tool,
    /// A dialog window.
    Dialog,
}

/// Who draws a window's frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WindowChrome {
    /// The OS, window manager or compositor.
    #[default]
    Native,
    /// macOS: the app's bar under the traffic lights.
    Inset,
    /// Decorations are off; the app's bar is the title bar.
    Integrated,
}

/// A window's display state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WindowState {
    /// Neither maximized, minimized nor full screen.
    #[default]
    Normal,
    /// Maximized.
    Maximized,
    /// Minimized.
    Minimized,
    /// Full screen.
    Fullscreen,
}

/// Points the OS reserves at each edge of a window (the traffic lights, a notch, a taskbar).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct EdgeInsets {
    /// Top.
    pub top: f32,
    /// Left.
    pub left: f32,
    /// Bottom.
    pub bottom: f32,
    /// Right.
    pub right: f32,
}

impl EdgeInsets {
    /// The same inset on every edge.
    pub const fn all(points: f32) -> EdgeInsets {
        EdgeInsets { top: points, left: points, bottom: points, right: points }
    }
}

/// One window's facts. The UI refreshes each viewport's copy in `set_input`.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct WindowInfo {
    /// What the window is for.
    pub kind: WindowKind,
    /// Who draws its frame.
    pub chrome: WindowChrome,
    /// Its display state.
    pub state: WindowState,
    /// What the OS reserves at its edges.
    pub insets: EdgeInsets,
    /// The window has OS focus.
    pub active: bool,
}

impl WindowInfo {
    /// A window of `kind` with default chrome, state and insets, active.
    pub fn new(kind: WindowKind) -> WindowInfo {
        WindowInfo { kind, active: true, ..WindowInfo::default() }
    }

    /// Sets who draws the frame.
    pub fn chrome(mut self, chrome: WindowChrome) -> Self {
        self.chrome = chrome;
        self
    }

    /// Sets the edge insets.
    pub fn insets(mut self, insets: EdgeInsets) -> Self {
        self.insets = insets;
        self
    }
}

/// The platform the app runs on and what it offers. The OS-level fields are fixed at install; `window` is the
/// main window's facts.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ShellInfo {
    /// The host OS, also in a browser.
    pub os: Os,
    /// Native or web.
    pub runtime: Runtime,
    /// The windowing backend.
    pub backend: Backend,
    /// The desktop.
    pub desktop: DesktopKind,
    /// What the host can do.
    pub caps: Caps,
    /// What a title bar double click does.
    pub double_click: DoubleClick,
    /// The main window's facts.
    pub window: std::sync::Arc<WindowInfo>,
}

impl ShellInfo {
    /// Where the menu bar lives: it follows the capabilities and the main window's frame.
    pub fn menu(&self) -> MenuPlacement {
        MenuPlacement::of(self.caps, self.window.chrome)
    }

    /// The shell of the running build: the OS, runtime and capabilities this binary has.
    pub fn current() -> ShellInfo {
        let os = Os::current();
        let runtime = Runtime::current();
        let caps = match runtime {
            Runtime::Native => Caps::native(os),
            Runtime::Web => Caps::web(),
        };
        ShellInfo {
            os,
            runtime,
            backend: Backend::default(),
            desktop: DesktopKind::default(),
            caps,
            double_click: DoubleClick::default(),
            window: std::sync::Arc::new(WindowInfo::new(WindowKind::Main)),
        }
    }

    /// Sets the capabilities, for a native build run as web in tests. The menu placement follows them.
    pub fn caps(mut self, caps: Caps) -> Self {
        self.caps = caps;
        self
    }

    /// Sets the runtime.
    pub fn runtime(mut self, runtime: Runtime) -> Self {
        self.runtime = runtime;
        self
    }

    /// Sets the OS.
    pub fn os(mut self, os: Os) -> Self {
        self.os = os;
        self
    }

    /// Sets the backend.
    pub fn backend(mut self, backend: Backend) -> Self {
        self.backend = backend;
        self
    }

    /// Sets the desktop.
    pub fn desktop(mut self, desktop: DesktopKind) -> Self {
        self.desktop = desktop;
        self
    }

    /// Sets the title bar double click.
    pub fn double_click(mut self, double_click: DoubleClick) -> Self {
        self.double_click = double_click;
        self
    }

    /// Sets the main window's facts. The menu placement follows its chrome.
    pub fn window(mut self, window: WindowInfo) -> Self {
        self.window = std::sync::Arc::new(window);
        self
    }
}

impl Default for ShellInfo {
    fn default() -> Self {
        ShellInfo::current()
    }
}
