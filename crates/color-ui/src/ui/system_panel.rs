//! The macOS Colors panel standing in for [`ColorPickerDialog`](super::ColorPickerDialog) once an app turns it on with
//! [`use_system_color_panel`].
//!
//! The panel is the shared `NSColorPanel`, as other Mac apps show it for More Colors: it opens on the dialog's colour
//! with Cancel and OK under it, and answers once, as the dialog does. OK (or Return) answers with the colour the panel
//! shows; Cancel, Escape or closing the panel answers Cancel. The buttons are push-on/push-off buttons whose state the
//! dialog reads each pass, so no Objective-C target or action (and no `unsafe`) is needed.
//!
//! AppKit objects stay on the main thread, so the panel is held there; a dialog holds a [`Shown`], which puts the
//! panel away when the dialog goes. The panel is shared, so one dialog has it at a time: another picker opened
//! meanwhile shows in its window, and each `Shown` reads, sets and closes only its own session.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use craft_theme::Srgba;

static SYSTEM: AtomicBool = AtomicBool::new(false);

/// The token of the next session, so a `Shown` acts only on its own.
static NEXT: AtomicU64 = AtomicU64::new(1);

/// Shows every [`ColorPickerDialog`](super::ColorPickerDialog) as the system Colors panel, with Cancel and OK, instead
/// of in the window. Only macOS has one; elsewhere this does nothing. An app calls it once at start; tests and apps
/// that don't keep the in-window dialog.
pub fn use_system_color_panel(on: bool) {
    SYSTEM.store(on, Ordering::Relaxed);
}

/// Whether the system Colors panel is up for a colour picker dialog now. The panel is shared: an app that also takes
/// its `changeColor:` (a font panel's text colour) ignores those while this is true, as they are the dialog's picks.
pub fn system_color_panel_shown() -> bool {
    imp::shown()
}

/// Whether the dialog shows as the system panel.
pub(crate) fn wanted() -> bool {
    cfg!(target_os = "macos") && SYSTEM.load(Ordering::Relaxed)
}

/// Takes the window's clicks and drags while the panel is up, as a modal dialog's layer does.
pub(crate) fn block_behind(ctx: &egui::Context) {
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::unique("craft_color_system_panel_modal"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| ui.allocate_rect(screen, egui::Sense::CLICK | egui::Sense::DRAG));
}

/// The panel's answer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Answer {
    Ok(Srgba),
    Cancel,
}

/// The panel is up for a dialog; dropping this puts it away.
#[derive(Debug)]
pub(crate) struct Shown(u64);

impl Shown {
    /// Shows the panel on `color` with the buttons `ok` and `cancel`. `None` when it can't show: off the main thread,
    /// not macOS, or another dialog has the panel.
    pub(crate) fn open(color: Srgba, ok: &str, cancel: &str) -> Option<Shown> {
        let token = NEXT.fetch_add(1, Ordering::Relaxed);
        imp::open(token, color, ok, cancel).then_some(Shown(token))
    }

    /// The answer, once there is one: a pressed button, or the panel closed. Its colour is opaque; the dialog keeps
    /// its own alpha, as the panel shows none.
    pub(crate) fn poll(&self) -> Option<Answer> {
        imp::poll(self.0)
    }

    /// Shows `color` in the panel: an edit made while it is up (automation, a screen pick).
    pub(crate) fn set(&self, color: Srgba) {
        imp::set(self.0, color);
    }
}

impl Drop for Shown {
    fn drop(&mut self) {
        imp::close(self.0);
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly as _};
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSBezelStyle, NSButton, NSButtonType, NSColor, NSColorPanel, NSColorSpace, NSControlStateValueOn, NSView,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    use super::Answer;
    use craft_theme::Srgba;

    /// The width of each button, the gap between them and the room around them, in points.
    const BUTTON: f64 = 90.0;
    const GAP: f64 = 12.0;
    const MARGIN: f64 = 16.0;
    const HEIGHT: f64 = 44.0;

    /// The shared Colors panel and its buttons while a dialog shows it, with the dialog's session token.
    struct Panel {
        token: u64,
        panel: Retained<NSColorPanel>,
        ok: Retained<NSButton>,
        cancel: Retained<NSButton>,
    }

    thread_local! {
        static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
    }

    pub(super) fn open(token: u64, color: Srgba, ok: &str, cancel: &str) -> bool {
        let Some(mtm) = MainThreadMarker::new() else { return false };
        if shown() {
            return false;
        }
        let panel = NSColorPanel::sharedColorPanel(mtm);
        let width = MARGIN * 2.0 + BUTTON * 2.0 + GAP;
        let accessory = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, HEIGHT)));
        let button = |title: &str, key: &str, x: f64| {
            let b = NSButton::initWithFrame(NSButton::alloc(mtm), NSRect::new(NSPoint::new(x, 8.0), NSSize::new(BUTTON, 28.0)));
            b.setButtonType(NSButtonType::PushOnPushOff);
            b.setBezelStyle(NSBezelStyle::Push);
            b.setTitle(&NSString::from_str(title));
            b.setKeyEquivalent(&NSString::from_str(key));
            b.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
            accessory.addSubview(&b);
            b
        };
        // The platform's order: Cancel, then OK at the end edge.
        let cancel = button(cancel, "\u{1b}", MARGIN);
        let ok = button(ok, "\r", MARGIN + BUTTON + GAP);
        panel.setShowsAlpha(false);
        panel.setColor(&ns_color(color));
        panel.setAccessoryView(Some(&accessory));
        panel.makeKeyAndOrderFront(None);
        PANEL.with_borrow_mut(|shown| *shown = Some(Panel { token, panel, ok, cancel }));
        true
    }

    fn ns_color(color: Srgba) -> Retained<NSColor> {
        NSColor::colorWithSRGBRed_green_blue_alpha(color.r.into(), color.g.into(), color.b.into(), color.a.into())
    }

    pub(super) fn shown() -> bool {
        PANEL.with_borrow(Option::is_some)
    }

    pub(super) fn poll(token: u64) -> Option<Answer> {
        PANEL.with_borrow(|shown| {
            let Panel { panel, ok, cancel, .. } = shown.as_ref().filter(|p| p.token == token)?;
            if ok.state() == NSControlStateValueOn {
                return Some(Answer::Ok(srgb(&panel.color())));
            }
            (cancel.state() == NSControlStateValueOn || !panel.isVisible()).then_some(Answer::Cancel)
        })
    }

    pub(super) fn set(token: u64, color: Srgba) {
        PANEL.with_borrow(|shown| {
            if let Some(p) = shown.as_ref().filter(|p| p.token == token) {
                p.panel.setColor(&ns_color(color));
            }
        });
    }

    /// The panel goes, without the buttons: the next user of the shared panel (a font panel's colour) gets it plain.
    pub(super) fn close(token: u64) {
        let mine = PANEL.with_borrow_mut(|shown| if shown.as_ref().is_some_and(|p| p.token == token) { shown.take() } else { None });
        if let Some(Panel { panel, .. }) = mine {
            panel.setAccessoryView(None);
            panel.orderOut(None);
        }
    }

    /// `color` in sRGB.
    fn srgb(color: &NSColor) -> Srgba {
        let srgb = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace());
        let color = srgb.as_deref().unwrap_or(color);
        #[expect(clippy::cast_possible_truncation, reason = "a colour component is in 0..=1")]
        let unit = |v: f64| v.clamp(0.0, 1.0) as f32;
        Srgba { r: unit(color.redComponent()), g: unit(color.greenComponent()), b: unit(color.blueComponent()), a: 1.0 }
    }
}

/// Elsewhere there is no system panel: [`wanted`] is false, so nothing opens.
#[cfg(not(target_os = "macos"))]
mod imp {
    use super::Answer;
    use craft_theme::Srgba;

    pub(super) fn open(_: u64, _: Srgba, _: &str, _: &str) -> bool {
        false
    }

    pub(super) fn shown() -> bool {
        false
    }

    pub(super) fn poll(_: u64) -> Option<Answer> {
        None
    }

    pub(super) fn set(_: u64, _: Srgba) {}

    pub(super) fn close(_: u64) {}
}
