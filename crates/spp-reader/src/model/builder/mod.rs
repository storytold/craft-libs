//! Turns a [`RawTree`] into a [`ProjectDocument`]. Never fails: what it does not know becomes a
//! [`RawEntry`] and is counted in the [`ModelReport`]. Split by level: texture sets, layers, effects, sources,
//! strokes.

mod brush;
mod decal;
mod effect;
mod fill_projection;
mod layer;
mod source;
mod stroke;
mod texture_set;

use super::document::ProjectDocument;
use super::fields::{self, SettingKind, UnhandledSetting};
use super::raw::{RawObject, RawTree};
use super::raw_entry::RawEntry;
use super::report::ModelReport;
use crate::decode::document::Scalar;
use std::collections::HashSet;

/// Deepest nesting of folders, effect groups and procedural inputs the builder follows; real files stay
/// far below (folders 4, effect groups 2).
const MAX_NESTING: usize = 64;

/// Builds the model and keeps the tally.
pub struct ModelBuilder {
    report: ModelReport,
    /// `document.bin`'s format: how a channel stored as an enum reads ([`Self::channel_bit`]).
    format_version: u32,
    /// Where unhandled fields go ([`Self::begin`], [`Self::end`]).
    settings: Settings,
}

/// The unhandled fields found so far, per owner (document, set, layer, effect) being built, innermost last; and the
/// objects already checked, by address (every object is checked once, by the innermost owner it belongs to).
#[derive(Default)]
struct Settings {
    open: Vec<Vec<UnhandledSetting>>,
    owners: HashSet<usize>,
    checked: HashSet<usize>,
}

fn address(object: &RawObject) -> usize {
    std::ptr::from_ref(object) as usize
}

/// From this format on a channel stored as an enum is its index in the mask; before it (seen: 55), the bit itself.
/// Formats 79 to 91 weren't seen.
const ENUM_CHANNEL_IS_INDEX: u32 = 92;

impl ModelBuilder {
    /// The channel bit stored in field `name`.
    fn channel_bit(&self, object: &RawObject, name: &str) -> Option<u64> {
        match object.scalar(name)? {
            Scalar::Enum(bit) if self.format_version < ENUM_CHANNEL_IS_INDEX => Some(*bit),
            other => other.channel_bit(),
        }
    }

    fn new(tree: &RawTree) -> ModelBuilder {
        let mut b =
            ModelBuilder { report: ModelReport::with_totals(&tree.classes), format_version: tree.format_version, settings: Settings::default() };
        b.report.unknown_tags = tree.unknown_tags;
        b
    }

    pub fn build(tree: &RawTree) -> (ProjectDocument, ModelReport) {
        let mut b = ModelBuilder::new(tree);
        let mut unhandled = Vec::new();
        let texture_sets = match &tree.root {
            Some(root) if root.class == "DataDocument" => {
                b.seen(root);
                b.begin(root);
                // the document's own fields first: they apply to every set
                b.check_one(root);
                let own = b.settings.open.last().cloned().unwrap_or_default();
                let mut sets: Vec<_> = root.objects("materials").map(|m| b.texture_set(m)).collect();
                for set in &mut sets {
                    set.unhandled.extend(own.iter().cloned());
                }
                unhandled = b.end(root);
                sets
            }
            Some(other) => {
                b.raw(other);
                Vec::new()
            }
            None => Vec::new(),
        };
        let doc = ProjectDocument { format_version: tree.format_version, texture_sets, unhandled };
        (doc, b.report)
    }

    /// A smart material's or smart mask's tree ([`super::preset`]).
    pub fn build_preset(tree: &RawTree) -> (super::preset::ProjectPreset, ModelReport) {
        use super::preset::ProjectPreset;
        let mut b = ModelBuilder::new(tree);
        if let Some(root) = &tree.root {
            b.begin(root);
        }
        let preset = match &tree.root {
            Some(root) if root.class == "DataLayersPreset" => {
                b.seen(root);
                match root.object("layerGroup") {
                    Some(group) => ProjectPreset::SmartMaterial(b.layer(group, 0)),
                    None => ProjectPreset::Other(root.class.clone()),
                }
            }
            Some(root) if root.class == "DataSmartMask" => {
                b.seen(root);
                match root.object("actionsStack") {
                    Some(stack) => ProjectPreset::SmartMask(b.effect_stack(stack, 0)),
                    None => ProjectPreset::SmartMask(Vec::new()),
                }
            }
            Some(other) => {
                b.raw(other);
                ProjectPreset::Other(other.class.clone())
            }
            None => ProjectPreset::Other(String::new()),
        };
        // a preset has no place for its root's own settings: the unknown ones are in the report
        if let Some(root) = &tree.root {
            b.end(root);
        }
        (preset, b.report)
    }

    /// Starts collecting the unhandled fields of `owner` (a document, set, layer or effect) and of the objects below
    /// it that no inner owner claims.
    fn begin(&mut self, owner: &RawObject) {
        self.settings.owners.insert(address(owner));
        self.settings.open.push(Vec::new());
    }

    /// Checks every object below `owner` not checked yet (inner owners checked theirs when they ended) and returns
    /// what [`Self::begin`] started collecting.
    fn end(&mut self, owner: &RawObject) -> Vec<UnhandledSetting> {
        let mut todo = vec![owner];
        while let Some(object) = todo.pop() {
            if address(object) != address(owner) && self.settings.owners.contains(&address(object)) {
                continue;
            }
            self.check_one(object);
            todo.extend(fields::children(object));
        }
        self.settings.open.pop().unwrap_or_default()
    }

    /// Checks `object`'s own fields once, into the innermost open owner.
    fn check_one(&mut self, object: &RawObject) {
        if !self.settings.checked.insert(address(object)) {
            return;
        }
        let report = &mut self.report;
        let open = &mut self.settings.open;
        fields::check(object, |setting| {
            if setting.kind == SettingKind::Unknown {
                *report.unknown_fields.entry(setting.key()).or_default() += 1;
            }
            if let Some(list) = open.last_mut() {
                list.push(setting);
            }
        });
    }

    /// Marks an object as turned into a typed value.
    fn seen(&mut self, object: &RawObject) {
        self.report.recognise(&object.class);
    }

    /// Keeps an object the model does not understand.
    fn raw(&mut self, object: &RawObject) -> RawEntry {
        self.report.keep_raw(&object.class);
        RawEntry::of(object)
    }

    /// The nesting limit was hit: keep the object raw and note it.
    fn too_deep(&mut self, object: &RawObject) -> RawEntry {
        self.report.depth_limited += 1;
        self.raw(object)
    }
}
