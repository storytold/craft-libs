//! Stories of the selection tools: one per tool, on a canvas over the mock picture, run by `ToolHost::run` as an app
//! does. The events each tool reports are applied to a story selection (a rectangle in the hard coverage a marquee or
//! outline gives), which the selection overlay shows. The first press inside the selection drags it.

use craft_behavior::EditEvent;
use craft_behavior::kurbo::{Point, Rect, Size};
use craft_ui::canvas::{Canvas, CanvasOutput, RasterSource as _, ToolHost};
use craft_ui::contract::ShowContainer as _;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;
use std::sync::{Arc, Mutex};

use super::{
    EdgeSearch, EdgeTracer, LassoTool, MagneticLassoTool, MarqueeKind, MarqueeTool, PolygonLassoTool, SelectEvent, SelectToolPass, SelectionProbe,
    SelectionShape,
};
use crate::trace::{MaskRect, MaskSource};
use crate::ui::mock_image::{MockImage, StoryView};
use crate::ui::selection::SelectionOverlay;

craft_ui::words! {
    /// The captions of the selection tool stories.
    #[non_exhaustive]
    pub(crate) struct SelectToolStoryWords {
        /// The name of the selection overlay.
        selection: "selectToolStory.selection" = "Selection";
    }
}

/// A tool a story runs, with the events it reports.
trait Shown: Send + 'static {
    fn run(&mut self, ui: &Ui, canvas: &CanvasOutput, probe: &dyn SelectionProbe);
    fn take(&mut self) -> Vec<SelectEvent>;
}

macro_rules! shown {
    ($tool:ty $(, $param:ident: $bound:path)?) => {
        impl$(<$param: $bound + Send + 'static>)? Shown for $tool {
            fn run(&mut self, ui: &Ui, canvas: &CanvasOutput, probe: &dyn SelectionProbe) {
                let mut pass = SelectToolPass::new(self, probe);
                let _ = ToolHost::new().run(ui, canvas, &mut pass);
            }

            fn take(&mut self) -> Vec<SelectEvent> {
                self.take_events()
            }
        }
    };
}

shown!(MarqueeTool);
shown!(LassoTool);
shown!(PolygonLassoTool);
shown!(MagneticLassoTool<T>, T: EdgeTracer);

/// A tool held across the passes of a story; clones share it.
struct Held(Arc<Mutex<Box<dyn Shown>>>);

impl Clone for Held {
    fn clone(&self) -> Self {
        Held(Arc::clone(&self.0))
    }
}

impl PartialEq for Held {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The story's selection: a rectangle, or nothing.
#[derive(Clone, Debug, PartialEq)]
struct Selection(Option<Rect>);

/// The selection as the probe and the overlay's mask see it.
struct RectMask {
    rect: Option<Rect>,
    size: [u32; 2],
}

impl RectMask {
    fn pixels(&self) -> Option<MaskRect> {
        self.rect.map(|rect| MaskRect::new(rect.x0.floor() as i32, rect.y0.floor() as i32, rect.x1.ceil() as i32, rect.y1.ceil() as i32))
    }
}

impl MaskSource for RectMask {
    fn size(&self) -> [u32; 2] {
        self.size
    }

    fn revision(&self) -> u64 {
        self.rect.map_or(0, |rect| {
            rect.x0.to_bits() ^ rect.y0.to_bits().rotate_left(16) ^ rect.x1.to_bits().rotate_left(32) ^ rect.y1.to_bits().rotate_left(48)
        })
    }

    fn content_bounds(&self) -> MaskRect {
        self.pixels().unwrap_or(MaskRect::new(0, 0, 0, 0))
    }

    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]) {
        let bounds = self.content_bounds();
        for (index, value) in out.iter_mut().enumerate() {
            let x = x0 + i32::try_from(index as u32 * step).unwrap_or(i32::MAX);
            *value = if (bounds.x0..bounds.x1).contains(&x) && (bounds.y0..bounds.y1).contains(&y) { 1.0 } else { 0.0 };
        }
    }
}

impl SelectionProbe for RectMask {
    fn document(&self) -> u64 {
        1
    }

    fn size(&self) -> craft_behavior::PixelSize {
        craft_behavior::PixelSize::new(self.size[0], self.size[1])
    }

    fn has_selection(&self) -> bool {
        self.rect.is_some()
    }

    fn inside_selection(&self, at: Point) -> bool {
        self.rect.is_some_and(|rect| rect.contains(at))
    }
}

/// Edges at the picture's house: the tracer snaps to the selection's rectangle where it is near.
struct Edges;

impl EdgeTracer for Edges {
    fn snap(&mut self, _: Point, _: EdgeSearch) -> Option<Point> {
        None
    }

    fn trace(&mut self, from: Point, to: Point, _: &[Point], _: EdgeSearch) -> Vec<Point> {
        vec![from, to]
    }
}

/// Applies what the tool reported to the story selection.
fn apply(selection: &mut Selection, events: Vec<SelectEvent>) {
    for event in events {
        match event {
            SelectEvent::Commit { shape: SelectionShape::Rectangle(rect) | SelectionShape::Ellipse(rect), .. } => selection.0 = Some(rect),
            SelectEvent::Commit { shape: SelectionShape::Polygon(points), .. } => {
                let first = points.first().copied().unwrap_or_default();
                selection.0 = Some(points.iter().fold(Rect::from_points(first, first), |rect, &p| rect.union_pt(p)));
            }
            SelectEvent::Deselect => selection.0 = None,
            SelectEvent::DragSelection(EditEvent::Commit { after, .. }) => {
                let delta = (f64::from(after.delta[0]), f64::from(after.delta[1]));
                selection.0 = selection.0.map(|rect| rect + craft_behavior::kurbo::Vec2::new(delta.0, delta.1));
            }
            SelectEvent::DragSelection(_) => {}
        }
    }
}

/// Runs the tool `name` on a canvas over the mock picture, with the story selection drawn by the overlay.
fn on_canvas(ui: &mut Ui, data: &mut StoryData, name: &'static str, init: fn() -> Box<dyn Shown>) -> StoryOutput {
    let held = data.value(name, Held(Arc::new(Mutex::new(init())))).clone();
    let selection = data.value("selection", Selection(Some(Rect::new(200.0, 150.0, 600.0, 450.0))));
    let image = MockImage::default();
    let size = image.size();
    let room = craft_ui::testing::stage(ui);
    let view = StoryView::Fit.of(room, size, ui.pixels_per_point());
    let words = Env::of(ui).words::<SelectToolStoryWords>();
    let mask = RectMask { rect: selection.0, size: [size.width, size.height] };
    let document = Size::new(f64::from(size.width), f64::from(size.height));
    let shown = Canvas::new("Document").document(document).view(view).show_at(ui, room, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &image);
        SelectionOverlay::new(words.selection.clone(), &mask).show_at(ui, body.rect);
    });
    if let Ok(mut tool) = held.0.lock() {
        tool.run(ui, &shown.output, &mask);
        apply(selection, tool.take());
    }
    StoryOutput::default()
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn marquee(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, "marquee", || Box::new(MarqueeTool::new(MarqueeKind::Rectangle)))
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn lasso(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, "lasso", || Box::new(LassoTool::new()))
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn polygonal_lasso(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, "polygon", || Box::new(PolygonLassoTool::new()))
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn magnetic_lasso(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data, "magnetic", || Box::new(MagneticLassoTool::new(Edges)))
}
