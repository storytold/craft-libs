//! The Win32 side of [`crate::native`]: a window procedure that answers `WM_NCHITTEST` from the app's frame parts and
//! reports the maximize button's pointer events, finding an app window among the thread's windows, and layered-window
//! opacity.
//!
//! No `unsafe` here: craft-win32 holds the subclassing and handle calls, and winsafe wraps the rest.

use crate::Button;
use crate::native::{CaptionEvent, Hits, hit_code};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use winsafe::{self as w, co};

/// `HTMAXBUTTON`, the hit-test result and the `wParam` of the non-client messages over the maximize button.
const HT_MAX_BUTTON: usize = 9;
/// `HTCAPTION`, the `wParam` of the non-client messages over the drag area.
const HT_CAPTION: usize = 2;

/// A window of this process by its handle value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Hwnd(isize);

impl Hwnd {
    pub(crate) fn new(raw: isize) -> Self {
        Hwnd(raw)
    }

    fn of(window: &w::HWND) -> Self {
        Hwnd(window.ptr() as isize)
    }

    fn handle(self) -> w::HWND {
        craft_win32::handle(self.0)
    }
}

/// A routed window: where the hit test goes, and what the pointer on the maximize button did so far.
struct Routed {
    hits: Arc<dyn Hits>,
    /// `TrackMouseEvent` is armed for the non-client leave.
    tracking: bool,
    /// The pointer is over the maximize button.
    over: bool,
    /// The primary button went down on the maximize button.
    pressed: bool,
}

static ROUTED: LazyLock<Mutex<HashMap<isize, Routed>>> = LazyLock::new(Default::default);

fn routed<R>(f: impl FnOnce(&mut HashMap<isize, Routed>) -> R) -> R {
    f(&mut ROUTED.lock().unwrap_or_else(PoisonError::into_inner))
}

/// How far, in physical pixels, a window's client origin may lie from the one the app reported.
const ORIGIN_SLOP: i32 = 2;

/// The window of this thread titled `title` that is not routed yet. With `client_origin` (physical screen pixels) it
/// must be the only such window whose client area starts within [`ORIGIN_SLOP`] of it; without, the only one. `None`
/// when that is not exactly one window, so two windows with one title are never told apart by guessing.
pub(crate) fn find(title: &str, client_origin: Option<[i32; 2]>) -> Option<Hwnd> {
    let mut candidates = Vec::new();
    let listed = w::EnumThreadWindows(w::GetCurrentThreadId(), |window: w::HWND| {
        let here = Hwnd::of(&window);
        if window.GetWindowText().is_ok_and(|text| text == title) && !routed(|routed| routed.contains_key(&here.0)) {
            let near = client_origin.is_none_or(|[x, y]| {
                window
                    .ClientToScreen(w::POINT::with(0, 0))
                    .is_ok_and(|origin| (origin.x - x).abs() <= ORIGIN_SLOP && (origin.y - y).abs() <= ORIGIN_SLOP)
            });
            if near {
                candidates.push(here);
            }
        }
        true
    });
    listed.ok()?;
    match candidates.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

/// The windows whose layered style this module added, so that making one opaque again removes only that.
static LAYERED_BY_US: LazyLock<Mutex<HashSet<isize>>> = LazyLock::new(Default::default);

fn layered_by_us<R>(f: impl FnOnce(&mut HashSet<isize>) -> R) -> R {
    f(&mut LAYERED_BY_US.lock().unwrap_or_else(PoisonError::into_inner))
}

/// Sets the window's opacity through a layered window, and sets it again on every call: the window system's own
/// style changes can drop the layered style, so a caller that keeps a window faded calls this each pass. Fully opaque
/// takes the layered style off again when this module added it.
pub(crate) fn set_opacity(hwnd: Hwnd, opacity: f32) -> bool {
    let window = hwnd.handle();
    let layered = window.style_ex().has(co::WS_EX::LAYERED);
    if opacity >= 1.0 {
        if layered && layered_by_us(|ours| ours.remove(&hwnd.0)) {
            window.set_style_ex(window.style_ex() & !co::WS_EX::LAYERED);
        }
        return true;
    }
    if !layered {
        window.set_style_ex(window.style_ex() | co::WS_EX::LAYERED);
        layered_by_us(|ours| ours.insert(hwnd.0));
    }
    let alpha = (opacity.max(0.0) * 255.0).round() as u8;
    window.SetLayeredWindowAttributes(w::COLORREF::from_rgb(0, 0, 0), alpha, co::LWA::ALPHA).is_ok()
}

/// Puts the procedure on `hwnd`, answering the hit test and the maximize button's pointer from `hits`. Returns whether
/// the window is routed (a window routed before keeps its first `hits`).
pub(crate) fn route(hwnd: Hwnd, hits: Arc<dyn Hits>) -> bool {
    if routed(|routed| routed.contains_key(&hwnd.0)) {
        return true;
    }
    if !craft_win32::subclass(hwnd.0, handler) {
        return false;
    }
    routed(|routed| routed.insert(hwnd.0, Routed { hits, tracking: false, over: false, pressed: false }));
    true
}

/// The messages this module answers; the window's own procedure gets the rest.
fn handler(window: &w::HWND, message: co::WM, wparam: usize, lparam: isize) -> Option<isize> {
    if message == co::WM::NCDESTROY {
        routed(|routed| routed.remove(&Hwnd::of(window).0));
        return None;
    }
    answer(window, message, wparam, lparam)
}

/// The result of a message this module handles, `None` to leave it to the window.
fn answer(window: &w::HWND, message: co::WM, wparam: usize, lparam: isize) -> Option<isize> {
    let key = Hwnd::of(window).0;
    let hits = routed(|routed| routed.get(&key).map(|held| held.hits.clone()))?;
    match message {
        co::WM::NCHITTEST => {
            let zone = hits.zone_at(client_of(window, lparam)?);
            hit_code(zone).and_then(|code| isize::try_from(code).ok())
        }
        co::WM::NCMOUSEMOVE if wparam == HT_MAX_BUTTON => {
            let (arm, entered) = routed(|routed| {
                let held = routed.get_mut(&key)?;
                let arm = !std::mem::replace(&mut held.tracking, true);
                Some((arm, !std::mem::replace(&mut held.over, true)))
            })?;
            if arm {
                track_leave(window);
            }
            if entered {
                hits.caption(CaptionEvent::Hover(Button::Maximize));
            }
            Some(0)
        }
        co::WM::NCMOUSEMOVE => {
            left(&key, &hits);
            None
        }
        co::WM::NCMOUSELEAVE => {
            routed(|routed| routed.get_mut(&key).map(|held| held.tracking = false));
            left(&key, &hits);
            None
        }
        co::WM::NCLBUTTONDOWN | co::WM::NCLBUTTONDBLCLK if wparam == HT_MAX_BUTTON => {
            routed(|routed| routed.get_mut(&key).map(|held| held.pressed = true));
            hits.caption(CaptionEvent::Press(Button::Maximize));
            Some(0)
        }
        // The window's own handling of this release would maximize the window; the app's button does it.
        co::WM::NCLBUTTONUP if wparam == HT_MAX_BUTTON => {
            if routed(|routed| routed.get_mut(&key).is_some_and(|held| std::mem::take(&mut held.pressed))) {
                hits.caption(CaptionEvent::Release(Button::Maximize));
            }
            Some(0)
        }
        // The drag area is the window's caption to the system, which would maximize on a double click and open the
        // system menu on a right click; the app's band decides both, so the pointer events reach it.
        co::WM::NCLBUTTONDBLCLK if wparam == HT_CAPTION => {
            hits.caption(CaptionEvent::BandDouble(client_of(window, lparam)?));
            Some(0)
        }
        co::WM::NCRBUTTONDOWN if wparam == HT_CAPTION => Some(0),
        co::WM::NCRBUTTONUP if wparam == HT_CAPTION => {
            hits.caption(CaptionEvent::BandMenu(client_of(window, lparam)?));
            Some(0)
        }
        _ => None,
    }
}

/// The client pixel a mouse message's screen position `lparam` (signed low and high words) names.
fn client_of(window: &w::HWND, lparam: isize) -> Option<[f32; 2]> {
    let screen = w::POINT::with(i32::from(lparam as i16), i32::from((lparam >> 16) as i16));
    let client = window.ScreenToClient(screen).ok()?;
    Some([client.x as f32, client.y as f32])
}

/// The pointer is no longer on the maximize button: its hover ends, and a press that began there is cancelled.
fn left(key: &isize, hits: &Arc<dyn Hits>) {
    let ended = routed(|routed| {
        let held = routed.get_mut(key)?;
        let was = std::mem::take(&mut held.over);
        was.then(|| std::mem::take(&mut held.pressed))
    });
    match ended {
        Some(true) => hits.caption(CaptionEvent::Cancel),
        Some(false) => hits.caption(CaptionEvent::Leave),
        None => {}
    }
}

/// Asks for a `WM_NCMOUSELEAVE` when the pointer leaves the window's non-client area.
fn track_leave(window: &w::HWND) {
    let mut track = w::TRACKMOUSEEVENT::default();
    track.dwFlags = co::TME::LEAVE | co::TME::NONCLIENT;
    track.hwndTrack = Hwnd::of(window).handle();
    // A failed request only means the leave is noticed at the next move over another zone.
    let _ = w::TrackMouseEvent(&mut track);
}
