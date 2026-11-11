//! `Inspector` (24.4): the properties of one target in a `Panel`, with the target's icon, name and kind in the header.

use super::words::PanelWords;
use crate::contract::{BodyCx, Props as _, Show as _, ShowContainer as _};
use crate::notices::{Notice, NoticeLayout};
use crate::primitives::IconSpec;
use crate::runtime::env::{Env, Label};
use crate::sections::{HeaderAction, MenuRows, Panel, PanelOutput};
use craft_theme::variant::Tone;
use egui::{AsIdSalt, IdSalt, Ui};

#[cfg(feature = "testing")]
mod stories;

/// A properties panel: a `Panel` whose header names the target (icon, name and, after it, the kind), and whose body
/// the caller fills with the target's controls.
///
/// - Target: [`target`](Inspector::target) gives the key of what is inspected. Every control of the body gets its id
///   from the key, so a target keeps its own field state (typing, scrubbing, scroll position) while another is
///   shown. State the host owns (the open state of a `Section` bound with `open` or kept under `remember`) outlives
///   a change of target.
/// - No target: the body is replaced by a centred notice, `empty` or "No properties", and the header shows the label.
/// - The body can hold a `SectionStack` for sections the user reorders.
#[derive(crate::contract::Pattern)]
pub struct Inspector<'a> {
    label: Label,
    target: Option<IdSalt>,
    icon: Option<IconSpec<'a>>,
    name: Option<Label>,
    kind: Option<Label>,
    empty: Option<Label>,
    actions: Vec<HeaderAction>,
    menu: Option<MenuRows<'a>>,
}

impl<'a> Inspector<'a> {
    /// An inspector named `label`; with no target it shows the empty notice.
    pub fn new(label: impl Into<Label>) -> Self {
        Inspector { label: label.into(), target: None, icon: None, name: None, kind: None, empty: None, actions: Vec::new(), menu: None }
    }

    /// Inspects the target `target`: the body shows, and its ids follow the key.
    #[must_use]
    pub fn target(mut self, target: impl AsIdSalt) -> Self {
        self.target = Some(IdSalt::new(target));
        self
    }

    /// The target's name, the header's title (the label without it).
    #[must_use]
    pub fn header(mut self, name: impl Into<Label>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// The icon before the target's name.
    #[must_use]
    pub fn icon(mut self, icon: IconSpec<'a>) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The target's kind, after its name; a header too narrow for both drops it first.
    #[must_use]
    pub fn kind(mut self, kind: impl Into<Label>) -> Self {
        self.kind = Some(kind.into());
        self
    }

    /// What the notice says when there is no target.
    #[must_use]
    pub fn empty(mut self, message: impl Into<Label>) -> Self {
        self.empty = Some(message.into());
        self
    }

    /// The action parts at the end of the header, in reading order.
    #[must_use]
    pub fn actions(mut self, actions: impl IntoIterator<Item = HeaderAction>) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }

    /// The rows of the menu the Menu action opens.
    #[must_use]
    pub fn menu(mut self, rows: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        self.menu = Some(MenuRows::new(rows));
        self
    }

    /// Shows the panel across the room offered and fills its body with `body`, or the empty notice.
    pub fn show(self, ui: &mut Ui, mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>)) -> PanelOutput {
        let Inspector { label, target, icon, name, kind, empty, actions, menu } = self;
        let shown = target.is_some();
        let title = if shown { name.unwrap_or(label) } else { label };
        let mut panel = Panel::new(title).actions(actions);
        if shown {
            if let Some(icon) = icon {
                panel = panel.icon(icon);
            }
            if let Some(kind) = kind {
                panel = panel.subtitle(kind);
            }
        }
        if let Some(salt) = target {
            panel = panel.id_salt(salt);
        }
        if let Some(rows) = menu {
            panel.set_menu_rows(rows);
        }
        let message = empty.unwrap_or_else(|| Env::of(ui).words::<PanelWords>().inspector_empty.clone());
        panel
            .show(ui, |ui, cx| {
                if shown {
                    body(ui, cx);
                } else {
                    Notice::new(Tone::Info, message.clone()).layout(NoticeLayout::Centred).show(ui);
                }
            })
            .output
    }
}
