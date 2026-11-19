//! `MemoView`: a view the framework runs only when one of its inputs changed, and otherwise adds again as its last run
//! left it.

use super::lazy::{Lazy, Shown};
use super::memo::{MemoKey, MemoProps};
use egui::Ui;
use std::hash::Hash;
use std::sync::Arc;

/// A memoized view: its content is a `fn`, which captures nothing, and its props ([`MemoProps`]) are its only input
/// besides the `Ui`, so the framework sees every input it reads. While its props key equals the key after its last
/// run, it adds that run again (paint, widgets, accessibility nodes, focus stops and the other per-pass
/// registrations) and returns what that run returned. It runs in a pass where its key differs, or where one of the
/// settled rules of [`Lazy::settles`] breaks: the pointer over it, focus in it, an intent routed to it, an animation in
/// it, an edit of a bound value it read, a move, or a change of its environment. Input elsewhere in the window and
/// repaint requests outside it do not run it: what they change in the data it shows reaches it through its props.
///
/// A run that ran in an earlier pass of the same frame is replayed too, when nothing it read was edited after it
/// started: a discard pass that another view's new size asked for replays every view that did not change.
///
/// The key is the props type, the `fn` and every input the props feed. A run that writes through its props (a
/// `&mut Tracked<T>` model, a [`Bind`](super::Bind)) settles on the props as it left them: the key is taken again after
/// the run, so a view does not run again for its own write.
///
/// What `show` returns holds layout facts only (a size, a rect, which part took the room). A replay returns the kept
/// value again, so an event returned through it would arrive once per pass. Events leave a view through its bindings,
/// its `Tracked` writes or commands.
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

impl<P: MemoProps, R: Clone + PartialEq + Send + Sync + 'static> MemoView<P, R> {
    /// The view `id` (unique among its siblings, as a `Lazy` key is) showing `props` through `show`.
    pub fn new(id: impl Hash, props: P, show: fn(&mut P, &mut Ui) -> R) -> MemoView<P, R> {
        let mut lazy = Lazy::new(id, 0.0);
        lazy.settles = true;
        MemoView { lazy, props, show }
    }

    /// Runs the view in a child `Ui` placed in flow in `ui`, or adds its last run again. Returns what the run returned,
    /// the kept value when it replayed; `None` only when it is out of view in a scroll area.
    pub fn show(self, ui: &mut Ui) -> Option<R> {
        let MemoView { lazy, mut props, show } = self;
        let ctx = ui.ctx().clone();
        let key = Self::key(&ctx, &props, show);
        let shown = lazy.run_keyed(ui, std::any::type_name::<P>(), crate::runtime::lazy::Key::Memo(key), |ui, run| {
            let out = show(&mut props, ui);
            run.after = Some(Self::key(&ctx, &props, show));
            run.output = Some(Arc::new(out.clone()));
            out
        });
        match shown {
            Shown::Ran(out) => Some(out),
            Shown::Replayed(kept) => kept.and_then(|kept| kept.downcast_ref::<R>().cloned()),
            Shown::OutOfView => None,
        }
    }

    /// The key of `props` shown through `show`: the props type, the `fn`'s address and the props' inputs. Two `fn`s
    /// the compiler merged into one address differ in their props type or show the same.
    fn key(ctx: &egui::Context, props: &P, show: fn(&mut P, &mut Ui) -> R) -> u128 {
        let mut key = MemoKey::new();
        key.feed_hash(&P::identity());
        key.write(&(show as usize).to_le_bytes());
        props.key(&mut key);
        #[cfg(feature = "testing")]
        crate::runtime::measure::count_memo_key_bytes(ctx, key.bytes());
        // The context is read by the count trials only.
        let _ = ctx;
        key.finish()
    }
}
