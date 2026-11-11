//! `LinkList`: the Links panel's rows, filter, sort, info section and actions.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

use craft_behavior::{DisplayOrder, EditEvent, IconId, ItemSource, Mixed, Selection, SelectionChange};
use craft_docpath::links::{Link, LinkStatus};
use craft_theme::variant::Tone;
use craft_ui::contract::{
    Bind, BodyCx, CollectionEvent, NoValue, Outcome, Pattern, PopupKind, Props, Show as _, ShowCollection as _, ShowContainer as _, Trigger,
};
use craft_ui::display::Divider;
use craft_ui::layout_components::Stack;
use craft_ui::list::{List, ListRow, ListWords, ThumbSize};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::primitives::textures::{Priority, TextureKey, ThumbPaint, ThumbRequest, ThumbSource, Thumbnails};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::memory::Remember as _;
use craft_ui::sections::{FooterAction, FooterBar, Section};
use egui::{Response, Ui};

use super::StatusBadge;

/// Which links the list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum LinkFilter {
    /// Every link.
    #[default]
    All,
    /// `LinkStatus::is_missing`.
    Missing,
    /// Modified.
    Modified,
    /// Embedded.
    Embedded,
}

/// The list's order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum LinkSort {
    /// By name, digits by value.
    #[default]
    Name,
    /// By extension, then name.
    Kind,
    /// Missing first, then Modified, then the rest; then name.
    Status,
}

/// The list's saved choices; the host stores them.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct LinkListSettings {
    /// The filter.
    pub filter: LinkFilter,
    /// The order.
    pub sort: LinkSort,
    /// Row thumbnails: None, Small, Medium, Large.
    pub thumbnails: ThumbSize,
    /// The Link Info section is open.
    pub info_open: bool,
}

/// What the app keeps for a [`LinkList`]: the selection (session state the app reads) and the
/// saved [`LinkListSettings`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LinkListState<K> {
    /// The selected links.
    pub selection: Selection<K>,
    /// The saved choices.
    pub settings: LinkListSettings,
}

impl<K> Default for LinkListState<K> {
    fn default() -> Self {
        Self { selection: Selection::default(), settings: LinkListSettings::default() }
    }
}

/// What the list reported, in order. Each is a `Set` the host runs as its own command, one undo
/// step each.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LinkListEvent<K> {
    /// The selection changed (a click, a key, a secondary click on an unselected row); already
    /// applied to [`LinkListState::selection`].
    Selection(SelectionChange<K>),
    /// Relink…: a file pick for the selection.
    Relink(Vec<K>),
    /// Relink to Folder…: a folder pick for the selection.
    RelinkToFolder(Vec<K>),
    /// Find missing links: the host opens the `RelinkDialog` on the missing set.
    FindMissing,
    /// Update Link on the modified links of the selection.
    Update(Vec<K>),
    /// Edit Original (the host's `openable_original` check stays its own).
    EditOriginal(K),
    /// Break Link (embed).
    BreakLink(Vec<K>),
    /// Reveal in the file manager.
    Reveal(K),
    /// Go To Link (double click).
    GoTo(K),
}

/// What [`LinkList::show`] returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host runs the list's events"]
pub struct LinkListOutput<K> {
    /// The list.
    pub response: Response,
    /// What happened, in order.
    pub events: Vec<LinkListEvent<K>>,
    /// The [`LinkListSettings`] changed (filter, sort, thumbnails, info section); the host saves them.
    pub settings_changed: bool,
}

impl<K> Outcome for LinkListOutput<K> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

craft_ui::words! {
    /// The Links panel's words, English by default.
    #[non_exhaustive]
    pub struct LinkListWords {
        list: "linkList.list" = "Links";
        show_all: "linkList.showAll" = "Show All";
        show_missing: "linkList.showMissing" = "Show Missing";
        show_modified: "linkList.showModified" = "Show Modified";
        show_embedded: "linkList.showEmbedded" = "Show Embedded";
        sort_name: "linkList.sortName" = "Sort by Name";
        sort_kind: "linkList.sortKind" = "Sort by Kind";
        sort_status: "linkList.sortStatus" = "Sort by Status";
        relink: "linkList.relink" = "Relink…";
        relink_folder: "linkList.relinkFolder" = "Relink to Folder…";
        find_missing: "linkList.findMissing" = "Find Missing Links…";
        update: "linkList.update" = "Update Link";
        edit_original: "linkList.editOriginal" = "Edit Original";
        break_link: "linkList.break" = "Break Link";
        reveal: "linkList.reveal" = "Reveal in File Manager";
        go_to: "linkList.goTo" = "Go to Link";
        info: "linkList.info" = "Link Info";
        /// `{n}` the page.
        page: template "linkList.page" = "Page {n}";
        missing: "linkList.missing" = "Missing";
        modified: "linkList.modified" = "Modified";
        unreadable: "linkList.unreadable" = "Unreadable";
        /// `{reason}` from the `refused_*` words.
        refused: template "linkList.refused" = "Not followed: {reason}";
        refused_network: "linkList.refusedNetwork" = "the path is on another computer";
        refused_device: "linkList.refusedDevice" = "the path names a device, not a file";
        refused_url: "linkList.refusedUrl" = "the link is not a file path";
        refused_invalid: "linkList.refusedInvalid" = "the path is not valid";
        embedded: "linkList.embedded" = "Embedded";
        thumbnails: "linkList.thumbnails" = "Thumbnails";
        menu: "linkList.menu" = "Links Menu";
        empty: "linkList.empty" = "No links";
    }
}

/// The thumbnails a list draws with: the cache and the app's source, keyed by the link key.
type ThumbPair<'a, K> = (&'a Thumbnails<K>, &'a dyn ThumbSource<K>);

type InfoFn<'a> = Box<dyn FnMut(&mut Ui) + 'a>;

/// The Links panel: rows of the document's links with thumbnail, name, page and status.
///
/// - **Rows.** `ListRow`s, striped: a thumbnail by `LinkListSettings::thumbnails`, the name, the
///   page as the detail line and the status glyph at the end ([`StatusBadge::icon`]); filtered by
///   `filter` and sorted by `sort`. The row is named "name, Page n, status in words".
/// - **Selection.** The collection's rules; [`LinkListEvent::Selection`] carries each change and
///   a double click reports `GoTo`. Delete does nothing: Break Link is a command, not a key.
/// - **Actions** (the footer, `.actions(true)`; also the menu button and the list's context
///   menu, one row set): Relink… (a selection), Relink to Folder… (a selection), Find Missing
///   Links… (any link missing), Update Link (any selected is Modified), Edit Original (any
///   selected is linked and not missing), Break Link (any linked), Reveal (one linked, not
///   missing), Go to Link (one); then the filter, the sort and the thumbnail size as radio rows.
///   The choices are in [`LinkListSettings`] (`settings_changed` reported).
/// - **Info section.** A `Section` "Link Info" (`info_open`) with the rows the host gives in
///   `.info(..)` (path, size, modified, dimensions, page).
///
/// ```ignore
/// let out = LinkList::new("Links", &links, monitor.statuses(), Bind::bound(&mut state.links)).show(ui);
/// ```
#[must_use = "show the list with `show`"]
#[derive(Pattern, Props)]
#[pattern(probe(LinkList<'static, u32>))]
pub struct LinkList<'a, K: TextureKey + std::fmt::Debug> {
    label: Label,
    links: &'a [Link<K>],
    statuses: &'a HashMap<K, LinkStatus>,
    state: Bind<'a, LinkListState<K>>,
    thumbnails: Option<ThumbPair<'a, K>>,
    /// Shows the action row under the list.
    #[prop]
    actions: bool,
    /// The Link Info section's body for the lead link.
    #[prop(boxed = "FnMut(&mut Ui) + 'a")]
    info: Option<InfoFn<'a>>,
}

impl<'a, K: TextureKey + std::fmt::Debug> LinkList<'a, K> {
    /// The list named `label` of `links` with their known `statuses` (from `LinkMonitor::statuses`;
    /// a link without one shows no glyph), over the app's `state`.
    pub fn new(label: impl Into<Label>, links: &'a [Link<K>], statuses: &'a HashMap<K, LinkStatus>, state: Bind<'a, LinkListState<K>>) -> Self {
        Self { label: label.into(), links, statuses, state, thumbnails: None, actions: true, info: None }
    }

    /// Row thumbnails keyed by the link key.
    pub fn thumbnails(mut self, thumbnails: &'a Thumbnails<K>, source: &'a dyn ThumbSource<K>) -> Self {
        self.thumbnails = Some((thumbnails, source));
        self
    }
}

/// What a pass keeps between the list and the footer: the state as read and as the pass changes it,
/// and what the selection allows.
#[derive(Clone)]
struct ListMemory<K> {
    before: LinkListState<K>,
    local: LinkListState<K>,
    facts: Facts<K>,
}

impl<K> Default for ListMemory<K> {
    fn default() -> Self {
        Self { before: LinkListState::default(), local: LinkListState::default(), facts: Facts::default() }
    }
}

/// The shown links as the list's item source.
struct Rows<'a, K> {
    shown: Vec<&'a Link<K>>,
}

impl<K: Clone> DisplayOrder<K> for Rows<'_, K> {
    fn len(&self) -> usize {
        self.shown.len()
    }

    fn key(&self, at: usize) -> K {
        self.shown[at].key.clone()
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.shown[at].name.as_str()))
    }
}

impl<K: Clone> ItemSource<K> for Rows<'_, K> {}

impl<K: TextureKey + std::fmt::Debug> LinkList<'_, K> {
    /// Draws the panel body and reports what happened.
    pub fn show(self, ui: &mut Ui) -> LinkListOutput<K> {
        let LinkList { label, links, statuses, state, thumbnails, actions, info } = self;
        let env = Env::of(ui);
        let words = env.words::<LinkListWords>();
        let list_words = env.words::<ListWords>();
        let collator = env.collator();
        let format = env.format();
        let list_id = ui.scope_id().with("link-rows");
        let state = RefCell::new(state);
        let info = RefCell::new(info);
        let events: RefCell<Vec<LinkListEvent<K>>> = RefCell::new(Vec::new());
        let commands: RefCell<Vec<Command>> = RefCell::new(Vec::new());
        let settings_changed = Cell::new(false);
        let has_footer = actions || info.borrow().is_some();
        let ctx = ui.ctx().clone();

        let finish = |cx: &mut BodyCx<'_>| {
            let (before, mut local, facts) = {
                let memory = cx.keep::<ListMemory<K>>();
                (memory.before.clone(), memory.local.clone(), memory.facts.clone())
            };
            for command in commands.take() {
                facts.run(command, &mut local.settings, &mut events.borrow_mut());
            }
            settings_changed.set(local.settings != before.settings);
            if local != before {
                let event = EditEvent::set(Mixed::Same(before), local, Arc::from(label.as_str()), None);
                let _ = state.borrow_mut().reborrow().write(cx, &event);
            }
        };

        let body = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
            let before = match state.borrow_mut().reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => LinkListState::default(),
            };
            let mut local = before.clone();
            let shown = visible_links(links, statuses, local.settings.filter, local.settings.sort, &collator);
            let source = Rows { shown };
            let mut selection = local.selection.clone();
            let page = |n: u32| format.integer(u64::from(n));
            let rows = RowInputs { ctx: &ctx, words: &words, page_number: &page, statuses, thumbnails, thumb_size: local.settings.thumbnails };
            let response = if source.shown.is_empty() {
                Notice::new(Tone::Neutral, words.empty.clone()).layout(NoticeLayout::Centred).show(ui).response
            } else {
                let out =
                    List::<K>::new(words.list.clone())
                        .id(list_id)
                        .selection(Bind::bound(&mut selection))
                        .show_items(ui, &source, &mut |_, icx| link_row(&rows, source.shown[icx.at], icx.at));
                for event in out.events {
                    match event {
                        CollectionEvent::Selection(change) => events.borrow_mut().push(LinkListEvent::Selection(change)),
                        CollectionEvent::Activate(key) => events.borrow_mut().push(LinkListEvent::GoTo(key)),
                        _ => {}
                    }
                }
                out.response
            };
            local.selection = selection;
            let facts = Facts::of(&source.shown, &local.selection, links, statuses);
            let ran = Menu::context(words.menu.clone(), &response)
                .show(ui, |ui, _| menu_rows(ui, &words, &list_words, &facts, &local.settings))
                .inner
                .flatten();
            commands.borrow_mut().extend(ran);
            let memory = cx.keep::<ListMemory<K>>();
            memory.before = before;
            memory.local = local;
            memory.facts = facts;
            if !has_footer {
                finish(cx);
            }
            response
        };

        let footer = |ui: &mut Ui, cx: &mut BodyCx<'_>| {
            // The footer runs in a sizing pass before the shown pass: the section lays out in both and writes memory only
            // when shown.
            if let Some(show_info) = info.borrow_mut().as_mut() {
                let mut open = cx.keep::<ListMemory<K>>().local.settings.info_open;
                Section::new(words.info.clone()).open(Bind::bound(&mut open)).show(ui, |ui, _| show_info(ui));
                if !ui.is_sizing_pass() {
                    cx.keep::<ListMemory<K>>().local.settings.info_open = open;
                }
            }
            if actions {
                let (facts, settings) = {
                    let memory = cx.keep::<ListMemory<K>>();
                    (memory.facts.clone(), memory.local.settings.clone())
                };
                let buttons = [
                    (Command::Relink, "link", &words.relink),
                    (Command::Update, "refresh", &words.update),
                    (Command::EditOriginal, "pencil", &words.edit_original),
                    (Command::GoTo, "target", &words.go_to),
                    (Command::BreakLink, "unlink", &words.break_link),
                ];
                let mut bar: Vec<FooterAction> = buttons
                    .iter()
                    .map(|(command, icon, label)| FooterAction::new(IconId::new(icon), (*label).clone()).enabled(facts.enabled(*command)))
                    .collect();
                bar.push(FooterAction::new(IconId::new("menu"), words.menu.clone()).popup(PopupKind::Menu));
                let shown = FooterBar::actions(bar).show_footer(ui);
                if let Some(command) = shown.picked.and_then(|at| buttons.get(at)).map(|(command, ..)| *command) {
                    commands.borrow_mut().push(command);
                }
                if let Some(opener) = shown.responses.last() {
                    let ran = Menu::new(words.menu.clone())
                        .opened_by(opener, Trigger::Click)
                        .show(ui, |ui, _| menu_rows(ui, &words, &list_words, &facts, &settings))
                        .inner
                        .flatten();
                    commands.borrow_mut().extend(ran);
                }
            }
            finish(cx);
        };

        let stack = Stack::new().fill();
        let stack = if has_footer { stack.footer(footer) } else { stack };
        let shown = stack.show(ui, body);
        let response = shown.inner.unwrap_or(shown.output);
        LinkListOutput { response, events: events.take(), settings_changed: settings_changed.get() }
    }
}

/// The status of `link`: the monitor's, else `Embedded` for an embedded link, else unknown.
fn status_of<K: TextureKey>(link: &Link<K>, statuses: &HashMap<K, LinkStatus>) -> Option<LinkStatus> {
    statuses.get(&link.key).copied().or(link.embedded.then_some(LinkStatus::Embedded))
}

/// Whether the link's file is the document's own data.
fn is_embedded(status: Option<LinkStatus>) -> bool {
    status == Some(LinkStatus::Embedded)
}

/// The links `filter` lets through, in the order of `sort`.
fn visible_links<'a, K: TextureKey>(
    links: &'a [Link<K>],
    statuses: &HashMap<K, LinkStatus>,
    filter: LinkFilter,
    sort: LinkSort,
    collator: &craft_i18n::Collator,
) -> Vec<&'a Link<K>> {
    let mut shown: Vec<&Link<K>> = links
        .iter()
        .filter(|link| {
            let status = status_of(link, statuses);
            match filter {
                LinkFilter::All => true,
                LinkFilter::Missing => status.is_some_and(LinkStatus::is_missing),
                LinkFilter::Modified => status == Some(LinkStatus::Modified),
                LinkFilter::Embedded => is_embedded(status),
            }
        })
        .collect();
    let extension = |name: &str| std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let rank = |link: &Link<K>| match status_of(link, statuses) {
        Some(status) if status.is_missing() => 0,
        Some(LinkStatus::Modified) => 1,
        _ => 2,
    };
    shown.sort_by(|a, b| {
        let by_name = collator.compare(&a.name, &b.name);
        match sort {
            LinkSort::Name => by_name,
            LinkSort::Kind => collator.compare(&extension(&a.name), &extension(&b.name)).then(by_name),
            LinkSort::Status => rank(a).cmp(&rank(b)).then(by_name),
        }
    });
    shown
}

/// One pass's inputs shared by every row.
struct RowInputs<'p, 'a, K: TextureKey> {
    ctx: &'p egui::Context,
    words: &'p LinkListWords,
    page_number: &'p dyn Fn(u32) -> String,
    statuses: &'p HashMap<K, LinkStatus>,
    thumbnails: Option<ThumbPair<'a, K>>,
    thumb_size: ThumbSize,
}

/// The row of `link`: striped by position, with its thumbnail, page and status glyph.
fn link_row<'r, K: TextureKey + std::fmt::Debug>(inputs: &RowInputs<'_, 'r, K>, link: &Link<K>, at: usize) -> ListRow<'r> {
    let RowInputs { ctx, words, page_number, statuses, thumbnails, thumb_size } = *inputs;
    let status = status_of(link, statuses);
    let glyph = status.and_then(|status| StatusBadge::icon(status).map(|icon| (icon, StatusBadge::words(status, words))));
    let page = link.page.map(|n| words.page.fill(&[("n", &page_number(n))]));
    let mut name = vec![link.name.clone()];
    name.extend(page.clone());
    if let Some((_, status_words)) = &glyph {
        name.push(status_words.as_str().to_owned());
    }
    let mut row = ListRow::new(link.name.clone()).striped(at % 2 == 1).accessible_name(name.join(", "));
    if let Some(page) = page {
        row = row.detail(page);
    }
    if thumb_size != ThumbSize::None {
        let key = link.key.clone();
        let ctx = ctx.clone();
        row = row.thumb(thumb_size, move |cx, rect| {
            let Some((thumbnails, source)) = thumbnails else {
                return;
            };
            let request = ThumbRequest::for_cell(rect.width().max(rect.height()), cx.pixels_per_point());
            let state = thumbnails.state(&ctx, source, &key, &request, Priority::VISIBLE);
            thumbnails.paint(cx, rect, &state, &ThumbPaint::default());
        });
    }
    if let Some((icon, status_words)) = glyph {
        row = row.tooltip(status_words.clone()).trailing_icon(icon, status_words);
    }
    row
}

/// What the selected rows allow, over the shown links in their order.
#[derive(Clone)]
struct Facts<K> {
    selected: Vec<K>,
    modified: Vec<K>,
    /// Linked and not missing: Edit Original and Reveal work on these.
    openable: Vec<K>,
    linked: Vec<K>,
    lead: Option<K>,
    any_missing: bool,
}

impl<K> Default for Facts<K> {
    fn default() -> Self {
        Self { selected: Vec::new(), modified: Vec::new(), openable: Vec::new(), linked: Vec::new(), lead: None, any_missing: false }
    }
}

impl<K: TextureKey> Facts<K> {
    fn of(shown: &[&Link<K>], selection: &Selection<K>, links: &[Link<K>], statuses: &HashMap<K, LinkStatus>) -> Self {
        let mut facts =
            Facts { any_missing: links.iter().any(|link| status_of(link, statuses).is_some_and(LinkStatus::is_missing)), ..Facts::default() };
        for link in shown.iter().filter(|link| selection.items.contains(&link.key)) {
            let status = status_of(link, statuses);
            facts.selected.push(link.key.clone());
            if status == Some(LinkStatus::Modified) {
                facts.modified.push(link.key.clone());
            }
            if !is_embedded(status) {
                facts.linked.push(link.key.clone());
                if !status.is_some_and(LinkStatus::is_missing) {
                    facts.openable.push(link.key.clone());
                }
            }
        }
        facts.lead = selection.lead.clone().filter(|lead| facts.selected.contains(lead)).or_else(|| facts.selected.first().cloned());
        facts
    }

    fn enabled(&self, command: Command) -> bool {
        match command {
            Command::Relink | Command::RelinkFolder => !self.selected.is_empty(),
            Command::FindMissing => self.any_missing,
            Command::Update => !self.modified.is_empty(),
            Command::EditOriginal => !self.openable.is_empty(),
            Command::BreakLink => !self.linked.is_empty(),
            Command::Reveal => self.selected.len() == 1 && self.openable.len() == 1,
            Command::GoTo => self.selected.len() == 1,
            Command::Filter(_) | Command::Sort(_) | Command::Thumbnails(_) => true,
        }
    }

    /// Runs `command`: an action reports its event, a choice changes `settings`.
    fn run(&self, command: Command, settings: &mut LinkListSettings, events: &mut Vec<LinkListEvent<K>>) {
        if !self.enabled(command) {
            return;
        }
        match command {
            Command::Relink => events.push(LinkListEvent::Relink(self.selected.clone())),
            Command::RelinkFolder => events.push(LinkListEvent::RelinkToFolder(self.selected.clone())),
            Command::FindMissing => events.push(LinkListEvent::FindMissing),
            Command::Update => events.push(LinkListEvent::Update(self.modified.clone())),
            Command::EditOriginal => events.extend(self.openable.first().cloned().map(LinkListEvent::EditOriginal)),
            Command::BreakLink => events.push(LinkListEvent::BreakLink(self.linked.clone())),
            Command::Reveal => events.extend(self.openable.first().cloned().map(LinkListEvent::Reveal)),
            Command::GoTo => events.extend(self.lead.clone().map(LinkListEvent::GoTo)),
            Command::Filter(filter) => settings.filter = filter,
            Command::Sort(sort) => settings.sort = sort,
            Command::Thumbnails(size) => settings.thumbnails = size,
        }
    }
}

/// A command of the menu and the action row.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    Relink,
    RelinkFolder,
    FindMissing,
    Update,
    EditOriginal,
    BreakLink,
    Reveal,
    GoTo,
    Filter(LinkFilter),
    Sort(LinkSort),
    Thumbnails(ThumbSize),
}

/// One menu row; running it records `command`.
fn menu_item(ui: &mut Ui, label: &Label, enabled: bool, chosen: Option<bool>, command: Command, ran: &mut Option<Command>) {
    let mut item = MenuItem::new(label.clone()).enabled(enabled);
    if let Some(chosen) = chosen {
        item = item.radio_mark(chosen);
    }
    if item.show(ui).ran {
        *ran = Some(command);
    }
}

/// The one row set of the menu button and the list's context menu: the actions, then the filter, the
/// sort and the thumbnail size as radio rows. Returns the command that ran.
fn menu_rows<K: TextureKey>(
    ui: &mut Ui,
    words: &LinkListWords,
    list_words: &ListWords,
    facts: &Facts<K>,
    settings: &LinkListSettings,
) -> Option<Command> {
    let mut ran = None;
    for (label, command) in [
        (&words.relink, Command::Relink),
        (&words.relink_folder, Command::RelinkFolder),
        (&words.find_missing, Command::FindMissing),
        (&words.update, Command::Update),
        (&words.edit_original, Command::EditOriginal),
        (&words.break_link, Command::BreakLink),
        (&words.reveal, Command::Reveal),
        (&words.go_to, Command::GoTo),
    ] {
        menu_item(ui, label, facts.enabled(command), None, command, &mut ran);
    }
    Divider::horizontal().show(ui);
    for (label, filter) in [
        (&words.show_all, LinkFilter::All),
        (&words.show_missing, LinkFilter::Missing),
        (&words.show_modified, LinkFilter::Modified),
        (&words.show_embedded, LinkFilter::Embedded),
    ] {
        menu_item(ui, label, true, Some(settings.filter == filter), Command::Filter(filter), &mut ran);
    }
    Divider::horizontal().show(ui);
    for (label, sort) in [(&words.sort_name, LinkSort::Name), (&words.sort_kind, LinkSort::Kind), (&words.sort_status, LinkSort::Status)] {
        menu_item(ui, label, true, Some(settings.sort == sort), Command::Sort(sort), &mut ran);
    }
    Divider::horizontal().show(ui);
    let _ = Menu::submenu_row(ui, words.thumbnails.clone(), |ui, _| {
        for size in ThumbSize::ALL {
            menu_item(ui, &size.label(list_words), true, Some(settings.thumbnails == size), Command::Thumbnails(size), &mut ran);
        }
    });
    ran
}
