//! Filter Gallery's session: the effect-layer stack.

use craft_ui::dialog::FieldRefusal;
use craft_ui::patterns::form_dialogs::{DialogFields, set_param_field};
use serde_json::{Value, json};

use super::catalog::FilterCatalog;

/// The identity of an effect layer within its session: it stays with the layer through every change but a deletion, so
/// a host can follow a layer across edits. The default is no session's id: a layer read from saved fields has it until
/// a session takes the layer in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LayerId(u64);

/// One effect layer: a filter, its fields (the catalog's parameters) and its eye. It serialises as
/// `{filter, params, visible}`, with a choice by its option's name, as the engine's effects do. Two layers are equal
/// when their content is: the id is identity, not content.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct EffectLayer {
    /// The filter's key in the catalog.
    pub filter: String,
    /// The filter's parameter values.
    #[serde(rename = "params")]
    pub fields: DialogFields,
    /// Whether the layer's eye is on.
    pub visible: bool,
    /// The layer's identity; not saved: the session issues one when it takes a layer read from saved fields in.
    #[serde(skip)]
    pub id: LayerId,
}

impl PartialEq for EffectLayer {
    fn eq(&self, other: &Self) -> bool {
        self.filter == other.filter && self.fields == other.fields && self.visible == other.visible
    }
}

impl EffectLayer {
    /// A visible layer of `filter` with the catalog's defaults (none for a key the catalog lacks), under `id`.
    fn of(filter: &str, catalog: &dyn FilterCatalog, id: LayerId) -> Self {
        let fields = catalog.filter(filter).map(|found| found.defaults()).unwrap_or_default();
        Self { filter: filter.to_owned(), fields, visible: true, id }
    }
}

/// A change of the stack, as the columns report it. Each is one undo step.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum EffectChange {
    /// A thumbnail or the Filter select picked `filter` for the effect layer at `index`: its
    /// fields start from the filter's defaults; the eye stays. Nothing changes for the same filter.
    #[non_exhaustive]
    SetFilter {
        /// The effect layer's index, bottom first.
        index: usize,
        /// The key of the filter to pick.
        filter: String,
    },
    /// The settings of the effect layer at `index`.
    #[non_exhaustive]
    Fields {
        /// The effect layer's index, bottom first.
        index: usize,
        /// The new parameter values.
        fields: DialogFields,
    },
    /// The whole stack replaced by `effects`.
    #[non_exhaustive]
    Replace {
        /// The effect layers, bottom first.
        effects: Vec<EffectLayer>,
    },
    /// New effect layer: a copy of the selected one, above it, selected.
    Add,
    /// Delete effect layer: the selected one, unless it is the last; the one below is selected.
    Delete,
    /// Move the effect layer at `from` to `to` (the move buttons, the collection's reorder keys, a drag).
    #[non_exhaustive]
    Move {
        /// The index the layer moves from.
        from: usize,
        /// The index the layer ends at.
        to: usize,
    },
    /// The eye of the effect layer at `index`.
    #[non_exhaustive]
    Visible {
        /// The effect layer's index, bottom first.
        index: usize,
        /// Whether the eye is on.
        visible: bool,
    },
}

impl EffectChange {
    /// Picks `filter` for the effect layer at `index`.
    pub fn set_filter(index: usize, filter: impl Into<String>) -> Self {
        Self::SetFilter { index, filter: filter.into() }
    }

    /// Sets the parameter values of the effect layer at `index`.
    pub fn with_fields(index: usize, fields: DialogFields) -> Self {
        Self::Fields { index, fields }
    }

    /// Moves the effect layer at `from` to `to`.
    pub fn move_to(from: usize, to: usize) -> Self {
        Self::Move { from, to }
    }

    /// Turns the eye of the effect layer at `index` on or off.
    pub fn set_visible(index: usize, visible: bool) -> Self {
        Self::Visible { index, visible }
    }
}

/// What an automation field sets: a change of the stack, or the selected layer.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SessionField {
    /// A change of the stack.
    Change(EffectChange),
    /// The layer at this index becomes the selected one.
    Select(usize),
}

/// What OK applies and the history snapshots: the effect layers, bottom first (the top row of the
/// stack is applied last). The selected layer is view state outside the history; undo
/// clamps it. Two sessions are equal when their layers are.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FilterGallerySession {
    /// The effect layers, bottom first.
    pub effects: Vec<EffectLayer>,
    /// The number of the next layer id; never goes down, an undo included, so no id is issued twice.
    pub(super) next_layer: u64,
}

impl PartialEq for FilterGallerySession {
    fn eq(&self, other: &Self) -> bool {
        self.effects == other.effects
    }
}

impl FilterGallerySession {
    /// One effect layer of `filter` with its defaults (a new gallery: the last filter used).
    pub fn new(filter: &str, catalog: &dyn FilterCatalog) -> Self {
        let mut session = Self { effects: Vec::new(), next_layer: 1 };
        let id = session.fresh_id();
        session.effects.push(EffectLayer::of(filter, catalog, id));
        session
    }

    /// A new layer id.
    fn fresh_id(&mut self) -> LayerId {
        let id = LayerId(self.next_layer);
        self.next_layer += 1;
        id
    }

    /// The index of the layer `id` now, or `None` once it is deleted.
    pub fn position(&self, id: LayerId) -> Option<usize> {
        self.effects.iter().position(|effect| effect.id == id)
    }

    /// The change an automation `field` set to `value` makes, as `effects`, `select`, `add`, `delete`, `toggle`, `move`,
    /// `filter` or `params`: `Ok(None)` for `add` or `delete` set to false, `Err` for any other field or a refused
    /// value.
    pub fn field_change(
        &self,
        catalog: &dyn FilterCatalog,
        selected: usize,
        field: &str,
        value: &serde_json::Value,
    ) -> Result<Option<SessionField>, String> {
        let rejects = || FieldRefusal::Rejects(value).message(field);
        let index = |value: &Value| value.as_u64().and_then(|at| usize::try_from(at).ok());
        let count = self.effects.len();
        let change = match field {
            "effects" => {
                let effects: Vec<EffectLayer> = serde_json::from_value(value.clone()).map_err(|_| rejects())?;
                if effects.is_empty() || effects.iter().any(|effect| catalog.filter(&effect.filter).is_none()) {
                    return Err(rejects());
                }
                EffectChange::Replace { effects }
            }
            "select" => {
                let at = index(value).ok_or_else(rejects)?;
                return Ok(Some(SessionField::Select(at.min(count.saturating_sub(1)))));
            }
            "add" | "delete" => match value.as_bool() {
                Some(true) if field == "add" => EffectChange::Add,
                Some(true) => EffectChange::Delete,
                Some(false) => return Ok(None),
                None => return Err(rejects()),
            },
            "toggle" => {
                let at = index(value).ok_or_else(rejects)?;
                let effect = self.effects.get(at).ok_or_else(rejects)?;
                EffectChange::set_visible(at, !effect.visible)
            }
            "move" => {
                let (from, to) = (value.get("from").and_then(index), value.get("to").and_then(index));
                let (Some(from), Some(to)) = (from, to) else { return Err(rejects()) };
                if from >= count || to >= count {
                    return Err(rejects());
                }
                EffectChange::move_to(from, to)
            }
            "filter" => {
                let key = value.as_str().filter(|key| catalog.filter(key).is_some()).ok_or_else(rejects)?;
                EffectChange::set_filter(selected, key)
            }
            "params" => {
                let (Some(effect), Some(entries)) = (self.effects.get(selected), value.as_object()) else { return Err(rejects()) };
                let filter = catalog.filter(&effect.filter).ok_or_else(|| FieldRefusal::Inactive.message(field))?;
                let mut fields = effect.fields.clone();
                for (key, entry) in entries {
                    set_param_field(&filter.params, &mut fields, key, entry.clone())?;
                }
                EffectChange::with_fields(selected, fields)
            }
            _ => return Err(FieldRefusal::Unknown.message(field)),
        };
        Ok(Some(SessionField::Change(change)))
    }

    /// The automation fields of the session: `effects` and `select`.
    pub fn json_fields(&self, selected: usize) -> Value {
        let effects = serde_json::to_value(&self.effects).unwrap_or_default();
        json!({ "effects": effects, "select": selected })
    }

    /// Applies `change`, keeping `selected` on the layer it named (a new or moved layer is
    /// selected). Returns whether the stack changed.
    pub fn apply(&mut self, change: EffectChange, catalog: &dyn FilterCatalog, selected: &mut usize) -> bool {
        let count = self.effects.len();
        if count == 0 {
            return false;
        }
        let current = (*selected).min(count - 1);
        let changed = match change {
            EffectChange::SetFilter { index, filter } => match self.effects.get_mut(index) {
                Some(effect) if effect.filter != filter && catalog.filter(&filter).is_some() => {
                    *effect = EffectLayer { visible: effect.visible, ..EffectLayer::of(&filter, catalog, effect.id) };
                    true
                }
                _ => false,
            },
            EffectChange::Fields { index, fields } => match self.effects.get_mut(index) {
                Some(effect) if effect.fields != fields => {
                    effect.fields = fields;
                    true
                }
                _ => false,
            },
            // The stack never empties: a gallery always has one layer to show.
            EffectChange::Replace { mut effects } if !effects.is_empty() && effects != self.effects => {
                for effect in &mut effects {
                    effect.id = self.fresh_id();
                }
                self.effects = effects;
                true
            }
            EffectChange::Add => {
                let mut copy = self.effects[current].clone();
                copy.id = self.fresh_id();
                self.effects.insert(current + 1, copy);
                *selected = current + 1;
                true
            }
            EffectChange::Delete if count > 1 => {
                self.effects.remove(current);
                *selected = current.saturating_sub(1);
                true
            }
            EffectChange::Move { from, to } if from < count && to < count && from != to => {
                let effect = self.effects.remove(from);
                self.effects.insert(to, effect);
                *selected = to;
                true
            }
            EffectChange::Visible { index, visible } => match self.effects.get_mut(index) {
                Some(effect) if effect.visible != visible => {
                    effect.visible = visible;
                    true
                }
                _ => false,
            },
            _ => false,
        };
        *selected = (*selected).min(self.effects.len() - 1);
        changed
    }
}
