//! Wheel and pinch navigation as pure rules: a pass's wheel share is classified into a [`WheelAction`] and applied to
//! a [`CanvasView`].
//!
//! The share comes from the wheel service; nothing here reads input. A [`Canvas`](crate::canvas::Canvas) given a view
//! reads its share with these rules and applies it to the view it reports; a wheel zoom follows the wheel at once,
//! with no ease.

use super::{NavigationOptions, WheelScheme};
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, Wheel, WheelNotch, WheelUnit, ZoomScheme};
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
    /// Page the view up or down by this many notches, whose wheel line in points is the second value: one page
    /// is the canvas's height less that line, so a page leaves one line of the page before it in sight. A page is a
    /// screen distance, like a pan, so it holds at every zoom.
    Page(f32, f32),
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
    /// The unit the device reported: `Line` for a wheel with notches, `Point` for a trackpad or a smooth scroll,
    /// `Page` for a device that counts whole pages.
    pub unit: WheelUnit,
    /// The pinch factor of a two-finger touch; 1 for none.
    pub touch_zoom: f32,
    /// The trackpad's rotation, in radians.
    pub rotation: f32,
    /// The twist of a two-finger touch, in radians.
    pub touch_rotation: f32,
    /// The pan of a multi-finger touch, in points.
    pub touch_pan: Vec2,
}

impl WheelInput {
    /// The input of `wheel` for a taker whose wheel line is `line` points, on `os`. A line is one zoom notch; a
    /// device that scrolls in points zooms a notch every default line ([`WheelNotch::DEFAULT`]), whatever the taker's
    /// line.
    pub fn of(wheel: &Wheel, line: f32, os: Platform) -> Self {
        let notch = if wheel.unit == WheelUnit::Line { line } else { WheelNotch::DEFAULT.points(0.0) };
        WheelInput {
            scroll: wheel.scroll,
            zoom: wheel.zoom,
            mods: wheel.mods,
            notch,
            os,
            unit: wheel.unit,
            touch_zoom: wheel.touch_zoom,
            rotation: wheel.rotation,
            touch_rotation: wheel.touch_rotation,
            touch_pan: wheel.touch_pan,
        }
    }

    /// The scroll along the wheel's axis whichever one the device reported it on (some systems send a modified wheel
    /// sideways).
    fn along(&self) -> f32 {
        self.scroll.x + self.scroll.y
    }
}

/// Mouse wheel and trackpad navigation, as in Photoshop:
///
/// - The wheel scrolls up and down; Cmd/Ctrl + wheel scrolls sideways; Shift + wheel pages the view up or down by its
///   height less one line (production's #12029). macOS keeps Shift + wheel sideways, where the system reports it that
///   way itself, and so does a device that reports its scroll as points rather than notches.
/// - Alt + wheel zooms about the pointer, 1.1 a notch; Shift + Alt + wheel steps the ladder. Alt with the Control
///   key is the same zoom: on macOS Control may join it, elsewhere Ctrl (cmd) without Shift (production's #10488).
/// - A wheel line pans the canvas [`NavigationOptions::wheel_line`] points; a zoom is one notch a line whatever the
///   wheel line.
/// - [`WheelScheme::Zoom`] (Zoom with Scroll Wheel) swaps them: the wheel zooms, Alt + wheel scrolls, Shift + wheel
///   steps the ladder.
/// - [`WheelScheme::Acrobat`]: Cmd/Ctrl + wheel zooms about the pointer, [`WheelScheme::ACROBAT_NOTCH`] a line;
///   Shift + wheel scrolls sideways; there is no ladder step.
/// - A trackpad pinch zooms about the pointer (Zoom with Trackpad Pinch).
///
/// That is [`WheelScheme::Scroll`]. Under [`WheelScheme::Document`], a document editor's scheme:
///
/// - The wheel scrolls up and down; Shift + wheel scrolls sideways.
/// - Ctrl or Cmd + wheel (without Alt) steps the zoom ladder one rung a notch about the pointer.
/// - A pinch zooms as in the Photoshop scheme; Zoom with Scroll Wheel does not apply.
///
/// The value keeps the part of a ladder-step notch a trackpad has not finished, so slow scrolling still steps.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelNavigation {
    residual: f32,
}

impl WheelNavigation {
    /// The zoom factor of one wheel notch.
    pub const NOTCH: f64 = 1.1;

    /// The most ladder steps one pass takes.
    const MOST_STEPS: f32 = 10.0;

    /// The view change for `wheel`, given to a taker whose wheel line is `line` points, on `os`, under `options`, if
    /// any.
    pub fn read(&mut self, wheel: &Wheel, line: f32, os: Platform, options: &NavigationOptions) -> Option<WheelAction> {
        self.classify(WheelInput::of(wheel, line, os), options)
    }

    /// The degrees `input` adds to the view's rotation: the trackpad's with `trackpad_rotate`, the fingers' twist with
    /// `touch_gestures`; `None` for none.
    /// The trackpad's rotation wins when the trackpad and the fingers both report one.
    pub fn rotation(input: &WheelInput, options: &NavigationOptions) -> Option<f64> {
        if !options.trackpad_rotate {
            return None;
        }
        let radians = if input.rotation != 0.0 {
            input.rotation
        } else if options.touch_gestures {
            input.touch_rotation
        } else {
            return None;
        };
        (radians.is_finite() && radians != 0.0).then(|| f64::from(radians).to_degrees())
    }

    /// Whether `mods` make the Alt zoom gesture on `os`: Alt, with the Control key allowed, which is `ctrl` on macOS
    /// (⌘ keeps the wheel out of it) and `cmd` elsewhere, where Shift with it keeps the wheel a scroll.
    fn alt_gesture(mods: Mods, os: Platform) -> bool {
        mods.alt && (!mods.cmd || (!os.is_mac() && !mods.shift))
    }

    /// The view change for one pass of `input` under `options`, if any.
    pub fn classify(&mut self, input: WheelInput, options: &NavigationOptions) -> Option<WheelAction> {
        let notch = if input.notch.is_finite() && input.notch > 0.0 { input.notch } else { 1.0 };
        // The fingers' pinch counts only with Enable Gestures; the trackpad's whatever it says.
        let factor = input.zoom * if options.touch_gestures { input.touch_zoom } else { 1.0 };
        let pinch = factor.is_finite() && factor > 0.0 && factor != 1.0;
        if options.pinch_zooms && pinch {
            self.residual = 0.0;
            return Some(WheelAction::Zoom(f64::from(factor)));
        }
        // Two fingers moving together pan before any modifier rule reads the pass; the content follows them.
        if options.touch_gestures && input.touch_pan != Vec2::ZERO && input.touch_pan.is_finite() {
            self.residual = 0.0;
            return Some(WheelAction::Pan(input.touch_pan));
        }
        let scroll_finite = input.scroll.x.is_finite() && input.scroll.y.is_finite();
        let plain = !input.mods.cmd && !input.mods.ctrl;
        // Whether the modifiers zoom, the zoom factor of one line, and whether Shift turns the zoom into ladder steps.
        let (zooms, line_factor, ladder) = match options.wheel {
            WheelScheme::Scroll => (Self::alt_gesture(input.mods, input.os), Self::NOTCH, true),
            WheelScheme::Zoom => (plain && !input.mods.alt, Self::NOTCH, true),
            WheelScheme::Acrobat => (!plain, WheelScheme::ACROBAT_NOTCH, false),
            WheelScheme::Document => return self.classify_document(input, notch, scroll_finite),
        };
        if zooms && ladder && input.mods.shift {
            return scroll_finite.then(|| self.snap(input.along(), notch)).flatten().map(WheelAction::SnapZoom);
        }
        self.residual = 0.0;
        if zooms {
            let factor = line_factor.powf(f64::from(input.along() / notch));
            return (scroll_finite && factor.is_finite() && factor > 0.0 && factor != 1.0).then_some(WheelAction::Zoom(factor));
        }
        if !scroll_finite || input.scroll == Vec2::ZERO {
            return None;
        }
        // Shift alone on a wheel that reports notches pages the view in the Photoshop (Scroll) scheme (production's
        // #12029): the Zoom scheme took its ladder step above, and the Acrobat scheme keeps Shift sideways below, as
        // next pinned it. Cmd/Ctrl keeps the sideways scroll below; so does macOS, where the system already sends
        // Shift + wheel sideways, and so does any scroll the device reports across (a trackpad's points, a
        // pre-folded wheel), which the reference application was not measured with.
        if matches!(options.wheel, WheelScheme::Scroll)
            && input.mods.shift
            && !(input.mods.cmd || input.mods.ctrl || input.mods.alt)
            && input.unit == WheelUnit::Line
            && !input.os.is_mac()
            && input.scroll.x == 0.0
        {
            // The count comes from scroll over the line the taker reads with, so a taker of another line pages by its own notch.
            return Some(WheelAction::Page(input.scroll.y / notch, notch));
        }
        let sideways = (input.mods.shift || input.mods.cmd || input.mods.ctrl) && input.scroll.x == 0.0;
        let scroll = if sideways { Vec2::new(input.scroll.y, 0.0) } else { input.scroll };
        Some(WheelAction::Pan(scroll))
    }

    /// The Document scheme's wheel: Ctrl or Cmd without Alt steps whole ladder rungs (a trackpad's partial notches add
    /// up in the residual); Shift scrolls sideways; anything else scrolls. `plain` is not used for the zoom test because
    /// it excludes the very keys that zoom here. Every pass that does not zoom drops the residual, so a scroll between
    /// two half notches never completes a rung.
    fn classify_document(&mut self, input: WheelInput, notch: f32, scroll_finite: bool) -> Option<WheelAction> {
        let zooms = (input.mods.cmd || input.mods.ctrl) && !input.mods.alt;
        if zooms {
            return scroll_finite.then(|| self.snap(input.along(), notch)).flatten().map(WheelAction::SnapZoom);
        }
        self.residual = 0.0;
        if !scroll_finite || input.scroll == Vec2::ZERO {
            return None;
        }
        let sideways = input.mods.shift && input.scroll.x == 0.0;
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
    /// [`zoom::wheel_anchor`], a ladder step at the pointer, both clamped to the ladder; a pan moves the view; a page
    /// moves it up or down by the view's height less the action's wheel line ([`WheelAction::Page`]). At a zoom
    /// limit the view stays put; Overscroll off clamps the result to the document. Returns if the view changed.
    pub fn apply(view: &mut CanvasView, action: WheelAction, pointer: Pos2, size: Size, pixels_per_point: f32, scheme: &ZoomScheme) -> bool {
        let before = *view;
        match action {
            WheelAction::Zoom(factor) => {
                let current = zoom::device(view, pixels_per_point);
                let next = scheme.clamp(current * factor, size);
                if factor.is_finite() && factor > 0.0 && (next / current - 1.0).abs() > 1e-12 {
                    let at = zoom::wheel_anchor(view, pointer, size);
                    zoom::about(view, at, next, false, pixels_per_point);
                }
            }
            WheelAction::SnapZoom(steps) => {
                let dir = steps.signum();
                let current = zoom::device(view, pixels_per_point);
                let next = (0..steps.unsigned_abs()).fold(current, |z, _| scheme.step(z, dir, size));
                if dir != 0 && (next / current - 1.0).abs() > 1e-12 {
                    zoom::about(view, pointer, next, false, pixels_per_point);
                }
            }
            WheelAction::Pan(delta) => super::pan(view, delta),
            WheelAction::Page(notches, line) => {
                let page = (view.rect.height() - line).max(0.0);
                let step = page * notches;
                if step.is_finite() && step != 0.0 {
                    super::pan(view, Vec2::new(0.0, step));
                }
            }
        }
        *view != before
    }

    /// The screen point [`apply`](Self::apply) keeps still when `action` zooms `view` with the pointer at `pointer`, for
    /// a document of `size`; `None` for a pan.
    pub fn zoom_anchor(view: &CanvasView, action: WheelAction, pointer: Pos2, size: Size) -> Option<Pos2> {
        match action {
            WheelAction::Zoom(_) => Some(zoom::wheel_anchor(view, pointer, size)),
            WheelAction::SnapZoom(_) => Some(pointer),
            WheelAction::Pan(_) | WheelAction::Page(_, _) => None,
        }
    }
}
