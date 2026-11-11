//! `CropOverlay`: the frame's gestures and keys through real input.

use craft_behavior::geometry::{BoxPart, CropBox, SnapKind, SnapLine, SnapTarget, SnapTargets, crop_view};
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{Axis, CanvasView, EditEvent};
use craft_ui::canvas::{Canvas, CropOverlay};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Frames, layout, pointer, trial};
use egui::{Event, Modifiers, Pos2, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

// Real input through the canvas: a frame per event batch, the view and the shown options held by the rig.

/// The room the rig shows the canvas in.
const ROOM: Vec2 = Vec2::new(400.0, 300.0);

/// The frame the drags start from, centred in the room's default view.
const RECT: Rect = Rect::new(100.0, 80.0, 300.0, 220.0);

#[derive(Clone)]
struct Setup {
    ratio: Option<f64>,
    default_frame: bool,
    image_mode: bool,
    auto_center: bool,
    bounds: Option<Rect>,
    snap_x: Option<f64>,
    snap_y: Option<f64>,
    straighten: bool,
    view: CanvasView,
}

impl Setup {
    fn new() -> Self {
        Setup {
            ratio: None,
            default_frame: false,
            image_mode: false,
            auto_center: false,
            bounds: None,
            snap_x: None,
            snap_y: None,
            straighten: false,
            view: CanvasView::new(egui::Rect::ZERO, Point::new(200.0, 150.0), 1.0),
        }
    }

    /// Image mode: the view shows `angle`'s frame upright.
    fn image(angle: f64, auto_center: bool) -> Self {
        let mut setup = Setup::new();
        setup.image_mode = true;
        setup.auto_center = auto_center;
        setup.view = crop_view(&CropBox::new(RECT, angle), &setup.view);
        setup
    }
}

/// What the overlay reported across the passes.
#[derive(Default)]
struct Seen {
    applied: bool,
    discarded: bool,
    straightened: Option<f64>,
    snapped: Vec<SnapLine>,
    picked: Option<BoxPart>,
    hovered: Option<BoxPart>,
    sizing: Option<(f64, f64)>,
    turning: Option<f64>,
    events: Vec<&'static str>,
}

struct Rig {
    frames: Frames,
    show: Box<dyn FnMut(&mut egui::Ui)>,
    crop: Rc<RefCell<CropBox>>,
    setup: Rc<RefCell<Setup>>,
    seen: Rc<RefCell<Seen>>,
}

impl Rig {
    fn new(crop: CropBox, setup: Setup) -> Self {
        let (crop, setup, seen) = (Rc::new(RefCell::new(crop)), Rc::new(RefCell::new(setup)), Rc::new(RefCell::new(Seen::default())));
        let (c, st, sn) = (Rc::clone(&crop), Rc::clone(&setup), Rc::clone(&seen));
        let show = move |ui: &mut egui::Ui| {
            let mut s = st.borrow().clone();
            s.view.rect = ui.available_rect_before_wrap();
            st.borrow_mut().view.rect = s.view.rect;
            let _ = Canvas::new("Document").view(s.view).show(ui, |ui, body| {
                let mut targets = SnapTargets::new();
                if let Some(x) = s.snap_x {
                    targets.add(SnapTarget::new(Axis::Y, x, SnapKind::Guide));
                }
                if let Some(y) = s.snap_y {
                    targets.add(SnapTarget::new(Axis::X, y, SnapKind::Guide));
                }
                let mut value = c.borrow_mut();
                let mut overlay = CropOverlay::new("Crop", Bind::bound(&mut *value)).snap(&targets);
                if let Some(ratio) = s.ratio {
                    overlay = overlay.ratio(ratio);
                }
                if let Some(bounds) = s.bounds {
                    overlay = overlay.bounds(bounds);
                }
                if s.default_frame {
                    overlay = overlay.default_frame();
                }
                if s.image_mode {
                    overlay = overlay.image_mode();
                }
                if s.auto_center {
                    overlay = overlay.auto_center();
                }
                if s.straighten {
                    overlay = overlay.straighten();
                }
                let out = overlay.show_at(ui, body.rect);
                let mut seen = sn.borrow_mut();
                if let Some(view) = out.view {
                    st.borrow_mut().view = view;
                }
                for event in &out.edit.events {
                    let kind = match event {
                        EditEvent::Begin { .. } => "begin",
                        EditEvent::Update { .. } => "update",
                        EditEvent::Commit { .. } => "commit",
                        EditEvent::Cancel { .. } => "cancel",
                        EditEvent::Set { .. } => "set",
                        _ => "other",
                    };
                    if seen.events.last() != Some(&kind) {
                        seen.events.push(kind);
                    }
                }
                seen.applied |= out.apply;
                seen.discarded |= out.discard;
                seen.straightened = out.straightened.or(seen.straightened);
                seen.snapped = out.snapped;
                seen.picked = out.picked;
                seen.hovered = out.hovered;
                seen.sizing = out.sizing;
                seen.turning = out.turning;
            });
        };
        let mut rig = Rig { frames: Frames::new(), show: Box::new(show), crop, setup, seen };
        for _ in 0..4 {
            rig.frame(Vec::new());
        }
        rig
    }

    fn upright() -> Self {
        Rig::new(CropBox::upright(RECT), Setup::new())
    }

    fn frame(&mut self, events: Vec<Event>) {
        let room = egui::Rect::from_min_size(Pos2::ZERO, ROOM);
        let show = &mut self.show;
        self.frames.frame(events, |ui| layout::in_region(ui, room, |ui| show(ui)));
    }

    fn view(&self) -> CanvasView {
        self.setup.borrow().view
    }

    /// The screen point of document point `(x, y)`.
    fn at(&self, x: f64, y: f64) -> Pos2 {
        self.view().to_screen(Point::new(x, y))
    }

    fn crop(&self) -> CropBox {
        *self.crop.borrow()
    }

    fn rect(&self) -> Rect {
        self.crop().rect
    }

    fn events(&self) -> Vec<&'static str> {
        self.seen.borrow().events.clone()
    }

    fn press(&mut self, at: Pos2) {
        self.frame(vec![pointer::moved(at)]);
        self.frame(vec![pointer::press(at)]);
    }

    fn move_to(&mut self, to: Pos2) {
        self.frame(vec![pointer::moved(to)]);
    }

    fn release(&mut self, at: Pos2) {
        self.frame(vec![pointer::release(at)]);
        self.frame(Vec::new());
    }

    /// Press at `from`, move through the middle to `to`, release, with `mods` held.
    fn drag(&mut self, from: Pos2, to: Pos2, mods: Modifiers) {
        self.frames.hold(mods);
        self.press(from);
        self.move_to(from.lerp(to, 0.5));
        self.move_to(to);
        self.release(to);
        self.frames.hold(Modifiers::NONE);
    }
}

fn near(a: &CropBox, b: &CropBox) -> bool {
    a.corners().iter().zip(b.corners()).all(|(p, q)| p.distance(q) < 1e-6) && (a.angle - b.angle).abs() < 1e-6
}

#[trial]
fn a_corner_drag_resizes_along_the_frame_axes_in_a_turned_view() {
    let b = CropBox::new(RECT, 20.0);
    let mut rig = Rig::new(b, {
        let mut s = Setup::new();
        s.view.rotation = 40.0;
        s.view.flip_x = true;
        s
    });
    let view = rig.view();
    let corner = view.to_screen(b.corners()[2]);
    let to = corner + Vec2::new(14.0, -9.0);
    let want = b.resized(BoxPart::Corner(2), view.to_doc(to) - view.to_doc(corner), None, false);
    rig.drag(corner, to, Modifiers::NONE);
    assert!(near(&rig.crop(), &want), "{:?} vs {want:?}", rig.crop());
    assert_eq!(rig.seen.borrow().picked, Some(BoxPart::Corner(2)));

    // Shift keeps the proportions; Alt resizes about the centre.
    let mut rig = Rig::upright();
    let (from, to) = (rig.at(300.0, 220.0), rig.at(340.0, 230.0));
    rig.drag(from, to, Modifiers::SHIFT);
    let r = rig.rect();
    assert!((r.width() / r.height() - 200.0 / 140.0).abs() < 1e-9, "{r:?}");
    let mut rig = Rig::upright();
    rig.drag(from, to, Modifiers::ALT);
    let c = rig.rect().center();
    assert!((c.x - 200.0).abs() < 1e-9 && (c.y - 150.0).abs() < 1e-9, "{:?}", rig.rect());
}

#[trial]
fn a_quick_drag_in_one_frame_starts_at_the_press() {
    let mut rig = Rig::upright();
    let from = rig.at(300.0, 220.0);
    let to = from + Vec2::new(40.0, 10.0);
    rig.frame(vec![pointer::moved(from)]);
    rig.frame(vec![pointer::press(from), pointer::moved(from + Vec2::new(20.0, 5.0)), pointer::moved(to)]);
    rig.frame(vec![pointer::release(to)]);
    rig.frame(Vec::new());
    assert_eq!(rig.rect(), Rect::new(100.0, 80.0, 340.0, 230.0));
    let events = rig.events();
    assert_eq!((events.first(), events.last()), (Some(&"begin"), Some(&"commit")), "{events:?}");
}

// Straighten.

// Image mode.

#[trial]
fn image_mode_auto_center_keeps_the_frame_centre_on_screen_and_the_handle_under_the_pointer() {
    for angle in [0.0, 30.0] {
        let b0 = CropBox::new(RECT, angle);
        let mut rig = Rig::new(b0, Setup::image(angle, true));
        let view0 = rig.view();
        let centre = view0.to_screen(b0.center());
        let corner = view0.to_screen(b0.corners()[2]);
        let to = corner + Vec2::new(40.0, 25.0);
        rig.press(corner);
        rig.move_to(to);
        rig.frame(Vec::new());
        let (b, view) = (rig.crop(), rig.view());
        assert!((b.rect.width() - RECT.width()).abs() > 1.0, "resized");
        assert!(view.to_screen(b.center()).distance(centre) < 1e-2, "centre kept at {angle}");
        assert!(view.to_screen(b.corners()[2]).distance(to) < 1e-1, "handle follows at {angle}");
        assert_eq!(view.rotation, -angle);
        assert!(b.corners()[0].distance(b0.corners()[0]) < 1e-6, "the opposite corner stays put on the image");
        rig.release(to);
    }
}

#[trial]
fn image_mode_a_move_snaps_and_stays_snapped_while_the_pointer_is_still() {
    for auto_center in [false, true] {
        let mut rig = Rig::new(CropBox::upright(RECT), {
            let mut s = Setup::image(0.0, auto_center);
            s.snap_x = Some(0.0);
            s
        });
        let from = rig.at(200.0, 150.0);
        rig.press(from);
        // The frame moves against the pointer: raw, its left edge would be at x = -3; the image follows the frame.
        let at = from + Vec2::new(103.0, 20.0);
        for pass in 0..4 {
            rig.move_to(at);
            let r = rig.rect();
            assert!(r.x0.abs() < 1e-3, "pass {pass}, auto_center={auto_center}: {r:?}");
            assert!((r.y0 - (RECT.y0 - 20.0)).abs() < 1e-3, "pass {pass}: {r:?}");
        }
        assert!(!rig.seen.borrow().snapped.is_empty(), "the snap line is reported");
        rig.release(at);
    }
}

// The shield and the saved settings.
