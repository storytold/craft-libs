//! `MemoView`: a view the framework runs only when one of its inputs changed, and otherwise adds again as its last run
//! left it.

use super::lazy::{Lazy, Shown};
use super::memo::{MemoKey, MemoProps};
use crate::runtime::env::{Env, Scoped};
use crate::runtime::shift::Shift;
use egui::Ui;
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

/// A memoized view: its content is a `fn`, which captures nothing, and its props ([`MemoProps`]) are its only input
/// besides the `Ui`, so the framework sees every input it reads. While its props key equals the key of its last run,
/// it adds that run again (paint, widgets, accessibility nodes, focus stops and the other per-pass
/// registrations) and returns what that run returned. It runs in a pass where its key differs, or where any of these
/// holds:
/// - the pointer is over it or was in its last run, a press started on it, or one of its widgets is dragged or holds
///   focus, or did;
/// - an intent of the pass is routed to a component in it (an AccessKit request on one of its nodes);
/// - an animation in it is not at its end, a texture it shows is still loading, something in it asked for a repaint
///   (`frame::request_repaint`), or a wake time it asked for came;
/// - a bound value, inspection record or command state it read changed;
/// - its place differs from its last run (see below), or the shown life of a component it showed ended;
/// - its last run asked for a discard pass or did something a replay cannot repeat (an overlay, paint on another
///   layer).
///
/// Input elsewhere in the window and repaint requests outside it do not run it: what they change in the data it shows
/// reaches it through its props.
///
/// A run that ran in an earlier pass of the same frame is replayed too, when nothing it read was edited after it
/// started and it asked for no discard pass: a discard pass that another view's new size asked for replays every view
/// that did not change.
///
/// The key is the props type, the `fn` and every input the props feed. What a run paints and returns comes from its
/// own state, the props as the run found them and wrote them on its way. A run that writes through its props (a
/// `&mut Tracked<T>` model, a [`Bind`](super::Bind)) moves its key: its record keeps the key from before the run, so the
/// view runs once more, reads the written value, and replays from then on. A write costs one more run, never a stale
/// replay.
///
/// Its other inputs are the framework's, and each reaches the key or the settled test:
/// - its place: the room it is given, the clip, the layer, the layout, the scale and the scope's style inputs (tokens,
///   theme, density, direction, language, window active), compared for every view;
/// - what the scope around it hands down beyond those (enabled, read-only, size, ink, the command chain, motion, every
///   app value, and the rest of `Scoped::scope_key`), in the key;
/// - the framework inputs it reads outside its props (`runtime::inputs`: the modifiers, the pointer, the keys held, the
///   window facts, the keyboard layout, the keymap, a drag, the focus, the open layers, the scroll view of a nested
///   `Lazy`): a change of one runs only the views that read it;
/// - the clock, read through `frame::stepped_clock`: the view runs at the next step;
/// - commands sent to a command scope instance inside it: it runs while one waits.
///
/// Its content is a layout room of its own, as a pane's is: the flow of the body around it (a row's shrink, a column's
/// sweep, a grid's cursor) does not reach in, so the room it is given reaches it only through its place. To the body
/// around it the view is one unit, sized by its content's measure.
///
/// What `show` returns holds layout facts only (a size, a rect, which part took the room). A replay returns the kept
/// value again, moved with the record when the room moved (`Shift`), so an event returned through it would arrive once
/// per pass. Events leave a view through an [`Outbox`] in its
/// props (picks, closes, drags: what its components report in the pass), its bindings, its `Tracked` writes or
/// commands. A measuring pass that takes the kept extent returns the kept value of the real run.
///
/// `show` is a `fn`: a closure that captures does not build.
///
/// ```compile_fail,E0308
/// # extern crate craft_ui_contract as craft_ui;
/// use craft_ui::contract::MemoView;
/// fn view(ui: &mut egui::Ui, count: u32) {
///     MemoView::new("count", (), move |_: &mut (), ui: &mut egui::Ui| {
///         ui.label(count.to_string());
///     })
///     .show(ui);
/// }
/// ```
///
/// ```
/// # extern crate craft_ui_contract as craft_ui;
/// use craft_ui::contract::MemoView;
/// fn view(ui: &mut egui::Ui, count: u32) -> Option<f32> {
///     MemoView::new("count", count, |count: &mut u32, ui: &mut egui::Ui| ui.label(count.to_string()).rect.width()).show(ui)
/// }
/// ```
#[must_use]
pub struct MemoView<P, R> {
    lazy: Lazy,
    props: P,
    show: fn(&mut P, &mut Ui) -> R,
}

impl<P: MemoProps, R: Clone + PartialEq + Shift + Send + Sync + 'static> MemoView<P, R> {
    /// The view `id` showing `props` through `show`. The id is unique as a `Lazy` key is: among the memoized views and
    /// `Lazy` children of the scroll area the view is in, nested containers included, or among those of its parent `Ui`
    /// outside every scroll area. A second view with its id in one pass is reported (a debug assertion, as for other
    /// duplicate ids).
    pub fn new(id: impl Hash, props: P, show: fn(&mut P, &mut Ui) -> R) -> MemoView<P, R> {
        MemoView { lazy: Lazy::new(id, 0.0), props, show }
    }

    /// Runs the view in a child `Ui` placed in flow in `ui`, or adds its last run again. Returns what the run returned,
    /// the kept value when it replayed; `None` only when it is out of view in a scroll area.
    pub fn show(self, ui: &mut Ui) -> Option<R> {
        let MemoView { lazy, mut props, show } = self;
        let ctx = ui.ctx().clone();
        let env = Env::of(ui);
        let key = Self::key(&ctx, &env, &props, show);
        let shown = lazy.run_keyed(ui, std::any::type_name::<P>(), crate::runtime::lazy::Key::Memo(key), |ui, run| {
            // The content is a layout room of its own: the flow of the body around it does not reach in.
            let installed = Env::install_for(ui, env.derive_local(Scoped::start_room));
            let out = show(&mut props, ui);
            Env::restore(&ctx, installed);
            run.output = Some(Arc::new(out.clone()));
            #[cfg(feature = "testing")]
            {
                run.same_output = Some(crate::runtime::audit::same::<R>);
            }
            out
        });
        match shown {
            Shown::Ran(out) => Some(out),
            // A replay at another place moves what the run returned with its record.
            Shown::Replayed(kept, shift) => {
                kept.and_then(|kept| kept.downcast_ref::<R>().map(|out| if shift.is_none() { out.clone() } else { out.shifted(&shift) }))
            }
            Shown::OutOfView => None,
        }
    }

    /// The key of `props` shown through `show` in the scope `env`: the props type, the `fn`'s address, the props' inputs
    /// and what the scope hands down beyond its style inputs (`Scoped::scope_key`). Two `fn`s the compiler merged into
    /// one address differ in their props type or show the same.
    fn key(ctx: &egui::Context, env: &Env, props: &P, show: fn(&mut P, &mut Ui) -> R) -> u128 {
        let mut key = MemoKey::of_view(ctx);
        key.feed_hash(&P::identity());
        key.write(&(show as usize).to_le_bytes());
        key.write(&env.scope_key().to_le_bytes());
        props.key(&mut key);
        #[cfg(feature = "testing")]
        crate::runtime::measure::count_memo_key_bytes(ctx, key.bytes());
        key.finish()
    }
}

/// The events a memoized view's run sends to the code around it, in the pass it runs: what its components reported (a
/// pick, a close, a drag that started). A run sends; a replay sends nothing. That is complete, because input reaches
/// only a view it engages (the pointer over it, focus in it, a held press, an intent routed to it), and an engaged view
/// runs. The view cannot read what it sent, so an outbox is no input and feeds nothing into the key.
///
/// ```
/// # extern crate craft_ui_contract as craft_ui;
/// use craft_ui::contract::{MemoProps, MemoView, Outbox};
/// #[derive(MemoProps)]
/// struct Row<'a> {
///     title: &'a str,
///     clicked: Outbox<'a, ()>,
/// }
/// fn row(props: &mut Row<'_>, ui: &mut egui::Ui) {
///     if ui.button(props.title).clicked() {
///         props.clicked.send(());
///     }
/// }
/// fn view(ui: &mut egui::Ui) -> bool {
///     let mut clicked = Vec::new();
///     let _ = MemoView::new("row", Row { title: "Open", clicked: Outbox::new(&mut clicked) }, row).show(ui);
///     !clicked.is_empty()
/// }
/// ```
pub struct Outbox<'a, E> {
    sent: &'a mut Vec<E>,
}

impl<'a, E> Outbox<'a, E> {
    /// An outbox whose events go into `sent`, after what it holds.
    pub fn new(sent: &'a mut Vec<E>) -> Outbox<'a, E> {
        Outbox { sent }
    }

    /// Sends `event`.
    pub fn send(&mut self, event: E) {
        self.sent.push(event);
    }
}

impl<E> fmt::Debug for Outbox<'_, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Outbox").field("sent", &self.sent.len()).finish()
    }
}

/// An outbox feeds nothing: the view only writes it.
impl<E> MemoProps for Outbox<'_, E> {
    fn key(&self, _: &mut MemoKey) {}
}
