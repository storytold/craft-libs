//! `ChannelsPanel`: the list over the app's rows, the footer, the menus and the events.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt::Debug;
use std::hash::Hash;

use craft_behavior::geometry::selection_op::SelectionOperation;
use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, EditEvent, IconId, ItemSource, Selection, SelectionChange};
use craft_ui::contract::{
    Bind, CollectionEvent, ColumnKind, ColumnSpec, ListDrag, Props as _, ReorderRule, ShowCollection as _, ShowContainer as _, SweepApply,
    ToggleColumn, request_rename,
};
use craft_ui::layout_components::Stack;
use craft_ui::list::{List, RowEvent, ThumbSize, ThumbTarget};
use craft_ui::primitives::textures::{ThumbSource, Thumbnails};
use craft_ui::runtime::commands::{CommandState, Commands as _};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::memory::Remember as _;
use egui::Ui;

use super::footer::{ChannelDrag, show_footer};
use super::labels::{ChannelsWords, shortcut_numbers};
use super::menu::{ChannelsCommands, lands, show_row_menu};
use super::model::{ChannelKind, ChannelRow, ChannelsEvent, ChannelsOutput, targeted_row};
use super::rows::{EYE, NAME, RowContext, ThumbHost, channel_row};

/// What the panel keeps between passes, in its root slot: the row the last context request opened the menu on.
#[derive(Clone)]
struct Session<K> {
    menu: Option<K>,
    /// The reader of the eye sweeps; Photoshop's channels have no Alt isolate, so it is plain.
    eyes: SweepApply,
}

impl<K> Default for Session<K> {
    fn default() -> Self {
        Session { menu: None, eyes: SweepApply::new() }
    }
}

/// The rows as the list reads them: flat, in the app's order.
struct Order<'a, K> {
    rows: &'a [ChannelRow<K>],
}

impl<K: Clone> DisplayOrder<K> for Order<'_, K> {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> K {
        self.rows[at].key.clone()
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.rows[at].name.as_str()))
    }
}

impl<K: Clone> ItemSource<K> for Order<'_, K> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        *column == EYE && self.rows[at].visible
    }
}

/// The Channels panel over the app's rows: composite, colour, layer-mask, alpha and spot channels and Quick Mask,
/// with thumbnails, eyes, the footer and the menus. It holds no document; what the user did comes back as
/// [`ChannelsEvent`]s.
///
/// ```ignore
/// let rows = channel_rows(&doc, &view);                     // the app, once per pass
/// let out = ChannelsPanel::new(&rows).thumbnails(&thumbs, &DocChannelThumbs { doc }).show(ui);
/// for event in out.events {
///     match event {
///         ChannelsEvent::Target(key) => app.run("channel.target", key),
///         ChannelsEvent::LoadSelection { key, operation } => app.load_selection(key, operation.name()),
///         _ => {}
///     }
/// }
/// ```
///
/// - **Layout**: the rows fill the panel's height above the footer: a [`List`] of `ListRow`s in the layer form.
/// - **Selection**: the list shows the rows the app marks `targeted` as selected (several while the composite is
///   targeted), over a selection rebuilt from the rows each pass; its own changes are not kept. A click or Shift-click
///   reports [`Target`](ChannelsEvent::Target); Cmd/Ctrl-click on the thumbnail reports
///   [`LoadSelection`](ChannelsEvent::LoadSelection) with the operation Shift and Alt ask for; ↑ ↓ report `Target`
///   for the new lead; the Load Selection command (Cmd/Ctrl+Enter) loads the lead.
/// - **Eyes**: the list's toggle column; a click or a sweep reports [`Visibility`](ChannelsEvent::Visibility) steps, one per eye crossed;
///   Escape restores them; the eye never targets.
/// - **Double click and Enter**: on the thumbnail of a row with options, or Enter, [`Options`](ChannelsEvent::Options);
///   on the name of an alpha or spot row, or F2, rename in place; an empty or unchanged name reports nothing, else
///   [`Rename`](ChannelsEvent::Rename).
/// - **Drag**: alpha and spot rows reorder, before or after another one or before Quick Mask
///   ([`Reorder`](ChannelsEvent::Reorder); Cmd/Ctrl+] and [ too). Any row drops on the footer's New and Load buttons,
///   and an alpha or spot row on Delete; a drop acts on the row grabbed.
/// - **Menus**: a right-click or the Menu key opens the row's context menu; [`panel_rows`](super::panel_rows) are the
///   panel menu's rows.
/// - **Right to left**: the list and the footer mirror; the chord sits at the trailing end.
#[must_use = "show the panel with `show`"]
#[derive(craft_ui::contract::Pattern)]
#[pattern(probe(ChannelsPanel<'static, u32>))]
pub struct ChannelsPanel<'a, K: Clone + Eq + Hash + Debug + Send + Sync + 'static> {
    rows: &'a [ChannelRow<K>],
    thumbnails: Option<ThumbHost<'a, K>>,
    thumbnail_size: ThumbSize,
    rename: Option<K>,
}

impl<'a, K: Clone + Eq + Hash + Debug + Send + Sync + 'static> ChannelsPanel<'a, K> {
    /// The panel showing `rows` in their order.
    pub fn new(rows: &'a [ChannelRow<K>]) -> Self {
        Self { rows, thumbnails: None, thumbnail_size: ThumbSize::Medium, rename: None }
    }

    /// The thumbnail cache and the app's source of the channels' pictures, keyed by the rows' keys. Without it the
    /// rows draw no pictures.
    pub fn thumbnails(mut self, thumbnails: &'a Thumbnails<K>, source: &'a dyn ThumbSource<K>) -> Self {
        self.thumbnails = Some(ThumbHost { thumbnails, source });
        self
    }

    /// The user's Panel Options thumbnail size (Photoshop's): `ThumbSize::None` hides the thumbnails. The default is
    /// `Medium`.
    pub fn thumbnail_size(mut self, size: ThumbSize) -> Self {
        self.thumbnail_size = size;
        self
    }

    /// Opens the rename field of the row `key` in this pass, as Rename Channel… in a menu does; the app calls it in
    /// the pass it handles its own rename command. Rows that don't rename (composite, colour, Quick Mask, layer mask)
    /// ignore it.
    pub fn start_rename(mut self, key: K) -> Self {
        self.rename = Some(key);
        self
    }

    /// Draws the panel over the available rect.
    pub fn show(self, ui: &mut Ui) -> ChannelsOutput<K> {
        let Self { rows, thumbnails, thumbnail_size, rename } = self;
        let env = Env::of(ui);
        let words = env.words::<ChannelsWords>();
        let list = ui.scope_id().with("channels-rows");
        let events: RefCell<Vec<ChannelsEvent<K>>> = RefCell::new(Vec::new());
        let ctx = ui.ctx().clone();
        let row_cx = RowContext { rows, numbers: shortcut_numbers(rows), thumbs: thumbnails, size: thumbnail_size, words: &words };

        let shown = Stack::new()
            .fill()
            .label(words.list.clone())
            .command_scope(<ChannelsCommands as craft_ui::contract::CommandSet>::SCOPE)
            .footer(|ui, _| {
                let out = show_footer(ui, rows, &words, list);
                events.borrow_mut().extend(out);
            })
            .show(ui, |ui, stack| {
                let mods = craft_ui::runtime::pointer::frame(&ctx).mods;
                while let Some((command, _)) = stack.take_command::<ChannelsCommands>() {
                    match (command, targeted_row(rows)) {
                        (ChannelsCommands::RenameChannel, Some(row)) if row.kind.renames() => request_rename(&ctx, list, &row.key),
                        _ => events.borrow_mut().extend(command.event(rows, mods)),
                    }
                }
                let kind = targeted_row(rows).map(|row| row.kind);
                for command in ChannelsCommands::ALL {
                    stack.publish(command, CommandState::new().enabled(command.applies_to(kind)));
                }
                if let Some(key) = rename.clone() {
                    request_rename(&ctx, list, &key);
                }

                let order = Order { rows };
                let lead = targeted_row(rows).map(|row| row.key.clone());
                let mut selection = Selection::new(rows.iter().filter(|row| row.targeted).map(|row| row.key.clone()).collect(), lead.clone(), lead);
                let kinds: Vec<(K, ChannelKind)> = rows.iter().map(|row| (row.key.clone(), row.kind)).collect();
                let payload_kinds = kinds.clone();
                let kind_of = |key: &K| kinds.iter().find(|(k, _)| k == key).map(|(_, kind)| *kind);
                let columns = vec![
                    ColumnSpec::new(EYE, Label::Empty, ColumnWidth::Controls(1.0))
                        .kind(ColumnKind::Toggle(ToggleColumn::sweepable()))
                        .icon(IconId::new("eye")),
                    ColumnSpec::new(NAME, Label::Empty, ColumnWidth::fill(1.0)),
                ];
                let out = List::new(words.list.clone())
                    .id(list)
                    .reorder(ReorderRule::new())
                    .allow_empty(false)
                    .rename(true)
                    .rename_column(NAME)
                    .deletable(true)
                    .columns(columns)
                    .selection(Bind::bound(&mut selection))
                    .can_rename(|key| kind_of(key).is_some_and(ChannelKind::renames))
                    .accepts(|drag: &ListDrag<K>, _over, insert| {
                        if !drag.keys.iter().all(|key| kind_of(key).is_some_and(ChannelKind::moves)) {
                            return false;
                        }
                        let moved_before = kinds[..insert.min(kinds.len())].iter().filter(|(key, _)| drag.keys.contains(key)).count();
                        let remaining: Vec<ChannelKind> = kinds.iter().filter(|(key, _)| !drag.keys.contains(key)).map(|(_, kind)| *kind).collect();
                        lands(&remaining, insert.saturating_sub(moved_before))
                    })
                    .drag_payload(move |drag: &ListDrag<K>| {
                        let kind = payload_kinds.iter().find(|(key, _)| *key == drag.grabbed).map(|(_, kind)| *kind)?;
                        Some(ChannelDrag { list: drag.list, grabbed: drag.grabbed.clone(), kind })
                    })
                    .show_items(ui, &order, &mut |_key, item| {
                        let at = item.at;
                        channel_row(&row_cx, at).cells(item.cells()).state(item.row)
                    });

                let mut found = Vec::new();
                map_events(rows, out.events, &mut found, stack.memory::<Session<K>>());
                events.borrow_mut().extend(found);

                let open = stack.memory::<Session<K>>().menu.clone();
                if let Some(row) = open.and_then(|key| rows.iter().find(|row| row.key == key))
                    && let Some(command) = show_row_menu(ui, &out.response, row, &words)
                {
                    match command {
                        ChannelsCommands::RenameChannel => request_rename(&ctx, list, &row.key),
                        ChannelsCommands::ChannelOptions => events.borrow_mut().push(ChannelsEvent::Options(row.key.clone())),
                        command => events.borrow_mut().push(ChannelsEvent::Command { command, key: command_key(command).then(|| row.key.clone()) }),
                    }
                }
                out.response
            });
        ChannelsOutput { events: events.into_inner(), response: shown.output }
    }
}

fn command_key(command: ChannelsCommands) -> bool {
    !matches!(
        command,
        ChannelsCommands::NewChannel | ChannelsCommands::NewSpotChannel | ChannelsCommands::SplitChannels | ChannelsCommands::MergeChannels
    )
}

/// What the list's events mean for the app: targets, loads, eyes, renames, options, moves and deletes.
fn map_events<K: Clone + PartialEq>(
    rows: &[ChannelRow<K>],
    list: Vec<CollectionEvent<K, RowEvent>>,
    events: &mut Vec<ChannelsEvent<K>>,
    session: &mut Session<K>,
) {
    let kind_of = |key: &K| rows.iter().find(|row| &row.key == key).map(|row| row.kind);
    for event in list {
        match event {
            CollectionEvent::Selection(SelectionChange::Set(keys)) => {
                if let Some(key) = keys.last() {
                    events.push(ChannelsEvent::Target(key.clone()));
                }
            }
            CollectionEvent::Item(key, RowEvent::ThumbClicked { target: ThumbTarget::First, mods, .. }) if mods.cmd => {
                events.push(ChannelsEvent::LoadSelection { key, operation: SelectionOperation::from_mods(mods) });
            }
            CollectionEvent::Item(key, RowEvent::ThumbClicked { clicks: 2, .. }) if kind_of(&key).is_some_and(ChannelKind::has_options) => {
                events.push(ChannelsEvent::Options(key));
            }
            CollectionEvent::Activate(key) if kind_of(&key).is_some_and(ChannelKind::has_options) => events.push(ChannelsEvent::Options(key)),
            CollectionEvent::Toggle(edit) => events.extend(session.eyes.feed(&edit).map(ChannelsEvent::Visibility)),
            CollectionEvent::Rename(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => {
                let name = after.name.trim();
                if !name.is_empty() && rows.iter().find(|row| row.key == after.key).is_some_and(|row| row.name != name) {
                    events.push(ChannelsEvent::Rename { key: after.key.clone(), text: name.to_owned() });
                }
            }
            CollectionEvent::Reorder(edit) => events.push(ChannelsEvent::Reorder(edit)),
            CollectionEvent::Delete(keys) => {
                for key in keys.into_iter().filter(|key| kind_of(key).is_some_and(ChannelKind::moves)) {
                    events.push(ChannelsEvent::Command { command: ChannelsCommands::Delete, key: Some(key) });
                }
            }
            CollectionEvent::ContextMenu(request) => session.menu = Some(request.key),
            _ => {}
        }
    }
}
