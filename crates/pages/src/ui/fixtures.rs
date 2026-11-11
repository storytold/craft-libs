//! Pages for stories and tests: a document of US Letter pages with generated pictures.

use super::view::{PageSource, PlacedPage};
use craft_ui::primitives::textures::{FakeThumbSource, Revision, ThumbRender, ThumbRequest, ThumbSource};
use craft_ui::style::DataColor;
use egui::{Color32, Pos2, Rect, Vec2, pos2};
use kurbo::Size;

/// The width of a US Letter page, points.
const LETTER_WIDTH: f64 = 612.0;
/// The height of a US Letter page, points.
const LETTER_HEIGHT: f64 = 792.0;
/// The gap between pages in [`demo_placed`], screen points.
const GAP: f32 = 12.0;

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
            PlacedPage { page, rect: Rect::from_min_size(pos2(origin.x, top), Vec2::new(width, height)), scale: 1.0, band: (0.0, height) }
        })
        .collect()
}
