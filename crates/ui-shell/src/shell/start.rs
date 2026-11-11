//! `StartScreen`: the page an app shows with no document open: New and Open, the recent files as a grid or a list,
//! pinned folders and templates.

#[cfg(feature = "testing")]
mod stories;

use super::recent::{RecentFile, RecentSort, RecentView, Template, ThumbKey, sort_recent};
use super::words::ShellWords;
use crate::buttons::Button;
use crate::choice::Segmented;
use crate::contract::{
    ActivateOn, Bind, CollectionEvent, Entry, Gap, NoValue, Outcome, Pad, Pattern, Props, Remember as _, Scroll, SelectMode, Show as _,
    ShowCollection as _, ShowContainer as _, Sizing, clicked,
};
use crate::layout_components::{Row, Stack};
use crate::list::{List, ListRow, ThumbAspect};
use crate::menu::{Menu, MenuItem};
use crate::notices::{Notice, NoticeLayout};
use crate::pickers::Select;
use crate::primitives::{CachedTexture, ContentPainterCx, IconSpec, Revision, TextureState, TextureStore};
use crate::runtime::commands::{CommandSet as _, Commands as _};
use crate::runtime::env::{AgeStyle, Env, Label};
use crate::sections::Section;
use crate::tile_grid::{CaptionPlace, Tile, TileGrid, TilePartEvent, TileStatus};
use craft_behavior::{IconId, SelectionChange, keyed};
use craft_theme::Variant;
use craft_theme::variant::Tone;
use egui::{Pos2, Rect, Response, Ui};
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
/// - **Thumbnails.** A file or template with a revision shows the picture the `thumbnails` store holds for its
///   [`ThumbKey`] and that revision: a placeholder until the store has it, a failure mark when loading failed.
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
    /// The store that holds the thumbnails and previews, which the app fills.
    #[prop]
    thumbnails: Option<TextureStore<ThumbKey>>,
}

impl<'a> StartScreen<'a> {
    /// A start screen of `recent` files, shown in `view` and ordered by `sort`; names sort in the env's language.
    pub fn new(view: Bind<'a, RecentView>, sort: Bind<'a, RecentSort>, recent: &'a [RecentFile]) -> Self {
        StartScreen { view, sort, recent, pinned: None, templates: &[], max_recent: RECENT_SHOWN, now: None, thumbnails: None }
    }
}

/// What each list of the page keeps between passes: the item that has the lead, which the commands act on.
#[derive(Clone, Debug, Default)]
struct Leads {
    recent: Option<PathBuf>,
    pinned: Option<PathBuf>,
}

/// What a card or row shows as its picture.
enum Picture {
    /// The item has no thumbnail.
    None,
    /// The store has no picture yet.
    Pending,
    /// A picture to draw.
    Ready(CachedTexture),
    /// Loading the picture failed.
    Failed,
}

/// What every section of one pass reads.
struct Page<'p> {
    ctx: &'p egui::Context,
    env: &'p Env,
    words: &'p ShellWords,
    thumbnails: Option<&'p TextureStore<ThumbKey>>,
    now: Option<i64>,
    actions: &'p RefCell<Vec<StartAction>>,
}

impl Page<'_> {
    fn push(&self, action: StartAction) {
        self.actions.borrow_mut().push(action);
    }

    /// How long before `now` `then` was, in words; `None` when either time is unknown.
    fn age(&self, then: Option<i64>) -> Option<String> {
        let (now, then) = (self.now?, then?);
        Some(self.env.relative_age(now.saturating_sub(then).max(0).unsigned_abs(), AgeStyle::Long))
    }

    fn picture(&self, key: ThumbKey, revision: Option<Revision>) -> Picture {
        let (Some(store), Some(revision)) = (self.thumbnails, revision) else { return Picture::None };
        let state = store.state(self.ctx, &key, revision);
        if let Some(texture) = state.shown() {
            return Picture::Ready(texture);
        }
        match state {
            TextureState::Failed { .. } => Picture::Failed,
            TextureState::Empty => Picture::None,
            _ => Picture::Pending,
        }
    }

    fn tile<'t>(&self, name: &str, detail: Option<String>, picture: Picture) -> Tile<'t> {
        let mut tile = Tile::new(Label::from(name.to_owned()));
        if let Some(detail) = detail {
            tile = tile.detail(detail);
        }
        match picture {
            Picture::Ready(texture) => tile.thumb(ThumbAspect::new(PICTURE_WIDE, PICTURE_HIGH), move |cx, rect| paint_picture(cx, rect, texture)),
            Picture::Failed => tile.status(TileStatus::Failed),
            Picture::Pending => tile,
            Picture::None => tile.icon(IconId::new("file")),
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

/// The picture inside `rect`, as large as fits with its own proportions.
fn paint_picture(cx: &ContentPainterCx, rect: Rect, texture: CachedTexture) {
    let size = texture.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    let fitted = Rect::from_center_size(rect.center(), size * scale);
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    cx.picture(texture.id, fitted, uv, cx.style_cx().tokens.shape.radius.control);
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
        let StartScreen { view, sort, recent, pinned, templates, max_recent, now, thumbnails } = self;
        let env = Env::of(ui);
        let collator = env.collator();
        let words = env.words::<ShellWords>();
        let ctx = ui.ctx().clone();
        let actions = RefCell::new(Vec::new());
        let page = Page { ctx: &ctx, env: &env, words: &words, thumbnails: thumbnails.as_ref(), now, actions: &actions };
        let view = RefCell::new(view);
        let sort = RefCell::new(sort);

        let shown = Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| {
            Stack::new().gap(Gap::Section).pad(Pad::Regular).fill_width().show(ui, |ui, cx| {
                let current_view = view.borrow_mut().reborrow().read(cx).and_then(|held| held.same().copied()).unwrap_or_default();
                let current_sort = sort.borrow_mut().reborrow().read(cx).and_then(|held| held.same().copied()).unwrap_or_default();
                let mut files: Vec<RecentFile> = recent.iter().take(max_recent).cloned().collect();
                sort_recent(&mut files, current_sort, &collator);

                Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                    if clicked(&Button::new(words.new.clone()).id_salt("new").variant(Variant::PRIMARY).show(ui)) {
                        page.push(StartAction::New);
                    }
                    if clicked(&Button::new(words.open.clone()).id_salt("open").show(ui)) {
                        page.push(StartAction::Open);
                    }
                });

                recent_section(&page, ui, &view, &sort, current_view, &files);
                if let Some(pinned) = pinned {
                    pinned_section(&page, ui, pinned);
                }
                if !templates.is_empty() {
                    templates_section(&page, ui, templates);
                }
            });
        });
        StartOutput { response: shown.output, actions: actions.into_inner() }
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
                let _ = Notice::new(Tone::Neutral, words.no_recent.clone()).layout(NoticeLayout::Centred).show(ui);
                return;
            }
            let file_of = |path: &PathBuf| files.iter().find(|file| &file.path == path);
            let source = keyed(files, |file: &RecentFile| file.path.clone()).with_text(|file| Cow::Borrowed(file.name.as_str()));
            let (response, (opened, removed)) = if current == RecentView::Grid {
                let out = page.grid::<PathBuf>(words.recent.clone(), true).show_items(ui, &source, &mut |path, _| {
                    let Some(file) = file_of(path) else { return Tile::new(Label::Empty) };
                    page.tile(&file.name, page.age(file.opened), page.picture(ThumbKey::Recent(file.path.clone()), file.thumbnail))
                        .corner_action(IconId::new("close"), StartRecentCommands::Remove)
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
                        let row = ListRow::new(Label::from(file.name.clone())).detail(folder_of(&file.path));
                        let row = match page.age(file.opened) {
                            Some(age) => row.trailing(age),
                            None => row,
                        };
                        row.icon(IconSpec::svg(IconId::new("file")))
                    });
                (out.response, read_events(out.events, &mut lead))
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
            page.tile(&template.name, detail, page.picture(ThumbKey::Template(template.id.clone()), template.thumbnail))
        });
        let mut lead = None;
        for id in read_events(out.response.events, &mut lead).0 {
            page.push(StartAction::UseTemplate(id));
        }
    });
}
