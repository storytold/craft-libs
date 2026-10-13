#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! One display selection policy, shared by desktop apps and the launcher.
//! Automatic follows the active session; explicit choices fail visibly when unavailable.

/// Environment override understood by every suite application.
pub const BACKEND_ENV: &str = "CRAFT_DISPLAY_BACKEND";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum DisplayMode {
    #[default]
    Auto,
    Wayland,
    X11,
}
impl DisplayMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Wayland => "wayland",
            Self::X11 => "x11",
        }
    }
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" | "automatic" => Ok(Self::Auto),
            "wayland" => Ok(Self::Wayland),
            "x11" | "xwayland" => Ok(Self::X11),
            _ => Err(format!("Unknown display backend {value:?}; choose auto, wayland or x11")),
        }
    }
}

/// Resolves without modifying the environment. An inherited Wayland descriptor counts as a
/// connection even when WAYLAND_DISPLAY is absent. Saved preferences apply when there is no
/// environment override; an explicit `auto` override follows the session.
pub fn resolve(preference: DisplayMode, env: impl Fn(&str) -> Option<String>) -> Result<DisplayMode, String> {
    let present = |key: &str| env(key).is_some_and(|value| !value.is_empty());
    let wayland = present("WAYLAND_DISPLAY") || present("WAYLAND_SOCKET");
    let x11 = present("DISPLAY");
    let mode = match env(BACKEND_ENV).filter(|value| !value.is_empty()) {
        Some(value) => DisplayMode::parse(&value)?,
        None => preference,
    };
    match mode {
        DisplayMode::Wayland if !wayland => Err("Wayland was selected but no Wayland connection is available".into()),
        DisplayMode::X11 if !x11 => Err("X11/XWayland was selected but DISPLAY is unavailable".into()),
        DisplayMode::Auto => Ok(match env("XDG_SESSION_TYPE").as_deref() {
            Some("x11") if x11 => DisplayMode::X11,
            Some("wayland") if wayland => DisplayMode::Wayland,
            _ if wayland => DisplayMode::Wayland,
            _ if x11 => DisplayMode::X11,
            _ => DisplayMode::Auto,
        }),
        explicit => Ok(explicit),
    }
}

/// Composes with an existing event-loop hook, so platform startup customizations survive.
#[cfg(all(any(feature = "wgpu", feature = "glow"), not(target_arch = "wasm32")))]
pub fn configure(options: &mut eframe::NativeOptions, preference: DisplayMode) -> eframe::Result<()> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use winit::platform::{wayland::EventLoopBuilderExtWayland, x11::EventLoopBuilderExtX11};
        let selected =
            resolve(preference, |key| std::env::var(key).ok()).map_err(|error| eframe::Error::AppCreation(Box::new(std::io::Error::other(error))))?;
        if selected != DisplayMode::Auto {
            let prior = options.event_loop_builder.take();
            options.event_loop_builder = Some(Box::new(move |builder| {
                if let Some(prior) = prior {
                    prior(builder);
                }
                match selected {
                    DisplayMode::Wayland => {
                        builder.with_wayland();
                    }
                    DisplayMode::X11 => {
                        builder.with_x11();
                    }
                    DisplayMode::Auto => {}
                }
            }));
        }
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let _ = (options, preference);
    Ok(())
}

/// Axes retained by the native tablet protocol, normalized and routed to this window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenSample {
    pub pressure: f64,
    pub tilt: [f64; 2],
    pub rotation: f64,
    pub eraser: bool,
}

/// Drains samples received by the window event loop. Call before processing a UI frame;
/// this does not open another connection, poll the compositor or schedule extra frames.
#[cfg(all(any(feature = "wgpu", feature = "glow"), unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
pub fn tablet_feed(cc: &eframe::CreationContext<'_>, mut sink: impl FnMut(Option<PenSample>) + 'static) -> Option<Box<dyn FnMut()>> {
    use winit::platform::wayland::{TabletPhase, WindowExtWayland};
    let window = cc.winit_window()?.clone();
    if !window.tablet_input_available() {
        return None;
    }
    Some(Box::new(move || {
        for sample in window.take_tablet_input() {
            sink(if sample.phase == TabletPhase::Left {
                None
            } else {
                Some(PenSample { pressure: sample.pressure, tilt: sample.tilt, rotation: sample.rotation, eraser: sample.eraser })
            });
        }
    }))
}

/// Native pen contacts are already represented in the axis feed. A painting app can
/// remove their duplicate egui touch forces while retaining pointer emulation and finger input.
#[cfg(all(feature = "native", unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
pub fn is_native_tablet_touch(id: u64) -> bool {
    id & 0xffff_ffff_0000_0000 == winit::platform::wayland::TABLET_TOUCH_TAG
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resolve_env(pref: DisplayMode, pairs: &[(&str, &str)]) -> Result<DisplayMode, String> {
        resolve(pref, |key| pairs.iter().find(|(name, _)| *name == key).map(|(_, value)| value.to_string()))
    }
    #[test]
    fn automatic_follows_session_even_with_both_servers() {
        for (session, expected) in [("wayland", DisplayMode::Wayland), ("x11", DisplayMode::X11)] {
            assert_eq!(
                resolve_env(DisplayMode::Auto, &[("XDG_SESSION_TYPE", session), ("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")]),
                Ok(expected)
            );
        }
    }
    #[test]
    fn descriptors_nested_sessions_and_empty_values() {
        assert_eq!(resolve_env(DisplayMode::Auto, &[("WAYLAND_SOCKET", "8")]), Ok(DisplayMode::Wayland));
        assert_eq!(resolve_env(DisplayMode::Auto, &[("XDG_SESSION_TYPE", "wayland"), ("DISPLAY", ":0")]), Ok(DisplayMode::X11));
        assert_eq!(resolve_env(DisplayMode::Auto, &[("WAYLAND_DISPLAY", ""), ("DISPLAY", ":0")]), Ok(DisplayMode::X11));
        assert_eq!(resolve_env(DisplayMode::Auto, &[]), Ok(DisplayMode::Auto));
    }
    #[test]
    fn explicit_overrides_saved_preferences_and_missing_backends_report_errors() {
        let both = [("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")];
        assert_eq!(resolve_env(DisplayMode::X11, &both), Ok(DisplayMode::X11));
        assert_eq!(resolve_env(DisplayMode::X11, &[(BACKEND_ENV, "wayland"), ("WAYLAND_DISPLAY", "wayland-0")]), Ok(DisplayMode::Wayland));
        assert!(resolve_env(DisplayMode::Auto, &[(BACKEND_ENV, "invalid")]).is_err());
        assert!(resolve_env(DisplayMode::Wayland, &[("DISPLAY", ":0")]).is_err());
        assert!(resolve_env(DisplayMode::X11, &[("WAYLAND_DISPLAY", "wayland-0")]).is_err());
        assert_eq!(resolve_env(DisplayMode::X11, &[(BACKEND_ENV, "auto"), ("DISPLAY", ":0")]), Ok(DisplayMode::X11));
        assert_eq!(
            resolve_env(DisplayMode::X11, &[(BACKEND_ENV, "auto"), ("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")]),
            Ok(DisplayMode::Wayland)
        );
    }
}
