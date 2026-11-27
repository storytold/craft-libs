//! [`TextStylesPanel`]: the Character Styles and Paragraph Styles panels. A list of the document's styles with
//! the one at the type selection marked ("+" after its name when the text overrides it), and Photoshop's footer:
//! Clear Override and Redefine, then Style Options, Duplicate, New and Delete. The styles, their attributes and the
//! commands are the app's; the panel reports what the user asked for.

use craft_behavior::{IconId, Selection, keyed};
use craft_theme::variant::Tone;
use craft_ui::bars::StatusMessage;
use craft_ui::contract::{
    ActivateOn, Bind, CollectionEvent, CommandSet, Commands as _, ListDrag, NoValue, Outcome, Pad, Props as _, Remember as _, SelectMode, Show as _,
    ShowCollection as _, ShowContainer as _,
};
use craft_ui::display::Divider;
use craft_ui::list::{List, ListRow, RowInk};
use craft_ui::menu::{Menu, MenuItem, PanelRow};
use craft_ui::runtime::commands::CommandState;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::sections::{FooterAction, FooterBar, Panel};
use egui::{Response, Ui};
use std::cell::{Cell, RefCell};

mod picker;
#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use picker::{NamedStyleRow, StylePreview, style_source};

craft_ui::words! {
    /// The panels' words, English by default.
    pub struct TextStylesWords {
        no_character_style: "textStyles.noCharacterStyle" = "No character style";
        row_tip: template "textStyles.rowTip" = "Click to apply, {key}-click to clear overrides, double-click for Style Options";
        alt_key: "textStyles.altKey" = "Alt";
        select_text: "textStyles.selectText" = "Select text to apply a character style";
        clear_override: "textStyles.clearOverride" = "Clear Override";
        redefine: "textStyles.redefine" = "Redefine style by current selection";
        options: "textStyles.options" = "Style Options…";
        duplicate: "textStyles.duplicate" = "Duplicate style";
        new_style: "textStyles.new" = "Create new style (from the selected type)";
        delete: "textStyles.delete" = "Delete style";
        /// The row menu's accessible name.
        row_menu: "textStyles.rowMenu" = "Style";
        menu_duplicate: "textStyles.menuDuplicate" = "Duplicate Style";
        menu_delete: "textStyles.menuDelete" = "Delete Style";
        menu_redefine: "textStyles.menuRedefine" = "Redefine Style";
        overridden_name: template "textStyles.overriddenName" = "{name}+";
        overridden_spoken: template "textStyles.overriddenSpoken" = "{name}, overridden";
    }
}

/// Which panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextStyleKind {
    /// The Character Styles panel.
    Character,
    /// The Paragraph Styles panel.
    Paragraph,
}

/// A style in the list. Id 0 is the base style: "None" for character styles (shown dimmed, never deleted,
/// redefined or edited) and "Basic Paragraph" for paragraph styles (editable, never deleted). With
/// [`TextStylesPanel::base_character_style`], character style 0 is editable and never deleted as well.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextStyleRow {
    /// The style's id; 0 is the base style.
    pub id: u64,
    /// The style's name.
    pub name: String,
}

impl TextStyleRow {
    /// The style `name` with the id `id`.
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        TextStyleRow { id, name: name.into() }
    }
}

/// The panel's commands. They run on the panel that holds the focus, from its panel menu, from a chord the keymap
/// binds, or from the palette; the panel answers with the same [`TextStyleAction`] its footer buttons give.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "textStyles", scope = "textStyles")]
#[non_exhaustive]
pub enum TextStyleCommand {
    /// Clear the overrides of the style at the type target.
    #[command(label = "Clear Override")]
    ClearOverride,
    /// The current style takes the target's attributes.
    #[command(label = "Redefine Style")]
    Redefine,
    /// A style from the selected type.
    #[command(label = "New Style")]
    New,
    /// Duplicate the selected styles.
    #[command(label = "Duplicate Style")]
    Duplicate,
    /// Delete the selected styles.
    #[command(label = "Delete Style")]
    Delete,
    /// Open the options of the selected style.
    #[command(label = "Style Options…")]
    Options,
    /// Highlight the styles no text uses.
    #[command(label = "Select All Unused")]
    SelectUnused,
}

/// What the user asked for this pass. The app runs the command on its document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextStyleAction {
    /// A click on a row with a type target: apply the style; `clear_overrides` with Alt held. For character styles
    /// only with text selected; at a bare caret the panel reports [`TextStylesPanelOutput::status`] instead.
    #[non_exhaustive]
    Apply {
        /// The id of the style to apply.
        id: u64,
        /// Whether to clear the local overrides (Alt held).
        clear_overrides: bool,
    },
    /// Clear Override, with a target whose style is overridden.
    ClearOverride,
    /// Redefine: the style of this id takes the target's attributes: the current style from the footer and the panel
    /// menu, the row's from its menu. Not for character style 0 unless it is a base style
    /// ([`TextStylesPanel::base_character_style`]).
    Redefine(u64),
    /// New: a style from the selected type when there is a target (`from_selection`), else a default one. The app
    /// opens its Style Options for the new id.
    #[non_exhaustive]
    New {
        /// Whether to start from the selected type's attributes.
        from_selection: bool,
        /// Whether Alt was held on the New footer button: the host asks for the new style's name.
        ask_name: bool,
    },
    /// Duplicate the selected row's style.
    Duplicate(u64),
    /// Delete the selected row's style (never id 0).
    Delete(u64),
    /// Style Options for a style: a double click or Enter on a row, or the footer button (not for character style 0
    /// unless it is a base style, [`TextStylesPanel::base_character_style`]).
    Options(u64),
    /// Duplicate with several rows highlighted: every highlighted style, in list order.
    DuplicateHighlighted,
    /// Delete with several rows highlighted: every highlighted style but id 0.
    DeleteHighlighted,
    /// The panel menu's Select All Unused: the app highlights the styles no text uses.
    SelectUnused,
}

impl TextStyleAction {
    /// An Apply of style `id`.
    pub const fn apply(id: u64, clear_overrides: bool) -> Self {
        TextStyleAction::Apply { id, clear_overrides }
    }

    /// A New: a style from the selected type when `from_selection`, else a default one; with `ask_name` the host asks
    /// for the new style's name.
    pub const fn new_style(from_selection: bool, ask_name: bool) -> Self {
        TextStyleAction::New { from_selection, ask_name }
    }
}

/// The panel's result.
#[derive(Debug)]
#[non_exhaustive]
pub struct TextStylesPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// What the user asked for this pass.
    pub action: Option<TextStyleAction>,
    /// A warning for the status bar ("Select text to apply a character style").
    pub status: Option<StatusMessage>,
}

impl Outcome for TextStylesPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What the panel knows of the last pass: the rules the footer, the panel menu and the keys share.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Facts {
    /// Whether style 0 is a base style the user edits: paragraph style 0, or character style 0 with
    /// [`TextStylesPanel::base_character_style`].
    base: bool,
    target: bool,
    overridden: bool,
    current: Option<u64>,
    /// The row the footer acts on: the lead of the selection, else the current style.
    picked: Option<u64>,
    /// The selected rows; Duplicate and Delete act on all of them from the second on.
    selected: Vec<u64>,
}

impl Facts {
    /// The action of a footer button or menu command, `None` when its condition doesn't hold.
    fn action(&self, command: TextStyleCommand) -> Option<TextStyleAction> {
        let editable = self.picked.filter(|id| *id != 0 || self.base);
        let several = self.selected.len() > 1;
        match command {
            TextStyleCommand::ClearOverride => (self.target && self.overridden).then_some(TextStyleAction::ClearOverride),
            TextStyleCommand::Redefine => self.current.filter(|id| self.target && (*id != 0 || self.base)).map(TextStyleAction::Redefine),
            TextStyleCommand::New => Some(TextStyleAction::new_style(self.target, false)),
            TextStyleCommand::Duplicate if several => Some(TextStyleAction::DuplicateHighlighted),
            TextStyleCommand::Delete if several => Some(TextStyleAction::DeleteHighlighted),
            TextStyleCommand::Duplicate => editable.map(TextStyleAction::Duplicate),
            TextStyleCommand::Delete => editable.filter(|id| *id != 0).map(TextStyleAction::Delete),
            TextStyleCommand::Options => editable.map(TextStyleAction::Options),
            TextStyleCommand::SelectUnused => Some(TextStyleAction::SelectUnused),
        }
    }
}

/// The row menu's commands, in its two groups.
const ROW_MENU: [&[TextStyleCommand]; 2] = [
    &[TextStyleCommand::Options, TextStyleCommand::Duplicate, TextStyleCommand::Delete],
    &[TextStyleCommand::Redefine, TextStyleCommand::ClearOverride],
];

impl Facts {
    /// The action of the row menu's `command` on the style `row`, which the right-click made the lead of `selected`;
    /// `None` when its condition doesn't hold. Redefine redefines the row's style, and Clear Override applies it without
    /// the overrides where a click would apply it (`applies`); the rest act as the footer's.
    fn row_action(&self, row: u64, command: TextStyleCommand, applies: bool) -> Option<TextStyleAction> {
        match command {
            TextStyleCommand::ClearOverride => applies.then_some(TextStyleAction::apply(row, true)),
            command => Facts { current: Some(row), picked: Some(row), ..self.clone() }.action(command),
        }
    }
}

/// The row whose menu the last context request opened.
#[derive(Clone, Debug, Default)]
struct RowMenu(Option<u64>);

/// The footer's buttons: the start group from the start edge, the rest from the end edge.
const FOOTER: [TextStyleCommand; 6] = [
    TextStyleCommand::ClearOverride,
    TextStyleCommand::Redefine,
    TextStyleCommand::Delete,
    TextStyleCommand::New,
    TextStyleCommand::Duplicate,
    TextStyleCommand::Options,
];

/// How many of the footer's buttons sit at its start edge: Clear Override and Redefine.
const START_ACTIONS: usize = 2;

/// One styles panel, built per pass:
///
/// ```ignore
/// let out = TextStylesPanel::new("Paragraph Styles", TextStyleKind::Paragraph, &rows, Bind::bound(&mut selected))
///     .current(at_target.paragraph, at_target.paragraph_override)
///     .target(has_type_target)
///     .show(ui);
/// if let Some(action) = out.action { run_style_command(action) }
/// ```
///
/// - Rows: the style at the type target has the hover look; the selected rows (the footer buttons act on the lead)
///   have the selection fill; "+" follows the current name while the text overrides it.
/// - Click: [`TextStyleAction::Apply`] (Alt-click clears overrides). Double click or Enter:
///   [`TextStyleAction::Options`]. The row's description says so.
/// - Keys: the list is one Tab stop; the arrows move the selection, Delete or Backspace deletes it (not id 0).
/// - With [`highlighting`](Self::highlighting), Shift-click selects a range and Ctrl or Cmd-click adds or removes a
///   row without applying. A row dragged onto New or Delete carries every selected style when it is one of them,
///   else itself.
/// - Right-click: selects the row (unless it is selected) and opens its menu: Style Options…, Duplicate Style and Delete
///   Style act as the footer's with that row as the lead; Redefine Style redefines the row's style, and Clear Override
///   applies it without the overrides.
/// - Footer: Clear Override and Redefine at the start edge; Style Options, Duplicate, New and Delete at the end edge,
///   each disabled while its condition doesn't hold. The panel menu ([`TextStylesPanel::panel_rows`]) has the same commands under the same conditions.
/// - Padding: none of its own. The panel is a dock body, docked or floating, and the dock group insets it.
#[must_use = "show the panel with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct TextStylesPanel<'a> {
    label: Label,
    kind: TextStyleKind,
    rows: &'a [TextStyleRow],
    selection: Bind<'a, Selection<u64>>,
    current: Option<u64>,
    overridden: bool,
    /// Whether there is type to apply styles to (a type layer selected or being edited).
    #[prop]
    target: bool,
    /// Whether the type target has text selected; at a bare caret a click on a character style sets
    /// [`TextStylesPanelOutput::status`] instead of applying.
    #[prop]
    text_selected: bool,
    /// Several rows select together: the footer's Duplicate and Delete act on all of them.
    #[prop]
    highlighting: bool,
    /// Character style 0 is a base style the user edits instead of "No character style": it takes Style Options,
    /// Duplicate and Redefine as paragraph style 0 does, is drawn like the other rows, and is still never deleted.
    #[prop]
    base_character_style: bool,
    /// The list's text with no rows, passed to the collection's `empty` prop.
    #[prop(into)]
    empty: Option<Label>,
    /// Scrolls the row of this id into view in this pass.
    #[prop]
    reveal: Option<u64>,
}

impl TextStylesPanel<'_> {
    /// The panel menu's rows for the docking host's `DockPanels::panel_rows`: Clear Override, Redefine Style, a
    /// separator, New Style, Duplicate Style, Delete Style, Style Options… and Select All Unused. Each row's enabled
    /// state is the one the panel publishes every pass from the facts its footer uses.
    pub fn panel_rows() -> Vec<PanelRow> {
        let command = |command: TextStyleCommand| PanelRow::Command(command.into());
        vec![
            command(TextStyleCommand::ClearOverride),
            command(TextStyleCommand::Redefine),
            PanelRow::Separator,
            command(TextStyleCommand::New),
            command(TextStyleCommand::Duplicate),
            command(TextStyleCommand::Delete),
            command(TextStyleCommand::Options),
            command(TextStyleCommand::SelectUnused),
        ]
    }
}

impl<'a> TextStylesPanel<'a> {
    /// The panel named `label` over `rows`; `selection` is the rows selected in the panel, kept by the app. Until the
    /// user picks one, the footer acts on the style at the type target.
    pub fn new(label: impl Into<Label>, kind: TextStyleKind, rows: &'a [TextStyleRow], selection: Bind<'a, Selection<u64>>) -> Self {
        TextStylesPanel {
            label: label.into(),
            kind,
            rows,
            selection,
            current: None,
            overridden: false,
            target: false,
            text_selected: true,
            highlighting: false,
            base_character_style: false,
            empty: None,
            reveal: None,
        }
    }

    /// The style at the type target (the selection's start, or the edited layer) and whether the text overrides it.
    pub fn current(mut self, id: Option<u64>, overridden: bool) -> Self {
        self.current = id;
        self.overridden = overridden;
        self
    }

    /// Shows the panel and returns what the user asked for.
    pub fn show(self, ui: &mut Ui) -> TextStylesPanelOutput {
        let TextStylesPanel {
            label,
            kind,
            rows,
            mut selection,
            current,
            overridden,
            target,
            text_selected,
            highlighting,
            base_character_style,
            empty,
            reveal,
        } = self;

        let words = Env::of(ui).words::<TextStylesWords>();
        let paragraph = kind == TextStyleKind::Paragraph;
        let base = paragraph || base_character_style;
        let action = Cell::new(None::<TextStyleAction>);
        let hint = Cell::new(false);
        let tip = words.row_tip.fill(&[("key", words.alt_key.as_str())]);
        // The footer bar is a container of its own, so the list's id comes from here, not from either body.
        let list_id = ui.scope_id().with(("styles", kind, label.as_str()));

        // The body runs before the footer and hands it this pass's facts: the footer bar is a container of its own, so
        // its memory is not the body's.
        let shown_facts = RefCell::new(Facts::default());
        let footer = |ui: &mut Ui, _: &mut craft_ui::contract::BodyCx<'_>| {
            let facts = shown_facts.borrow().clone();
            let actions = FOOTER.map(|command| {
                let (icon, text) = match command {
                    TextStyleCommand::ClearOverride => ("ban", &words.clear_override),
                    TextStyleCommand::Redefine => ("check", &words.redefine),
                    TextStyleCommand::Options => ("settings", &words.options),
                    TextStyleCommand::Duplicate => ("copy", &words.duplicate),
                    TextStyleCommand::New => ("plus", &words.new_style),
                    _ => ("trash", &words.delete),
                };
                let button = FooterAction::new(IconId::new(icon), text.clone()).enabled(facts.action(command).is_some());
                match command {
                    TextStyleCommand::New => button
                        .takes_drops_disabled(true)
                        .accepts_drops::<ListDrag<u64>>(move |drag| drag.list == list_id && drag.keys.iter().any(|id| *id != 0 || base)),
                    TextStyleCommand::Delete => button
                        .takes_drops_disabled(true)
                        .accepts_drops::<ListDrag<u64>>(move |drag| drag.list == list_id && drag.keys.iter().any(|id| *id != 0)),
                    _ => button,
                }
            });
            let bar = FooterBar::actions(actions).at_start(START_ACTIONS).show_footer(ui);
            if let Some(drop) = &bar.dropped
                && let Some(drag) = drop.payload::<ListDrag<u64>>()
            {
                let many = drag.keys.len() > 1;
                let dropped = match FOOTER.get(drop.index) {
                    Some(TextStyleCommand::Delete) if many => Some(TextStyleAction::DeleteHighlighted),
                    Some(TextStyleCommand::Delete) => (drag.grabbed != 0).then_some(TextStyleAction::Delete(drag.grabbed)),
                    Some(TextStyleCommand::New) if many => Some(TextStyleAction::DuplicateHighlighted),
                    Some(TextStyleCommand::New) => (drag.grabbed != 0 || base).then_some(TextStyleAction::Duplicate(drag.grabbed)),
                    _ => None,
                };
                if dropped.is_some() {
                    action.set(dropped);
                }
            } else if let Some(command) = bar.picked.and_then(|at| FOOTER.get(at))
                && let Some(found) = facts.action(*command)
            {
                action.set(Some(match found {
                    TextStyleAction::New { from_selection, .. } => TextStyleAction::new_style(from_selection, bar.mods.alt),
                    other => other,
                }));
            }
        };

        let shown =
            Panel::new(label.clone()).pad(Pad::None).collection_body().command_scope(TextStyleCommand::SCOPE).footer(footer).show(ui, |ui, cx| {
                let before = cx.memory::<Facts>().clone();
                while let Some((command, _)) = cx.take_command::<TextStyleCommand>() {
                    if let Some(found) = before.action(command) {
                        action.set(Some(found));
                    }
                }
                let held = selection.reborrow().read(cx).and_then(|held| held.same().cloned()).unwrap_or_default();
                let facts = Facts { base, target, overridden, current, picked: held.lead.or(current), selected: held.items.clone() };
                for command in [
                    TextStyleCommand::ClearOverride,
                    TextStyleCommand::Redefine,
                    TextStyleCommand::New,
                    TextStyleCommand::Duplicate,
                    TextStyleCommand::Delete,
                    TextStyleCommand::Options,
                    TextStyleCommand::SelectUnused,
                ] {
                    cx.publish(command, CommandState::new().enabled(facts.action(command).is_some()));
                }
                shown_facts.replace(facts.clone());
                *cx.memory::<Facts>() = facts;

                let source = keyed(rows, |row: &TextStyleRow| row.id);
                let mode = if highlighting { SelectMode::Multi } else { SelectMode::Single };
                let list = List::new(label.clone())
                    .id(list_id)
                    .select(mode)
                    .activate(ActivateOn::Click)
                    .allow_empty(true)
                    .deletable(true)
                    .max_rows(8)
                    .can_drag(move |_| highlighting)
                    .drag_payload(|drag: &ListDrag<u64>| Some(drag.clone()))
                    .selection(selection.reborrowed());
                let list = match reveal {
                    Some(id) => list.reveal(id),
                    None => list,
                };
                let list = match &empty {
                    Some(text) => list.empty(text.clone()),
                    None => list,
                };
                let shown_rows = list.show_items(ui, &source, &mut |id, _| {
                    let Some(row) = rows.iter().find(|row| row.id == *id) else { return ListRow::new(Label::Empty) };
                    let is_current = target && current == Some(*id);
                    let marked = is_current && overridden;
                    let none = *id == 0 && !base;
                    let name = if marked { words.overridden_name.fill(&[("name", row.name.as_str())]) } else { row.name.clone() };
                    let spoken = if marked { words.overridden_spoken.fill(&[("name", row.name.as_str())]) } else { row.name.clone() };
                    let description = if none { words.no_character_style.as_str().to_owned() } else { tip.clone() };
                    let styled = ListRow::new(name).accessible_name(spoken).description(description).highlight(is_current);
                    if none { styled.ink(RowInk::Dimmed) } else { styled }
                });
                let (events, second) = (shown_rows.events, shown_rows.activation_clicks >= 2);
                if let Some(row) = events.iter().rev().find_map(|event| match event {
                    CollectionEvent::ContextMenu(request) => Some(request.key),
                    _ => None,
                }) {
                    cx.memory::<RowMenu>().0 = Some(row);
                }

                let mods = craft_ui::runtime::pointer::mods(ui.ctx());
                let alt = mods.alt;
                let plain = !mods.shift && !mods.cmd;
                for event in events {
                    match event {
                        CollectionEvent::Activate(id) if second && (id != 0 || base) => action.set(Some(TextStyleAction::Options(id))),
                        CollectionEvent::Activate(id) if target && (paragraph || text_selected) && plain => {
                            action.set(Some(TextStyleAction::apply(id, alt)));
                        }
                        // A character style at a bare caret has nothing to apply to: the attempt warns instead.
                        CollectionEvent::Activate(_) if target && !paragraph && !text_selected && plain => hint.set(true),
                        CollectionEvent::Delete(keys) if keys.len() > 1 => action.set(Some(TextStyleAction::DeleteHighlighted)),
                        CollectionEvent::Delete(keys) => {
                            if let Some(id) = keys.first().copied().filter(|id| *id != 0) {
                                action.set(Some(TextStyleAction::Delete(id)));
                            }
                        }
                        _ => {}
                    }
                }

                if let Some(row) = cx.memory::<RowMenu>().0.filter(|row| rows.iter().any(|shown| shown.id == *row)) {
                    let selected = selection.reborrow().read(cx).and_then(|held| held.same().map(|held| held.items.clone())).unwrap_or_default();
                    let facts = Facts { selected, ..cx.memory::<Facts>().clone() };
                    let applies = target && (paragraph || text_selected);
                    let label = |command: TextStyleCommand| match command {
                        TextStyleCommand::Options => &words.options,
                        TextStyleCommand::Duplicate => &words.menu_duplicate,
                        TextStyleCommand::Delete => &words.menu_delete,
                        TextStyleCommand::Redefine => &words.menu_redefine,
                        _ => &words.clear_override,
                    };
                    let menu = Menu::context(words.row_menu.clone(), &shown_rows.response).show(ui, |ui, _| {
                        let mut ran = None;
                        for (at, group) in ROW_MENU.into_iter().enumerate() {
                            if at > 0 {
                                Divider::horizontal().show(ui);
                            }
                            for &command in group {
                                let found = facts.row_action(row, command, applies);
                                if MenuItem::new(label(command).clone()).enabled(found.is_some()).show(ui).ran {
                                    ran = found;
                                }
                            }
                        }
                        ran
                    });
                    if let Some(found) = menu.inner.flatten() {
                        action.set(Some(found));
                    }
                }
            });
        TextStylesPanelOutput {
            response: shown.output.response,
            action: action.get(),
            status: hint.get().then(|| StatusMessage::new(words.select_text.clone()).tone(Tone::Warn)),
        }
    }
}
