//! The Color Picker dialog, the droppers, the eyedropper, the colour samplers and the screen pick.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_cms::convert::CmykConverter;
use craft_color_ui::ui::{
    CaptureResult, CaptureSource, ColorPicker, ColorPickerLayout, ColorSamplerPass, ColorSamplerTool, ColorSamplers, MAX_SAMPLERS, PickedColor,
    SamplerCache, SamplerEvent, SamplerReadouts, ScreenColorPicker, ScreenPick, ScreenPickerWords, StillCapture,
};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, ToolFixture, trial};
use egui::{Context, Modifiers, Pos2, Rect, vec2};

const SIZE: [u32; 2] = [300, 200];

#[trial]
fn readouts_are_cached_per_document_and_converter() {
    use std::cell::Cell;
    let mut samplers = ColorSamplers::new();
    samplers.add([1.0, 1.0], [10, 10]).unwrap();
    let reads = Cell::new(0);
    let read = |_: [f64; 2]| {
        reads.set(reads.get() + 1);
        Srgba { r: 0.0, g: 0.0, b: 1.0, a: 1.0 }
    };
    let mut cache = SamplerCache::default();
    let coated = CmykConverter::coated();
    let first = SamplerReadouts::new(&samplers, read).revision(3).document("a").cmyk(coated).get(&mut cache);
    let again = SamplerReadouts::new(&samplers, read).revision(3).document("a").cmyk(coated).get(&mut cache);
    assert!(Arc::ptr_eq(&first, &again) && reads.get() == 1, "unchanged: from the cache");
    SamplerReadouts::new(&samplers, read).revision(3).document("b").cmyk(coated).get(&mut cache);
    assert_eq!(reads.get(), 2, "another document at the same revision reads again");
    let fogra = CmykConverter::new(Arc::new(craft_cms::Builtin::CoatedFogra39.profile().clone())).unwrap();
    let other = SamplerReadouts::new(&samplers, read).revision(3).document("a").cmyk(&fogra).get(&mut cache);
    assert_eq!(reads.get(), 3, "another CMYK working space reads again");
    assert_ne!(other[0].cmyk, first[0].cmyk);
}

fn sampler_tool(points: &[[f64; 2]]) -> ColorSamplerPass<'static> {
    let mut samplers = ColorSamplers::new();
    for p in points {
        samplers.add(*p, SIZE).unwrap();
    }
    // The fixture owns the pass for the whole trial, so the tool and the points live as long.
    let samplers: &'static ColorSamplers = Box::leak(Box::new(samplers));
    let tool: &'static mut ColorSamplerTool = Box::leak(Box::new(ColorSamplerTool::new()));
    tool.pass(samplers, SIZE)
}

/// The document's origin at the room's top left, one pixel per point: a view the canvas shows as given, since it
/// keeps the fixture's document over the whole room.
fn sampler_view(rect: Rect) -> CanvasView {
    CanvasView::new(rect, Point::new(f64::from(rect.width()) / 2.0, f64::from(rect.height()) / 2.0), 1.0)
}

fn sampler_fixture(points: &[[f64; 2]]) -> ToolFixture<ColorSamplerPass<'static>> {
    let mut tool = ToolFixture::with_view("Color Sampler", sampler_tool(points), sampler_view, |_, _| {});
    tool.fixture().room(vec2(800.0, 600.0));
    tool.fixture().settle();
    tool
}

/// The screen position of document point `doc` under `sampler_view`.
fn doc_to_screen(tool: &ToolFixture<ColorSamplerPass<'static>>, doc: [f64; 2]) -> Pos2 {
    tool.canvas().rect.min + vec2(doc[0] as f32, doc[1] as f32)
}

fn events(tool: &ToolFixture<ColorSamplerPass<'static>>) -> Vec<SamplerEvent> {
    tool.tool_mut().take_events()
}

#[trial]
fn a_new_point_lands_at_the_release_and_a_dragged_point_moves_on_release() {
    // A new point: nothing at the press, an Add at the release; off the canvas or on Escape, nothing.
    let mut tool = sampler_fixture(&[]);
    let at = doc_to_screen(&tool, [100.5, 50.5]);
    tool.press_at(at);
    assert!(events(&tool).is_empty(), "a press only starts the carry");
    tool.release_at(at);
    assert_eq!(events(&tool), [SamplerEvent::add([100.0, 50.0])]);
    let outside = doc_to_screen(&tool, [320.5, 50.5]);
    tool.press_at(at).release_at(outside);
    assert!(events(&tool).is_empty(), "released off the canvas places none");
    tool.press_at(at).escape().release_at(at);
    assert!(events(&tool).is_empty(), "Escape drops the carry");

    // A point: a click 2 px off moves it to the release pixel; dropped off the canvas it is removed.
    let mut tool = sampler_fixture(&[[100.0, 50.0]]);
    let on = doc_to_screen(&tool, [100.5, 50.5]);
    let near = doc_to_screen(&tool, [102.5, 50.5]);
    tool.press_at(on).release_at(on);
    assert!(events(&tool).is_empty(), "a click on the point's own pixel moves nothing");
    tool.press_at(near).release_at(near);
    assert_eq!(events(&tool), [SamplerEvent::moved(0, [102.0, 50.0])]);
    let outside = doc_to_screen(&tool, [320.5, 50.5]);
    tool.drag_from(on, outside - on);
    assert_eq!(events(&tool), [SamplerEvent::removed(0)], "a point dropped off the canvas is removed");

    // Alt: a click on a point removes it, off a point it adds.
    tool.fixture().hold(Modifiers::ALT);
    tool.press_at(on).release_at(on);
    assert_eq!(events(&tool), [SamplerEvent::removed(0)]);
    let empty = doc_to_screen(&tool, [200.5, 150.5]);
    tool.press_at(empty).release_at(empty);
    assert_eq!(events(&tool), [SamplerEvent::add([200.0, 150.0])]);
    tool.fixture().hold(Modifiers::NONE);

    // At the limit nothing is carried and one notice is set.
    let full: Vec<[f64; 2]> = (0..MAX_SAMPLERS).map(|i| [10.0 + 20.0 * i as f64, 10.0]).collect();
    let mut tool = sampler_fixture(&full);
    let empty = doc_to_screen(&tool, [100.5, 150.5]);
    tool.press_at(empty).release_at(empty);
    assert!(events(&tool).is_empty(), "no point is placed at the limit");
    assert!(tool.tool_mut().take_error().is_some(), "the refused press sets a notice");
    assert!(tool.tool_mut().take_error().is_none(), "once");
}

/// A capture that has no answer until `ready` is set.
struct Late {
    ready: Arc<std::sync::atomic::AtomicBool>,
    result: std::sync::Mutex<Option<CaptureResult>>,
}

impl CaptureSource for Late {
    fn poll(&self) -> Option<CaptureResult> {
        if self.ready.load(std::sync::atomic::Ordering::Relaxed) { self.result.lock().ok()?.take() } else { None }
    }
}

const PICKED: Srgba = Srgba { r: 0.25, g: 0.5, b: 0.75, a: 1.0 };

#[trial]
fn a_screen_pick_cancelled_while_queued_or_beaten_by_a_cancel_or_a_target_change_answers_nothing() {
    let ctx = Context::default();
    let still = || -> Box<dyn CaptureSource> { Box::new(StillCapture::color(PICKED)) };

    // Cancelled while queued: no capture starts and the next pick is not poisoned.
    let mut pick = ScreenPick::new();
    let started = std::cell::Cell::new(false);
    pick.start("a");
    pick.cancel();
    pick.tick_with(&ctx, 1, &|| {
        started.set(true);
        still()
    });
    assert!(!started.get() && !pick.busy(), "a cancelled request starts nothing");
    pick.start("b");
    pick.tick_with(&ctx, 1, &still);
    assert_eq!(pick.take("b"), Some(PICKED), "the next pick answers");

    // A capture that is ready in the pass the pick is cancelled loses to the cancel.
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let late = |ready: &Arc<std::sync::atomic::AtomicBool>| -> Box<dyn CaptureSource> {
        Box::new(Late { ready: ready.clone(), result: std::sync::Mutex::new(Some(Ok(craft_platform::screen::Capture::Color([0.25, 0.5, 0.75])))) })
    };
    let mut pick = ScreenPick::new();
    pick.start("a");
    pick.tick_with(&ctx, 1, &|| late(&ready));
    assert!(pick.busy());
    ready.store(true, std::sync::atomic::Ordering::Relaxed);
    pick.cancel();
    pick.tick_with(&ctx, 1, &|| late(&ready));
    assert!(!pick.busy() && pick.take("a").is_none(), "the cancel wins over a finished capture");

    // A target change discards the pending result.
    let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut pick = ScreenPick::new();
    pick.start("a");
    pick.tick_with(&ctx, 1, &|| late(&ready));
    ready.store(true, std::sync::atomic::Ordering::Relaxed);
    pick.tick_with(&ctx, 2, &|| late(&ready));
    assert!(!pick.busy() && pick.take("a").is_none(), "another target discards the answer");

    // Only the owner takes a finished colour.
    let mut pick = ScreenPick::new();
    pick.start("a");
    pick.tick_with(&ctx, 1, &still);
    assert_eq!(pick.take("b"), None, "another owner takes nothing");
    assert_eq!(pick.take("a"), Some(PICKED));
}

#[trial]
fn an_invalid_capture_ends_the_pick_with_a_status() {
    let ctx = Context::default();
    let words = ScreenPickerWords::default();
    let images = |images: Vec<craft_platform::screen::ScreenImage>| -> Box<dyn CaptureSource> {
        Box::new(StillCapture::new(Ok(craft_platform::screen::Capture::Images(images))))
    };

    // A pixel buffer of the wrong length.
    let mut broken = monitor([0, 0], [4, 4], 1);
    broken.rgba = Arc::new(vec![0; 7]);
    let mut pick = ScreenPick::new();
    pick.start("a");
    pick.tick_with(&ctx, 1, &|| images(vec![broken.clone()]));
    assert!(!pick.busy() && pick.take("a").is_none(), "a malformed image ends the pick without a colour");
    assert!(pick.take_status(&words).is_some(), "and says why");
    assert!(pick.take_status(&words).is_none(), "once");

    // A valid profile of another colour space: a CMYK monitor profile is not an RGB profile. The smallest grids make
    // it as valid as the default coated profile, which takes far longer to build.
    let mut odd = monitor([0, 0], [4, 4], 1);
    let params = craft_cms::synth::CmykParams { grid_a2b: 2, grid_b2a: 2, ..craft_cms::synth::CmykParams::default() };
    odd.icc_profile = Some(craft_cms::synth::cmyk_profile(&params).expect("the synthetic CMYK profile").to_bytes());
    let mut pick = ScreenPick::new();
    pick.start("a");
    pick.tick_with(&ctx, 1, &|| images(vec![odd.clone()]));
    assert!(!pick.busy() && pick.take("a").is_none());
    assert!(pick.take_status(&words).is_some(), "a profile of another colour space says why");
}

/// A fixture showing the capture windows of a pick that started over one 200 x 150 monitor.
fn capture_fixture() -> (Fixture, Rc<RefCell<ScreenPick>>) {
    let pick = Rc::new(RefCell::new(ScreenPick::new()));
    pick.borrow_mut().start("a");
    let shared = pick.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut pick = shared.borrow_mut();
        let image = monitor([0, 0], [200, 150], 1);
        pick.tick_with(ui.ctx(), 1, &move || Box::new(StillCapture::new(Ok(craft_platform::screen::Capture::Images(vec![image.clone()])))));
        ScreenColorPicker::new(&mut pick, "Test").show(ui);
    });
    fixture.room(vec2(300.0, 200.0));
    fixture.settle();
    assert!(pick.borrow().busy(), "a valid capture keeps the pick running while its windows show");
    (fixture, pick)
}

#[trial]
fn every_way_out_of_the_capture_windows_ends_the_pick() {
    use craft_ui::testing::pointer;
    let words = ScreenPickerWords::default();

    let (mut fixture, pick) = capture_fixture();
    fixture.key(egui::Key::Escape, Modifiers::NONE);
    assert!(!pick.borrow().busy(), "Escape ends the pick");
    assert!(pick.borrow_mut().take("a").is_none() && pick.borrow_mut().take_status(&words).is_none(), "quietly");

    let (mut fixture, pick) = capture_fixture();
    let at = Pos2::new(50.0, 40.0);
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
    assert!(!pick.borrow().busy(), "a right-click ends the pick");
    assert!(pick.borrow_mut().take("a").is_none() && pick.borrow_mut().take_status(&words).is_none(), "quietly");

    let (mut fixture, pick) = capture_fixture();
    fixture.click_at(at);
    assert!(!pick.borrow().busy(), "a click ends the pick");
    assert!(pick.borrow_mut().take("a").is_some(), "with the colour under it");
}

#[trial]
fn the_sampler_tool_meets_the_tool_rules() {
    let mut tool = ToolFixture::with_view("Color Sampler", sampler_tool(&[]), sampler_view, |_, _| {});
    tool.check();
}

/// A monitor of `size` pixels at `origin` in the virtual desktop whose pixel (x, y) is (x, y, `tag`), so a pick shows
/// which monitor and which pixel it read.
fn monitor(origin: [i32; 2], size: [u32; 2], tag: u8) -> craft_platform::screen::ScreenImage {
    let rgba = (0..size[1]).flat_map(|y| (0..size[0]).flat_map(move |x| [x as u8, y as u8, tag, 255])).collect();
    craft_platform::screen::ScreenImage { origin, size, scale: 1.0, rgba: Arc::new(rgba), icc_profile: None, name: format!("monitor {tag}") }
}

/// Dragging from the picker's screen button and releasing outside the app's window picks the colour under the pointer on
/// whichever monitor it is over, not only inside the window. Guards the capture of every monitor ending without a colour.
#[trial]
fn a_release_outside_the_window_picks_the_colour_under_the_pointer_on_any_monitor() {
    let pick = Rc::new(RefCell::new(ScreenPick::new()));
    let shared = pick.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut color = PickedColor::rgb(Srgba::rgb(0.2, 0.5, 0.8));
        let out = ColorPicker::new("Foreground color", Bind::bound(&mut color)).layout(ColorPickerLayout::Compact).eyedropper(true).show(ui);
        if let Some(gesture) = out.screen_pick {
            shared.borrow_mut().hold(ui.ctx(), "fill", gesture);
        }
    });
    fixture.room(vec2(720.0, 640.0));
    fixture.settle();
    let button = fixture.by_label("Sample color from the screen").expect("the screen button");
    // The window is the first monitor; the second one starts where the first ends.
    let by = vec2(900.0 - button.center().x, 50.0 - button.center().y);
    fixture.drag(&button, by);
    assert!(pick.borrow().busy(), "a release outside the window captures the screen");
    let monitors = [monitor([0, 0], [720, 700], 1), monitor([720, 0], [400, 700], 2)];
    let capture = || -> Box<dyn CaptureSource> { Box::new(StillCapture::new(Ok(craft_platform::screen::Capture::Images(monitors.to_vec())))) };
    pick.borrow_mut().tick_with(&Context::default(), 1, &capture);
    assert!(!pick.borrow().busy(), "the pick ends with the capture");
    let color = pick.borrow_mut().take("fill").expect("the colour under the pointer");
    let released = button.center() + by;
    let under = |m: &craft_platform::screen::ScreenImage| {
        let (x, y) = ((released.x as i32 - m.origin[0]) as usize, (released.y as i32 - m.origin[1]) as usize);
        let at = (y * m.size[0] as usize + x) * 4;
        Srgba::rgb(f32::from(m.rgba[at]) / 255.0, f32::from(m.rgba[at + 1]) / 255.0, f32::from(m.rgba[at + 2]) / 255.0)
    };
    assert_eq!(color, under(&monitors[1]), "the pixel under the pointer on the monitor it is over");
    assert_ne!(color, under(&monitors[0]), "not the same coordinates on the first monitor");
}
