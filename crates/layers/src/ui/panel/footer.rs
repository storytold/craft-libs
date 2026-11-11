//! The footer: `LayersFooterButton`, `LayersFooterAction`, the press menus and the footer drops.

use craft_behavior::IconId;
use craft_ui::contract::{PopupKind, Trigger};
use craft_ui::menu::Menu;
use craft_ui::primitives::IconSpec;
use craft_ui::sections::{FooterAction, FooterBar};
use egui::Ui;

use super::LayersPanelWords;
use super::context::PanelContext;
use super::drag::LayersDrag;
use super::events::LayersEvent;
use crate::ui::tree::{LayerRowKey, LayerTree};

/// The footer buttons that take dropped rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LayersFooterButton {
    /// The trash.
    Delete,
    /// New Layer.
    NewLayer,
    /// New Group.
    NewGroup,
}

/// A press on a footer button, with the modifiers held as Photoshop reads them.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayersFooterAction {
    /// Link the selected layers.
    Link,
    /// New Layer: `ask` (Alt) opens the New Layer dialog, `below` (Cmd/Ctrl) adds below the active layer.
    #[non_exhaustive]
    NewLayer {
        /// Ask for the options first.
        ask: bool,
        /// Add below the active layer.
        below: bool,
    },
    /// New Group: `from_selection` while several layers are selected or Shift is held.
    #[non_exhaustive]
    NewGroup {
        /// Group the selected layers.
        from_selection: bool,
        /// Ask for the options first.
        ask: bool,
        /// Add below the active layer.
        below: bool,
    },
    /// Delete the selected layers; `confirm` unless Alt is held.
    #[non_exhaustive]
    Delete {
        /// Ask before deleting.
        confirm: bool,
    },
    /// Add a mask to the active layer; `invert` (Alt) hides everything.
    #[non_exhaustive]
    AddMask {
        /// The mask starts black.
        invert: bool,
    },
    /// A pick in the adjustment menu.
    Adjustment(String),
    /// A pick in the layer style menu.
    Style(String),
}

// Reading order of the footer's buttons.
const LINK: usize = 0;
const STYLE: usize = 1;
const MASK: usize = 2;
const ADJUSTMENT: usize = 3;
const GROUP: usize = 4;
const LAYER: usize = 5;
const DELETE: usize = 6;

fn button_at(at: usize) -> Option<LayersFooterButton> {
    match at {
        GROUP => Some(LayersFooterButton::NewGroup),
        LAYER => Some(LayersFooterButton::NewLayer),
        DELETE => Some(LayersFooterButton::Delete),
        _ => None,
    }
}

/// Which drags a button takes: Delete takes any row of the panel's list or a mask thumbnail; New
/// Layer and New Group take layer rows.
pub(super) fn accepts_footer_drop<K>(button: LayersFooterButton, drag: &LayersDrag<K>, list: egui::Id) -> bool {
    if drag.list != list || drag.keys.is_empty() {
        return false;
    }
    match button {
        LayersFooterButton::Delete => true,
        LayersFooterButton::NewLayer | LayersFooterButton::NewGroup => drag.keys.iter().all(LayerRowKey::is_layer),
    }
}

pub(super) fn show_footer<T: LayerTree>(ui: &mut Ui, cx: &PanelContext<'_, T>, words: &LayersPanelWords) -> Vec<LayersEvent<T::Id>> {
    let state = cx.tree.footer();
    let list = cx.list;
    let takes = |button: LayersFooterButton| move |drag: &LayersDrag<T::Id>| accepts_footer_drop(button, drag, list);
    let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
    let mask_name = state.mask_label.clone().map_or_else(|| words.add_mask.clone(), craft_ui::runtime::env::Label::from);
    let actions = [
        FooterAction::new(icon("link"), words.link_layers.clone()).enabled(state.link_enabled),
        FooterAction::new(icon("fx"), words.add_style.clone()).popup(PopupKind::Menu),
        FooterAction::new(icon("layer_mask"), mask_name).enabled(state.mask_label.is_some()),
        FooterAction::new(icon("adjustment_layer"), words.add_adjustment.clone()).popup(PopupKind::Menu),
        FooterAction::new(icon("new_folder"), words.new_group.clone()).accepts_drops(takes(LayersFooterButton::NewGroup)),
        FooterAction::new(icon("new_layer"), words.new_layer.clone()).accepts_drops(takes(LayersFooterButton::NewLayer)),
        FooterAction::new(icon("trash"), craft_ui::runtime::env::Label::from(state.delete_label.clone()))
            .enabled(state.delete_enabled)
            .takes_drops_disabled(true)
            .accepts_drops(takes(LayersFooterButton::Delete)),
    ];
    let out = FooterBar::actions(actions).show_footer(ui);

    let mut events = Vec::new();
    if let Some(at) = out.picked {
        let mods = out.mods;
        let group_selection = (mods.shift || cx.selection.layers.len() > 1) && cx.selection.active.is_some();
        events.extend(
            match at {
                LINK => Some(LayersFooterAction::Link),
                MASK => Some(LayersFooterAction::AddMask { invert: mods.alt }),
                GROUP => Some(LayersFooterAction::NewGroup { from_selection: group_selection, ask: mods.alt, below: mods.cmd }),
                LAYER => Some(LayersFooterAction::NewLayer { ask: mods.alt, below: mods.cmd }),
                DELETE => Some(LayersFooterAction::Delete { confirm: !mods.alt }),
                _ => None,
            }
            .map(LayersEvent::Footer),
        );
    }
    if let Some(dropped) = &out.dropped
        && let (Some(button), Some(drag)) = (button_at(dropped.index), dropped.payload::<LayersDrag<T::Id>>())
    {
        events.push(LayersEvent::FooterDrop { button, rows: drag.keys.clone() });
    }
    if let Some(response) = out.responses.get(STYLE)
        && let Some(chosen) = Menu::new(words.add_style.clone()).opened_by(response, Trigger::Press).pick(ui, &state.style_menu).picked
    {
        events.push(LayersEvent::Footer(LayersFooterAction::Style(chosen)));
    }
    if let Some(response) = out.responses.get(ADJUSTMENT)
        && let Some(chosen) = Menu::new(words.add_adjustment.clone()).opened_by(response, Trigger::Press).pick(ui, &state.adjustment_menu).picked
    {
        events.push(LayersEvent::Footer(LayersFooterAction::Adjustment(chosen)));
    }
    events
}
