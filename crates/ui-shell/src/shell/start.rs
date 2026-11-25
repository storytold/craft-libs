//! `StartScreen`: the page an app shows with no document open: New and Open, the recent files as a grid or a list,
//! pinned folders and templates.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::recent::{RecentFile, RecentSort, RecentView, Template, ThumbKey, sort_recent};
use super::recent_list::{RecentList, recent_body};
use super::words::ShellWords;
use crate::buttons::button::ButtonStyleKey;
use crate::buttons::{Button, ControlPlace, Link};
use crate::choice::Segmented;
use crate::contract::{
    ActivateOn, Bind, BodyCx, CollectionEvent, Entry, Gap, NoValue, Outcome, Pad, PadEdges, Pattern, Props, Remember as _, RowAlign, Scroll,
    SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, clicked,
};
use crate::display::{Divider, Icon, Shade, Text};
use crate::layout_components::{Row, Stack};
use crate::list::{List, ListRow, RowEvent, ThumbAspect, TrailingFace, TrailingPart};
use crate::menu::{Menu, MenuItem};
use crate::notices::{Notice, NoticeLayout};
use crate::pickers::Select;
use crate::primitives::{
    CheckerArea, ContentPainterCx, HAlign, IconSpec, Priority, RectSide, Rule, RuleKind, ThumbPaint, ThumbRequest, ThumbSource, Thumbnails,
    paint_surround,
};
use crate::runtime::commands::{self, CommandRef, FireSource};
use crate::runtime::commands::{CommandSet as _, Commands as _};
use crate::runtime::env::{AgeStyle, Env, Label, Overrides, with_env};
use crate::sections::{HeaderStrength, Section};
use crate::style::{Corners, DataColor, StyleCx, SurfaceKind, SurroundLook, TypeRole, rgba_color};
use crate::tile_grid::{CaptionPlace, CardText, GridZoom, Tile, TileGrid};
use craft_behavior::{DisplayOrder, IconId, ItemSource, Proposal, SelectionChange, keyed};
use craft_theme::variant::{CustomVariant, Emphasis, RadiusStep, Tone, VariantLook};
use craft_theme::{PageSizes, Rgba, Tokens, Variant};
use egui::{CornerRadius, Rect, Response, Ui, UiBuilder, Vec2, pos2, vec2};
use std::borrow::Cow;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// The cards of the home screen start at twice the card thumbnail wide and grow to fill their row: one or two to a
/// row in a panel.
const CARD_ZOOM: f32 = 2.0;
/// The width and height parts of a card's picture.
pub(super) const PICTURE_WIDE: u16 = 16;
pub(super) const PICTURE_HIGH: u16 = 9;
/// The aspect of the recent file pictures on the desktop Home page.
pub(super) const HOME_RECENT_PICTURE_ASPECT: f32 = 3.0 / 2.0;

/// The number of recent files shown until `max_recent` says otherwise.
const RECENT_SHOWN: usize = 12;

const HOME_BREAKPOINT: f32 = 640.0;
const HOME_RAIL_WIDTH: f32 = 170.0;
const HOME_COMPACT_WIDTH: f32 = 520.0;
const HOME_DROP_WIDTH: f32 = 540.0;
const HOME_DROP_HEIGHT: f32 = 200.0;
const HOME_DROP_ICON: f32 = 34.0;
// design: allow[literal-size/vec2](the desktop Home page's outer margins are larger than any spacing token)
const HOME_DESKTOP_INSET: Vec2 = vec2(36.0, 54.0);
const HOME_DROP_LOOK: CustomVariant = CustomVariant::new::<ButtonStyleKey>("craft_ui.home-drop", home_drop_look);
/// New and Open on the desktop Home page: the control radius, not the pill of dialog buttons.
const HOME_NEW_LOOK: CustomVariant = CustomVariant::new::<ButtonStyleKey>("craft_ui.home-new", home_new_look);
const HOME_OPEN_LOOK: CustomVariant = CustomVariant::new::<ButtonStyleKey>("craft_ui.home-open", home_open_look);
/// A row of the Home rail (Home, Learn, the community links): no edge or fill at rest, muted ink that turns normal
/// under the pointer. The selected row keeps normal ink and an accent bar at its leading edge.
const HOME_ROW_LOOK: CustomVariant = CustomVariant::new::<ButtonStyleKey>("craft_ui.home-row", home_row_look);
const HOME_ROW_SELECTED_LOOK: CustomVariant = CustomVariant::new::<ButtonStyleKey>("craft_ui.home-row-selected", home_row_selected_look);

/// The revealed part of a recent row that takes the file off the list.
pub(super) const REMOVE: TrailingPart = TrailingPart::new(0);

/// The part of a listed pinned folder's heading that unpins the folder.
const UNPIN: TrailingPart = TrailingPart::new(0);

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
    /// The "more templates" link: the app shows its template gallery.
    MoreTemplates,
    /// A file listed under a pinned folder was opened: the app opens that path.
    OpenPinnedFile(PathBuf),
    /// A pinned folder was folded: the app keeps it folded.
    FoldPinned(PathBuf),
    /// A pinned folder's More row was chosen: the app lists all of its files.
    ExpandPinned(PathBuf),
}

/// How the start screen is laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum StartLayout {
    /// The recent files and templates as a gallery.
    #[default]
    Gallery,
    /// A greeting page: a title, the templates, then the recent files as rows. It shows neither Pinned nor the drop card.
    Greeting,
    /// One centred column, at most `size.page.column` wide and a page margin from every edge: the title and subtitle,
    /// New and Open, the recent files as rows when there are any, the app's section, the templates as cards that keep
    /// their size and wrap, then the links, the call to action as a pill. It shows neither Pinned, the drop card nor
    /// the recent files' view switch and sort.
    Column,
}

/// The column's template cards keep the card thumbnail's size, so as many as fit stand in a row and the rest wrap.
const COLUMN_CARD_ZOOM: f32 = 1.0;

/// The call to action among a Column page's links: the primary variant at pill corners, with its icon.
const CALL_TO_ACTION_LOOK: CustomVariant = CustomVariant::new::<ButtonStyleKey>("craft_ui.start-call-to-action", call_to_action_look);

fn call_to_action_look(tokens: &Tokens) -> VariantLook {
    Variant::PRIMARY.resolve(tokens).radius(RadiusStep::Pill)
}

/// Where a host section sits on the start screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StartPlace {
    /// Before the Recent section.
    BeforeRecent,
    /// After the last built-in section.
    AfterSections,
}

/// The most files a pinned folder lists before its More row.
pub const PINNED_FILES_SHOWN: usize = 8;

/// A pinned folder as the start screen lists it, with the files the app found in it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PinnedFolder {
    /// The folder's path; also the identity of the entry.
    pub path: PathBuf,
    /// The name shown.
    pub name: String,
    /// The folder's files, once the app has listed them.
    pub listing: Option<FolderListing>,
    /// The folder shows no files.
    pub folded: bool,
    /// The folder lists all of its files, not the first [`PINNED_FILES_SHOWN`].
    pub expanded: bool,
}

impl PinnedFolder {
    /// A pinned folder at `path` shown as `name`, not yet listed, unfolded and not expanded.
    pub fn new(path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        PinnedFolder { path: path.into(), name: name.into(), listing: None, folded: false, expanded: false }
    }

    /// This folder with its files, `listing`, as the app found them.
    #[must_use]
    pub fn listing(mut self, listing: FolderListing) -> Self {
        self.listing = Some(listing);
        self
    }

    /// This folder shown folded (no files) or not.
    #[must_use]
    pub fn folded(mut self, folded: bool) -> Self {
        self.folded = folded;
        self
    }

    /// This folder listing all of its files, or only the first [`PINNED_FILES_SHOWN`].
    #[must_use]
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }
}

/// What the app knows of a pinned folder's files.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FolderListing {
    /// The app is still listing the folder.
    Pending,
    /// The folder cannot be listed.
    Unavailable,
    /// The folder's files, and how many it holds in all.
    Listed {
        /// The files to show.
        files: Vec<PinnedFile>,
        /// The number of files in the folder.
        total: usize,
    },
}

/// A file listed under a pinned folder.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PinnedFile {
    /// The file's full path.
    pub path: PathBuf,
    /// The name shown.
    pub name: String,
    /// A second line under the name.
    pub note: Option<String>,
}

impl PinnedFile {
    /// A file at `path` shown as `name`, with no second line.
    pub fn new(path: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        PinnedFile { path: path.into(), name: name.into(), note: None }
    }

    /// This file with a second line, `note`, under its name.
    #[must_use]
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
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

/// An app's own section of the start screen: its title and its body.
type AppSection<'a> = (Label, RefCell<&'a mut dyn FnMut(&mut Ui)>);

/// The start screen, shown in a shell's body when no document is open.
///
/// - **Layout.** New and Open (the first Tab stops; with `offers_new(false)` New is left out and Open leads), then the
///   host's [`StartPlace::BeforeRecent`] sections, Recent with the view switch and the sort (left out with
///   `max_recent(0)`), Pinned Folders when the app gives the list (even an empty one), the app's own
///   [`app_section`](StartScreen::app_section) when it gives one, Templates when it gives any, then the host's
///   [`StartPlace::AfterSections`] sections, in call order. The page scrolls as a whole, and each grid and list takes
///   the height of its rows.
/// - **Greeting layout.** With `layout(StartLayout::Greeting)` the page shows the `title`, the templates without a
///   section heading, the `more_templates` link, then the recent files as rows in the order the app gave them, then the
///   app's own section: no view switch, no sort, no Pinned Folders and no drop card.
/// - **Recent files.** A grid of cards or a list, by the `view` binding, in the order of the `sort` binding. A click or
///   Enter opens a file; arrows move; the context menu (secondary click, the menu key or Shift+F10) and the
///   [`StartRecentCommands`] remove it from the list, clear the list or show it in its folder. A file's
///   [`note`](RecentFile::note) shows in place of its age.
/// - **Pinned folders.** Each folder is a heading row, with a disclosure when the app lists it (still listing, empty
///   or unavailable too). A listed folder holds its files, the first
///   [`PINNED_FILES_SHOWN`] until the app expands it, then a More row; the heading's trailing note says when the app is
///   still listing, cannot list, found no files or kept only the newest. The app owns fold and expansion: the
///   disclosure and Left/Right report [`StartAction::FoldPinned`], the More row [`StartAction::ExpandPinned`], and the
///   app passes `folded` and `expanded` back.
/// - **Thumbnails.** With `thumbnails(store, source)`, each card, row and template asks the store for its
///   [`ThumbKey`] at the size it paints: a placeholder until the picture is ready, a failure mark when loading failed,
///   the file icon when the source has no picture for the key. Without it cards show the file icon.
/// - **Links.** `links` adds a rail beside the page (New and Open, then the links) in a room that holds the rail and
///   the sections' least width, host sections included, else the links follow the sections. A double click on the
///   page's own room opens a file, as Open does.
/// - **No recent files.** With a `drop_hint` the Recent section is a card that opens a file on a click, Enter or Space.
/// - **State.** The app owns the list, the view and the sort. The screen writes a changed view or sort through its
///   binding and reports everything else as [`StartAction`]s.
///
/// ```ignore
/// let folders = vec![PinnedFolder::new("/home/ana/Work", "Work").listing(listing)];
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
    pinned: Option<&'a [PinnedFolder]>,
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
    /// One app section, drawn after Recent and Pinned Folders and before Templates.
    section: Option<AppSection<'a>>,
    /// How the page is laid out.
    #[prop]
    layout: StartLayout,
    /// The page's heading.
    #[prop(into)]
    title: Option<Label>,
    /// The line under the title on a [`StartLayout::Column`] page.
    #[prop(into)]
    subtitle: Option<Label>,
    /// The heading of the templates section, in place of the built-in word.
    #[prop(into)]
    templates_title: Option<Label>,
    /// The heading over the links on a [`StartLayout::Column`] page.
    #[prop(into)]
    links_title: Option<Label>,
    /// The text of the link that fires [`StartAction::MoreTemplates`].
    #[prop(into)]
    more_templates: Option<Label>,
    /// The width and height parts of a card and row picture (16:9 unless the pictures are another shape, as a page 3:4).
    picture: (u16, u16),
    /// The zoom of a card's picture, a factor on the card thumbnail: the cards keep that size and do not grow to fill
    /// the row, so as many as fit stand side by side (default: twice the card thumbnail, grown to fill the row).
    #[prop(quantity = "zoom factor")]
    card_zoom: Option<f32>,
    /// The pictures are opaque pages on this paper, which is document data: no transparency grid shows, and a file
    /// without a picture shows its icon on the card, not on a sunken tile (default: photos).
    #[prop]
    page_paper: Option<DataColor>,
    /// The desktop Home layout set by [`Self::home_layout`].
    home: Option<HomeLayout<'a>>,
    /// The New… button shows (true by default).
    #[prop]
    offers_new: bool,
    /// The New button's text, in place of the built-in word.
    #[prop(into)]
    new_label: Option<Label>,
    /// The Open button's text, in place of the built-in word.
    #[prop(into)]
    open_label: Option<Label>,
    /// The Open… button shows (true by default); an app that opens files from its title bar leaves it out. A double
    /// click on the page's empty room still asks for Open.
    #[prop]
    offers_open: bool,
    /// The heading of the pinned folders section, in place of the built-in word.
    #[prop(into)]
    pinned_title: Option<Label>,
    /// PdfCraft's section look: the Recent, Pinned Folders and Templates headings in the heading strength and quiet,
    /// a ghost Pin Folder button, and a Clear button among Recent's controls that reports
    /// [`StartAction::ClearRecent`]. By default the sections keep their plain titles and buttons and Recent has no
    /// Clear button.
    #[prop(flag)]
    heading_sections: bool,
    /// The pinned folders stand above the recent files, as production's Folders section does. By default they follow
    /// the recent files.
    #[prop(flag)]
    pinned_first: bool,
    /// The host's sections, each at its place, in call order.
    sections: Vec<(StartPlace, HostSection<'a>)>,
}

struct HomeLayout<'a> {
    title: Label,
    footer_links: &'a [StartLink],
}

/// A host section's body. A cell, because the page's body runs for the measuring pass and the shown pass and calls
/// every section in both.
type HostSection<'a> = RefCell<Box<dyn FnMut(&mut Ui, &mut BodyCx<'_>) + 'a>>;

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
    selected: bool,
    primary: bool,
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
        StartLink { label: label.into(), icon, target: StartLinkTarget::Url(url.into()), selected: false, primary: false }
    }

    /// A link that fires `command`.
    pub fn command(label: impl Into<Label>, icon: Option<IconId>, command: impl Into<CommandRef>) -> Self {
        StartLink { label: label.into(), icon, target: StartLinkTarget::Command(command.into()), selected: false, primary: false }
    }

    /// Highlights this command link on a Home rail.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Makes this link the page's call to action: on a [`StartLayout::Column`] page it stands as a pill in the
    /// primary variant with its icon, where the other links show as text alone. The other layouts show it as a link.
    #[must_use]
    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
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
            section: None,
            home: None,
            layout: StartLayout::default(),
            title: None,
            subtitle: None,
            templates_title: None,
            links_title: None,
            more_templates: None,
            picture: (PICTURE_WIDE, PICTURE_HIGH),
            card_zoom: None,
            page_paper: None,
            offers_new: true,
            new_label: None,
            open_label: None,
            offers_open: true,
            pinned_title: None,
            heading_sections: false,
            pinned_first: false,
            sections: Vec::new(),
        }
    }

    /// The shape of the pictures of cards and rows, wide to high (default 16:9). A zero part keeps the default.
    pub fn picture_aspect(mut self, wide: u16, high: u16) -> Self {
        if wide > 0 && high > 0 {
            self.picture = (wide, high);
        }
        self
    }

    /// Uses a Home layout with a title above the page and a separate group of links at the bottom of its rail.
    /// The navigation links come from [`Self::links`]. Narrow rooms put both groups below the content.
    pub fn home_layout(mut self, title: impl Into<Label>, footer_links: &'a [StartLink]) -> Self {
        self.home = Some(HomeLayout { title: title.into(), footer_links });
        self
    }

    /// One app section titled `title`, drawn as a fixed section after Recent and Pinned Folders and before Templates.
    /// `body` may run more than once in a pass: the rail measures it.
    pub fn app_section(mut self, title: impl Into<Label>, body: &'a mut dyn FnMut(&mut Ui)) -> Self {
        self.section = Some((title.into(), RefCell::new(body)));
        self
    }

    /// Shows `body` as a host section at `place` on the start screen, after the sections given there before it. Its
    /// least width counts in whether the rail fits beside the page.
    pub fn section(mut self, place: StartPlace, body: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        self.sections.push((place, RefCell::new(Box::new(body))));
        self
    }

    /// Cards, rows and templates ask `thumbnails.state(..)` for what they show; `source` gives each key's revision.
    pub fn thumbnails(mut self, thumbnails: &'a Thumbnails<ThumbKey>, source: &'a dyn ThumbSource<ThumbKey>) -> Self {
        self.thumbnails = Some((thumbnails, source));
        self
    }
}

/// What each list of the page keeps between passes: the item that has the lead, which the commands act on.
#[derive(Clone, Debug, Default)]
pub(super) struct Leads {
    pub(super) recent: Option<PathBuf>,
    pub(super) pinned: Option<PinnedKey>,
}

/// The thumbnails and the source of each key's revision.
pub(super) type ThumbHost<'p> = (&'p Thumbnails<ThumbKey>, &'p dyn ThumbSource<ThumbKey>);

/// What every section of one pass reads.
pub(super) struct Page<'p> {
    pub(super) env: &'p Env,
    pub(super) words: &'p ShellWords,
    pub(super) thumbnails: Option<ThumbHost<'p>>,
    /// The width and height parts of a picture.
    pub(super) picture: (u16, u16),
    /// The zoom of a card's picture, when the app sets one.
    pub(super) card_zoom: Option<f32>,
    /// The paper of page pictures, when the app's pictures are pages.
    pub(super) paper: Option<DataColor>,
    pub(super) drop_hint: Option<&'p Label>,
    pub(super) now: Option<i64>,
    pub(super) actions: &'p RefCell<Vec<StartAction>>,
    /// New… shows beside Open.
    pub(super) offers_new: bool,
    /// Open… shows.
    pub(super) offers_open: bool,
    /// The heading of the pinned folders section, in place of the built-in word.
    pub(super) pinned_title: Option<&'p Label>,
    /// The heading of the templates section, in place of the built-in word.
    pub(super) templates_title: Option<&'p Label>,
    /// The New and Open buttons' texts, in place of the built-in words.
    pub(super) new_label: Option<&'p Label>,
    pub(super) open_label: Option<&'p Label>,
    /// PdfCraft's section look (`StartScreen::heading_sections`).
    pub(super) heading_sections: bool,
}

impl<'p> Page<'p> {
    pub(super) fn push(&self, action: StartAction) {
        self.actions.borrow_mut().push(action);
    }

    /// The New button's text: the app's, else the built-in word.
    fn new_label(&self) -> Label {
        self.new_label.cloned().unwrap_or_else(|| self.words.new.clone())
    }

    /// The Open button's text: the app's, else the built-in word.
    fn open_label(&self) -> Label {
        self.open_label.cloned().unwrap_or_else(|| self.words.open.clone())
    }

    /// A section of the page titled `title`: in the heading strength and quiet under `heading_sections`.
    pub(super) fn section(&self, title: Label) -> Section<'static> {
        let section = Section::fixed(title);
        if self.heading_sections { section.strength(HeaderStrength::Heading).quiet_header() } else { section }
    }

    /// The line a recent file shows under its card's name and at its row's end: the app's note, else the age.
    pub(super) fn recent_detail(&self, file: &RecentFile) -> Option<String> {
        file.note.clone().or_else(|| self.age(file.opened))
    }

    /// How long before `now` `then` was, in words; `None` when either time is unknown.
    pub(super) fn age(&self, then: Option<i64>) -> Option<String> {
        let (now, then) = (self.now?, then?);
        Some(self.env.relative_age(now.saturating_sub(then).max(0).unsigned_abs(), AgeStyle::Long))
    }

    /// A card named `name` whose picture is the thumbnail of `key`, asked for at the cell it paints in; the file icon
    /// without `thumbnails`.
    pub(super) fn tile<'t>(&self, name: &str, detail: Option<String>, key: ThumbKey) -> Tile<'t>
    where
        'p: 't,
    {
        let mut tile = Tile::new(Label::from(name.to_owned()));
        if let Some(detail) = detail {
            tile = tile.detail(detail);
        }
        match self.thumbnails {
            Some((thumbnails, source)) => tile.thumb(ThumbAspect::new(self.picture.0, self.picture.1), {
                let paper = self.paper;
                move |cx, rect| paint_thumb(cx, rect, thumbnails, source, &key, paper)
            }),
            None => tile.icon(IconId::new("file")),
        }
    }

    pub(super) fn grid<K: Clone + Eq + std::hash::Hash + Send + Sync + 'static>(&self, label: Label, deletable: bool) -> TileGrid<'static, K> {
        let grid = TileGrid::<K>::new(label)
            .select(SelectMode::Single)
            .deletable(deletable)
            .activate(ActivateOn::Click)
            .caption(CaptionPlace::Below)
            .detail_line()
            .card()
            .aspect(f32::from(self.picture.0) / f32::from(self.picture.1));
        let grid = match self.card_zoom {
            Some(zoom) => grid.zoom(GridZoom::Fixed(f64::from(zoom))),
            None => grid.zoom(GridZoom::Fixed(f64::from(CARD_ZOOM))).fill_row(),
        };
        grid.sizing(Sizing::FillWidth).natural_height(true)
    }
}

/// The thumbnail of `key` in `rect`, asked for at the size it paints. The store paints the placeholder while the
/// picture is not ready and the failure mark when it failed; a key the source has no picture for (`Blank`) gets the
/// file icon on the sunken tile.
pub(super) fn paint_thumb(
    cx: &ContentPainterCx,
    rect: Rect,
    thumbnails: &Thumbnails<ThumbKey>,
    source: &dyn ThumbSource<ThumbKey>,
    key: &ThumbKey,
    paper: Option<DataColor>,
) {
    let request = ThumbRequest::for_cell(rect.width().max(rect.height()), cx.pixels_per_point());
    let state = thumbnails.state(cx.painter.ctx(), source, key, &request, Priority::VISIBLE);
    let paint = match paper {
        Some(paper) => ThumbPaint::default().checker(false).paper(paper),
        None => ThumbPaint::default(),
    };
    if thumbnails.paint(cx, rect, &state, &paint) {
        return;
    }
    if paper.is_none() {
        cx.surface(rect, SurfaceKind::Sunken, Corners::None);
    }
    let side = cx.style_cx().tokens.size.icon.lg;
    cx.icon(Rect::from_center_size(rect.center(), Vec2::splat(side)), &IconSpec::svg(IconId::new("file")));
}

/// The folder a path is in, for the line under a file's name.
pub(super) fn folder_of(path: &Path) -> String {
    path.parent().map(|folder| folder.display().to_string()).unwrap_or_default()
}

/// What a collection's events mean for a section: the keys to open and the keys to delete, and the lead after them.
pub(super) fn read_events<K: Clone, E>(events: Vec<CollectionEvent<K, E>>, lead: &mut Option<K>) -> (Vec<K>, Vec<K>) {
    let (mut opened, mut deleted) = (Vec::new(), Vec::new());
    for event in events {
        match event {
            CollectionEvent::Activate(key) => {
                *lead = Some(key.clone());
                opened.push(key);
            }
            CollectionEvent::LeadMoved { key, .. } => *lead = Some(key),
            CollectionEvent::Delete(keys) => deleted.extend(keys),
            CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => {
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
        let StartScreen {
            view,
            sort,
            recent,
            pinned,
            templates,
            max_recent,
            now,
            thumbnails,
            links,
            drop_hint,
            section,
            layout,
            title,
            subtitle,
            templates_title,
            links_title,
            more_templates,
            picture,
            card_zoom,
            page_paper,
            home,
            offers_new,
            new_label,
            open_label,
            offers_open,
            pinned_title,
            heading_sections,
            pinned_first,
            sections,
        } = self;
        let env = Env::of(ui);
        let collator = env.collator();
        let words = env.words::<ShellWords>();
        let actions = RefCell::new(Vec::new());
        let page = Page {
            env: &env,
            words: &words,
            thumbnails,
            picture,
            card_zoom: if layout == StartLayout::Column { card_zoom.or(Some(COLUMN_CARD_ZOOM)) } else { card_zoom },
            paper: page_paper,
            drop_hint: drop_hint.as_ref(),
            now,
            offers_new,
            offers_open,
            pinned_title: pinned_title.as_ref(),
            templates_title: templates_title.as_ref(),
            new_label: new_label.as_ref(),
            open_label: open_label.as_ref(),
            heading_sections,
            actions: &actions,
        };
        let view = RefCell::new(view);
        let sort = RefCell::new(sort);

        let room = ui.available_rect_before_wrap();
        // The page's surround is square; a Column page stands on the page surface.
        if layout == StartLayout::Column {
            ui.painter().rect_filled(room, CornerRadius::ZERO, rgba_color(env.tokens.color.surface.page));
        } else {
            paint_surround(ui.painter(), CheckerArea::Rect(room, CornerRadius::ZERO), &SurroundLook::of(&env.tokens).shown(env.canvas_pattern));
        }

        let rail_stack = || Stack::new().gap(Gap::Tight).pad(Pad::Regular).label(words.start_links.clone());
        let rail_body = |ui: &mut Ui| {
            new_and_open(&page, ui, Sizing::Content);
            if offers_new || offers_open {
                let _ = Divider::horizontal().show(ui);
            }
            link_items(ui, links, false);
        };
        let page_stack = || Stack::new().gap(Gap::Section).pad(Pad::Regular).fill_width();
        // The page stack fills its room, so its own measure is the room. The least width is that of the same body in a
        // content-sized column, where a fill-width child only follows the column.
        let page_content = || Stack::new().gap(Gap::Section).pad(Pad::Regular);
        // The host's sections at `place`, each one child of the page, so they stand one section gap apart.
        let host_sections = |ui: &mut Ui, place: StartPlace| {
            for (_, body) in sections.iter().filter(|(held, _)| *held == place) {
                let _ = Stack::new().fill_width().show(ui, |ui, cx| {
                    let mut held = body.borrow_mut();
                    let run: &mut dyn FnMut(&mut Ui, &mut BodyCx<'_>) = &mut **held;
                    run(ui, cx);
                });
            }
        };
        // `beside` is whether the rail stands beside the page; without it New and Open lead the page and the links follow
        // the sections.
        // The app's own section, under the heading `heading` makes of its title.
        let app_section = |ui: &mut Ui, heading: fn(Label) -> Section<'static>| {
            if let Some((title, body)) = &section {
                let _ = heading(title.clone()).show(ui, |ui, _| {
                    let mut body = body.borrow_mut();
                    (*body)(ui);
                });
            }
        };
        let page_body = |ui: &mut Ui, cx: &mut BodyCx<'_>, beside: bool, drop_area: Option<(f32, bool)>| {
            if !beside && (offers_new || offers_open) {
                let _ = Row::new().gap(Gap::Tight).show(ui, |ui, _| new_and_open(&page, ui, Sizing::Content));
            }
            host_sections(ui, StartPlace::BeforeRecent);
            if layout == StartLayout::Greeting {
                greeting_body(&page, ui, title.as_ref(), more_templates.as_ref(), templates, recent, max_recent);
                app_section(ui, page_section);
            } else {
                // With `pinned_first` the pinned folders stand above the recent files, as production's Folders section
                // does; else they follow them.
                if let Some(pinned) = pinned.filter(|_| pinned_first) {
                    pinned_section(&page, ui, pinned);
                }
                if max_recent > 0 {
                    let current_view = view.borrow_mut().reborrow().read(cx).and_then(|held| held.same().copied()).unwrap_or_default();
                    let current_sort = sort.borrow_mut().reborrow().read(cx).and_then(|held| held.same().copied()).unwrap_or_default();
                    let mut files: Vec<RecentFile> = recent.iter().take(max_recent).cloned().collect();
                    sort_recent(&mut files, current_sort, &collator);
                    if let (true, Some(hint), Some((center, wide))) = (files.is_empty(), page.drop_hint, drop_area) {
                        home_drop(&page, ui, hint, center, wide);
                    } else {
                        let desktop_home = drop_area.is_some_and(|(_, wide)| wide);
                        recent_section(&page, ui, &view, &sort, current_view, &files, desktop_home);
                    }
                }
                if let Some(pinned) = pinned.filter(|_| !pinned_first) {
                    pinned_section(&page, ui, pinned);
                }
                app_section(ui, page_section);
                if !templates.is_empty() {
                    templates_section(&page, ui, templates);
                }
            }
            host_sections(ui, StartPlace::AfterSections);
            if !beside && !links.is_empty() {
                let _ = Stack::new().gap(Gap::Tight).label(words.start_links.clone()).show(ui, |ui, _| link_items(ui, links, false));
            }
        };

        if layout == StartLayout::Column {
            let PageSizes { column, margin, .. } = env.tokens.size.page;
            // The page senses its own surface for the empty-room double click, as the other layouts do. The page margin
            // stands beside the column; `size.page.margin` above it and below it.
            let page_stack = Stack::new().fill().pad(Pad::Page).pad_edges(PadEdges::Inline).scroll(Scroll::vertical()).senses_surface();
            let shown = page_stack.show(ui, |ui, cx| {
                if cx.surface_double_pressed() {
                    page.push(StartAction::Open);
                }
                let room = ui.available_rect_before_wrap();
                let width = room.width().min(column);
                let at = Rect::from_min_size(pos2(room.center().x - width / 2.0, room.top() + margin), vec2(width, room.height()));
                ui.scope_builder(UiBuilder::new().max_rect(at), |ui| {
                    let _ = Stack::new().id_salt("start-column").gap(Gap::PageSection).fill_width().show(ui, |ui, _| {
                        column_heading(&page, ui, title.as_ref(), subtitle.as_ref());
                        host_sections(ui, StartPlace::BeforeRecent);
                        // The recent files as rows in the app's order, only when there are any.
                        let files: Vec<RecentFile> = recent.iter().take(max_recent).cloned().collect();
                        if !files.is_empty() {
                            let _ = column_section(words.recent.clone())
                                .command_scope(StartRecentCommands::SCOPE)
                                .show(ui, |ui, cx| recent_body(&page, ui, cx, &files, RecentView::List, &words.no_recent, false));
                        }
                        app_section(ui, column_section);
                        if !templates.is_empty() {
                            let heading = page.templates_title.cloned().unwrap_or_else(|| words.templates.clone());
                            let _ = column_section(heading).show(ui, |ui, _| templates_grid(&page, ui, templates));
                        }
                        host_sections(ui, StartPlace::AfterSections);
                        column_links(&page, ui, links_title.as_ref(), links);
                    });
                });
                ui.add_space(env.tokens.size.page.margin);
            });
            return StartOutput { response: shown.output, actions: actions.into_inner() };
        }

        if let Some(home) = home {
            let response = home.show(ui, &page, links, |ui, cx, center, wide| page_body(ui, cx, true, Some((center, wide))));
            return StartOutput { response, actions: actions.into_inner() };
        }

        // The rail stands beside the sections when the room holds the rail, the gap and the sections' least width; a
        // narrower page keeps New and Open in its row and lists the links after the sections.
        let rail = !links.is_empty() && {
            let rail_width = rail_stack().measure_in(ui, Proposal::Ideal, |ui, _| rail_body(ui)).x;
            let sections_least = page_content().measure_in(ui, Proposal::Min, |ui, cx| page_body(ui, cx, true, None)).x;
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
                    let _ = page_stack().show(ui, |ui, cx| page_body(ui, cx, true, None));
                });
            } else {
                let _ = page_stack().show(ui, |ui, cx| page_body(ui, cx, false, None));
            }
        });
        StartOutput { response: shown.output, actions: actions.into_inner() }
    }
}

impl HomeLayout<'_> {
    fn show(&self, ui: &mut Ui, page: &Page<'_>, links: &[StartLink], mut sections: impl FnMut(&mut Ui, &mut BodyCx<'_>, f32, bool)) -> Response {
        // Keep the opening gesture on the fixed surface, outside the two independent scrolling regions.
        Stack::new()
            .fill()
            .senses_surface()
            .show(ui, |ui, cx| {
                if cx.surface_double_pressed() {
                    page.push(StartAction::Open);
                }
                let room = ui.available_rect_before_wrap();
                let wide = room.width() >= HOME_BREAKPOINT;
                let content = if wide {
                    let (rail, content) = home_columns(room, page.env.direction.is_rtl());
                    self.rail(ui, page, links, rail);
                    content
                } else {
                    Rect::from_center_size(room.center(), vec2(room.width().min(HOME_COMPACT_WIDTH), room.height()))
                };
                let gap = if wide { Gap::CardRow } else { Gap::Section };
                let _ =
                    Stack::new().id_salt("home-page").fill().pad(Pad::Regular).gap(gap).scroll(Scroll::vertical()).show_at(ui, content, |ui, cx| {
                        let top = ui.available_rect_before_wrap().top();
                        if !wide {
                            let _ = Row::new().gap(Gap::Tight).fill_width().show(ui, |ui, _| home_new_and_open(page, ui));
                        }
                        let _ = Text::new(self.title.clone()).role(TypeRole::Heading).show(ui);
                        home_divider(ui);
                        sections(ui, cx, top + content.height() / 2.0, wide);
                        if !wide {
                            home_links(ui, page, "home-nav", links, Gap::Tight);
                            home_links(ui, page, "home-footer", self.footer_links, Gap::Tight);
                        }
                    });
            })
            .output
    }

    fn rail(&self, ui: &mut Ui, page: &Page<'_>, links: &[StartLink], rect: Rect) {
        let stack = || Stack::new().id_salt("home-rail").pad(Pad::Regular).gap(Gap::Section);
        let top = |ui: &mut Ui| {
            let _ = Stack::new().id_salt("home-actions").gap(Gap::Stack).fill_width().show(ui, |ui, _| {
                home_new_and_open(page, ui);
            });
            home_divider(ui);
            home_links(ui, page, "home-nav", links, Gap::Section);
        };
        let footer = |ui: &mut Ui| {
            if !self.footer_links.is_empty() {
                home_divider(ui);
                home_links(ui, page, "home-footer", self.footer_links, Gap::Section);
            }
        };
        // A short window scrolls the entire rail so the fixed footer cannot crowd the opening buttons out.
        let footer_fits = !self.footer_links.is_empty()
            && stack()
                .measure_in(ui, Proposal::Ideal, |ui, _| {
                    top(ui);
                    footer(ui);
                })
                .y
                <= rect.height();
        let rail = stack().fill().scroll(Scroll::vertical());
        if footer_fits {
            let _ = rail.footer(|ui, _| footer(ui)).show_at(ui, rect, |ui, _| top(ui));
        } else {
            let _ = rail.show_at(ui, rect, |ui, _| {
                top(ui);
                footer(ui);
            });
        }
    }
}

fn home_columns(room: Rect, rtl: bool) -> (Rect, Rect) {
    // Leave enough height for the two scroll regions on short windows.
    let inset_y = HOME_DESKTOP_INSET.y.min((room.height() - HOME_DROP_HEIGHT).max(0.0) / 2.0);
    let room = room.shrink2(vec2(HOME_DESKTOP_INSET.x, inset_y));
    let gap = HOME_DESKTOP_INSET.x;
    if rtl {
        let rail = Rect::from_min_max(pos2(room.right() - HOME_RAIL_WIDTH, room.top()), room.max);
        let content = Rect::from_min_max(room.min, pos2(rail.left() - gap, room.bottom()));
        (rail, content)
    } else {
        let rail = Rect::from_min_max(room.min, pos2(room.left() + HOME_RAIL_WIDTH, room.bottom()));
        let content = Rect::from_min_max(pos2(rail.right() + gap, room.top()), room.max);
        (rail, content)
    }
}

fn home_links(ui: &mut Ui, page: &Page<'_>, id: &'static str, links: &[StartLink], gap: Gap) {
    if !links.is_empty() {
        let _ =
            Stack::new().id_salt(id).gap(gap).label(page.words.start_links.clone()).fill_width().show(ui, |ui, cx| home_link_items(ui, cx, links));
    }
}

fn home_divider(ui: &mut Ui) {
    let env = Env::of(ui);
    let style = StyleCx::of_env(ui.ctx(), &env);
    let mut look = style.rule(RuleKind::Divider);
    look.color = style.color(home_edge(&env.tokens)).egui();
    let scale = ui.ctx().pixels_per_point();
    let room = ui.available_rect_before_wrap();
    let line = Rect::from_min_size(room.min, vec2(room.width(), look.width.points(scale)));
    let _ = Stack::new().show_at_taking_room(ui, line, |ui, _| ui.painter().extend(Rule::edge(&look, line, RectSide::Top, scale)));
}

/// Panel dividers can match the canvas surround exactly, so use existing muted ink on that surround.
fn home_edge(tokens: &Tokens) -> Rgba {
    let color = &tokens.color;
    if color.line.divider == color.surface.canvas { color.text.faint } else { color.line.divider }
}

fn home_new_look(tokens: &Tokens) -> VariantLook {
    Variant::PRIMARY.resolve(tokens).radius(RadiusStep::Control)
}

fn home_open_look(tokens: &Tokens) -> VariantLook {
    home_drop_look(tokens)
}

fn home_row_look(tokens: &Tokens) -> VariantLook {
    let mut look = Variant::GHOST.resolve(tokens).radius(RadiusStep::Control);
    look.text.rest = tokens.color.text.muted;
    look.muted.rest = tokens.color.text.muted;
    look
}

fn home_row_selected_look(tokens: &Tokens) -> VariantLook {
    Variant::GHOST.resolve(tokens).radius(RadiusStep::Control)
}

/// New and Open on the desktop Home page, filling the rail (or the narrow page's row).
fn home_new_and_open(page: &Page<'_>, ui: &mut Ui) {
    if page.offers_new {
        let new = Button::new(page.new_label()).id_salt("new").variant(Variant::Custom(HOME_NEW_LOOK)).sizing(Sizing::FillWidth);
        if clicked(&new.show(ui)) {
            page.push(StartAction::New);
        }
    }
    if page.offers_open {
        let open = Button::new(page.open_label()).id_salt("open").variant(Variant::Custom(HOME_OPEN_LOOK)).sizing(Sizing::FillWidth);
        if clicked(&open.show(ui)) {
            page.push(StartAction::Open);
        }
    }
}

/// The rows of a Home rail group: each link is a full-width row with its icon, a URL opening in the browser (the URL
/// is its tooltip) and a command firing as a pointer activation. A selected row has an accent bar at its leading edge.
fn home_link_items(ui: &mut Ui, cx: &mut BodyCx<'_>, links: &[StartLink]) {
    for (at, link) in links.iter().enumerate() {
        let look = if link.selected { HOME_ROW_SELECTED_LOOK } else { HOME_ROW_LOOK };
        let mut button =
            Button::new(link.label.clone()).id_salt(("start-link", at)).variant(Variant::Custom(look)).align(HAlign::Start).sizing(Sizing::FillWidth);
        if let Some(icon) = &link.icon {
            button = button.icon(icon.clone());
        }
        if let StartLinkTarget::Url(url) = &link.target {
            button = button.tooltip(url.clone());
        }
        let shown = button.show(ui);
        if link.selected {
            home_selected_bar(ui, shown.response().rect);
        }
        if clicked(&shown) {
            match &link.target {
                StartLinkTarget::Url(url) => cx.open_url(url, false),
                StartLinkTarget::Command(command) => {
                    let _ = commands::fire(ui.ctx(), command.clone(), FireSource::Pointer);
                }
            }
        }
    }
}

/// The accent bar at the leading edge of the selected Home row, as tall as its icon.
fn home_selected_bar(ui: &mut Ui, row: Rect) {
    let env = Env::of(ui);
    let style = StyleCx::of_env(ui.ctx(), &env);
    // The accent mark's look: `accent.solid`, `stroke.thick` wide.
    let look = style.rule(RuleKind::DropMark);
    let scale = ui.ctx().pixels_per_point();
    let bar = Rect::from_center_size(row.center(), vec2(row.width(), env.tokens.size.icon.md.min(row.height())));
    let side = if env.direction.is_rtl() { RectSide::Right } else { RectSide::Left };
    ui.painter().extend(Rule::edge(&look, bar, side, scale));
}

fn home_drop_look(tokens: &Tokens) -> VariantLook {
    let mut look = Variant::SECONDARY.resolve(tokens).radius(RadiusStep::Control);
    if let Some(edge) = &mut look.edge {
        edge.rest = home_edge(tokens);
    }
    look
}

fn home_drop(page: &Page<'_>, ui: &mut Ui, hint: &Label, center: f32, wide: bool) {
    let room = ui.available_rect_before_wrap();
    let width = HOME_DROP_WIDTH.min(room.width());
    let top = (center - HOME_DROP_HEIGHT / 2.0).max(room.top());
    let slot = Rect::from_min_max(room.min, pos2(room.right(), top + HOME_DROP_HEIGHT));
    let left = room.center().x - width / 2.0;
    let card = Rect::from_min_max(pos2(left, top), pos2(left + width, slot.bottom()));
    let _ = Stack::new().id_salt("home-drop").show_at_taking_room(ui, slot, |ui, _| {
        if clicked(&Button::new(hint.clone()).id_salt("drop").variant(Variant::Custom(HOME_DROP_LOOK)).show_at(ui, card)) {
            page.push(StartAction::Open);
        }
        let tokens = &page.env.tokens;
        let (side, offset) = if wide {
            (HOME_DROP_ICON, (HOME_DROP_ICON + tokens.typography.label.line_height) / 2.0 + tokens.space.gap.stack)
        } else {
            let side = tokens.size.icon.lg;
            (side, side + tokens.space.gap.tight)
        };
        let icon = Rect::from_center_size(card.center() - vec2(0.0, offset), Vec2::splat(side));
        let _ = Icon::new(IconSpec::svg(IconId::new("image"))).show_at(ui, icon);
    });
}

/// The greeting page's sections: the title, the templates without a section heading, the more-templates link and the
/// recent files as rows in the order the app gave them. Like production's File > Home page it shows neither Pinned nor
/// the drop card, so the `pinned` and `drop_hint` props do not apply to it.
fn greeting_body(
    page: &Page<'_>,
    ui: &mut Ui,
    title: Option<&Label>,
    more_templates: Option<&Label>,
    templates: &[Template],
    recent: &[RecentFile],
    max_recent: usize,
) {
    if let Some(title) = title {
        Text::new(title.clone()).role(TypeRole::Heading).show(ui);
    }
    if !templates.is_empty() {
        templates_grid(page, ui, templates);
    }
    if let Some(label) = more_templates
        && clicked(&Link::new(label.clone()).id_salt("more-templates").show(ui))
    {
        page.push(StartAction::MoreTemplates);
    }
    let mut list = RecentList::new(recent).max_recent(max_recent).now(page.now);
    if let Some((thumbnails, source)) = page.thumbnails {
        list = list.thumbnails(thumbnails, source);
    }
    for action in list.show(ui).actions {
        page.push(action);
    }
}

/// A section of the gallery and greeting pages, its body a section gap under its title.
fn page_section(title: Label) -> Section<'static> {
    Section::fixed(title).header_gap(Gap::Section)
}

/// A section of a Column page: its title a heading with no rule, its body the page's heading gap under it.
fn column_section(title: Label) -> Section<'static> {
    Section::fixed(title).quiet_header().header_gap(Gap::PageHeading)
}

/// `body` centred across the column.
fn centred(ui: &mut Ui, body: impl FnMut(&mut Ui, &mut BodyCx<'_>)) {
    let _ = Row::new().align(RowAlign::Center).fill_width().show(ui, body);
}

/// The top of a Column page: the title over the subtitle, then New and Open as page actions, each centred.
fn column_heading(page: &Page<'_>, ui: &mut Ui, title: Option<&Label>, subtitle: Option<&Label>) {
    let (heading, actions) = (title.is_some() || subtitle.is_some(), page.offers_new || page.offers_open);
    if !heading && !actions {
        return;
    }
    let _ = Stack::new().id_salt("start-heading").gap(Gap::PageLead).fill_width().show(ui, |ui, _| {
        if heading {
            let _ = Stack::new().gap(Gap::PageTitle).fill_width().show(ui, |ui, _| {
                if let Some(title) = title {
                    centred(ui, |ui, _| drop(Text::new(title.clone()).role(TypeRole::Display).show(ui)));
                }
                if let Some(subtitle) = subtitle {
                    centred(ui, |ui, _| drop(Text::new(subtitle.clone()).shade(Shade::Muted).show(ui)));
                }
            });
        }
        if actions {
            let place = |overrides: &mut Overrides| {
                overrides.values.set(ControlPlace::PageAction);
            };
            let _ = with_env(ui, place, |ui| {
                Row::new().align(RowAlign::Center).gap(Gap::PageActions).fill_width().show(ui, |ui, _| new_and_open(page, ui, Sizing::Content))
            });
        }
    });
}

/// The links of a Column page under their heading, each centred: the call to action ([`StartLink::primary`]) as a
/// pill with its icon, the others as links without their icons, each run of them in rows `size.page.link` high a
/// page link gap apart. A URL opens in the browser (the URL is its tooltip); a command fires as a pointer activation.
fn column_links(page: &Page<'_>, ui: &mut Ui, title: Option<&Label>, links: &[StartLink]) {
    if links.is_empty() {
        return;
    }
    let stack = Stack::new().id_salt("start-links").gap(Gap::PageHeading).label(page.words.start_links.clone()).fill_width();
    let _ = stack.show(ui, |ui, cx| {
        if let Some(title) = title {
            centred(ui, |ui, _| drop(Text::new(title.clone()).role(TypeRole::BodyStrong).show(ui)));
        }
        let mut at = 0;
        for run in links.chunk_by(|a, b| !a.primary && !b.primary) {
            if run[0].primary {
                column_link(ui, cx, at, &run[0]);
            } else {
                let _ = Stack::new().gap(Gap::PageLink).fill_width().show(ui, |ui, cx| {
                    for (offset, link) in run.iter().enumerate() {
                        column_link(ui, cx, at + offset, link);
                    }
                });
            }
            at += run.len();
        }
    });
}

/// Link `at` of a Column page, centred: the call to action as a pill, another as a link.
fn column_link(ui: &mut Ui, cx: &mut BodyCx<'_>, at: usize, link: &StartLink) {
    let url = match &link.target {
        StartLinkTarget::Url(url) => Some(url),
        StartLinkTarget::Command(_) => None,
    };
    let mut activated = false;
    let row = Row::new().align(RowAlign::Center).fill_width();
    let row = if link.primary { row } else { row.least_height(Env::of(ui).tokens.size.page.link) };
    let _ = row.show(ui, |ui, _| {
        activated = if link.primary {
            let mut button = Button::new(link.label.clone()).id_salt(("start-link", at)).variant(Variant::Custom(CALL_TO_ACTION_LOOK));
            if let Some(icon) = &link.icon {
                button = button.icon(icon.clone());
            }
            if let Some(url) = url {
                button = button.tooltip(url.clone());
            }
            let place = |overrides: &mut Overrides| {
                overrides.values.set(ControlPlace::CallToAction);
            };
            clicked(&with_env(ui, place, |ui| button.show(ui)))
        } else {
            let mut text = Link::new(link.label.clone()).id_salt(("start-link", at));
            if let Some(url) = url {
                text = text.url(url.clone()).tooltip(url.clone());
            }
            clicked(&text.show(ui))
        };
    });
    // A link opens its own URL; the pill is a button, so the page opens its URL.
    match (&link.target, activated) {
        (StartLinkTarget::Url(url), true) if link.primary => cx.open_url(url, false),
        (StartLinkTarget::Command(command), true) => drop(commands::fire(ui.ctx(), command.clone(), FireSource::Pointer)),
        _ => {}
    }
}

/// New and Open, the first Tab stops of the page; Open alone where the app offers no New.
fn new_and_open(page: &Page<'_>, ui: &mut Ui, sizing: Sizing) {
    if page.offers_new && clicked(&Button::new(page.new_label()).id_salt("new").variant(Variant::PRIMARY).sizing(sizing).show(ui)) {
        page.push(StartAction::New);
    }
    if page.offers_open && clicked(&Button::new(page.open_label()).id_salt("open").sizing(sizing).show(ui)) {
        page.push(StartAction::Open);
    }
}

/// The links, one under another: a URL opens through its `Link` (the URL is its tooltip), a command fires as a
/// pointer activation.
fn link_items(ui: &mut Ui, links: &[StartLink], home: bool) {
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
                if home {
                    let variant = if link.selected { Variant::look(Emphasis::Soft, Tone::Accent) } else { Variant::GHOST };
                    button = button.variant(variant).align(HAlign::Start).sizing(Sizing::FillWidth);
                }
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
    desktop_home: bool,
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
    let _ = page
        .section(words.recent.clone())
        .header_gap(Gap::Section)
        .command_scope(StartRecentCommands::SCOPE)
        .controls(|ui, _| {
            let _ = Segmented::new(words.view.clone(), view.borrow_mut().reborrowed(), &modes).show(ui);
            let _ = Select::new(words.sort.clone(), sort.borrow_mut().reborrowed(), &orders).show(ui);
            // Clear empties the list, as the menu's Clear Recent Files does; with no files there is nothing to clear.
            if page.heading_sections {
                let clear = Button::new(words.clear_recent.clone()).icon(IconId::new("trash")).enabled(!files.is_empty()).id_salt("clear").show(ui);
                if clicked(&clear) {
                    page.push(StartAction::ClearRecent);
                }
            }
        })
        .show(ui, |ui, cx| recent_body(page, ui, cx, files, current, &words.no_recent, desktop_home));
}

/// A row of the pinned folders' list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum PinnedKey {
    /// The heading of the folder at this path.
    Folder(PathBuf),
    /// A file listed under `folder`; the folder is part of the key because nested pinned folders can list one file twice.
    File { folder: PathBuf, path: PathBuf },
    /// The More row of the folder at this path.
    More(PathBuf),
}

impl PinnedKey {
    /// What opening the row asks of the app, by a click, Enter or the menu's Open.
    fn opened(self) -> StartAction {
        match self {
            PinnedKey::Folder(path) => StartAction::OpenPinned(path),
            PinnedKey::File { path, .. } => StartAction::OpenPinnedFile(path),
            PinnedKey::More(path) => StartAction::ExpandPinned(path),
        }
    }
}

/// What a row of the pinned folders shows.
#[derive(Clone, Copy)]
enum PinnedRow<'f> {
    Folder(&'f PinnedFolder),
    File(&'f PinnedFile),
    More { expanded: bool },
}

/// The files the app listed in `folder`; none while it is not listed.
fn listed_files(folder: &PinnedFolder) -> &[PinnedFile] {
    match &folder.listing {
        Some(FolderListing::Listed { files, .. }) => files,
        _ => &[],
    }
}

/// The files of `folder` the list shows: none while folded, the first [`PINNED_FILES_SHOWN`] until expanded.
fn shown_files(folder: &PinnedFolder) -> &[PinnedFile] {
    let files = listed_files(folder);
    match (folder.folded, folder.expanded) {
        (true, _) => &[],
        (false, true) => files,
        (false, false) => files.get(..PINNED_FILES_SHOWN).unwrap_or(files),
    }
}

/// The note at the end of a listed folder's heading: the state of its listing, or that the app kept only the newest
/// files; `None` for a folder whose files all show.
fn folder_note(words: &ShellWords, folder: &PinnedFolder) -> Option<Label> {
    match &folder.listing {
        None => None,
        Some(FolderListing::Pending) => Some(words.looking_for_files.clone()),
        Some(FolderListing::Unavailable) => Some(words.folder_unavailable.clone()),
        Some(FolderListing::Listed { files, .. }) if files.is_empty() => Some(words.folder_empty.clone()),
        Some(FolderListing::Listed { files, total }) if *total > files.len() => {
            let (shown, total) = (files.len().to_string(), total.to_string());
            Some(Label::from(words.newest_of.fill(&[("shown", shown.as_str()), ("total", total.as_str())])))
        }
        Some(_) => None,
    }
}

/// The source of the pinned folders' list: each folder's heading, then the files it shows and its More row when it
/// holds more than [`PINNED_FILES_SHOWN`]. A heading is a tree group when the app lists the folder.
struct PinnedRows<'f> {
    rows: Vec<(PinnedKey, PinnedRow<'f>)>,
}

impl<'f> PinnedRows<'f> {
    fn new(folders: &'f [PinnedFolder]) -> Self {
        let mut rows = Vec::new();
        for folder in folders {
            rows.push((PinnedKey::Folder(folder.path.clone()), PinnedRow::Folder(folder)));
            rows.extend(
                shown_files(folder)
                    .iter()
                    .map(|file| (PinnedKey::File { folder: folder.path.clone(), path: file.path.clone() }, PinnedRow::File(file))),
            );
            if !folder.folded && listed_files(folder).len() > PINNED_FILES_SHOWN {
                rows.push((PinnedKey::More(folder.path.clone()), PinnedRow::More { expanded: folder.expanded }));
            }
        }
        PinnedRows { rows }
    }

    fn row(&self, key: &PinnedKey) -> Option<PinnedRow<'f>> {
        self.index_of(key).and_then(|at| self.rows.get(at)).map(|(_, row)| *row)
    }
}

impl DisplayOrder<PinnedKey> for PinnedRows<'_> {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> PinnedKey {
        self.rows[at].0.clone()
    }

    fn index_of(&self, key: &PinnedKey) -> Option<usize> {
        self.rows.iter().position(|(held, _)| held == key)
    }

    fn depth(&self, at: usize) -> usize {
        match self.rows[at].1 {
            PinnedRow::Folder(_) => 0,
            PinnedRow::File(_) | PinnedRow::More { .. } => 1,
        }
    }

    /// Every folder the app lists folds, one with no files to show too, so each heading has its disclosure.
    fn has_children(&self, at: usize) -> bool {
        matches!(self.rows[at].1, PinnedRow::Folder(folder) if folder.listing.is_some())
    }

    fn open(&self, at: usize) -> bool {
        matches!(self.rows[at].1, PinnedRow::Folder(folder) if !folder.folded)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        match self.rows[at].1 {
            PinnedRow::Folder(folder) => Some(Cow::Borrowed(folder.name.as_str())),
            PinnedRow::File(file) => Some(Cow::Borrowed(file.name.as_str())),
            PinnedRow::More { .. } => None,
        }
    }
}

impl ItemSource<PinnedKey> for PinnedRows<'_> {}

/// The list row of a pinned folders' row.
fn pinned_row<'r>(words: &ShellWords, row: PinnedRow<'_>) -> ListRow<'r> {
    match row {
        PinnedRow::Folder(folder) => {
            let mut heading = ListRow::new(Label::from(folder.name.clone()))
                .detail(folder.path.display().to_string())
                .icon(IconSpec::svg(IconId::new("folder")))
                .depth(0);
            // A folder the app does not list keeps the plain row; its menu and Delete still unpin it.
            if folder.listing.is_none() {
                return heading;
            }
            heading = heading.group(!folder.folded);
            if let Some(note) = folder_note(words, folder) {
                heading = heading.trailing(note);
            }
            heading.trailing_part(UNPIN, TrailingFace::Strong(words.unpin.clone()), words.unpin_folder.fill(&[("name", folder.name.as_str())]))
        }
        PinnedRow::File(file) => {
            let row = ListRow::new(Label::from(file.name.clone()))
                .depth(1)
                .icon(IconSpec::svg(IconId::new("file")))
                .tooltip(file.path.display().to_string());
            match &file.note {
                Some(note) => row.detail(note.clone()),
                None => row,
            }
        }
        PinnedRow::More { expanded } => ListRow::new(if expanded { words.show_less.clone() } else { words.show_more.clone() }).depth(1),
    }
}

fn pinned_section(page: &Page<'_>, ui: &mut Ui, folders: &[PinnedFolder]) {
    let words = page.words;
    let _ = page
        .section(page.pinned_title.cloned().unwrap_or_else(|| words.pinned.clone()))
        .header_gap(Gap::Section)
        .command_scope(StartPinnedCommands::SCOPE)
        .controls(|ui, _| {
            let pin = Button::new(words.pin_folder.clone()).id_salt("pin").icon(IconId::new("new_folder"));
            let pin = if page.heading_sections { pin.variant(Variant::GHOST) } else { pin };
            if clicked(&pin.show(ui)) {
                page.push(StartAction::PinFolder);
            }
        })
        .show(ui, |ui, cx| {
            let rows = PinnedRows::new(folders);
            let mut lead = cx.keep::<Leads>().pinned.clone().filter(|key| rows.row(key).is_some());
            // One rule for the menu row, the palette and Delete: only a folder's heading unpins.
            cx.publish(StartPinnedCommands::Unpin, commands::CommandState::new().enabled(matches!(lead, Some(PinnedKey::Folder(_)))));
            while let Some((command, _)) = cx.take_command::<StartPinnedCommands>() {
                let action = match (command, lead.clone()) {
                    (StartPinnedCommands::Open, Some(key)) => Some(key.opened()),
                    (StartPinnedCommands::Unpin, Some(PinnedKey::Folder(path))) => Some(StartAction::UnpinFolder(path)),
                    _ => None,
                };
                if let Some(action) = action {
                    page.push(action);
                }
            }
            if folders.is_empty() {
                let _ = Notice::new(Tone::Neutral, words.no_pinned.clone()).layout(NoticeLayout::Centred).show(ui);
                return;
            }
            let out = List::<PinnedKey>::new(words.pinned.clone())
                .select(SelectMode::Single)
                .activate(ActivateOn::Click)
                .tree(true)
                .sizing(Sizing::FillWidth)
                .natural_height(true)
                .show_items(ui, &rows, &mut |key, _| match rows.row(key) {
                    Some(row) => pinned_row(words, row),
                    None => ListRow::new(Label::Empty),
                });
            for event in &out.events {
                match event {
                    // The list keeps no open state: the disclosure and Left/Right ask the app to fold or unfold the
                    // folder, which reports only a change from what the app passed.
                    CollectionEvent::Open(change) => {
                        if let PinnedKey::Folder(path) = &change.key
                            && folders.iter().any(|folder| &folder.path == path && folder.folded == change.open)
                        {
                            page.push(StartAction::FoldPinned(path.clone()));
                        }
                    }
                    CollectionEvent::Item(PinnedKey::Folder(path), RowEvent::Trailing { part, .. }) if *part == UNPIN => {
                        page.push(StartAction::UnpinFolder(path.clone()));
                    }
                    _ => {}
                }
            }
            for key in read_events(out.events, &mut lead).0 {
                page.push(key.opened());
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
    let _ = page
        .section(page.templates_title.cloned().unwrap_or_else(|| page.words.templates.clone()))
        .header_gap(Gap::Section)
        .show(ui, |ui, _| templates_grid(page, ui, templates));
}

/// The template cards, which report the chosen one as [`StartAction::UseTemplate`].
fn templates_grid(page: &Page<'_>, ui: &mut Ui, templates: &[Template]) {
    let words = page.words;
    let source = keyed(templates, |template: &Template| template.id.clone()).with_text(|template| Cow::Borrowed(template.name.as_str()));
    let mut grid = page.grid::<String>(words.templates.clone(), false);
    // Templates of page sizes are page cards: the page on a picture twice as wide as high.
    if templates.iter().any(|template| template.page.is_some()) {
        grid = grid.card_text(CardText::Page).aspect(2.0);
    }
    let out = grid.show_items(ui, &source, &mut |id, _| {
        let Some(template) = templates.iter().find(|template| &template.id == id) else { return Tile::new(Label::Empty) };
        let detail = (!template.detail.is_empty()).then(|| template.detail.clone());
        match template.page {
            Some((width, height)) => {
                let tile = Tile::new(Label::from(template.name.clone())).page(width, height);
                match detail {
                    Some(detail) => tile.detail(detail),
                    None => tile,
                }
            }
            None => page.tile(&template.name, detail, ThumbKey::Template(template.id.clone())),
        }
    });
    let mut lead = None;
    for id in read_events(out.response.events, &mut lead).0 {
        page.push(StartAction::UseTemplate(id));
    }
}
