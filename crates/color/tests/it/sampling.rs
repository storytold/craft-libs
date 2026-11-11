//! The Color Picker dialog, the droppers, the eyedropper, the colour samplers and the screen pick.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_cms::convert::CmykConverter;
use craft_color::ui::{
    CaptureSource, ColorPicker, ColorPickerLayout, ColorSamplerTool, ColorSamplers, PickedColor, SamplerCache, SamplerReadouts, ScreenPick,
    StillCapture,
};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, ToolFixture, trial};
use egui::{Context, Rect, vec2};

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

fn sampler_tool(points: &[[f64; 2]]) -> ColorSamplerTool<'static> {
    let samplers: &'static mut ColorSamplers = Box::leak(Box::new(ColorSamplers::new()));
    for p in points {
        samplers.add(*p, SIZE).unwrap();
    }
    ColorSamplerTool::new(samplers, SIZE)
}

#[trial]
fn the_sampler_tool_meets_the_tool_rules() {
    let view = |rect: Rect| CanvasView::new(rect, Point::new(f64::from(SIZE[0]) / 2.0, f64::from(SIZE[1]) / 2.0), 1.0);
    let mut tool = ToolFixture::with_view("Color Sampler", sampler_tool(&[]), view, |_, _| {});
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
