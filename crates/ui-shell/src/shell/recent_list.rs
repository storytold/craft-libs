//! `RecentList`: the recent files as rows with their age and thumbnail, and the body every recent view of the start
//! page shares.

use super::recent::{RecentFile, RecentView, ThumbKey};
use super::start::{
    Leads, PICTURE_HIGH, PICTURE_WIDE, Page, REMOVE, StartAction, StartOutput, StartRecentCommands, ThumbHost, folder_of, paint_thumb, read_events,
};
use super::words::ShellWords;
use crate::buttons::Button;
use crate::contract::{
    ActivateOn, BodyCx, CollectionEvent, Gap, Props as _, Remember as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, clicked,
};
use crate::list::{List, ListRow, RowEvent, ThumbAspect, ThumbSize, TrailingFace};
use crate::menu::{Menu, MenuItem};
use crate::notices::{Notice, NoticeLayout};
use crate::primitives::{IconSpec, ThumbSource, Thumbnails};
use crate::runtime::commands::{self, CommandSet as _, Commands as _};
use crate::runtime::env::{Env, Label};
use crate::sections::Section;
use crate::tile_grid::{Tile, TilePartEvent};
use craft_behavior::{IconId, keyed};
use craft_theme::variant::Tone;
use egui::Ui;
use std::borrow::Cow;
use std::cell::RefCell;
use std::path::PathBuf;

/// The recent files as rows, with the commands of a recent file and its context menu. The files arrive in the order
/// the app wants them shown; the list does not sort.
#[must_use = "show the list with `show`"]
pub struct RecentList<'a> {
    recent: &'a [RecentFile],
    max_recent: usize,
    empty: Option<Label>,
    now: Option<i64>,
    thumbnails: Option<ThumbHost<'a>>,
}

impl<'a> RecentList<'a> {
    /// A list of `recent` files.
    pub fn new(recent: &'a [RecentFile]) -> Self {
        RecentList { recent, max_recent: usize::MAX, empty: None, now: None, thumbnails: None }
    }

    /// At most this many recent files show.
    #[must_use]
    pub fn max_recent(mut self, max_recent: usize) -> Self {
        self.max_recent = max_recent;
        self
    }

    /// The text shown when there are no recent files.
    #[must_use]
    pub fn empty(mut self, empty: impl Into<Label>) -> Self {
        self.empty = Some(empty.into());
        self
    }

    /// The time the age lines count from, in seconds since the Unix epoch. Without it the rows show no age.
    #[must_use]
    pub fn now(mut self, now: Option<i64>) -> Self {
        self.now = now;
        self
    }

    /// Rows ask `thumbnails.state(..)` for what they show; `source` gives each key's revision.
    pub fn thumbnails(mut self, thumbnails: &'a Thumbnails<ThumbKey>, source: &'a dyn ThumbSource<ThumbKey>) -> Self {
        self.thumbnails = Some((thumbnails, source));
        self
    }

    /// Shows the rows in their own section and reports what the user did.
    pub fn show(self, ui: &mut Ui) -> StartOutput {
        let RecentList { recent, max_recent, empty, now, thumbnails } = self;
        let env = Env::of(ui);
        let words = env.words::<ShellWords>();
        let actions = RefCell::new(Vec::new());
        let page = Page { env: &env, words: &words, thumbnails, drop_hint: None, now, actions: &actions };
        let files: Vec<RecentFile> = recent.iter().take(max_recent).cloned().collect();
        let empty = empty.unwrap_or_else(|| words.no_recent.clone());
        let shown = Section::fixed(words.recent.clone())
            .header_gap(Gap::Section)
            .command_scope(StartRecentCommands::SCOPE)
            .show(ui, |ui, cx| recent_body(&page, ui, cx, &files, RecentView::List, &empty));
        StartOutput { response: shown.output.response, actions: actions.into_inner() }
    }
}

/// The body of a recent section: the commands of the recent files, the rows as a grid or a list, the events as
/// actions and the context menu. `empty` is the text of the no-files state when the page has no drop card.
pub(super) fn recent_body(page: &Page<'_>, ui: &mut Ui, cx: &mut BodyCx<'_>, files: &[RecentFile], current: RecentView, empty: &Label) {
    let words = page.words;
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
                let _ = Notice::new(Tone::Neutral, empty.clone()).layout(NoticeLayout::Centred).show(ui);
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
}
