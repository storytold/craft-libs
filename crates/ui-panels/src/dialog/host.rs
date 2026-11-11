//! [`DialogHost`]: the dialogs an app has open, as values. One `open`, one `take`.
//!
//! The id-only [`DialogStack`] is the routing structure under it. The host owns the dialog values on top of it,
//! runs them each pass in the order the stack says, queues the startup modals and keeps the results until the app
//! reads them. A dialog's own container binds its open state to the dialog's entry in the host.

use std::any::{Any, TypeId};
use std::fmt;

use egui::{Context, Ui};

use super::stack::{DialogId, DialogStack};
use crate::contract::{Pattern, PatternKind, PatternProbe};

/// How a dialog on the [`DialogHost`] finished. `T` is the dialog's own answer type: a struct of
/// values for a form, an enum for a prompt with three buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DialogResult<T> {
    /// OK, Enter, or the button the dialog counts as its answer.
    Ok(T),
    /// Cancel, Esc, the close box, or a parent that closed over it.
    Cancel,
}

impl<T> DialogResult<T> {
    /// The answer, or `None` for Cancel.
    pub fn ok(self) -> Option<T> {
        match self {
            DialogResult::Ok(value) => Some(value),
            DialogResult::Cancel => None,
        }
    }
}

/// A dialog the host owns and shows: the app builds the value, [`DialogHost::open`]s it and reads
/// [`DialogHost::take`] on a later pass.
///
/// `show` shows one pass of the dialog's own container. The overlay host makes only the top modal layer
/// interactive; `stack` tells the dialog where it stands. It returns `Some` on the pass the dialog finished; the
/// host removes it then and stores the result. `Send` so a host can live in egui memory behind an
/// `Arc<Mutex<_>>` (stories, tests).
pub trait StackDialog: Any + Send {
    /// What the app reads back. `()` for a notice.
    type Result: Send + 'static;

    /// The id the stack knows the dialog by; stable for the dialog's life
    /// ([`dialog_id`](super::dialog_id) of the dialog's key).
    fn id(&self) -> DialogId;

    /// Shows one pass; `Some` when the dialog finished this pass.
    fn show(&mut self, ui: &mut Ui, stack: &DialogStack) -> Option<DialogResult<Self::Result>>;

    /// The key of the command the dialog answers for, when it has one. The host keeps it with the
    /// dialog's result, so a [`Cancel`](DialogResult::Cancel) still says which dialog it was
    /// ([`DialogHost::take_keyed`]).
    fn key(&self) -> Option<&str> {
        None
    }

    /// Which of several dialogs of this type this is, when the app keeps more than one open
    /// (`ParamDialog::instance`). The host replaces an open dialog
    /// only by one of the same type and instance; `None` (the default) is one dialog per type.
    fn instance(&self) -> Option<DialogId> {
        None
    }

    /// Answers OK as a click on the default button would, without drawing: runs what OK runs
    /// (storing remembered values) and returns the answer. `None` when OK is disabled or the
    /// dialog has no programmatic OK (the default). Used by [`DialogHost::confirm`].
    fn confirm(&mut self, _ctx: &Context) -> Option<Self::Result> {
        None
    }

    /// Called when the host closes the dialog from outside with [`DialogResult::Cancel`]
    /// ([`DialogHost::cancel`], a dialog of the same type opened over it, or its parent closing),
    /// so it can release what it holds: a pick ticket (`Files::forget`), an armed dropper. The
    /// dialog's own Cancel button does this itself before it returns. Does nothing by default.
    fn cancelled(&mut self, _ctx: &Context) {}
}

/// The suite's probe of the stack dialog `T`: `#[pattern(dialog)]` registers through it, so only a `StackDialog`
/// takes the attribute.
#[doc(hidden)]
pub const fn dialog_probe<T: StackDialog + Pattern + 'static>(
    crate_name: &'static str,
    crate_target: &'static str,
    module: &'static str,
    purpose: &'static str,
) -> PatternProbe {
    PatternProbe::kinded::<T>(PatternKind::Dialog, crate_name, crate_target, module, purpose)
}

/// Where a startup modal goes in the queue when several apply at launch (spec question 5).
/// Lower shows first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum StartupStage {
    /// Recover unsaved documents (craft-settings recovery): before anything the user could lose.
    Recovery,
    /// A first-launch offer (craft-keymap's import of another editor's shortcuts): after the
    /// recovery of unsaved work, before any warning.
    Offer,
    /// A warning about this run (PhotoCraft's GPU fallback notice).
    Warning,
    /// The update prompt (craft-update): last, it can wait.
    Update,
}

/// Type-erased [`StackDialog`] the host stores.
trait AnyStackDialog: Send {
    fn id(&self) -> DialogId;
    fn result_type(&self) -> TypeId;
    fn key(&self) -> Option<String>;
    fn instance(&self) -> Option<DialogId>;
    fn confirm_erased(&mut self, ctx: &Context) -> Option<Box<dyn Any + Send>>;
    fn show_erased(&mut self, ui: &mut Ui, stack: &DialogStack) -> Option<Box<dyn Any + Send>>;
    /// `DialogResult::<D::Result>::Cancel`, boxed, for a dialog closed from outside.
    fn cancelled(&self) -> Box<dyn Any + Send>;
    /// [`StackDialog::cancelled`].
    fn release(&mut self, ctx: &Context);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<D: StackDialog> AnyStackDialog for D {
    fn id(&self) -> DialogId {
        StackDialog::id(self)
    }

    fn result_type(&self) -> TypeId {
        TypeId::of::<D>()
    }

    fn key(&self) -> Option<String> {
        StackDialog::key(self).map(str::to_owned)
    }

    fn instance(&self) -> Option<DialogId> {
        StackDialog::instance(self)
    }

    fn confirm_erased(&mut self, ctx: &Context) -> Option<Box<dyn Any + Send>> {
        self.confirm(ctx).map(|result| Box::new(DialogResult::Ok(result)) as Box<dyn Any + Send>)
    }

    fn show_erased(&mut self, ui: &mut Ui, stack: &DialogStack) -> Option<Box<dyn Any + Send>> {
        self.show(ui, stack).map(|result| Box::new(result) as Box<dyn Any + Send>)
    }

    fn release(&mut self, ctx: &Context) {
        StackDialog::cancelled(self, ctx);
    }

    fn cancelled(&self) -> Box<dyn Any + Send> {
        Box::new(DialogResult::<D::Result>::Cancel)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The dialogs an app has open, their results, and the startup queue. The app keeps one and
/// calls [`run`](DialogHost::run) once per pass, after its own UI.
///
/// ```ignore
/// if ui.button("Gaussian Blur…").clicked() {
///     dialogs.open(ParamDialog::new("gaussian-blur", "Gaussian Blur", params));
/// }
/// dialogs.run(ui);
/// if let Some(DialogResult::Ok(fields)) = dialogs.take::<ParamDialog>() {
///     doc.run("filter.gaussianBlur", fields.values());
/// }
/// ```
///
/// - One modal at a time: the newest open dialog's layer takes the input ([`DialogStack`] names it). A child opened with [`open_child`](DialogHost::open_child) sits over its
///   parent and closes with it.
/// - Startup modals [`enqueue`](DialogHost::enqueue) in [`StartupStage`] order and open one at a
///   time, each when the stack is empty, so the GPU warning and the update prompt never overlap
///   (gap 75).
/// - A result stays until [`take`](DialogHost::take)n or until a dialog of the same type and
///   instance is opened again. One open dialog per type and instance: [`open`](DialogHost::open)
///   of a second `ParamDialog` while one with the same [`instance`](StackDialog::instance) is
///   open cancels the first and drops any untaken result of that type and instance (the new
///   dialog's result replaces it). A dialog without an instance is one per type. Dialogs of one
///   type with different instances stay open together, and the `_by_id` calls
///   ([`get_mut_by_id`](DialogHost::get_mut_by_id), [`confirm_by_id`](DialogHost::confirm_by_id),
///   [`cancel_by_id`](DialogHost::cancel_by_id), [`take_by_id`](DialogHost::take_by_id)) reach
///   one of them; the per-type calls reach the most recently opened `D`.
///   [`open_unless_open`](DialogHost::open_unless_open) refuses while any `D` is open and leaves
///   it alone (PhotoCraft's `confirmationPending`).
/// - Automation: [`get`](DialogHost::get) and [`get_mut`](DialogHost::get_mut) reach the open
///   dialog's value, so a script sets fields and a test reads them like a user would.
#[derive(Default)]
pub struct DialogHost {
    stack: DialogStack,
    open: Vec<Box<dyn AnyStackDialog>>,
    queued: Vec<(StartupStage, Box<dyn AnyStackDialog>)>,
    results: Vec<StoredResult>,
    /// Dialogs another of their type replaced, waiting for the pass's `run` to run their
    /// [`StackDialog::cancelled`] (`open` has no `Context`).
    replaced: Vec<Box<dyn AnyStackDialog>>,
}

/// A finished dialog's answer, with the key the dialog had.
struct StoredResult {
    type_id: TypeId,
    id: DialogId,
    instance: Option<DialogId>,
    key: Option<String>,
    result: Box<dyn Any + Send>,
}

impl fmt::Debug for DialogHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DialogHost").field("stack", &self.stack).field("open", &self.open.len()).field("queued", &self.queued.len()).finish()
    }
}

impl DialogHost {
    /// An empty host with no open or queued dialogs.
    pub fn new() -> Self {
        Self::default()
    }

    /// The routing stack, for a [`Dialog`](super::Dialog) the app shows itself beside the host's
    /// (`Dialog::new(..).stack(dialogs.stack())`). Push its id with [`DialogStack::push`] and
    /// remove it on the closing outcome, as before.
    pub fn stack(&self) -> &DialogStack {
        &self.stack
    }

    /// Opens `dialog` on top. Its id goes on the stack now; the dialog shows from the next
    /// [`run`](DialogHost::run). Returns the id.
    pub fn open<D: StackDialog>(&mut self, dialog: D) -> DialogId {
        self.open_with_parent(Box::new(dialog), None)
    }

    /// Opens `dialog` unless a dialog of its type is already open; the first keeps the input and
    /// `dialog` is returned in `Err`. Use it for prompts that must not replace each other (a
    /// delete confirmation open when the changed-on-disk question arrives).
    pub fn open_unless_open<D: StackDialog>(&mut self, dialog: D) -> Result<DialogId, D> {
        if self.is_open::<D>() { Err(dialog) } else { Ok(self.open(dialog)) }
    }

    /// Opens `dialog` as a child of `parent`: shown over it, disabled with it, closed when the
    /// parent closes (a contour editor from Layer Style).
    pub fn open_child<D: StackDialog>(&mut self, dialog: D, parent: DialogId) -> DialogId {
        self.open_with_parent(Box::new(dialog), Some(parent))
    }

    /// Queues a startup modal. It opens when the stack is empty and no earlier stage waits;
    /// two of one stage open in `enqueue` order.
    pub fn enqueue<D: StackDialog>(&mut self, stage: StartupStage, dialog: D) {
        let at = self.queued.partition_point(|(queued, _)| *queued <= stage);
        self.queued.insert(at, (stage, Box::new(dialog)));
    }

    /// Runs every open dialog for this pass, oldest first; removes the ones that finished and stores their
    /// results; opens the next queued startup modal when the stack is empty. Call once per pass, after the app's
    /// own UI.
    pub fn run(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        self.release_replaced(ctx);
        self.open_next_queued();
        let mut finished = Vec::new();
        for dialog in &mut self.open {
            if let Some(result) = dialog.show_erased(ui, &self.stack) {
                finished.push((dialog.id(), result));
            }
        }
        for (id, result) in finished {
            let Some(at) = self.open.iter().position(|dialog| dialog.id() == id) else { continue };
            let dialog = self.open.remove(at);
            self.store_result(dialog.as_ref(), result);
            self.stack.remove_tree(id);
            self.cancel_orphans(Some(ctx));
        }
        if self.open_next_queued() {
            crate::runtime::frame::request_repaint(ctx);
        }
    }

    /// The result of the last `D` that finished or was cancelled, once. `None` when no result
    /// waits: it was taken already, or no `D` has finished. Opening a `D` drops an untaken result
    /// of the type; if that `open` replaced a `D` that was still open, the replaced one's
    /// `Cancel` waits instead. Results are per type, so the app tells two `D` apart by what the
    /// result carries (a key, a tag). A `D` opened with an instance ([`StackDialog::instance`])
    /// keeps its result apart, for [`take_by_id`](DialogHost::take_by_id) only.
    pub fn take<D: StackDialog>(&mut self) -> Option<DialogResult<D::Result>> {
        self.take_keyed::<D>().map(|(result, _)| result)
    }

    /// [`take`](DialogHost::take) with the key the dialog had ([`StackDialog::key`]): what a
    /// `Cancel` after a replace-by-open needs to tell which command it was. `None` key for a
    /// dialog without one.
    pub fn take_keyed<D: StackDialog>(&mut self) -> Option<(DialogResult<D::Result>, Option<String>)> {
        let at = self.results.iter().rposition(|stored| stored.type_id == TypeId::of::<D>() && stored.instance.is_none())?;
        self.take_at::<D>(at)
    }

    /// [`take`](DialogHost::take) for the dialog `id` names, among several `D` of different
    /// instances.
    pub fn take_by_id<D: StackDialog>(&mut self, id: DialogId) -> Option<DialogResult<D::Result>> {
        let at = self.results.iter().rposition(|stored| stored.type_id == TypeId::of::<D>() && stored.id == id)?;
        self.take_at::<D>(at).map(|(result, _)| result)
    }

    fn take_at<D: StackDialog>(&mut self, at: usize) -> Option<(DialogResult<D::Result>, Option<String>)> {
        let stored = self.results.remove(at);
        let result = stored.result.downcast::<DialogResult<D::Result>>().ok()?;
        Some((*result, stored.key))
    }

    /// Answers the open `D` with OK, as its default button would (for automation: no pointer, no
    /// pass): runs what OK runs, closes the dialog and its children, and keeps the answer for
    /// [`take`](DialogHost::take). `false` when no `D` is open, OK is disabled, or `D` has no
    /// programmatic OK.
    pub fn confirm<D: StackDialog>(&mut self, ctx: &Context) -> bool {
        match self.position::<D>() {
            Some(at) => self.confirm_at(ctx, at),
            None => false,
        }
    }

    /// [`confirm`](DialogHost::confirm) for the open `D` that `id` names.
    pub fn confirm_by_id<D: StackDialog>(&mut self, ctx: &Context, id: DialogId) -> bool {
        match self.position_by_id::<D>(id) {
            Some(at) => self.confirm_at(ctx, at),
            None => false,
        }
    }

    fn confirm_at(&mut self, ctx: &Context, at: usize) -> bool {
        let Some(result) = self.open[at].confirm_erased(ctx) else { return false };
        let id = self.open[at].id();
        let dialog = self.open.remove(at);
        self.store_result(dialog.as_ref(), result);
        self.stack.remove_tree(id);
        self.cancel_orphans(Some(ctx));
        true
    }

    /// Closes the open `D` with [`DialogResult::Cancel`] (and its children), as Esc would, and runs
    /// [`StackDialog::cancelled`] on each.
    pub fn cancel<D: StackDialog>(&mut self, ctx: &Context) -> bool {
        match self.position::<D>() {
            Some(at) => self.cancel_at(ctx, at),
            None => false,
        }
    }

    /// [`cancel`](DialogHost::cancel) for the open `D` that `id` names.
    pub fn cancel_by_id<D: StackDialog>(&mut self, ctx: &Context, id: DialogId) -> bool {
        match self.position_by_id::<D>(id) {
            Some(at) => self.cancel_at(ctx, at),
            None => false,
        }
    }

    fn cancel_at(&mut self, ctx: &Context, at: usize) -> bool {
        let id = self.open[at].id();
        let mut dialog = self.open.remove(at);
        dialog.release(ctx);
        self.store_result(dialog.as_ref(), dialog.cancelled());
        self.stack.remove_tree(id);
        self.cancel_orphans(Some(ctx));
        true
    }

    /// The open dialog `id` ([`dialog_id`](super::dialog_id) of its key) as `Any`, for an app with
    /// many dialog types that looks the top one up by id and downcasts.
    pub fn get_dyn(&self, id: DialogId) -> Option<&dyn Any> {
        self.open.iter().find(|dialog| dialog.id() == id).map(|dialog| dialog.as_any())
    }

    /// [`get_dyn`](DialogHost::get_dyn), mutably.
    pub fn get_dyn_mut(&mut self, id: DialogId) -> Option<&mut dyn Any> {
        self.open.iter_mut().find(|dialog| dialog.id() == id).map(|dialog| dialog.as_any_mut())
    }

    /// The open `D` when it is the dialog `id` names (the host holds one `D` at a time; the id
    /// tells which command it is).
    pub fn get_by_id<D: StackDialog>(&self, id: DialogId) -> Option<&D> {
        self.get_dyn(id)?.downcast_ref()
    }

    /// The open `D`, for the app to feed (a preview image) or a script to read.
    pub fn get<D: StackDialog>(&self) -> Option<&D> {
        self.open.iter().rfind(|dialog| dialog.result_type() == TypeId::of::<D>())?.as_any().downcast_ref()
    }

    /// [`get`](DialogHost::get), mutably.
    pub fn get_mut<D: StackDialog>(&mut self) -> Option<&mut D> {
        self.open.iter_mut().rfind(|dialog| dialog.result_type() == TypeId::of::<D>())?.as_any_mut().downcast_mut()
    }

    /// [`get_mut`](DialogHost::get_mut) for the open `D` that `id` names.
    pub fn get_mut_by_id<D: StackDialog>(&mut self, id: DialogId) -> Option<&mut D> {
        self.get_dyn_mut(id)?.downcast_mut()
    }

    /// The most recently opened `D`.
    fn position<D: StackDialog>(&self) -> Option<usize> {
        self.open.iter().rposition(|dialog| dialog.result_type() == TypeId::of::<D>())
    }

    fn position_by_id<D: StackDialog>(&self, id: DialogId) -> Option<usize> {
        self.open.iter().position(|dialog| dialog.id() == id && dialog.result_type() == TypeId::of::<D>())
    }

    fn store_result(&mut self, dialog: &dyn AnyStackDialog, result: Box<dyn Any + Send>) {
        let (type_id, instance) = (dialog.result_type(), dialog.instance());
        self.results.retain(|stored| stored.type_id != type_id || stored.instance != instance);
        self.results.push(StoredResult { type_id, id: dialog.id(), instance, key: dialog.key(), result });
    }

    fn open_with_parent(&mut self, dialog: Box<dyn AnyStackDialog>, parent: Option<DialogId>) -> DialogId {
        let (type_id, instance) = (dialog.result_type(), dialog.instance());
        self.results.retain(|stored| stored.type_id != type_id || stored.instance != instance);
        if let Some(at) = self.open.iter().position(|open| open.result_type() == type_id && open.instance() == instance) {
            let old = self.open.remove(at);
            self.stack.remove_tree(old.id());
            self.store_result(old.as_ref(), old.cancelled());
            self.replaced.push(old);
            self.cancel_orphans(None);
        }
        let id = dialog.id();
        self.stack.push(id, parent);
        self.open.push(dialog);
        id
    }

    /// Closes the open dialogs the stack no longer holds (children of a dialog that closed), with
    /// `Cancel`. Without a `Context` their [`StackDialog::cancelled`] waits for the next `run`.
    fn cancel_orphans(&mut self, ctx: Option<&Context>) {
        let (kept, orphans): (Vec<_>, Vec<_>) = std::mem::take(&mut self.open).into_iter().partition(|dialog| self.stack.contains(dialog.id()));
        self.open = kept;
        for mut dialog in orphans {
            self.store_result(dialog.as_ref(), dialog.cancelled());
            match ctx {
                Some(ctx) => dialog.release(ctx),
                None => self.replaced.push(dialog),
            }
        }
    }

    /// Runs [`StackDialog::cancelled`] on the dialogs `open` closed without a `Context`.
    fn release_replaced(&mut self, ctx: &Context) {
        for mut dialog in std::mem::take(&mut self.replaced) {
            dialog.release(ctx);
        }
    }

    /// Opens the first queued startup modal if nothing else is open.
    fn open_next_queued(&mut self) -> bool {
        if !self.stack.is_empty() || !self.open.is_empty() || self.queued.is_empty() {
            return false;
        }
        let (_, dialog) = self.queued.remove(0);
        self.open_with_parent(dialog, None);
        true
    }

    /// Whether `D` is open.
    pub fn is_open<D: StackDialog>(&self) -> bool {
        self.get::<D>().is_some()
    }

    /// Whether any dialog is open or a startup modal waits: the app's shortcuts and canvas input
    /// are blocked meanwhile ([`any_modal`](super::any_modal) answers for the frame).
    pub fn is_empty(&self) -> bool {
        self.open.is_empty() && self.queued.is_empty()
    }
}
