//! Wheel and pinch navigation as pure rules: a pass's wheel share is classified into a [`WheelAction`] and applied to
//! a [`CanvasView`].
//!
//! The share comes from the wheel service; nothing here reads input. A [`Canvas`](crate::canvas::Canvas) given a view
//! reads its share with these rules and applies it to the view it reports; a wheel zoom follows the wheel at once,
//! with no ease.

use super::NavigationOptions;
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, Wheel, WheelNotch, WheelUnit, ZoomLadder};
use craft_keymap::{Mods, Platform};
use egui::{Pos2, Vec2};

/// What the wheel or a pinch does to the view this pass.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum WheelAction {
    /// Multiply the zoom by this factor about the pointer (1.1 a notch).
    Zoom(f64),
    /// Step this many ladder rungs about the pointer (positive in, negative out).
    SnapZoom(i32),
    /// Move the image by this many screen points: the content follows the delta.
    Pan(Vec2),
}

/// One pass of wheel input over the canvas: the wheel share without the service's bookkeeping.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WheelInput {
    /// Scroll in points, content direction.
    pub scroll: Vec2,
    /// The pinch factor; 1 for none.
    pub zoom: f32,
    /// The modifiers held.
    pub mods: Mods,
    /// Points per zoom notch.
    pub notch: f32,
    /// The OS, which decides where the Control key is among the modifiers.
    pub os: Platform,
}

impl WheelInput {
    /// The input of `wheel` for a taker whose wheel line is `line` points, on `os`. A line is one zoom notch; a
    /// device that scrolls in points zooms a notch every default line ([`WheelNotch::DEFAULT`]), whatever the taker's
    /// line.
    pub fn of(wheel: &Wheel, line: f32, os: Platform) -> Self {
        let notch = if wheel.unit == WheelUnit::Line { line } else { WheelNotch::DEFAULT.points(0.0) };
        WheelInput { scroll: wheel.scroll, zoom: wheel.zoom, mods: wheel.mods, notch, os }
    }

    /// The scroll along the wheel's axis whichever one the device reported it on (some systems send a modified wheel
    /// sideways).
    fn along(&self) -> f32 {
        self.scroll.x + self.scroll.y
    }
}

/// Mouse wheel and trackpad navigation, as in Photoshop:
///
/// - The wheel scrolls up and down; Shift or Cmd/Ctrl + wheel scrolls sideways.
/// - Alt + wheel zooms about the pointer, 1.1 a notch; Shift + Alt + wheel steps the ladder. Alt with the Control
///   key is the same zoom: on macOS Control may join it, elsewhere Ctrl (cmd) without Shift (production's #10488).
/// - A wheel line pans the canvas [`WheelNavigation::LINE`] points.
/// - Zoom with Scroll Wheel swaps them: the wheel zooms, Alt + wheel scrolls, Shift + wheel steps the ladder.
/// - A trackpad pinch zooms about the pointer (Zoom with Trackpad Pinch).
///
/// The value keeps the part of a ladder-step notch a trackpad has not finished, so slow scrolling still steps.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelNavigation {
    residual: f32,
}

impl WheelNavigation {
    /// The zoom factor of one wheel notch.
    pub const NOTCH: f64 = 1.1;

    /// The points one wheel line pans the canvas: Photoshop's 16 (production's #12028).
    pub const LINE: f32 = 16.0;

    /// The most ladder steps one pass takes.
    const MOST_STEPS: f32 = 10.0;

    /// The view change for `wheel`, given to a taker whose wheel line is `line` points, on `os`, under `options`, if
    /// any.
    pub fn read(&mut self, wheel: &Wheel, line: f32, os: Platform, options: &NavigationOptions) -> Option<WheelAction> {
        self.classify(WheelInput::of(wheel, line, os), options)
    }

    /// Whether `mods` make the Alt zoom gesture on `os`: Alt, with the Control key allowed, which is `ctrl` on macOS
    /// (⌘ keeps the wheel out of it) and `cmd` elsewhere, where Shift with it keeps the wheel a scroll.
    fn alt_gesture(mods: Mods, os: Platform) -> bool {
        mods.alt && (!mods.cmd || (!os.is_mac() && !mods.shift))
    }

    /// The view change for one pass of `input` under `options`, if any.
    pub fn classify(&mut self, input: WheelInput, options: &NavigationOptions) -> Option<WheelAction> {
        let notch = if input.notch.is_finite() && input.notch > 0.0 { input.notch } else { 1.0 };
        let pinch = input.zoom.is_finite() && input.zoom > 0.0 && input.zoom != 1.0;
        if options.pinch_zooms && pinch {
            self.residual = 0.0;
            return Some(WheelAction::Zoom(f64::from(input.zoom)));
        }
        let scroll_finite = input.scroll.x.is_finite() && input.scroll.y.is_finite();
        let plain = !input.mods.cmd && !input.mods.ctrl;
        let zooms = if options.zoom_with_wheel { plain && !input.mods.alt } else { Self::alt_gesture(input.mods, input.os) };
        if zooms && input.mods.shift {
            return scroll_finite.then(|| self.snap(input.along(), notch)).flatten().map(WheelAction::SnapZoom);
        }
        self.residual = 0.0;
        if zooms {
            let factor = Self::NOTCH.powf(f64::from(input.along() / notch));
            return (scroll_finite && factor.is_finite() && factor > 0.0 && factor != 1.0).then_some(WheelAction::Zoom(factor));
        }
        if !scroll_finite || input.scroll == Vec2::ZERO {
            return None;
        }
        let sideways = (input.mods.shift || input.mods.cmd || input.mods.ctrl) && input.scroll.x == 0.0;
        let scroll = if sideways { Vec2::new(input.scroll.y, 0.0) } else { input.scroll };
        Some(WheelAction::Pan(scroll))
    }

    /// The whole ladder steps `dy` points add to what was left over.
    fn snap(&mut self, dy: f32, notch: f32) -> Option<i32> {
        let total = self.residual + dy / notch;
        if !total.is_finite() {
            self.residual = 0.0;
            return None;
        }
        let steps = total.trunc().clamp(-Self::MOST_STEPS, Self::MOST_STEPS);
        self.residual = total - steps;
        (steps != 0.0).then_some(steps as i32)
    }

    /// Applies `action` to `view` with the pointer at `pointer`, for a document of `size`: a factor zoom anchors at
    /// [`zoom::wheel_anchor`], a ladder step at the pointer, both clamped to the ladder; a pan moves the view. At a
    /// zoom limit the view stays put. With Overscroll off the canvas clamps the result to the document. Returns
    /// whether the view changed.
    pub fn apply(view: &mut CanvasView, action: WheelAction, pointer: Pos2, size: Size, pixels_per_point: f32) -> bool {
        let before = *view;
        match action {
            WheelAction::Zoom(factor) => {
                let current = zoom::device(view, pixels_per_point);
                let next = ZoomLadder::clamp(current * factor, size);
                if factor.is_finite() && factor > 0.0 && (next / current - 1.0).abs() > 1e-12 {
                    let at = zoom::wheel_anchor(view, pointer, size);
                    zoom::about(view, at, next, false, pixels_per_point);
                }
            }
            WheelAction::SnapZoom(steps) => {
                let dir = steps.signum();
                let current = zoom::device(view, pixels_per_point);
                let next = (0..steps.unsigned_abs()).fold(current, |z, _| ZoomLadder::step(z, dir, size));
                if dir != 0 && (next / current - 1.0).abs() > 1e-12 {
                    zoom::about(view, pointer, next, false, pixels_per_point);
                }
            }
            WheelAction::Pan(delta) => super::pan(view, delta),
        }
        *view != before
    }
}
