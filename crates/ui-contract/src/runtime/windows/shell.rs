//! The shell and window info each viewport sees.

use super::host::scale_of;
use super::spec::chrome_of;
use super::{Record, shared};
use crate::runtime::egui::viewports;
use crate::runtime::frame::PerViewport;
#[cfg(feature = "testing")]
use craft_host::target::Runtime;
use craft_host::target::{Caps, EdgeInsets, Os, ShellInfo, WindowChrome, WindowInfo, WindowKind, WindowState};
use egui::{Context, ViewportId};
use std::sync::Arc;

pub(super) fn base_shell(ctx: &Context) -> ShellInfo {
    shared(ctx, |shared| shared.shell.clone()).unwrap_or_default()
}

/// The operating system of the shell: the one source of the key table, the chords shown, the drop operation and
/// the chrome. It does not differ between viewports.
#[doc(hidden)]
pub fn os_of(ctx: &Context) -> Os {
    shared(ctx, |shared| shared.shell.as_ref().map(|shell| shell.os)).unwrap_or_else(|| ShellInfo::default().os)
}

/// Makes the shell run on `os`, as a test of the OS matrix does: the capabilities and the menu placement of a
/// native build follow it. egui's own OS is the caller's to set.
#[cfg(feature = "testing")]
pub(crate) fn set_os(ctx: &Context, os: Os) {
    shared(ctx, |shared| {
        let root_chrome = shared.root_frame.as_ref().map(chrome_of);
        let shell = shared.shell.get_or_insert_with(ShellInfo::default);
        shell.os = os;
        if shell.runtime == Runtime::Native {
            shell.caps = Caps::native(os);
            if let Some(chrome) = root_chrome {
                std::sync::Arc::make_mut(&mut shell.window).chrome = chrome;
            }
        }
    });
}

/// Makes the root window's frame `chrome`, as a host with that frame reports: a native build's menu placement
/// follows it. The installed decision, when there is one, decides the chrome instead.
#[cfg(feature = "testing")]
pub(crate) fn set_chrome(ctx: &Context, chrome: WindowChrome) {
    shared(ctx, |shared| {
        let shell = shared.shell.get_or_insert_with(ShellInfo::default);
        std::sync::Arc::make_mut(&mut shell.window).chrome = chrome;
    });
}

fn state_of(facts: &viewports::Facts) -> WindowState {
    if facts.fullscreen {
        WindowState::Fullscreen
    } else if facts.minimized {
        WindowState::Minimized
    } else if facts.maximized {
        WindowState::Maximized
    } else {
        WindowState::Normal
    }
}

/// `viewport`'s window facts.
pub(super) fn window_info(ctx: &Context, viewport: ViewportId, base: &ShellInfo) -> WindowInfo {
    let (facts, kind, mut chrome, insets) =
        PerViewport::<Record>::with(ctx, viewport, |record| (record.facts, record.kind, record.chrome, record.insets));
    if viewport == ViewportId::ROOT {
        chrome = shared(ctx, |shared| shared.root_frame.as_ref().map(chrome_of)).or(chrome);
    }
    let mut info = if viewport == ViewportId::ROOT { (*base.window).clone() } else { WindowInfo::new(WindowKind::Tool) };
    if let Some(kind) = kind {
        info.kind = kind;
    }
    if let Some(chrome) = chrome {
        info.chrome = chrome;
    }
    if let Some(insets) = insets {
        info.insets = insets;
    }
    if let Some(facts) = facts {
        info.active = facts.focused;
        info.state = state_of(&facts);
    }
    info
}

/// The shell as `viewport` sees it: the OS-level fields from install, the window facts of that viewport.
#[doc(hidden)]
pub fn shell_of(ctx: &Context, viewport: ViewportId) -> ShellInfo {
    let mut shell = base_shell(ctx);
    shell.window = Arc::new(window_info(ctx, viewport, &shell));
    shell
}

/// Stores `facts` as the viewport's window facts and, on macOS where the app draws the title bar, measures the
/// traffic lights for `WindowInfo.insets` (21.2). The measurement is AppKit's, so it runs here, outside every egui lock.
pub(super) fn refresh(ctx: &Context, viewport: ViewportId, facts: viewports::Facts) {
    let mut lights = PerViewport::<Record>::with(ctx, viewport, |record| {
        record.facts = Some(facts);
        std::mem::take(&mut record.lights)
    });
    let lights_over_content = os_of(ctx) == Os::Mac && window_info(ctx, viewport, &base_shell(ctx)).chrome != WindowChrome::Native;
    let insets = lights_over_content
        .then(|| {
            let scale = scale_of(ctx, &facts)?;
            lights.inset(scale, facts.fullscreen, craft_window::mac::os_default, craft_window::mac::measure)
        })
        .map(|inset| {
            let mut insets = EdgeInsets::all(0.0);
            insets.left = inset.map_or(0.0, |inset| inset.leading);
            insets
        });
    PerViewport::<Record>::with(ctx, viewport, |record| {
        record.lights = lights;
        record.insets = insets;
    });
}
