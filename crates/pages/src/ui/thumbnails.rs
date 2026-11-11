//! `PageThumbnails`: the Pages panel of Acrobat and the slide strip of PowerPoint, on craft-ui's [`TileGrid`]: page
//! thumbnails with the current page marked, several picked with Cmd/Ctrl- and Shift-click, dragged to reorder, a
//! context menu the app fills, and the [`ThumbnailSlider`] for their size.
//!
//! The grid does the work: layout, selection on the app's [`Selection`], the keys, reorder drags, pictures through
//! `Thumbnails` over the app's [`PageSource`], context menus as [`ContextMenuRequest`]s, captions, sections and file
//! drops. This module maps pages onto tiles and the grid's events onto page events.
//!
//! ```ignore
//! let out = PageThumbnails::new("Pages", &doc_pages, &mut picked, view.current).editable(!doc.read_only).size(&mut settings.thumbnail_zoom).show(ui);
//! for event in out.events {
//!     match event {
//!         ThumbnailEvent::Go(page) => view.go_to(page),
//!         ThumbnailEvent::Activate(page) => start_show(page),
//!         ThumbnailEvent::Reorder(EditEvent::Commit { value, .. }) => doc.apply(Edit::MovePages(value)),
//!         ThumbnailEvent::Delete(pages) => doc.delete_pages(&pages),
//!         ThumbnailEvent::ContextMenu(request) => page_menu.open(request),
//!         _ => {}
//!     }
//! }
//! ```

#[cfg(feature = "screen-testing")]
mod stories;

use super::view::PageSource;
use craft_behavior::{EditEvent, IconId, Selection, SelectionChange, keyed};
use craft_ui::buttons::Button;
use craft_ui::contract::{
    Bind, CollectionEvent, ContextMenuRequest, NoValue, Outcome, ReorderMove, ReorderRule, Show as _, ShowCollection as _, clicked,
};
use craft_ui::list::ThumbAspect;
use craft_ui::primitives::textures::{Priority, ThumbPaint, ThumbRequest, ThumbnailOptions, Thumbnails};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::DataColor;
use craft_ui::tile_grid::{CaptionPlace, GridEvent, ThumbnailSlider, Tile, TileGrid, TileGroup, TileMark, TileZoom, ZoomStep};
use egui::{Pos2, Response, Ui};

/// The thumbnail zoom: 0.5 to 1.5 of the card thumbnail in steps of 0.25, 1 by default (PdfCraft's
/// `THUMBNAIL_ZOOM_RANGE` and its minus and plus step). The size row's [`ThumbnailSlider`] and the grid's zoom keys
/// step it.
pub const THUMBNAIL_ZOOM: TileZoom = TileZoom::bounded(0.5, 1.5, 0.25, 1.0);

craft_ui::words! {
    /// The panel's words, English by default (PdfCraft's and DeckCraft's strings).
    pub struct ThumbnailWords {
        pages: "pageThumbnails.title" = "Pages";
        size: "pageThumbnails.size" = "Thumbnail size";
        page: template "pageThumbnails.page" = "Page {label}";
        hidden: "pageThumbnails.hidden" = "Hidden";
        animated: "pageThumbnails.animated" = "Has animations";
        new_page: "pageThumbnails.newPage" = "New Slide";
        count: count "pages.count" = "{n} page" | "{n} pages";
    }
}

/// A named run of pages (PowerPoint's sections): a grid group header with a disclosure chevron, the name and the
/// count; its context menu is the app's ([`ThumbnailEvent::SectionMenu`]).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ThumbnailSection {
    /// The section's name.
    pub name: String,
    /// The section's first page.
    pub first: usize,
    /// Number of pages in the section.
    pub count: usize,
    /// Open (its pages shown); the app keeps it and applies [`ThumbnailEvent::SectionOpen`].
    pub open: bool,
}

/// Marks on one thumbnail (PowerPoint), drawn by the grid with [`CaptionPlace::Start`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ThumbnailMarks {
    /// A hidden slide: the tile's `hidden` (the picture under the veil, a slash across the number), and
    /// [`ThumbnailWords::hidden`] in its accessible name.
    pub hidden: bool,
    /// The slide has animations or a transition: a mark with a star and [`ThumbnailWords::animated`] under the
    /// number.
    pub animated: bool,
}

/// Where page `p` (0-based) is after `mv` (craft-ui's [`ReorderMove`] over page indices: the pages `keys` go to
/// position `to`, counted once they are out) in a document of `count` pages.
// pdfcraft: crates/ui-egui/src/panels/pages.rs:283 `moved_index`.
pub fn moved_page(mv: &ReorderMove<usize>, p: usize, count: usize) -> usize {
    if p >= count {
        return p;
    }
    let mut keys = mv.keys.clone();
    keys.sort_unstable();
    keys.dedup();
    let moved = match keys.iter().position(|key| *key == p) {
        Some(k) => mv.to + k,
        None => {
            let rest = (0..p).filter(|q| !keys.contains(q)).count();
            if rest >= mv.to { rest + keys.len() } else { rest }
        }
    };
    moved.min(count - 1)
}

/// What the panel reports, in the order it happened this pass.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ThumbnailEvent {
    /// The picked set changed (Cmd/Ctrl-click, Shift-click, Shift-arrows, Escape, a secondary click on an unpicked
    /// page); the panel has applied it to the selection it was given.
    Selection(SelectionChange<usize>),
    /// A click, or an arrow key moving the lead: this page becomes the current page.
    Go(usize),
    /// A drag of pages as one edit gesture, or a stacking key (Cmd/Ctrl+] and [) as a `Set`, as craft-ui's tables
    /// report reorders. Only when [`editable`](PageThumbnails::editable).
    Reorder(EditEvent<ReorderMove<usize>>),
    /// Delete or Backspace with the panel focused: the picked pages, or the current page when none are; never when
    /// every page is picked (a document keeps one page), never read-only.
    Delete(Vec<usize>),
    /// A double-click or Enter on a thumbnail (PowerPoint starts the slide show from it).
    Activate(usize),
    /// A click on the New Slide button ([`PageThumbnails::new_page_row`]).
    NewPage,
    /// The app's menu for the picked pages is asked for (secondary click, Menu key, Shift+F10). A secondary click on
    /// an unpicked page makes it the picked set first ([`Selection`](Self::Selection)), without going to it.
    ContextMenu(ContextMenuRequest<usize>),
    /// The app's menu for section `section` is asked for (secondary click on its header).
    #[non_exhaustive]
    SectionMenu {
        /// Index of the section.
        section: usize,
        /// Where a menu opened from the keyboard goes: the header's bottom start corner.
        at: Pos2,
    },
    /// A section header's chevron (or AccessKit's Expand or Collapse on it) asks to open or close it.
    #[non_exhaustive]
    SectionOpen {
        /// Index of the section.
        section: usize,
        /// The state asked for: `true` to open the section, `false` to close it.
        open: bool,
    },
    /// Files dragged in from outside dropped before page `insert`.
    #[non_exhaustive]
    FilesDropped {
        /// The position the files drop at.
        insert: usize,
    },
}

/// The panel's result.
#[derive(Debug)]
#[must_use = "the app applies the panel's events"]
#[non_exhaustive]
pub struct PageThumbnailsOutput {
    /// The panel: its focus and rect.
    pub response: Response,
    /// What happened this pass, in order.
    pub events: Vec<ThumbnailEvent>,
}

impl Outcome for PageThumbnailsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The thumbnails panel body, built per pass: a [`ThumbnailSlider`] row (with [`size`](Self::size)) over a craft-ui
/// [`TileGrid`] of the pages.
///
/// - Look (Acrobat; PowerPoint with [`CaptionPlace::Start`]): cards whose picture is the grid's card thumbnail
///   times the zoom, the aspect page 0's (`PageSource::page_size`; other pages are fitted into the card), a caption
///   with the page label (`PageSource::page_label`). The current page is the grid's current tile; picked pages are
///   its selected tiles. Pictures come from `Thumbnails<usize>` over [`PageSource`].
/// - Pointer (behaviour: Photoshop's panels, then PdfCraft, as the grid): a plain click goes to the page
///   ([`ThumbnailEvent::Go`]); Cmd/Ctrl-click toggles a page in the picked set and Shift-click extends it.
/// - Drag (when editable): the grid's reorder rule; the grid draws the insert line and scrolls near its edges.
/// - Keys (one Tab stop, the grid's): arrows go to the next page, Shift extends, Home and End, Enter activates,
///   Delete and Backspace delete (the current page when nothing is picked), Cmd/Ctrl+] and [ move the picked
///   pages, the zoom keys step [`THUMBNAIL_ZOOM`], the Menu key and Shift+F10 ask for the menu.
/// - AccessKit: the grid's listbox named [`ThumbnailWords::pages`], one option per page named
///   [`ThumbnailWords::page`], selected when picked, the current page marked current.
#[derive(craft_ui::contract::Pattern)]
pub struct PageThumbnails<'a> {
    label: Label,
    source: &'a dyn PageSource,
    picked: &'a mut Selection<usize>,
    current: usize,
    paper: DataColor,
    editable: bool,
    zoom: Option<&'a mut f64>,
    caption: CaptionPlace,
    sections: &'a [ThumbnailSection],
    marks: Option<&'a dyn Fn(usize) -> ThumbnailMarks>,
    new_page_row: bool,
}

impl<'a> PageThumbnails<'a> {
    /// Thumbnails of `source`'s pages, named `label`; `picked` is the set page commands act on, kept by the app. A page
    /// whose picture is still loading shows the document's `paper`.
    pub fn new(label: impl Into<Label>, source: &'a dyn PageSource, picked: &'a mut Selection<usize>, current: usize, paper: DataColor) -> Self {
        PageThumbnails {
            label: label.into(),
            source,
            picked,
            current,
            paper,
            editable: true,
            zoom: None,
            caption: CaptionPlace::Below,
            sections: &[],
            marks: None,
            new_page_row: false,
        }
    }

    /// Off: no drag, no stacking keys, no Delete, no file drops (a read-only document).
    #[must_use]
    pub fn editable(mut self, on: bool) -> Self {
        self.editable = on;
        self
    }

    /// Shows the size row above the grid: craft-ui's [`ThumbnailSlider`] over [`THUMBNAIL_ZOOM`], editing `zoom` (a
    /// view preference, not the document). The grid's zoom keys step it too.
    #[must_use]
    pub fn size(mut self, zoom: &'a mut f64) -> Self {
        self.zoom = Some(zoom);
        self
    }

    /// Where page labels go (default [`CaptionPlace::Below`], Acrobat; [`CaptionPlace::Start`] is PowerPoint's number
    /// at the left).
    #[must_use]
    pub fn caption_place(mut self, place: CaptionPlace) -> Self {
        self.caption = place;
        self
    }

    /// PowerPoint's sections, as the grid's collapsible groups: title the name, detail the count.
    #[must_use]
    pub fn sections(mut self, sections: &'a [ThumbnailSection]) -> Self {
        self.sections = sections;
        self
    }

    /// Each page's [`ThumbnailMarks`].
    #[must_use]
    pub fn marks(mut self, marks: &'a dyn Fn(usize) -> ThumbnailMarks) -> Self {
        self.marks = Some(marks);
        self
    }

    /// The New Slide button under the grid (DeckCraft).
    #[must_use]
    pub fn new_page_row(mut self, on: bool) -> Self {
        self.new_page_row = on;
        self
    }

    /// Shows the size row, the grid and the New Slide button. Section headers are the grid's group rows; their
    /// keys count on from the last page (`count + section`), so a header never shares a key with a page.
    // pdfcraft: crates/ui-egui/src/panels/pages.rs:90 `pages`, :36 `thumbnail_size`, :295 `page_drag`; deckcraft:
    // crates/ui-egui/src/panels/thumbs.rs:116 `list` (sections, number at the left, hidden and animation marks).
    pub fn show(self, ui: &mut Ui) -> PageThumbnailsOutput {
        let PageThumbnails { label, source, picked, current, paper, editable, zoom, caption, sections, marks, new_page_row } = self;
        let env = Env::of(ui);
        let words = env.words::<ThumbnailWords>();
        let count = source.page_count();
        let mut events = Vec::new();

        let mut zoom = zoom;
        if let Some(zoom) = zoom.as_deref_mut() {
            ThumbnailSlider::new(words.size.clone(), Bind::bound(zoom), THUMBNAIL_ZOOM).show(ui);
        }
        let scale = zoom.as_deref().copied().unwrap_or(1.0);

        let order: Vec<usize> = (0..count).collect();
        let tiles = keyed(&order, |&page| page);
        let groups: Vec<TileGroup<usize>> = sections
            .iter()
            .enumerate()
            .map(|(at, section)| {
                let mut group = TileGroup::new(count + at, section.first, section.count, section.name.clone());
                group.open = section.open;
                group.detail(words.count.text(section.count as u64, &env.format().integer(section.count as u64)))
            })
            .collect();
        let aspect = if count == 0 { 1.0 } else { aspect_of(source, 0) };
        let mut grid = TileGrid::<usize>::new(label)
            .selection(Bind::bound(&mut *picked))
            .aspect(aspect)
            .zoom(scale as f32)
            .caption(caption)
            .current(Some(current))
            .groups(&groups);
        if !groups.is_empty() {
            grid = grid.collapsible();
        }
        if editable {
            grid = grid.reorder(ReorderRule::new()).takes_files(true).deletable(true);
        }
        let ctx = ui.ctx().clone();
        let thumbnails = Thumbnails::<usize>::in_context(&ctx, "page-thumbnails", ThumbnailOptions::new());
        let shown = grid.show_items(ui, &tiles, &mut |&page, _| {
            let page_label = source.page_label(page);
            let page_marks = marks.map(|marks| marks(page)).unwrap_or_default();
            let mut accessible = vec![words.page.fill(&[("label", page_label.as_str())])];
            let mut tile = Tile::new(page_label);
            if page_marks.hidden {
                accessible.push(words.hidden.as_str().to_owned());
                tile = tile.hidden();
            }
            if page_marks.animated {
                tile = tile.mark(TileMark::new(IconId::new("star"), words.animated.clone()));
            }
            let thumbs = &thumbnails;
            tile.accessible_name(accessible.join(", ")).thumb(page_aspect(source, page), move |cx, rect| {
                let ctx = cx.painter.ctx().clone();
                let request = ThumbRequest::for_cell(rect.width().max(rect.height()), cx.painter.pixels_per_point());
                let state = thumbs.state(&ctx, source, &page, &request, Priority::VISIBLE);
                thumbs.paint(cx, rect, &state, &ThumbPaint::default().paper(paper));
            })
        });

        for event in shown.events {
            match event {
                GridEvent::GroupOpen { group, open, .. } => events.push(ThumbnailEvent::SectionOpen { section: group, open }),
                GridEvent::GroupMenu { group, at, .. } => events.push(ThumbnailEvent::SectionMenu { section: group, at }),
                _ => {}
            }
        }
        for event in shown.response.events {
            match event {
                CollectionEvent::Selection(change) => events.push(ThumbnailEvent::Selection(change)),
                CollectionEvent::LeadMoved { key, .. } if key < count => events.push(ThumbnailEvent::Go(key)),
                CollectionEvent::Activate(page) if page < count => events.push(ThumbnailEvent::Activate(page)),
                CollectionEvent::Delete(pages) => {
                    let pages: Vec<usize> = pages.into_iter().filter(|page| *page < count).collect();
                    if editable && !pages.is_empty() && pages.len() < count {
                        events.push(ThumbnailEvent::Delete(pages));
                    }
                }
                CollectionEvent::Reorder(edit) if editable => events.push(ThumbnailEvent::Reorder(edit)),
                CollectionEvent::ContextMenu(request) if request.key < count => events.push(ThumbnailEvent::ContextMenu(request)),
                CollectionEvent::FilesDropped { insert, .. } if editable => events.push(ThumbnailEvent::FilesDropped { insert }),
                CollectionEvent::Zoom { factor, .. } => {
                    if let Some(zoom) = zoom.as_deref_mut() {
                        *zoom = THUMBNAIL_ZOOM.step(*zoom, if factor > 1.0 { ZoomStep::In } else { ZoomStep::Out });
                    }
                }
                _ => {}
            }
        }
        if new_page_row && clicked(&Button::new(words.new_page.clone()).show(ui)) {
            events.push(ThumbnailEvent::NewPage);
        }
        PageThumbnailsOutput { response: shown.response.response, events }
    }
}

/// Page `at`'s width over its height; 1 for a page without a size.
fn aspect_of(source: &dyn PageSource, at: usize) -> f32 {
    let size = source.page_size(at);
    if size.width > 0.0 && size.height > 0.0 { (size.width / size.height) as f32 } else { 1.0 }
}

/// Page `at`'s picture shape as the tile takes it.
fn page_aspect(source: &dyn PageSource, at: usize) -> ThumbAspect {
    let size = source.page_size(at);
    ThumbAspect::new(size.width.round().clamp(1.0, f64::from(u16::MAX)) as u16, size.height.round().clamp(1.0, f64::from(u16::MAX)) as u16)
}
