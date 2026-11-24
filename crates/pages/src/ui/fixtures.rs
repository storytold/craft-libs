//! Pages for stories and tests: a document of US Letter pages with generated pictures.

use super::view::{PageSource, PlacedPage};
use craft_behavior::CanvasView;
use craft_ui::primitives::textures::{FakeThumbSource, Revision, TextureState, ThumbRender, ThumbRequest, ThumbSource};
use craft_ui::style::DataColor;
use egui::{Color32, Pos2, Rect, Vec2, pos2};
use kurbo::Size;

/// The width of a US Letter page, points.
const LETTER_WIDTH: f64 = 612.0;
/// The height of a US Letter page, points.
const LETTER_HEIGHT: f64 = 792.0;
/// The gap between pages in [`demo_placed`], screen points.
const GAP: f32 = 12.0;

/// The room around the page [`fitted_page`] leaves, screen points: the margin handles sit just outside the page.
const FIT_MARGIN: f64 = 24.0;

/// Fits `view` to one US Letter page and returns that page as the view places it, so every edge of the page is on
/// screen whatever the size of `view.rect`.
pub fn fitted_page(view: &mut CanvasView, pixels_per_point: f32) -> PlacedPage {
    let size = Size::new(LETTER_WIDTH, LETTER_HEIGHT);
    craft_behavior::canvas::zoom::fit(view, size, pixels_per_point, FIT_MARGIN, &craft_behavior::ZoomScheme::PHOTOSHOP);
    let rect = Rect::from_min_max(view.to_screen(kurbo::Point::ZERO), view.to_screen(kurbo::Point::new(LETTER_WIDTH, LETTER_HEIGHT)));
    PlacedPage::new(0, rect, view.zoom, (0.0, rect.height()))
}

/// The paper of the demo document.
pub const PAPER: DataColor = DataColor::new(Color32::from_rgb(0xfa, 0xf8, 0xf2));
/// The ink of the demo document's text.
pub const INK: DataColor = DataColor::new(Color32::from_rgb(0x1c, 0x1c, 0x20));

/// A document of `count` US Letter pages whose pictures the fake thumbnail source makes.
pub struct DemoPages {
    count: usize,
    fake: FakeThumbSource<usize>,
}

impl DemoPages {
    /// A document of `count` pages.
    pub fn new(count: usize) -> Self {
        DemoPages { count, fake: FakeThumbSource::new().aspect(612, 792) }
    }

    /// A document of `count` pages whose pictures render as jobs on the loader's queue, so a page shows its loading
    /// state until its job lands.
    pub fn rendering(count: usize) -> Self {
        DemoPages { count, fake: FakeThumbSource::new().aspect(612, 792).later(true) }
    }

    /// A document of `count` pages whose every picture fails to render.
    pub fn failing(count: usize) -> Self {
        DemoPages { count, fake: FakeThumbSource::new().aspect(612, 792).failing(0..count) }
    }
}

impl ThumbSource<usize> for DemoPages {
    fn revision(&self, key: &usize, request: &ThumbRequest) -> Revision {
        self.fake.revision(key, request)
    }

    fn render(&self, key: &usize, request: &ThumbRequest) -> ThumbRender {
        self.fake.render(key, request)
    }
}

impl PageSource for DemoPages {
    fn page_count(&self) -> usize {
        self.count
    }

    fn page_size(&self, _: usize) -> Size {
        Size::new(LETTER_WIDTH, LETTER_HEIGHT)
    }
}

/// `count` US Letter pages one under another from `origin`, at scale 1 with 12-point gaps, as a page view places
/// them.
pub fn demo_placed(count: usize, origin: Pos2) -> Vec<PlacedPage> {
    let (width, height) = (LETTER_WIDTH as f32, LETTER_HEIGHT as f32);
    (0..count)
        .map(|page| {
            let top = origin.y + page as f32 * (height + GAP);
            // A fixture has no texture to show, so its pages have no picture yet.
            PlacedPage {
                page,
                rect: Rect::from_min_size(pos2(origin.x, top), Vec2::new(width, height)),
                scale: 1.0,
                band: (0.0, height),
                picture: TextureState::Missing,
                picture_revision: None,
            }
        })
        .collect()
}
