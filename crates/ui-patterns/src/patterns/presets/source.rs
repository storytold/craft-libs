//! `PresetSource`: the presets a browser or a picker shows, as the host's inputs and the `fn`s that build the rows
//! from them, so a memoized view keys on the inputs and builds the rows in its own run.

use super::model::{PresetKind, PresetRow, ThumbInfo};
use crate::contract::{ListDrag, MemoKey, MemoProps};
use crate::primitives::ContentPainterCx;
use crate::runtime::derived::derived;
use crate::runtime::env::Env;
use egui::{Context, Id, Rect};
use std::sync::Arc;

/// Whether a drag of presets may drop at a place: the inputs, the drag, the preset under the pointer and the insert
/// position.
pub type AcceptsFn<I, K> = fn(&I, &ListDrag<K>, Option<&K>, usize) -> bool;

/// The host's thumbnail painter: the inputs, the painter, the thumbnail's rect and the row.
pub type ThumbFn<I, K> = fn(&I, &ContentPainterCx, Rect, ThumbInfo<'_, K>);

/// The `fn`s of a [`PresetSource`]: how the rows are built from the inputs `I`, and what the host answers for a row.
/// Each is a `fn`, which captures nothing, so the inputs are all a row can depend on.
pub struct PresetFns<I, K> {
    rows: fn(&I, &Env) -> Vec<PresetRow<K>>,
    can_rename: Option<fn(&I, &K) -> bool>,
    can_delete: Option<fn(&I, &K) -> bool>,
    can_move: Option<fn(&I, &K) -> bool>,
    accepts: Option<AcceptsFn<I, K>>,
    thumb: Option<ThumbFn<I, K>>,
}

impl<I, K> Clone for PresetFns<I, K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<I, K> Copy for PresetFns<I, K> {}

impl<I, K> PresetFns<I, K> {
    /// Rows built by `rows`; every row may be renamed, deleted and moved, each preset paints its own `preview`.
    pub fn new(rows: fn(&I, &Env) -> Vec<PresetRow<K>>) -> Self {
        PresetFns { rows, can_rename: None, can_delete: None, can_move: None, accepts: None, thumb: None }
    }

    /// Which presets and groups may be renamed.
    #[must_use]
    pub fn can_rename(mut self, test: fn(&I, &K) -> bool) -> Self {
        self.can_rename = Some(test);
        self
    }

    /// Which presets and groups the row menu, the Delete key and the footer may delete.
    #[must_use]
    pub fn can_delete(mut self, test: fn(&I, &K) -> bool) -> Self {
        self.can_delete = Some(test);
        self
    }

    /// Which presets the row menu's Move to may move.
    #[must_use]
    pub fn can_move(mut self, test: fn(&I, &K) -> bool) -> Self {
        self.can_move = Some(test);
        self
    }

    /// Refuses the reorder landings for which the test is false: it gets the drag, the row under the pointer (none
    /// over an empty browser) and the position before which the dragged rows would go.
    #[must_use]
    pub fn accepts(mut self, test: AcceptsFn<I, K>) -> Self {
        self.accepts = Some(test);
        self
    }

    /// Paints every preset's thumbnail (a swatch, a gradient ramp) in place of its row's own `preview`.
    #[must_use]
    pub fn thumb(mut self, paint: ThumbFn<I, K>) -> Self {
        self.thumb = Some(paint);
        self
    }

    /// Feeds the addresses of the `fn`s: two sources over equal inputs key apart when they build or answer
    /// differently.
    fn key(&self, key: &mut MemoKey) {
        let address = |f: Option<usize>| f.unwrap_or(0).to_le_bytes();
        key.write(&(self.rows as usize).to_le_bytes());
        key.write(&address(self.can_rename.map(|f| f as usize)));
        key.write(&address(self.can_delete.map(|f| f as usize)));
        key.write(&address(self.can_move.map(|f| f as usize)));
        key.write(&address(self.accepts.map(|f| f as usize)));
        key.write(&address(self.thumb.map(|f| f as usize)));
    }
}

/// The presets a browser or a picker shows: the host's inputs (props: plain values, or a `Tracked` model for a large
/// library, which keys by revision) and the [`PresetFns`] that build the rows from them. A memoized view holds the
/// source in its props, so it runs when the inputs change and builds the rows in its own run.
///
/// The rows are built once for each key of the inputs and the scope's words, number format and units, and kept
/// (`runtime::derived`): every reader of the same source in a pass shares them.
pub struct PresetSource<'a, K>(Box<dyn Source<K> + Send + 'a>);

impl<'a, K: Clone + PartialEq + Send + Sync + 'static> PresetSource<'a, K> {
    /// The presets `fns` builds from `inputs`.
    pub fn new<I: MemoProps + Send + 'a>(inputs: I, fns: PresetFns<I, K>) -> Self {
        PresetSource(Box::new(WithInputs { inputs, fns }))
    }

    /// The rows in the scope `env`: the kept ones while the inputs and the scope's text and units are unchanged.
    pub fn rows(&self, ctx: &Context, env: &Env) -> Arc<Vec<PresetRow<K>>> {
        let units = env.units();
        let mut key = MemoKey::new();
        self.0.key(&mut key);
        crate::contract::MemoProps::key(&units, &mut key);
        let id = Id::NULL.with(("preset-rows", key.finish()));
        derived(ctx, id, &(self, &units), || self.0.rows(env))
    }

    /// Whether the row `key` may be renamed.
    pub fn can_rename(&self, key: &K) -> bool {
        self.0.can_rename(key)
    }

    /// Whether the row `key` may be deleted.
    pub fn can_delete(&self, key: &K) -> bool {
        self.0.can_delete(key)
    }

    /// Whether the preset `key` may be moved to another group.
    pub fn can_move(&self, key: &K) -> bool {
        self.0.can_move(key)
    }

    /// Whether a reorder landing passes the host's test.
    pub fn accepts(&self, drag: &ListDrag<K>, over: Option<&K>, insert: usize) -> bool {
        self.0.accepts(drag, over, insert)
    }

    /// Whether the host paints thumbnails itself.
    pub fn paints_thumbs(&self) -> bool {
        self.0.paints_thumbs()
    }

    /// Paints a thumbnail with the host's painter; `false` when it has none, and the row's `preview` paints.
    pub fn thumb(&self, cx: &ContentPainterCx, rect: Rect, info: ThumbInfo<'_, K>) -> bool {
        self.0.thumb(cx, rect, info)
    }
}

impl<'a, K: Clone + PartialEq + MemoProps + Send + Sync + 'static> PresetSource<'a, K> {
    /// The prebuilt `rows`, for a host that builds them itself. Keyed by each row's key, name, depth and kind, which
    /// are all a named preset row shows.
    pub fn listed(rows: &'a [PresetRow<K>]) -> Self {
        PresetSource::new(ListedRows(rows), PresetFns::new(|listed: &ListedRows<'_, K>, _| listed.0.to_vec()))
    }
}

/// The inputs of [`PresetSource::listed`].
struct ListedRows<'a, K>(&'a [PresetRow<K>]);

impl<K: MemoProps> MemoProps for ListedRows<'_, K> {
    fn key(&self, key: &mut MemoKey) {
        for row in self.0 {
            row.key.key(key);
            row.label.key(key);
            row.depth.key(key);
            (row.kind == PresetKind::Item).key(key);
        }
        self.0.len().key(key);
    }
}

/// By its inputs and its `fn`s.
impl<K> MemoProps for PresetSource<'_, K> {
    fn key(&self, key: &mut MemoKey) {
        self.0.key(key);
    }
}

/// A source: its rows and answers, and the key of its inputs and `fn`s.
trait Source<K> {
    fn key(&self, key: &mut MemoKey);
    fn rows(&self, env: &Env) -> Vec<PresetRow<K>>;
    fn can_rename(&self, key: &K) -> bool;
    fn can_delete(&self, key: &K) -> bool;
    fn can_move(&self, key: &K) -> bool;
    fn accepts(&self, drag: &ListDrag<K>, over: Option<&K>, insert: usize) -> bool;
    fn paints_thumbs(&self) -> bool;
    fn thumb(&self, cx: &ContentPainterCx, rect: Rect, info: ThumbInfo<'_, K>) -> bool;
}

struct WithInputs<I, K> {
    inputs: I,
    fns: PresetFns<I, K>,
}

impl<I: MemoProps, K> Source<K> for WithInputs<I, K> {
    fn key(&self, key: &mut MemoKey) {
        key.feed_hash(&I::identity());
        self.fns.key(key);
        self.inputs.key(key);
    }

    fn rows(&self, env: &Env) -> Vec<PresetRow<K>> {
        (self.fns.rows)(&self.inputs, env)
    }

    fn can_rename(&self, key: &K) -> bool {
        self.fns.can_rename.is_none_or(|test| test(&self.inputs, key))
    }

    fn can_delete(&self, key: &K) -> bool {
        self.fns.can_delete.is_none_or(|test| test(&self.inputs, key))
    }

    fn can_move(&self, key: &K) -> bool {
        self.fns.can_move.is_none_or(|test| test(&self.inputs, key))
    }

    fn accepts(&self, drag: &ListDrag<K>, over: Option<&K>, insert: usize) -> bool {
        self.fns.accepts.is_none_or(|test| test(&self.inputs, drag, over, insert))
    }

    fn paints_thumbs(&self) -> bool {
        self.fns.thumb.is_some()
    }

    fn thumb(&self, cx: &ContentPainterCx, rect: Rect, info: ThumbInfo<'_, K>) -> bool {
        let Some(paint) = self.fns.thumb else { return false };
        paint(&self.inputs, cx, rect, info);
        true
    }
}
