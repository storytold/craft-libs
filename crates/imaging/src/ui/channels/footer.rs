//! The footer buttons and their drops (`ChannelsFooterButton`).

use std::fmt::Debug;
use std::hash::Hash;

use craft_behavior::IconId;
use craft_ui::primitives::IconSpec;
use craft_ui::sections::{FooterAction, FooterBar};
use egui::{Id, Ui};

use super::labels::ChannelsWords;
use super::model::{ChannelKind, ChannelRow, ChannelsEvent, targeted_for_delete, targeted_row};

/// A button of the Channels footer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ChannelsFooterButton {
    /// Delete the targeted channel when it can be deleted.
    Delete,
    /// Create new channel; a row dropped on it is duplicated.
    New,
    /// Save selection as channel; takes no drops.
    SaveSelection,
    /// Load channel as selection; a row dropped on it is loaded.
    LoadSelection,
}

/// What a row drag carries for the footer's buttons: the list it started in, the row the pointer grabbed and that
/// row's deletion capabilities.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ChannelDrag<K> {
    pub(super) list: Id,
    pub(super) grabbed: K,
    pub(super) kind: ChannelKind,
    pub(super) can_delete_color: bool,
}

/// Delete takes saved channels, a layer mask or an opted-in process colour. New and Load take
/// every row. Save takes none.
fn accepts_footer_drop(button: ChannelsFooterButton, kind: ChannelKind, can_delete_color: bool) -> bool {
    match button {
        ChannelsFooterButton::Delete => can_delete_color || !matches!(kind, ChannelKind::Composite | ChannelKind::Color | ChannelKind::QuickMask(_)),
        ChannelsFooterButton::New | ChannelsFooterButton::LoadSelection => true,
        ChannelsFooterButton::SaveSelection => false,
    }
}

// Photoshop's order from the end edge.
const DELETE: usize = 0;
const NEW: usize = 1;
const SAVE: usize = 2;
const LOAD: usize = 3;

fn button_at(at: usize) -> Option<ChannelsFooterButton> {
    match at {
        LOAD => Some(ChannelsFooterButton::LoadSelection),
        SAVE => Some(ChannelsFooterButton::SaveSelection),
        NEW => Some(ChannelsFooterButton::New),
        DELETE => Some(ChannelsFooterButton::Delete),
        _ => None,
    }
}

/// Shows the footer and returns its events in order.
///
/// - Buttons from the leading end: Load channel as selection, Save selection as channel, New, Delete. Delete is
///   disabled for a click unless the target is deletable. A process colour must be the only process target.
/// - A click reports [`ChannelsEvent::Footer`] with `rows` = [`targeted_row`] and the modifiers held.
/// - Delete, New and Load take a dragged row of the panel's list: a drop reports [`ChannelsEvent::FooterDrop`] with
///   the row the pointer grabbed. A button shows the drop outline, and Delete reads enabled, while the grabbed row is
///   one it accepts; otherwise it shows nothing and reports nothing.
pub(super) fn show_footer<K: Clone + Eq + Hash + Debug + Send + Sync + 'static>(
    ui: &mut Ui,
    rows: &[ChannelRow<K>],
    words: &ChannelsWords,
    list: Id,
) -> Vec<ChannelsEvent<K>> {
    let target = targeted_row(rows);
    let delete_target = targeted_for_delete(rows);
    let delete_enabled = delete_target.is_some_and(|row| accepts_footer_drop(ChannelsFooterButton::Delete, row.kind, row.can_delete_color()));
    let takes = |button: ChannelsFooterButton| {
        move |drag: &ChannelDrag<K>| drag.list == list && accepts_footer_drop(button, drag.kind, drag.can_delete_color)
    };
    let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
    let actions = [
        FooterAction::new(icon("trash"), words.delete.clone())
            .enabled(delete_enabled)
            .takes_drops_disabled(true)
            .accepts_drops(takes(ChannelsFooterButton::Delete)),
        FooterAction::new(icon("plus"), words.new.clone()).accepts_drops(takes(ChannelsFooterButton::New)),
        FooterAction::new(icon("rect_dashed"), words.save_selection.clone()),
        FooterAction::new(icon("ellipse_dashed"), words.load_selection.clone()).accepts_drops(takes(ChannelsFooterButton::LoadSelection)),
    ];
    let out = FooterBar::actions(actions).show_footer(ui);
    let mut events = Vec::new();
    if let Some(button) = out.picked.and_then(button_at) {
        let target = if button == ChannelsFooterButton::Delete { delete_target } else { target };
        events.push(ChannelsEvent::Footer { button, rows: target.map(|row| row.key.clone()).into_iter().collect(), mods: out.mods });
    }
    if let Some(dropped) = &out.dropped
        && let (Some(button), Some(drag)) = (button_at(dropped.index), dropped.payload::<ChannelDrag<K>>())
    {
        events.push(ChannelsEvent::FooterDrop { button, rows: vec![drag.grabbed.clone()] });
    }
    events
}
