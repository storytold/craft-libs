//! Input from the shell beyond touches: keyboards, trackpads and mice, the stylus, the clipboard,
//! and the app's life cycle. One enum, so each platform layer has one entry point.

use egui::{Event, Key, Modifiers, MouseWheelSource, MouseWheelUnit, Pos2, TouchPhase, Vec2};

/// Something the shell reports.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// A key went down or up.
    Key {
        /// egui's key name: `A`, `Enter`, `ArrowLeft`, `Escape`.
        key: String,
        /// Down (`true`) or up.
        pressed: bool,
        /// Modifier keys held with it.
        modifiers: Modifiers,
    },
    /// Text typed (on-screen keyboard, dictation, a hardware key with a character).
    Text(String),
    /// Paste this text (the shell read the clipboard because the user asked to paste).
    PasteText(String),
    /// Paste this image (RGBA8, unpremultiplied), read from the clipboard on the user's request.
    PasteImage {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// `width × height × 4` bytes.
        rgba: Vec<u8>,
    },
    /// Paste: the clipboard still holds the image the app put there last
    /// ([`crate::Platform::copy_image`]), so the shell didn't read it back. The app pastes its own
    /// copy, at full fidelity; egui hears `Event::PasteImage` with an empty image.
    PasteOwnImage,
    /// A trackpad or mouse pointer hovering, without a button down, in points.
    Hover(Pos2),
    /// The hovering pointer left the view.
    HoverGone,
    /// Scrolling (trackpad, mouse wheel).
    Scroll {
        /// Where the pointer is, in points.
        pos: Pos2,
        /// In points: positive moves content right and down.
        delta: Vec2,
    },
    /// A secondary click (right click, two-finger trackpad click, control-click) at `pos`.
    SecondaryClick(Pos2),
    /// A stylus sample. The shell sends stylus contact as this instead of a touch.
    Stylus(Stylus),
    /// The app is leaving the screen and may be terminated: save recovery data now.
    Suspend,
    /// The app is back on screen.
    Resume,
    /// The OS is short of memory: let go of what can be rebuilt.
    MemoryWarning,
    /// The device's thermal state or power saving changed (and once as the app starts).
    Power(Power),
    /// The shell finished writing a document back to `location` ([`crate::Platform::save`]):
    /// `Err` with its reason when it couldn't.
    WriteDone {
        /// Where it wrote, as the request named it.
        location: crate::Location,
        /// `Err` with the reason it couldn't write.
        result: Result<(), String>,
    },
}

/// How hot the device is, in the four steps iPadOS reports (Android's seven fold into them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Thermal {
    /// No throttling.
    #[default]
    Nominal,
    /// Slightly warm: the system may throttle a little; nothing the user would notice yet.
    Fair,
    /// Hot: the system is cutting performance. Do less (lower preview quality, fewer frames).
    Serious,
    /// Very hot: the system is about to cut performance hard. Do the least that still works.
    Critical,
}

impl Thermal {
    /// From `ProcessInfo.ThermalState.rawValue` (0 nominal … 3 critical); hotter values are critical.
    #[cfg(any(target_os = "ios", test))]
    pub(crate) fn from_apple(raw: u32) -> Self {
        match raw {
            0 => Self::Nominal,
            1 => Self::Fair,
            2 => Self::Serious,
            _ => Self::Critical,
        }
    }

    /// From `PowerManager.THERMAL_STATUS_*`: none; light and moderate ("UX not largely
    /// impacted"); severe ("largely impacted"); critical, emergency and shutdown. Unknown (below
    /// zero, or an API level without thermal status) is nominal.
    #[cfg(any(target_os = "android", test))]
    pub(crate) fn from_android(status: i32) -> Self {
        match status {
            ..=0 => Self::Nominal,
            1 | 2 => Self::Fair,
            3 => Self::Serious,
            _ => Self::Critical,
        }
    }
}

/// The device's power state: how hot it is and whether the user asked it to save power (Low
/// Power Mode, Battery Saver).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Power {
    /// How hot the device is.
    pub thermal: Thermal,
    /// Low Power Mode (iPadOS) or Battery Saver (Android) is on.
    pub low_power: bool,
}

impl Power {
    /// Whether the app should do less per frame (cheaper previews, smaller GPU budgets): the
    /// device is hot enough to be throttled, or the user asked it to save power. The shells
    /// already lower the frame rate in the same states.
    #[must_use]
    pub fn constrained(self) -> bool {
        self.low_power || self.thermal >= Thermal::Serious
    }

    /// The most frames a second to draw: 30 when critical, 60 when hot or saving power, else
    /// the display's own rate (`None`). `CraftKit` sets its display link to the same.
    #[must_use]
    pub fn max_frame_rate(self) -> Option<u32> {
        match self.thermal {
            Thermal::Critical => Some(30),
            _ if self.constrained() => Some(60),
            _ => None,
        }
    }
}

/// One stylus reading, in the shell's terms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stylus {
    /// Stable for one stroke from start to end.
    pub id: u64,
    /// Start, move, end or cancel.
    pub phase: TouchPhase,
    /// In points from the view's top-left corner.
    pub pos: Pos2,
    /// 0..=1.
    pub pressure: f32,
    /// Angle from the screen in radians: 0 lying flat, π/2 upright.
    pub altitude: f32,
    /// Direction the stylus leans, in radians clockwise from the view's +x axis.
    pub azimuth: f32,
    /// The eraser end (or an eraser mode the device reports) is in use.
    pub eraser: bool,
}

impl Stylus {
    /// Tilt along x and y in degrees (-90..90), the W3C Pointer Events convention most painting
    /// code uses: an upright stylus has no tilt.
    #[must_use]
    pub fn tilt_degrees(&self) -> (f32, f32) {
        let lean = (std::f32::consts::FRAC_PI_2 - self.altitude.clamp(0.0, std::f32::consts::FRAC_PI_2)).to_degrees();
        let (sin, cos) = self.azimuth.sin_cos();
        ((lean * cos).clamp(-90.0, 90.0), (lean * sin).clamp(-90.0, 90.0))
    }
}

/// The egui events a key stands for. Command-C/X become Copy/Cut, as egui-winit makes them, so
/// text fields and the app's own copy handling see what they see on the desktop.
pub(crate) fn key_events(key: &str, pressed: bool, modifiers: Modifiers) -> Vec<Event> {
    let Some(key) = Key::from_name(key) else {
        return Vec::new();
    };
    let mut events = vec![Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }];
    if pressed && modifiers.command {
        match key {
            Key::C => events.push(Event::Copy),
            Key::X => events.push(Event::Cut),
            _ => {}
        }
    }
    events
}

/// A hover sample as an egui event, or nothing while a touch drives the pointer. A stylus that
/// hovers (Apple Pencil on M2+ iPads, S Pen) reports where its tip would land, at its tilt; lifting
/// it can deliver that position before the touch ends, which would drag the pressed pointer
/// there and draw a straight line off the end of every stroke.
pub(crate) fn hover_event(touching: bool, pos: Option<Pos2>) -> Option<Event> {
    if touching {
        return None;
    }
    Some(pos.map_or(Event::PointerGone, Event::PointerMoved))
}

pub(crate) fn scroll_events(pos: Pos2, delta: Vec2, modifiers: Modifiers) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::MouseWheel { unit: MouseWheelUnit::Point, delta, phase: TouchPhase::Move, source: MouseWheelSource::Unknown, modifiers },
    ]
}

pub(crate) fn secondary_click_events(pos: Pos2, modifiers: Modifiers) -> Vec<Event> {
    let button = |pressed| Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers };
    vec![Event::PointerMoved(pos), button(true), button(false)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_c_and_x_also_copy_and_cut() {
        let cmd = Modifiers::COMMAND;
        assert!(key_events("C", true, cmd).contains(&Event::Copy));
        assert!(key_events("X", true, cmd).contains(&Event::Cut));
        assert!(!key_events("C", false, cmd).contains(&Event::Copy), "only on press");
        assert!(!key_events("C", true, Modifiers::NONE).contains(&Event::Copy), "a plain C is a C");
        assert!(key_events("NotAKey", true, cmd).is_empty(), "unknown names are dropped, not guessed");
    }

    #[test]
    fn hover_never_moves_a_pointer_a_touch_is_pressing() {
        let at = Pos2::new(5.0, 6.0);
        assert_eq!(hover_event(true, Some(at)), None, "a hovering tip mid-stroke would draw a line to it");
        assert_eq!(hover_event(true, None), None, "nor end the stroke's pointer");
        assert_eq!(hover_event(false, Some(at)), Some(Event::PointerMoved(at)));
        assert_eq!(hover_event(false, None), Some(Event::PointerGone));
    }

    #[test]
    fn a_secondary_click_presses_and_releases_the_secondary_button() {
        let events = secondary_click_events(Pos2::new(5.0, 6.0), Modifiers::NONE);
        let presses: Vec<bool> = events
            .iter()
            .filter_map(|e| match e {
                Event::PointerButton { button: egui::PointerButton::Secondary, pressed, .. } => Some(*pressed),
                _ => None,
            })
            .collect();
        assert_eq!(presses, [true, false]);
    }

    #[test]
    fn both_platforms_thermal_states_fold_into_four_steps() {
        let apple: Vec<Thermal> = (0..5).map(Thermal::from_apple).collect();
        assert_eq!(apple, [Thermal::Nominal, Thermal::Fair, Thermal::Serious, Thermal::Critical, Thermal::Critical]);
        let android: Vec<Thermal> = (-1..8).map(Thermal::from_android).collect();
        assert_eq!(
            android,
            [
                Thermal::Nominal,
                Thermal::Nominal,
                Thermal::Fair,
                Thermal::Fair,
                Thermal::Serious,
                Thermal::Critical,
                Thermal::Critical,
                Thermal::Critical,
                Thermal::Critical
            ]
        );
    }

    #[test]
    fn a_hot_device_or_power_saving_asks_for_less() {
        let power = |thermal, low_power| Power { thermal, low_power }.constrained();
        assert!(!power(Thermal::Nominal, false));
        assert!(!power(Thermal::Fair, false), "warm is still normal");
        assert!(power(Thermal::Serious, false));
        assert!(power(Thermal::Critical, false));
        assert!(power(Thermal::Nominal, true), "the user asked to save power");
        let rate = |thermal, low_power| Power { thermal, low_power }.max_frame_rate();
        assert_eq!(rate(Thermal::Fair, false), None);
        assert_eq!(rate(Thermal::Nominal, true), Some(60));
        assert_eq!(rate(Thermal::Serious, false), Some(60));
        assert_eq!(rate(Thermal::Critical, true), Some(30));
    }

    #[test]
    fn an_upright_stylus_has_no_tilt_and_a_flat_one_leans_fully() {
        let upright = Stylus {
            id: 1,
            phase: TouchPhase::Move,
            pos: Pos2::ZERO,
            pressure: 0.5,
            altitude: std::f32::consts::FRAC_PI_2,
            azimuth: 1.0,
            eraser: false,
        };
        let (x, y) = upright.tilt_degrees();
        assert!(x.abs() < 1e-3 && y.abs() < 1e-3, "{x} {y}");
        let flat_right = Stylus { altitude: 0.0, azimuth: 0.0, ..upright };
        let (x, y) = flat_right.tilt_degrees();
        assert!((x - 90.0).abs() < 1e-3 && y.abs() < 1e-3, "{x} {y}");
    }
}
