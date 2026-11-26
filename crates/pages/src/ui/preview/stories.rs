//! Stories of the page preview.

use super::{PagePicture, PagePreview};
use crate::ui::fixtures::{DemoPages, PAPER};
use crate::ui::thumbnails::PageThumbnails;
use craft_behavior::Selection;
use craft_ui::contract::{Props as _, Show as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The pages of the document behind the preview.
const PREVIEW_PAGES: usize = 8;

/// The document's thumbnails, and over the window the preview of the page they mark as current while the preview is
/// open. Close or Escape closes it; the card's Show dialog opens it again.
#[story(PagePreview<'static>, covers = [Open], drive = Drive::Never, title = "Overlay")]
fn open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pages = DemoPages::new(PREVIEW_PAGES);
    let mut at = *data.value("at", 2_usize);
    let behind = PageThumbnails::new("Pages", &pages, data.value("picked", Selection::default()), at, PAPER).show(ui);
    let mut shown = StoryOutput::from(behind);
    let mut event = None;
    if *data.value("open", true) {
        let every: Vec<usize> = (0..PREVIEW_PAGES).collect();
        let out = PagePreview::new(&pages, &every, &mut at, PAPER).show(ui);
        if out.closed {
            event = Some("Closed".to_owned());
            *data.value("open", true) = false;
        } else if out.stepped {
            event = Some(format!("Page {}", at + 1));
        }
        shown = StoryOutput::from(out);
    }
    *data.value("at", 2_usize) = at;
    let last = data.note("last", None::<String>);
    if event.is_some() {
        *last = event;
    }
    let readout = last.as_ref().map_or_else(|| "No event yet".to_owned(), |event| format!("Last event: {event}"));
    shown.readout(readout)
}

#[story(PagePicture<'static>, covers = [Default], drive = Drive::Never)]
fn page_picture(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let pages = DemoPages::new(5);
    PagePicture::new(&pages, 2, "Page unavailable", PAPER).label("Page 3").show(ui).into()
}
