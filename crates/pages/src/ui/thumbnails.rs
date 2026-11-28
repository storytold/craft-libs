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

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::view::PageSource;
use craft_behavior::{CopyKeys, EditEvent, IconId, MiddleButton, Selection, SelectionChange, SelectionModel as _, keyed};
use craft_ui::contract::{
    Bind, CollectionEvent, ContextMenuRequest, DropAt, DropKind, DropPlace, IdSource, NoValue, OsFiles, Outcome, PendingDrop, Props as _,
    ReorderMove, ReorderRule, Show as _, ShowCollection as _, Sizing,
};
use craft_ui::list::{ThumbAspect, ToggleState};
use craft_ui::primitives::textures::{Priority, ThumbPaint, ThumbRequest, ThumbnailOptions, Thumbnails, texture_side};
use craft_ui::runtime::commands::CommandRef;
use craft_ui::runtime::drag::OsFile;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::DataColor;
use craft_ui::tile_grid::{
    CaptionPlace, GridEvent, GridZoom, TailRow, ThumbnailSlider, Tile, TileActionPlace, TileGrid, TileGroup, TileMark, TilePartEvent, TileZoom,
};
use egui::{Id, Pos2, Response, Ui};

/// The thumbnail zoom: 0.5 to 1.5 of the card thumbnail in steps of 0.25, 1 by default (PdfCraft's
/// `THUMBNAIL_ZOOM_RANGE` and its minus and plus step). The size row's [`ThumbnailSlider`] and the grid's zoom keys
/// step it.
pub const THUMBNAIL_ZOOM: TileZoom = TileZoom::bounded(0.5, 1.5, 0.25, 1.0);

/// The Organize grid's zoom: 0.5 to 3 of the card thumbnail in steps of 0.25, 1 by default.
pub const ORGANIZE_ZOOM: TileZoom = TileZoom::bounded(0.5, 3.0, 0.25, 1.0);

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
    /// Delete or Backspace with the panel focused or under the pointer: the picked pages, or the current page when
    /// none are; never when
    /// every page is picked (a document keeps one page), never read-only.
    Delete(Vec<usize>),
    /// A double-click or Enter on a thumbnail (PowerPoint starts the slide show from it).
    Activate(usize),
    /// A click, Enter or Space on the New Slide row ([`PageThumbnails::new_page_row`]).
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
        /// The files dropped.
        files: Vec<OsFile>,
    },
    /// A click on action `action` of [`PageThumbnails::actions`] in page `page`'s band.
    #[non_exhaustive]
    Action {
        /// The page.
        page: usize,
        /// Index of the action in [`PageThumbnails::actions`].
        action: usize,
    },
    /// A click on the insert gap at position `0`-based `insert`, before that page (the page count after the last).
    Insert(usize),
}

/// How the thumbnails are arranged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ThumbnailArrangement {
    /// The side strip: the size row over the grid.
    #[default]
    Strip,
    /// The Organize grid: no size row, check boxes, actions in the band and insert gaps when editable.
    Organize,
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
    /// While pages are dragged, the move their release would make.
    pub drop_target: Option<ReorderMove<usize>>,
    /// While files from the OS are held over the grid, the page they would land before (the page count after the
    /// last).
    pub file_gap: Option<usize>,
    /// A middle-button auto-scroll is latched on the grid ([`PageThumbnails::middle_button`] with
    /// `MiddleButton::AutoScroll`).
    pub auto_scrolling: bool,
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
///   ([`ThumbnailEvent::Go`]) and picks it; Cmd/Ctrl-click toggles a page in the picked set and Shift-click extends
///   it. With [`picks_beside_current`](Self::picks_beside_current) the picked set is kept beside the current page.
/// - Drag (when editable): the grid's reorder rule; the grid draws the insert line and scrolls near its edges.
/// - Keys (one Tab stop, the grid's): arrows go to the next page, Shift extends, Home and End, Enter activates,
///   Delete and Backspace delete (the current page when nothing is picked), Cmd/Ctrl+] and [ move the picked
///   pages, the zoom keys step [`THUMBNAIL_ZOOM`] ([`ORGANIZE_ZOOM`] in Organize), the Menu key and Shift+F10 ask for
///   the menu.
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
    arrangement: ThumbnailArrangement,
    actions: &'a [(IconId, CommandRef)],
    action_label: Option<&'a dyn Fn(usize, usize) -> Label>,
    insert_label: Option<&'a dyn Fn(usize) -> Label>,
    middle_button: Option<MiddleButton>,
    invert_middle: bool,
    id: IdSource,
    reveal: Option<usize>,
    picks_beside_current: bool,
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
            arrangement: ThumbnailArrangement::Strip,
            actions: &[],
            action_label: None,
            insert_label: None,
            middle_button: None,
            invert_middle: false,
            id: IdSource::Auto,
            reveal: None,
            picks_beside_current: false,
        }
    }

    /// Keeps the picked pages beside the current page (Acrobat's Pages panel): a plain click goes to its page and
    /// empties the pick, and Cmd/Ctrl- and Shift-clicks change the pick without going to the page, the current page
    /// counting as picked while nothing is. Off (PowerPoint's slide strip, Organize), a plain click picks the page it
    /// goes to.
    #[must_use]
    pub fn picks_beside_current(mut self, on: bool) -> Self {
        self.picks_beside_current = on;
        self
    }

    /// How the thumbnails are arranged: the side strip, or the Organize grid.
    #[must_use]
    pub fn arrangement(mut self, arrangement: ThumbnailArrangement) -> Self {
        self.arrangement = arrangement;
        self
    }

    /// The actions in each page's band in Organize; a click reports [`ThumbnailEvent::Action`].
    #[must_use]
    pub fn actions(mut self, actions: &'a [(IconId, CommandRef)]) -> Self {
        self.actions = actions;
        self
    }

    /// The accessible name of action `action` of [`PageThumbnails::actions`] in page `page`'s band, called as
    /// `label(action, page)`: a name that says which page it acts on ("Rotate page 1 clockwise"). Without it an action
    /// reads as its command's label.
    #[must_use]
    pub fn action_label(mut self, label: &'a dyn Fn(usize, usize) -> Label) -> Self {
        self.action_label = Some(label);
        self
    }

    /// The name of the insert gap before page `n` in Organize; an editable Organize grid shows insert gaps only with
    /// a name for them.
    #[must_use]
    pub fn insert_label(mut self, label: &'a dyn Fn(usize) -> Label) -> Self {
        self.insert_label = Some(label);
        self
    }

    /// What the middle button does on the grid.
    #[must_use]
    pub fn middle_button(mut self, button: MiddleButton) -> Self {
        self.middle_button = Some(button);
        self
    }

    /// Flips the direction of the grid's middle-button pan and auto-scroll (the user's Invert middle setting).
    #[must_use]
    pub fn invert_middle(mut self, on: bool) -> Self {
        self.invert_middle = on;
        self
    }

    /// Uses `id` as the grid's id, so the host can end a press open on it with [`cancel_press`](Self::cancel_press).
    #[must_use]
    pub fn id(mut self, id: Id) -> Self {
        self.id = IdSource::Id(id);
        self
    }

    /// Ends a press or page drag open on the grid with id `id`, as Escape does, for a host whose pages changed under
    /// the drag; nothing happens with no open press.
    pub fn cancel_press(ctx: &egui::Context, id: Id) {
        craft_ui::primitives::cancel_press_of(ctx, id);
    }

    /// Scrolls to `page` and moves the lead there in this pass: a one-shot request, not held between passes.
    #[must_use]
    pub fn reveal(mut self, page: Option<usize>) -> Self {
        self.reveal = page;
        self
    }

    /// Off: no drag, no stacking keys, no Delete, no file drops (a read-only document).
    #[must_use]
    pub fn editable(mut self, on: bool) -> Self {
        self.editable = on;
        self
    }

    /// Shows the size row above the grid: craft-ui's [`ThumbnailSlider`] over [`THUMBNAIL_ZOOM`], editing `zoom` (a
    /// view preference, not the document). The grid's zoom keys, Ctrl/Cmd with the wheel and a pinch step it too. In
    /// Organize there is no size row (the host's bar holds the slider) and the grid steps `zoom` over
    /// [`ORGANIZE_ZOOM`].
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

    /// The New Slide row after the last thumbnail, a row of the grid (DeckCraft).
    #[must_use]
    pub fn new_page_row(mut self, on: bool) -> Self {
        self.new_page_row = on;
        self
    }

    /// Shows the size row and the grid with its New Slide row. Section headers are the grid's group rows; their
    /// keys count on from the last page (`count + section`), so a header never shares a key with a page.
    // pdfcraft: crates/ui-egui/src/panels/pages.rs:90 `pages`, :36 `thumbnail_size`, :295 `page_drag`; deckcraft:
    // crates/ui-egui/src/panels/thumbs.rs:116 `list` (sections, number at the left, hidden and animation marks).
    pub fn show(self, ui: &mut Ui) -> PageThumbnailsOutput {
        let PageThumbnails {
            label,
            source,
            picked,
            current,
            paper,
            editable,
            zoom,
            caption,
            sections,
            marks,
            new_page_row,
            arrangement,
            actions,
            action_label,
            insert_label,
            middle_button,
            invert_middle,
            id,
            reveal,
            picks_beside_current,
        } = self;
        let organize = arrangement == ThumbnailArrangement::Organize;
        let env = Env::of(ui);
        let words = env.words::<ThumbnailWords>();
        let count = source.page_count();
        let mut events = Vec::new();

        let mut zoom = zoom;
        if !organize && let Some(zoom) = zoom.as_deref_mut() {
            ThumbnailSlider::new(words.size.clone(), Bind::bound(zoom), THUMBNAIL_ZOOM).show(ui);
        }
        let range = if organize { ORGANIZE_ZOOM } else { THUMBNAIL_ZOOM };
        let grid_zoom = match zoom {
            Some(zoom) => GridZoom::Bound(Bind::bound(zoom), range),
            None => GridZoom::Fixed(1.0),
        };
        // The grid holds the selection while it shows, so the tiles' checks read the picked set as it was before.
        let checked: Vec<bool> = if organize { (0..count).map(|page| picked.contains(&page)).collect() } else { Vec::new() };

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
        // The New Slide row's key comes after the pages' and the section headers'.
        let new_page = count + groups.len();
        let tail: Vec<TailRow<usize>> = if new_page_row { vec![TailRow::new(new_page, words.new_page.clone())] } else { Vec::new() };
        let aspect = grid_aspect(source, organize);
        let mut grid = TileGrid::<usize>::new(label)
            .id_source(id)
            .selection(Bind::bound(&mut *picked))
            .aspect(aspect)
            .zoom(grid_zoom)
            .caption(caption)
            .current(Some(current))
            .picks_beside_current(picks_beside_current)
            .groups(&groups)
            .tail(&tail);
        if !organize {
            grid = grid.card_columns(1).sizing(Sizing::Content);
        }
        if let Some(page) = reveal {
            grid = grid.reveal(page);
        }
        if let Some(button) = middle_button {
            grid = grid.middle_button(button);
        }
        grid = grid.invert_middle(invert_middle);
        if !groups.is_empty() {
            grid = grid.collapsible();
        }
        if organize {
            grid = grid.checks(true).action_place(TileActionPlace::Band);
        }
        if editable {
            grid = editing(grid, &words, organize, new_page, insert_label);
        }
        let ctx = ui.ctx().clone();
        let thumbnails = Thumbnails::<usize>::in_context(&ctx, "page-thumbnails", ThumbnailOptions::new().max_side(texture_side(&ctx)));
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
            if organize {
                let on = checked.get(page).copied().unwrap_or(false);
                tile = tile.check(if on { ToggleState::On } else { ToggleState::Off });
                for (at, (icon, command)) in actions.iter().enumerate() {
                    tile = match action_label {
                        Some(label) => tile.action_named(icon.clone(), command.clone(), label(at, page)),
                        None => tile.action(icon.clone(), command.clone()),
                    };
                }
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
                GridEvent::Insert { at, .. } if editable && at <= count => events.push(ThumbnailEvent::Insert(at)),
                _ => {}
            }
        }
        let (drop_target, file_gap) = match shown.response.pending_drop {
            Some(PendingDrop::Reorder(mv)) => (Some(mv), None),
            Some(PendingDrop::Drop(DropAt::Gap { insert, .. })) => (None, Some(insert)),
            _ => (None, None),
        };
        for event in shown.response.events {
            match event {
                CollectionEvent::Selection { change, .. } => events.push(ThumbnailEvent::Selection(change)),
                CollectionEvent::LeadMoved { key, .. } if key < count => events.push(ThumbnailEvent::Go(key)),
                CollectionEvent::Activate(page) if page < count => events.push(ThumbnailEvent::Activate(page)),
                CollectionEvent::Activate(key) if key == new_page => events.push(ThumbnailEvent::NewPage),
                CollectionEvent::Delete(pages) => {
                    let pages: Vec<usize> = pages.into_iter().filter(|page| *page < count).collect();
                    if editable && !pages.is_empty() && pages.len() < count {
                        events.push(ThumbnailEvent::Delete(pages));
                    }
                }
                CollectionEvent::Reorder(edit) if editable => events.push(ThumbnailEvent::Reorder(edit)),
                CollectionEvent::ContextMenu(request) if request.key < count => events.push(ThumbnailEvent::ContextMenu(request)),
                CollectionEvent::Dropped { at: DropAt::Gap { insert, .. }, payload, .. } if editable => {
                    if let Some(files) = payload.get::<OsFiles>() {
                        events.push(ThumbnailEvent::FilesDropped { insert, files: files.files.clone() });
                    }
                }
                // A check toggles its page in the picked set whatever modifiers are held, and goes to it.
                CollectionEvent::Item(page, TilePartEvent::Check { .. }) if page < count => {
                    let change = SelectionChange::Toggle(page);
                    picked.apply(change.clone());
                    events.push(ThumbnailEvent::Selection(change));
                    events.push(ThumbnailEvent::Go(page));
                }
                CollectionEvent::Item(page, TilePartEvent::Action(action)) if page < count && action < actions.len() => {
                    events.push(ThumbnailEvent::Action { page, action });
                }
                _ => {}
            }
        }
        let auto_scrolling = shown.response.auto_scrolling;
        PageThumbnailsOutput { response: shown.response.response, events, drop_target, file_gap, auto_scrolling }
    }
}

/// The grid's tile aspect: Organize's portrait frame for every page, else the first page's.
fn grid_aspect(source: &dyn PageSource, organize: bool) -> f32 {
    // Organize uses a portrait page frame for every page, so landscape pages fit inside it instead of collapsing the
    // room reserved for the controls around the picture.
    if organize {
        0.85
    } else if source.page_count() == 0 {
        1.0
    } else {
        aspect_of(source, 0)
    }
}

/// An editable grid's rules: reordering and copying, file drops between pages, deleting, and Organize's insert gaps.
fn editing<'g>(
    grid: TileGrid<'g, usize>,
    words: &ThumbnailWords,
    organize: bool,
    new_page: usize,
    insert_label: Option<&'g dyn Fn(usize) -> Label>,
) -> TileGrid<'g, usize> {
    // Pages copy as files do in a file manager: Ctrl off macOS, Option on macOS.
    let rule = if organize { ReorderRule::new().copy_with(CopyKeys::Platform) } else { ReorderRule::new() };
    let grid = grid
        .reorder(rule)
        .accepts(move |_, _, insert| insert <= new_page)
        .drag_count(words.count.counted())
        .takes_drop(DropKind::new(DropPlace::Between, |_: &OsFiles, _| true))
        .deletable(true)
        .delete_target(craft_ui::contract::DeleteTarget::FocusOrHover);
    match insert_label {
        Some(insert_label) if organize => grid.insert_gaps(insert_label),
        _ => grid,
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
