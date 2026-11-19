//! `PathSetting`: a path setting of Preferences, typed or picked through the host's file service, kept in the draft.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::SettingsDraft;
use crate::contract::{Bind, NoValue, Outcome, Props as _, ShowContainer as _};
use crate::form::Field;
use crate::patterns::form_dialogs::PathRow;
use crate::runtime::env::Label;
use craft_behavior::EditEvent;
use craft_host::{PickId, PickRequest};
use egui::{Response, Ui};

/// What a path setting reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PathSettingOutput {
    /// The row's response.
    pub response: Response,
    /// The draft value changed this pass: a typed path was committed or the host's answer arrived.
    pub changed: bool,
    /// Browse filed this pick through the host's file service this pass.
    pub browsing: Option<PickId>,
}

impl Outcome for PathSettingOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A path setting for a custom row or page of Preferences ([`CustomSettings`](super::CustomSettings)): the label, then a
/// text field with the path and a Browse… button, over the draft value at `key`. A part of the key may name a list
/// entry (`performance.scratchDisks[1].path`), so each entry of a list setting gets its own row. A typed path is
/// written into the draft when it is committed (Enter or focus loss); Browse asks the host's file service for
/// `request` ([`PathRow::pick`]) and the answer is written into the draft on the pass it arrives. Only `key` changes,
/// so Apply writes just that setting. Tab order: the field, then Browse.
#[must_use = "show the setting with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct PathSetting<'a> {
    label: Label,
    key: &'a str,
    draft: &'a mut SettingsDraft,
    request: PickRequest,
    /// Shown in the empty field.
    #[prop(into)]
    placeholder: Option<Label>,
}

impl<'a> PathSetting<'a> {
    /// The setting at `key` of `draft`, labelled `label`; Browse asks the host to pick `request`.
    pub fn new(label: impl Into<Label>, key: &'a str, draft: &'a mut SettingsDraft, request: PickRequest) -> Self {
        PathSetting { label: label.into(), key, draft, request, placeholder: None }
    }

    /// Shows the setting.
    pub fn show(self, ui: &mut Ui) -> PathSettingOutput {
        let PathSetting { label, key, draft, request, placeholder } = self;
        let mut changed = false;
        let mut browsing = None;
        let shown = Field::new(label.clone()).id_salt(key).show(ui, |ui, _| {
            let before: String = draft.get(key).unwrap_or_default();
            let mut row = PathRow::new(label.clone(), Bind::given(before.clone())).pick(request.clone());
            if let Some(placeholder) = placeholder.clone() {
                row = row.placeholder(placeholder);
            }
            let row = row.show(ui);
            browsing = browsing.or(row.browsing);
            let typed = row
                .edit
                .into_iter()
                .filter_map(|event| match event {
                    EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => Some(after),
                    _ => None,
                })
                .next_back();
            if let Some(after) = typed.or(row.picked)
                && after != before
            {
                draft.set(key, &after);
                changed = true;
            }
        });
        PathSettingOutput { response: shown.output, changed, browsing }
    }
}
