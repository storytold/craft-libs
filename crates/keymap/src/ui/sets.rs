//! The set row (§2.6 of the dialog plan): the set picker, then New Set…, Rename…, Delete, Save,
//! Revert Set, Export, Import… and Summarize…; actions that do not fit go into a More menu. New
//! Set… and Rename… open an inline form under the row.

use craft_ui::layout::fit_toolbar;
use egui::{ComboBox, Key as EguiKey, RichText, TextEdit, Ui};

use super::{Draw, FIELD_W};
use crate::profiles::{Profiles, set_name};
use crate::ui::state::{DialogState, SetForm, SetFormKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    New,
    Rename,
    Delete,
    Save,
    Revert,
    /// Export as craft-keymap JSON; with app formats the item is a menu.
    Export,
    ExportAs(&'static str),
    Import,
    Summarize,
}

struct Item {
    act: Act,
    text: String,
    id: &'static str,
    enabled: bool,
}

impl Draw<'_, '_> {
    /// Drawn above the header when the app passes sets and the dialog is editable.
    pub(super) fn sets_row(&mut self, ui: &mut Ui) {
        if !self.env.options.editable {
            return;
        }
        let Some((p, b)) = self.env.sets.as_ref() else { return };
        let names = p.names(b);
        let active = p.active_name(b).to_string();
        let builtin = Profiles::is_builtin(&active, b);
        let modified = DialogState::is_modified(self.env);
        let items = self.set_items(builtin, modified);
        let mut act = None;
        ui.add_enabled_ui(!self.state.modal(), |ui| {
            ui.horizontal(|ui| {
                let set_label = self.ui("Set:");
                let l = ui.label(self.vis(&set_label));
                let mut pick = None;
                self.combo(ui, "set", &l, &set_label, &active, |ui| {
                    for n in &names {
                        if ui.selectable_label(*n == active, self.vis(n)).clicked() {
                            pick = Some(n.clone());
                        }
                    }
                });
                if modified {
                    ui.label(RichText::new(self.vis(&self.ui("(modified)"))).color(self.style.dim));
                }
                if let Some(n) = pick.filter(|n| *n != active) {
                    self.state.set_form = None;
                    self.state.switch_set(self.env, &n);
                }
                act = self.set_toolbar(ui, &items);
            });
            self.set_form(ui);
        });
        if let Some(a) = act {
            self.set_act(a, &active);
        }
    }

    fn set_items(&self, builtin: bool, modified: bool) -> Vec<Item> {
        let item = |act, text: &str, id, enabled| Item { act, text: self.ui(text), id, enabled };
        let export = if self.env.options.export_formats.is_empty() { "Export…" } else { "Export" };
        let mut items = vec![
            item(Act::New, "New Set…", "shortcuts.set.new", true),
            item(Act::Rename, "Rename…", "shortcuts.set.rename", !builtin),
            item(Act::Delete, "Delete", "shortcuts.set.delete", !builtin),
            item(Act::Save, "Save", "shortcuts.set.save", modified),
            item(Act::Revert, "Revert Set", "shortcuts.set.revert", modified),
            item(Act::Export, export, "shortcuts.set.export", true),
            item(Act::Import, "Import…", "shortcuts.set.import", true),
        ];
        if self.env.options.summary {
            items.push(item(Act::Summarize, "Summarize…", "shortcuts.summarize", true));
        }
        items
    }

    /// The actions that fit in the row's remaining width, then More with the rest.
    fn set_toolbar(&self, ui: &mut Ui, items: &[Item]) -> Option<Act> {
        let more = self.ui("More");
        let natural: Vec<f32> = items.iter().map(|i| self.button_width(ui, &i.text)).collect();
        let spacing = ui.spacing().item_spacing.x;
        let (shown, overflow): (Vec<usize>, Vec<usize>) = match fit_toolbar(&natural, ui.available_width(), spacing, self.button_width(ui, &more)) {
            Some(fit) => (fit.shown.iter().map(|&(i, _)| i).collect(), fit.overflow),
            None => (Vec::new(), (0..items.len()).collect()),
        };
        let mut act = None;
        for i in shown {
            act = act.or(self.set_item(ui, &items[i]));
        }
        if !overflow.is_empty() {
            let r = ui.menu_button(self.vis(&more), |ui| {
                for &i in &overflow {
                    act = act.or(self.set_item(ui, &items[i]));
                }
            });
            self.tag(|| "shortcuts.set.more".into(), r.response.rect, &more);
        }
        act
    }

    fn set_item(&self, ui: &mut Ui, item: &Item) -> Option<Act> {
        if item.act == Act::Export && !self.env.options.export_formats.is_empty() {
            let mut act = None;
            let r = ui.menu_button(self.vis(&item.text), |ui| {
                if self.button(ui, true, &self.ui("Keyboard Shortcuts (JSON)"), "shortcuts.set.export.json") {
                    act = Some(Act::Export);
                }
                for f in &self.env.options.export_formats {
                    let label = self.env.text.ui(f.label).into_owned();
                    if self.button(ui, true, &label, &format!("shortcuts.set.export.{}", f.id)) {
                        act = Some(Act::ExportAs(f.id));
                    }
                }
            });
            self.tag(|| item.id.into(), r.response.rect, &item.text);
            return act;
        }
        self.button(ui, item.enabled, &item.text, item.id).then_some(item.act)
    }

    fn set_act(&mut self, act: Act, active: &str) {
        let form = |kind, name: &str| Some(SetForm { kind, name: name.to_string(), focus: true });
        match act {
            Act::New => self.state.set_form = form(SetFormKind::New { based_on: None }, ""),
            Act::Rename => self.state.set_form = form(SetFormKind::Rename, active),
            Act::Delete => self.state.ask_delete_set(self.env),
            Act::Save => self.state.save_set(self.env),
            Act::Revert => self.state.revert_set(self.env),
            Act::Export => self.state.export_set(self.env),
            Act::ExportAs(f) => self.state.export_as(self.env, f),
            Act::Import => self.state.request_import(self.env),
            Act::Summarize => self.state.summarize(self.env),
        }
    }

    /// New Set… (Name, Based on) or Rename… (Name), under the set row.
    fn set_form(&mut self, ui: &mut Ui) {
        let Some(mut form) = self.state.set_form.clone() else { return };
        let names = self.env.sets.as_ref().map(|(p, b)| p.names(b)).unwrap_or_default();
        let (mut submit, mut cancel) = (false, false);
        ui.horizontal_wrapped(|ui| {
            let name_label = self.ui("Name:");
            let l = ui.label(self.vis(&name_label));
            let edit = ui.add(TextEdit::singleline(&mut form.name).id(self.base.with("set_name")).desired_width(FIELD_W)).labelled_by(l.id);
            self.text_field(&edit);
            self.tag(|| "shortcuts.setForm.name".into(), edit.rect, &name_label);
            if std::mem::take(&mut form.focus) {
                edit.request_focus();
            }
            if edit.lost_focus() {
                let (enter, escape) = ui.input(|i| (i.key_pressed(EguiKey::Enter), i.key_pressed(EguiKey::Escape)));
                submit |= enter;
                cancel |= escape;
            }
            if let SetFormKind::New { based_on } = &mut form.kind {
                let based_label = self.ui("Based on:");
                let l = ui.label(self.vis(&based_label));
                let current = self.ui("Current shortcuts");
                let shown = based_on.clone().unwrap_or_else(|| current.clone());
                let combo = ComboBox::from_id_salt(self.base.with("based_on")).selected_text(self.vis(&shown)).show_ui(ui, |ui| {
                    if ui.selectable_label(based_on.is_none(), self.vis(&current)).clicked() {
                        *based_on = None;
                    }
                    for n in &names {
                        if ui.selectable_label(based_on.as_ref() == Some(n), self.vis(n)).clicked() {
                            *based_on = Some(n.clone());
                        }
                    }
                });
                let combo = combo.response.labelled_by(l.id);
                self.tag(|| "shortcuts.setForm.basedOn".into(), combo.rect, &based_label);
            }
            let ok = match form.kind {
                SetFormKind::New { .. } => self.ui("Create"),
                SetFormKind::Rename => self.ui("Rename"),
            };
            submit |= self.button(ui, set_name(&form.name).is_some(), &ok, "shortcuts.setForm.ok");
            cancel |= self.button(ui, true, &self.ui("Cancel"), "shortcuts.setForm.cancel");
        });
        if cancel {
            self.state.set_form = None;
            return;
        }
        if !submit || set_name(&form.name).is_none() {
            self.state.set_form = Some(form);
            return;
        }
        self.state.message = None;
        match &form.kind {
            SetFormKind::New { based_on } => self.state.new_set(self.env, &form.name, based_on.as_deref()),
            SetFormKind::Rename => self.state.rename_set(self.env, &form.name),
        }
        // A refused name keeps the form open for another try.
        let refused = self.state.message.as_ref().is_some_and(|m| m.refused);
        self.state.set_form = refused.then_some(form);
    }
}
