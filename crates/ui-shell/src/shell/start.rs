//! `StartScreen`: the page an app shows with no document open: New and Open, the recent files as a grid or a list,
//! pinned folders and templates.

#[cfg(feature = "testing")]
mod stories;

use super::recent::{RecentFile, RecentSort, RecentView, Template, ThumbKey, sort_recent};
use super::words::ShellWords;
use crate::buttons::{Button, Link};
use crate::choice::Segmented;
use crate::contract::{
    ActivateOn, Bind, BodyCx, CollectionEvent, Entry, Gap, NoValue, Outcome, Pad, Pattern, Props, Remember as _, Scroll, SelectMode, Show as _,
    ShowCollection as _, ShowContainer as _, Sizing, clicked,
};
use crate::display::{Divider, Icon};
use crate::layout_components::{Row, Stack};
use crate::list::{List, ListRow, RowEvent, ThumbAspect, ThumbSize, TrailingFace, TrailingPart};
use crate::menu::{Menu, MenuItem};
use crate::notices::{Notice, NoticeLayout};
use crate::pickers::Select;
use crate::primitives::{CheckerArea, ContentPainterCx, IconSpec, Priority, ThumbPaint, ThumbRequest, ThumbSource, Thumbnails};
use crate::runtime::commands::{self, CommandRef, FireSource};
use crate::runtime::commands::{CommandSet as _, Commands as _};
use crate::runtime::env::{AgeStyle, Env, Label};
use crate::sections::Section;
use crate::style::{Corners, SurfaceKind, SurroundLook, paint_surround};
use crate::tile_grid::{CaptionPlace, Tile, TileGrid, TilePartEvent};
use craft_behavior::{IconId, Proposal, SelectionChange, keyed};
use craft_theme::Variant;
use craft_theme::variant::Tone;
use egui::{CornerRadius, Rect, Response, Ui, Vec2};
use std::borrow::Cow;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// The cards of the home screen start at twice the card thumbnail wide and grow to fill their row: one or two to a
/// row in a panel.
const CARD_ZOOM: f32 = 2.0;
/// The width and height parts of a card's picture.
const PICTURE_WIDE: u16 = 16;
const PICTURE_HIGH: u16 = 9;

/// The number of recent files shown until `max_recent` says otherwise.
const RECENT_SHOWN: usize = 12;

/// The revealed part of a recent row that takes the file off the list.
const REMOVE: TrailingPart = TrailingPart::new(0);

/// The commands of a recent file's context menu. They act on the file that has the lead in the recent files, which a
/// secondary click or the context-menu key gives to the file the menu is for. Delete at the lead of the recent files is
/// the list's own delete request and reports the same `RemoveRecent`; `Remove` shows that chord in the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "startRecent", scope = "startRecent")]
#[non_exhaustive]
pub enum StartRecentCommands {
    /// Open the file.
    #[command(label = "Open")]
    Open,
    /// Take the file off the list. The file itself stays.
    #[command(label = "Remove from Recent", default = "Delete")]
    Remove,
    /// Empty the list.
    #[command(label = "Clear Recent Files")]
    Clear,
    /// Show the file in its folder.
    #[command(label = "Show in Folder")]
    Reveal,
}

/// The commands of a pinned folder's context menu. They act on the folder that has the lead in the pinned folders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "startPinned", scope = "startPinned")]
#[non_exhaustive]
pub enum StartPinnedCommands {
    /// Show the folder.
    #[command(label = "Open")]
    Open,
    /// Stop listing the folder. Delete runs it while focus is in the pinned folders.
    #[command(label = "Unpin", default = "Delete")]
    Unpin,
}

/// What the user did on the start screen this pass. The app runs it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StartAction {
    /// New…: the app opens its New Document dialog.
    New,
    /// Open…: the app opens its file dialog.
    Open,
    /// A recent file was opened: the app opens that path.
    OpenRecent(PathBuf),
    /// The file's "Remove from Recent": the app drops the entry.
    RemoveRecent(PathBuf),
    /// "Clear Recent Files": the app drops every entry.
    ClearRecent,
    /// "Show in Folder" on a recent file.
    RevealRecent(PathBuf),
    /// A pinned folder was opened: the app shows that folder.
    OpenPinned(PathBuf),
    /// "Pin Folder…": the app asks for a folder.
    PinFolder,
    /// A pinned folder's "Unpin": the app stops listing that folder.
    UnpinFolder(PathBuf),
    /// A template card was chosen; the string is [`Template::id`].
    UseTemplate(String),
}

/// What a start screen pass reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the app runs the start screen's actions"]
pub struct StartOutput {
    /// The page's response.
    pub response: Response,
    /// What the user did, in order.
    pub actions: Vec<StartAction>,
}

impl Outcome for StartOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The start screen, shown in a shell's body when no document is open.
///
/// - **Layout.** New and Open (the first Tab stops), then Recent with the view switch and the sort, Pinned Folders
///   when the app gives the list (even an empty one) and Templates when it gives any. The page scrolls as a whole, and each grid and list takes the height of its rows.
/// - **Recent files.** A grid of cards or a list, by the `view` binding, in the order of the `sort` binding. A click or
///   Enter opens a file; arrows move; the context menu (secondary click, the menu key or Shift+F10) and the
///   [`StartRecentCommands`] remove it from the list, clear the list or show it in its folder.
/// - **Thumbnails.** With `thumbnails(store, source)`, each card, row and template asks the store for its
///   [`ThumbKey`] at the size it paints: a placeholder until the picture is ready, a failure mark when loading failed,
///   the file icon when the source has no picture for the key. Without it cards show the file icon.
/// - **Links.** `links` adds a rail beside the page (New and Open, then the links) in a room wide enough, else the
///   links follow the sections. A double click on the page's own room opens a file, as Open does.
/// - **No recent files.** With a `drop_hint` the Recent section is a card that opens a file on a click, Enter or Space.
/// - **State.** The app owns the list, the view and the sort. The screen writes a changed view or sort through its
///   binding and reports everything else as [`StartAction`]s.
///
/// ```ignore
/// let out = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &recent).pinned(&folders).show(ui);
/// for action in out.actions { /* the app opens, removes, pins... */ }
/// ```
#[must_use = "show the start screen with `show`"]
#[derive(Pattern, Props)]
pub struct StartScreen<'a> {
    view: Bind<'a, RecentView>,
    sort: Bind<'a, RecentSort>,
    recent: &'a [RecentFile],
    /// The pinned folders in the app's order. The section shows once this is given; an app that cannot pin leaves it
    /// out.
    #[prop]
    pinned: Option<&'a [PathBuf]>,
    /// The template cards; the section is left out when there are none.
    #[prop]
    templates: &'a [Template],
    /// At most this many recent files show.
    #[prop]
    max_recent: usize,
    /// The time the age lines count from, in seconds since the Unix epoch. Without it the cards show no age.
    #[prop]
    now: Option<i64>,
    /// The thumbnails and the source of each key's revision: cards, rows and templates ask `Thumbnails::state`.
    thumbnails: Option<(&'a Thumbnails<ThumbKey>, &'a dyn ThumbSource<ThumbKey>)>,
    /// The links beside the page: New and Open at the rail's top, then these.
    #[prop]
    links: &'a [StartLink],
    /// With no recent files, the Recent body is a drop card with this hint that opens a file on a click.
    #[prop(into)]
    drop_hint: Option<Label>,
}

/// A link on the start screen's rail.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct StartLink {
    /// The link's text.
    pub label: Label,
    /// The link's icon, a catalogue name (`home`, `globe`).
    pub icon: Option<IconId>,
    /// What the link does.
    pub target: StartLinkTarget,
}

/// What a [`StartLink`] does.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StartLinkTarget {
    /// Opens the URL itself.
    Url(String),
    /// Fires the command.
    Command(CommandRef),
}

impl StartLink {
    /// A link that opens `url`, shown as its tooltip.
    pub fn url(label: impl Into<Label>, icon: Option<IconId>, url: impl Into<String>) -> Self {
        StartLink { label: label.into(), icon, target: StartLinkTarget::Url(url.into()) }
    }

    /// A link that fires `command`.
    pub fn command(label: impl Into<Label>, icon: Option<IconId>, command: impl Into<CommandRef>) -> Self {
        StartLink { label: label.into(), icon, target: StartLinkTarget::Command(command.into()) }
    }
}

impl<'a> StartScreen<'a> {
    /// A start screen of `recent` files, shown in `view` and ordered by `sort`; names sort in the env's language.
    pub fn new(view: Bind<'a, RecentView>, sort: Bind<'a, RecentSort>, recent: &'a [RecentFile]) -> Self {
        StartScreen {
            view,
            sort,
            recent,
            pinned: None,
            templates: &[],
            max_recent: RECENT_SHOWN,
            now: None,
            thumbnails: None,
            links: &[],
            drop_hint: None,
        }
    }

    /// Cards, rows and templates ask `thumbnails.state(..)` for what they show; `source` gives each key's revision.
    #[must_use]
    pub fn thumbnails(mut self, thumbnails: &'a Thumbnails<ThumbKey>, source: &'a dyn ThumbSource<ThumbKey>) -> Self {
        self.thumbnails = Some((thumbnails, source));
        self
    }
}

/// What each list of the page keeps between passes: the item that has the lead, which the commands act on.
#[derive(Clone, Debug, Default)]
struct Leads {
    recent: Option<PathBuf>,
    pinned: Option<PathBuf>,
}

/// The thumbnails and the source of each key's revision.
type ThumbHost<'p> = (&'p Thumbnails<ThumbKey>, &'p dyn ThumbSource<ThumbKey>);

/// What every section of one pass reads.
struct Page<'p> {
    env: &'p Env,
    words: &'p ShellWords,
    thumbnails: Option<ThumbHost<'p>>,
    drop_hint: Option<&'p Label>,
    now: Option<i64>,
    actions: &'p RefCell<Vec<StartAction>>,
}

impl<'p> Page<'p> {
    fn push(&self, action: StartAction) {
        self.actions.borrow_mut().push(action);
    }

    /// How long before `now` `then` was, in words; `None` when either time is unknown.
    fn age(&self, then: Option<i64>) -> Option<String> {
        let (now, then) = (self.now?, then?);
        Some(self.env.relative_age(now.saturating_sub(then).max(0).unsigned_abs(), AgeStyle::Long))
    }

    /// A card named `name` whose picture is the thumbnail of `key`, asked for at the cell it paints in; the file icon
    /// without `thumbnails`.
    fn tile<'t>(&self, name: &str, detail: Option<String>, key: ThumbKey) -> Tile<'t>
    where
        'p: 't,
    {
        let mut tile = Tile::new(Label::from(name.to_owned()));
        if let Some(detail) = detail {
            tile = tile.detail(detail);
        }
        match self.thumbnails {
            Some((thumbnails, source)) => {
                tile.thumb(ThumbAspect::new(PICTURE_WIDE, PICTURE_HIGH), move |cx, rect| paint_thumb(cx, rect, thumbnails, source, &key))
            }
            None => tile.icon(IconId::new("file")),
        }
    }

    fn grid<K: Clone + Eq + std::hash::Hash + Send + Sync + 'static>(&self, label: Label, deletable: bool) -> TileGrid<'static, K> {
        TileGrid::<K>::new(label)
            .select(SelectMode::Single)
            .deletable(deletable)
            .activate(ActivateOn::Click)
            .caption(CaptionPlace::Below)
            .detail_line()
            .card()
            .aspect(f32::from(PICTURE_WIDE) / f32::from(PICTURE_HIGH))
            .zoom(CARD_ZOOM)
            .fill_row()
            .sizing(Sizing::FillWidth)
            .natural_height(true)
    }
}

/// The thumbnail of `key` in `rect`, asked for at the size it paints. The store paints the placeholder while the
/// picture is not ready and the failure mark when it failed; a key the source has no picture for (`Blank`) gets the
/// file icon on the sunken tile.
fn paint_thumb(cx: &ContentPainterCx, rect: Rect, thumbnails: &Thumbnails<ThumbKey>, source: &dyn ThumbSource<ThumbKey>, key: &ThumbKey) {
    let request = ThumbRequest::for_cell(rect.width().max(rect.height()), cx.pixels_per_point());
    let state = thumbnails.state(cx.painter.ctx(), source, key, &request, Priority::VISIBLE);
    if thumbnails.paint(cx, rect, &state, &ThumbPaint::default()) {
        return;
    }
    cx.surface(rect, SurfaceKind::Sunken, Corners::None);
    let side = cx.style_cx().tokens.size.icon.lg;
    cx.icon(Rect::from_center_size(rect.center(), Vec2::splat(side)), &IconSpec::svg(IconId::new("file")));
}

/// The folder a path is in, for the line under a file's name.
fn folder_of(path: &Path) -> String {
    path.parent().map(|folder| folder.display().to_string()).unwrap_or_default()
}

/// What a collection's events mean for a section: the keys to open and the keys to delete, and the lead after them.
fn read_events<K: Clone, E>(events: Vec<CollectionEvent<K, E>>, lead: &mut Option<K>) -> (Vec<K>, Vec<K>) {
    let (mut opened, mut deleted) = (Vec::new(), Vec::new());
    for event in events {
        match event {
            CollectionEvent::Activate(key) => {
                *lead = Some(key.clone());
                opened.push(key);
            }
            CollectionEvent::LeadMoved { key, .. } => *lead = Some(key),
            CollectionEvent::Delete(keys) => deleted.extend(keys),
            CollectionEvent::Selection(SelectionChange::Set(keys)) => {
                if let Some(key) = keys.into_iter().last() {
                    *lead = Some(key);
                }
            }
            _ => {}
        }
    }
    (opened, deleted)
}

impl StartScreen<'_> {
    /// Shows the page across the available room and reports what the user did.
    pub fn show(self, ui: &mut Ui) -> StartOutput {
        let StartScreen { view, sort, recent, pinned, templates, max_recent, now, thumbnails, links, drop_hint } = self;
        let env = Env::of(ui);
        let collator = env.collator();
        let words = env.words::<ShellWords>();
        let actions = RefCell::new(Vec::new());
        let page = Page { env: &env, words: &words, thumbnails, drop_hint: drop_hint.as_ref(), now, actions: &actions };
        let view = RefCell::new(view);
        let sort = RefCell::new(sort);

        let room = ui.available_rect_before_wrap();
        // The page's surround is square.
        paint_surround(ui.painter(), CheckerArea::Rect(room, CornerRadius::ZERO), &SurroundLook::of(&env.tokens).shown(env.canvas_pattern));

        let rail_stack = || Stack::new().gap(Gap::Tight).pad(Pad::Regular).label(words.start_links.clone());
        let rail_body = |ui: &mut Ui| {
            new_and_open(&page, ui);
            let _ = Divider::horizontal().show(ui);
            link_items(ui, links);
        };
        let page_stack = || Stack::new().gap(Gap::Section).pad(Pad::Regular).fill_width();
        // `beside` is whether the rail stands beside the page; without it New and Open lead the page and the links follow
        // the sections.
        let page_body = |ui: &mut Ui, cx: &mut BodyCx<'_>, beside: bool| {
            let current_view = view.borrow_mut().reborrow().read(cx).and_then(|held| held.same().copied()).unwrap_or_default();
            let current_sort = sort.borrow_mut().reborrow().read(cx).and_then(|held| held.same().copied()).unwrap_or_default();
            let mut files: Vec<RecentFile> = recent.iter().take(max_recent).cloned().collect();
            sort_recent(&mut files, current_sort, &collator);

            if !beside {
                let _ = Row::new().gap(Gap::Tight).show(ui, |ui, _| new_and_open(&page, ui));
            }
            recent_section(&page, ui, &view, &sort, current_view, &files);
            if let Some(pinned) = pinned {
                pinned_section(&page, ui, pinned);
            }
            if !templates.is_empty() {
                templates_section(&page, ui, templates);
            }
            if !beside && !links.is_empty() {
                let _ = Stack::new().gap(Gap::Tight).label(words.start_links.clone()).show(ui, |ui, _| link_items(ui, links));
            }
        };

        // The rail stands beside the sections when the room holds the rail, the gap and the sections' least width; a
        // narrower page keeps New and Open in its row and lists the links after the sections.
        let rail = !links.is_empty() && {
            let rail_width = rail_stack().measure_in(ui, Proposal::Ideal, |ui, _| rail_body(ui)).x;
            let sections_least = page_stack().measure_in(ui, Proposal::Min, |ui, cx| page_body(ui, cx, true)).x;
            ui.available_width() >= rail_width + env.tokens.space.gap.section + sections_least
        };

        // The page senses its own surface for the empty-room double click (`BodyCx::surface_double_pressed`). A pass reads
        // only its own input, so a second pass of one frame reports the double click only if it holds the second press.
        let shown = Stack::new().fill().scroll(Scroll::vertical()).senses_surface().show(ui, |ui, cx| {
            if cx.surface_double_pressed() {
                page.push(StartAction::Open);
            }
            if rail {
                let _ = Row::new().gap(Gap::Section).fill_width().show(ui, |ui, _| {
                    let _ = rail_stack().show(ui, |ui, _| rail_body(ui));
                    let _ = page_stack().show(ui, |ui, cx| page_body(ui, cx, true));
                });
            } else {
                let _ = page_stack().show(ui, |ui, cx| page_body(ui, cx, false));
            }
        });
        StartOutput { response: shown.output, actions: actions.into_inner() }
    }
}

/// New and Open, the first Tab stops of the page.
fn new_and_open(page: &Page<'_>, ui: &mut Ui) {
    let words = page.words;
    if clicked(&Button::new(words.new.clone()).id_salt("new").variant(Variant::PRIMARY).show(ui)) {
        page.push(StartAction::New);
    }
    if clicked(&Button::new(words.open.clone()).id_salt("open").show(ui)) {
        page.push(StartAction::Open);
    }
}

/// The links, one under another: a URL opens through its `Link` (the URL is its tooltip), a command fires as a
/// pointer activation.
fn link_items(ui: &mut Ui, links: &[StartLink]) {
    for (at, link) in links.iter().enumerate() {
        match &link.target {
            StartLinkTarget::Url(url) => {
                let _ = Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                    if let Some(icon) = &link.icon {
                        let _ = Icon::new(IconSpec::svg(icon.clone())).id_salt(("start-link-icon", at)).show(ui);
                    }
                    let _ = Link::new(link.label.clone()).id_salt(("start-link", at)).url(url.clone()).tooltip(url.clone()).show(ui);
                });
            }
            StartLinkTarget::Command(command) => {
                let mut button = Button::new(link.label.clone()).id_salt(("start-link", at));
                if let Some(icon) = &link.icon {
                    button = button.icon(icon.clone());
                }
                if clicked(&button.show(ui)) {
                    let _ = commands::fire(ui.ctx(), command.clone(), FireSource::Pointer);
                }
            }
        }
    }
}

fn recent_section(
    page: &Page<'_>,
    ui: &mut Ui,
    view: &RefCell<Bind<'_, RecentView>>,
    sort: &RefCell<Bind<'_, RecentSort>>,
    current: RecentView,
    files: &[RecentFile],
) {
    let words = page.words;
    let modes = [
        Entry::new(RecentView::Grid, words.grid_view.clone()).icon(IconId::new("layout_grid")),
        Entry::new(RecentView::List, words.list_view.clone()).icon(IconId::new("list")),
    ];
    let orders = [
        Entry::new(RecentSort::Opened, words.sort_opened.clone()),
        Entry::new(RecentSort::Name, words.sort_name.clone()),
        Entry::new(RecentSort::Created, words.sort_created.clone()),
    ];
    let _ = Section::fixed(words.recent.clone())
        .header_gap(Gap::Section)
        .command_scope(StartRecentCommands::SCOPE)
        .controls(|ui, _| {
            let _ = Segmented::new(words.view.clone(), view.borrow_mut().reborrowed(), &modes).show(ui);
            let _ = Select::new(words.sort.clone(), sort.borrow_mut().reborrowed(), &orders).show(ui);
        })
        .show(ui, |ui, cx| {
            let mut lead = cx.keep::<Leads>().recent.clone().filter(|path| files.iter().any(|file| &file.path == path));
            // One rule for the menu row, the palette and the keys: where the platform cannot reveal a file, the command is disabled.
            cx.publish(StartRecentCommands::Reveal, commands::CommandState::new().enabled(craft_platform::open::CAN_REVEAL));
            while let Some((command, _)) = cx.take_command::<StartRecentCommands>() {
                let action = match (command, lead.clone()) {
                    (StartRecentCommands::Clear, _) => Some(StartAction::ClearRecent),
                    (StartRecentCommands::Open, Some(path)) => Some(StartAction::OpenRecent(path)),
                    (StartRecentCommands::Remove, Some(path)) => Some(StartAction::RemoveRecent(path)),
                    (StartRecentCommands::Reveal, Some(path)) => Some(StartAction::RevealRecent(path)),
                    _ => None,
                };
                if let Some(action) = action {
                    page.push(action);
                }
            }
            if files.is_empty() {
                match page.drop_hint {
                    Some(hint) => {
                        // The card is a button, so a click, Enter and Space open a file and it has its own Tab stop.
                        let card = Button::new(hint.clone()).id_salt("drop").icon(IconId::new("image")).sizing(Sizing::FillWidth).show(ui);
                        if clicked(&card) {
                            page.push(StartAction::Open);
                        }
                    }
                    None => {
                        let _ = Notice::new(Tone::Neutral, words.no_recent.clone()).layout(NoticeLayout::Centred).show(ui);
                    }
                }
                return;
            }
            let file_of = |path: &PathBuf| files.iter().find(|file| &file.path == path);
            let source = keyed(files, |file: &RecentFile| file.path.clone()).with_text(|file| Cow::Borrowed(file.name.as_str()));
            let (response, (opened, removed)) = if current == RecentView::Grid {
                let out = page.grid::<PathBuf>(words.recent.clone(), true).show_items(ui, &source, &mut |path, _| {
                    let Some(file) = file_of(path) else { return Tile::new(Label::Empty) };
                    page.tile(&file.name, page.age(file.opened), ThumbKey::Recent(file.path.clone()))
                        .corner_action(IconId::new("close"), StartRecentCommands::Remove)
                        .tooltip(file.path.display().to_string())
                });
                let corner: Vec<PathBuf> = out
                    .response
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        CollectionEvent::Item(key, TilePartEvent::CornerAction) => Some(key.clone()),
                        _ => None,
                    })
                    .collect();
                let (opened, mut removed) = read_events(out.response.events, &mut lead);
                removed.extend(corner);
                (out.response.response, (opened, removed))
            } else {
                let out = List::<PathBuf>::new(words.recent.clone())
                    .select(SelectMode::Single)
                    .deletable(true)
                    .activate(ActivateOn::Click)
                    .sizing(Sizing::FillWidth)
                    .natural_height(true)
                    .show_items(ui, &source, &mut |path, _| {
                        let Some(file) = file_of(path) else { return ListRow::new(Label::Empty) };
                        let row = ListRow::new(Label::from(file.name.clone())).detail(folder_of(&file.path)).tooltip(file.path.display().to_string());
                        let row = match page.age(file.opened) {
                            Some(age) => row.trailing(age),
                            None => row,
                        };
                        let row = row.trailing_part_revealed(
                            REMOVE,
                            TrailingFace::Icon(IconSpec::svg(IconId::new("close"))),
                            words.remove_recent.fill(&[("name", file.name.as_str())]),
                        );
                        match page.thumbnails {
                            Some((thumbnails, source)) => {
                                let key = ThumbKey::Recent(file.path.clone());
                                row.thumb_aspect(ThumbSize::Medium, ThumbAspect::new(PICTURE_WIDE, PICTURE_HIGH), move |cx, rect| {
                                    paint_thumb(cx, rect, thumbnails, source, &key);
                                })
                            }
                            None => row.icon(IconSpec::svg(IconId::new("file"))),
                        }
                    });
                // The revealed remove part reports `RowEvent::Trailing`, which `read_events` does not read.
                let part: Vec<PathBuf> = out
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        CollectionEvent::Item(key, RowEvent::Trailing { part, .. }) if *part == REMOVE => Some(key.clone()),
                        _ => None,
                    })
                    .collect();
                let (opened, mut removed) = read_events(out.events, &mut lead);
                removed.extend(part);
                (out.response, (opened, removed))
            };
            for path in opened {
                page.push(StartAction::OpenRecent(path));
            }
            for path in removed {
                page.push(StartAction::RemoveRecent(path));
            }
            if lead.is_some() {
                let _ = Menu::context(words.recent_menu.clone(), &response).show(ui, |ui, _| {
                    let _ = MenuItem::command(StartRecentCommands::Open).show(ui);
                    let _ = MenuItem::command(StartRecentCommands::Remove).show(ui);
                    let _ = MenuItem::command(StartRecentCommands::Clear).show(ui);
                    let _ = MenuItem::command(StartRecentCommands::Reveal).show(ui);
                });
            }
            cx.keep::<Leads>().recent = lead;
        });
}

fn pinned_section(page: &Page<'_>, ui: &mut Ui, folders: &[PathBuf]) {
    let words = page.words;
    let _ = Section::fixed(words.pinned.clone())
        .header_gap(Gap::Section)
        .command_scope(StartPinnedCommands::SCOPE)
        .controls(|ui, _| {
            if clicked(&Button::new(words.pin_folder.clone()).id_salt("pin").icon(IconId::new("new_folder")).show(ui)) {
                page.push(StartAction::PinFolder);
            }
        })
        .show(ui, |ui, cx| {
            let mut lead = cx.keep::<Leads>().pinned.clone().filter(|path| folders.contains(path));
            while let Some((command, _)) = cx.take_command::<StartPinnedCommands>() {
                let Some(path) = lead.clone() else { continue };
                page.push(match command {
                    StartPinnedCommands::Open => StartAction::OpenPinned(path),
                    StartPinnedCommands::Unpin => StartAction::UnpinFolder(path),
                });
            }
            if folders.is_empty() {
                let _ = Notice::new(Tone::Neutral, words.no_pinned.clone()).layout(NoticeLayout::Centred).show(ui);
                return;
            }
            let source = keyed(folders, |folder: &PathBuf| folder.clone());
            let out = List::<PathBuf>::new(words.pinned.clone())
                .select(SelectMode::Single)
                .activate(ActivateOn::Click)
                .sizing(Sizing::FillWidth)
                .natural_height(true)
                .show_items(ui, &source, &mut |path, _| {
                    let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
                    ListRow::new(Label::from(name)).detail(path.display().to_string()).icon(IconSpec::svg(IconId::new("folder")))
                });
            for path in read_events(out.events, &mut lead).0 {
                page.push(StartAction::OpenPinned(path));
            }
            if lead.is_some() {
                let _ = Menu::context(words.pinned_menu.clone(), &out.response).show(ui, |ui, _| {
                    let _ = MenuItem::command(StartPinnedCommands::Open).show(ui);
                    let _ = MenuItem::command(StartPinnedCommands::Unpin).show(ui);
                });
            }
            cx.keep::<Leads>().pinned = lead;
        });
}

fn templates_section(page: &Page<'_>, ui: &mut Ui, templates: &[Template]) {
    let words = page.words;
    let _ = Section::fixed(words.templates.clone()).header_gap(Gap::Section).show(ui, |ui, _| {
        let source = keyed(templates, |template: &Template| template.id.clone()).with_text(|template| Cow::Borrowed(template.name.as_str()));
        let out = page.grid::<String>(words.templates.clone(), false).show_items(ui, &source, &mut |id, _| {
            let Some(template) = templates.iter().find(|template| &template.id == id) else { return Tile::new(Label::Empty) };
            let detail = (!template.detail.is_empty()).then(|| template.detail.clone());
            page.tile(&template.name, detail, ThumbKey::Template(template.id.clone()))
        });
        let mut lead = None;
        for id in read_events(out.response.events, &mut lead).0 {
            page.push(StartAction::UseTemplate(id));
        }
    });
}
