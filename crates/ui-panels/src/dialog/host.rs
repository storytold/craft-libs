//! [`DialogHost`]: the dialogs an app has open, as values. One `open`, one `take`.
//!
//! The id-only [`DialogStack`] is the routing structure under it. The host owns the dialog values on top of it,
//! runs them each pass in the order the stack says, queues the startup modals and keeps the results until the app
//! reads them. A dialog's own container binds its open state to the dialog's entry in the host.

use std::any::{Any, TypeId};
use std::fmt;
use std::ops::{Deref, DerefMut};

use egui::{Context, Ui};
use serde_json::Value;

use super::held::HeldOk;
use super::stack::{DialogId, DialogStack};
use crate::contract::{Pattern, PatternKind, PatternProbe};
use crate::runtime::requests::ColorRequest;

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

    /// Whether the dialog is modeless now: current state, so a dialog may change it while open.
    fn modeless(&self) -> bool {
        false
    }

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

    /// A child the dialog asks the host to open over it, taken once after [`show`](StackDialog::show): the host opens
    /// it with [`DialogHost::open_child`] under this dialog (a Replace prompt from a file chooser). `None` (the default)
    /// asks for nothing.
    fn take_child(&mut self) -> Option<ChildDialog> {
        None
    }

    /// Answers OK as a click on the default button would, without drawing: runs what OK runs
    /// (storing remembered values) and returns the answer. `None` when OK is disabled, the answer
    /// is one the dialog offers no enabled button for (a prompt's choice), or the dialog has no
    /// programmatic OK (the default). A dialog that holds its OK files the answer as its button
    /// does and returns `None` ([`held_state`](StackDialog::held_state) is then `Waiting`); once the app
    /// accepted the held OK, it returns the held answer. Used by [`DialogHost::confirm`].
    fn confirm(&mut self, _ctx: &Context) -> Option<Self::Result> {
        None
    }

    /// Where the dialog's held OK stands; `None` (the default) for a dialog whose OK holds nothing now.
    fn held_state(&self) -> Option<HeldOk> {
        None
    }

    /// The canvas the dialog shows the document on, which takes the app's tool input in document points while the
    /// dialog is open: a full-window workspace's. `None` (the default) for a dialog without one.
    fn canvas(&self) -> Option<egui::Id> {
        None
    }

    /// The dialog's values as a JSON object, keyed as [`set_json_field`](StackDialog::set_json_field) takes them:
    /// what automation lists for the open dialog (`ui.inspect`). `None` (the default) for a dialog without values.
    fn json_fields(&self) -> Option<Value> {
        None
    }

    /// Sets the value `field` names, as the user editing that control would (`ui.dialog.set`); a prompt's answer
    /// is its `choice` field, one of the answers it has an enabled button for. `Err` says why the dialog refused:
    /// no such field, a control that takes no input now, or a value the control can't hold. Refuses everything by
    /// default. The host asks for the pass that shows the change; an implementation asks for none.
    fn set_json_field(&mut self, _ctx: &Context, field: &str, _value: Value) -> Result<(), String> {
        Err(FieldRefusal::NoFields.message(field))
    }

    /// The facts a story reports about the dialog's last pass, as [`Outcome::story_facts`](crate::contract::Outcome)
    /// reports a component's: hosted forms keep their component's. Empty by default.
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        crate::testing::StoryOutput::default()
    }

    /// Called when the host closes the dialog from outside with [`DialogResult::Cancel`]
    /// ([`DialogHost::cancel`], a dialog of the same type opened over it, or its parent closing),
    /// so it can release what it holds: a pick ticket (`Files::forget`), an armed dropper. The
    /// dialog's own Cancel button does this itself before it returns. Does nothing by default.
    fn cancelled(&mut self, _ctx: &Context) {}

    /// Shows `color` live in the dialog's colour row `key` while a child picker edits it (`None` ends the preview).
    /// Hosted dialogs with colour rows override it; does nothing by default.
    fn preview_color(&mut self, key: &str, color: Option<craft_theme::Srgba>) {
        let _ = (key, color);
    }

    /// A colour row's button press, once; the picked colour is written into row `key` next pass
    /// ([`answer_color`](StackDialog::answer_color)). `None` (the default) for a dialog without colour rows.
    fn take_color_request(&mut self) -> Option<ColorRequest> {
        None
    }

    /// Writes the picked `color` into the colour row `key` a [`ColorRequest`] named. Does nothing by default.
    fn answer_color(&mut self, key: &str, color: craft_theme::Srgba) {
        let _ = (key, color);
    }

    /// Sets every field of `fields` or none, as [`set_json_field`](StackDialog::set_json_field) sets one: the
    /// default snapshots [`json_fields`](StackDialog::json_fields), sets each field in turn and, on the first
    /// refusal, restores the snapshot and returns the refusal.
    fn set_json_fields(&mut self, ctx: &Context, fields: &[(String, Value)]) -> Result<(), String> {
        let snapshot = self.json_fields();
        for (field, value) in fields {
            if let Err(refusal) = self.set_json_field(ctx, field, value.clone()) {
                if let Some(Value::Object(before)) = &snapshot {
                    for (name, old) in before {
                        let _ = self.set_json_field(ctx, name, old.clone());
                    }
                }
                return Err(refusal);
            }
        }
        Ok(())
    }

    /// Whether the dialog has a Reset now. `false` by default.
    fn can_reset(&self) -> bool {
        false
    }

    /// Puts the values back as the dialog's Reset does; the dialog stays open. `false` when it has no Reset (the
    /// default).
    fn reset(&mut self, _ctx: &Context) -> bool {
        false
    }
}

/// Why a dialog refused `ui.dialog.set`; each has one wording, which every dialog's
/// [`set_json_field`](StackDialog::set_json_field) returns through [`message`](FieldRefusal::message).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum FieldRefusal<'a> {
    /// The dialog has no fields.
    NoFields,
    /// The dialog has no field of that name.
    Unknown,
    /// The field's row is hidden or disabled.
    Inactive,
    /// A field automation reads but cannot set.
    ReadOnly,
    /// The field cannot hold this value.
    Rejects(&'a serde_json::Value),
}

impl FieldRefusal<'_> {
    /// The refusal's wording for `field`.
    pub fn message(&self, field: &str) -> String {
        match self {
            FieldRefusal::NoFields => format!("unknown field `{field}`: this dialog has no fields"),
            FieldRefusal::Unknown => format!("unknown field `{field}`"),
            FieldRefusal::Inactive => format!("`{field}` takes no input now: its row is hidden or disabled"),
            FieldRefusal::ReadOnly => format!("`{field}` is read only"),
            FieldRefusal::Rejects(value) => format!("`{field}` can't take {value}"),
        }
    }
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

/// A dialog one open dialog asks the host to open as its child ([`StackDialog::take_child`]).
pub struct ChildDialog(Box<dyn AnyStackDialog>);

impl ChildDialog {
    /// Wraps `dialog` for [`StackDialog::take_child`].
    pub fn new<D: StackDialog>(dialog: D) -> Self {
        Self(Box::new(dialog))
    }
}

impl std::fmt::Debug for ChildDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildDialog").finish_non_exhaustive()
    }
}

/// Type-erased [`StackDialog`] the host stores.
trait AnyStackDialog: Send {
    fn id(&self) -> DialogId;
    fn modeless(&self) -> bool;
    fn result_type(&self) -> TypeId;
    fn key(&self) -> Option<String>;
    fn instance(&self) -> Option<DialogId>;
    fn confirm_erased(&mut self, ctx: &Context) -> Option<Box<dyn Any + Send>>;
    fn held_state(&self) -> Option<HeldOk>;
    fn canvas(&self) -> Option<egui::Id>;
    fn json_fields(&self) -> Option<Value>;
    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String>;
    fn show_erased(&mut self, ui: &mut Ui, stack: &DialogStack) -> Option<Box<dyn Any + Send>>;
    /// [`StackDialog::take_child`], unwrapped.
    fn take_child_erased(&mut self) -> Option<Box<dyn AnyStackDialog>>;
    /// `DialogResult::<D::Result>::Cancel`, boxed, for a dialog closed from outside.
    fn cancelled(&self) -> Box<dyn Any + Send>;
    /// [`StackDialog::cancelled`].
    fn release(&mut self, ctx: &Context);
    fn preview_color(&mut self, key: &str, color: Option<craft_theme::Srgba>);
    fn take_color_request(&mut self) -> Option<ColorRequest>;
    fn answer_color(&mut self, key: &str, color: craft_theme::Srgba);
    fn set_json_fields(&mut self, ctx: &Context, fields: &[(String, Value)]) -> Result<(), String>;
    fn can_reset(&self) -> bool;
    fn reset(&mut self, ctx: &Context) -> bool;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<D: StackDialog> AnyStackDialog for D {
    fn modeless(&self) -> bool {
        StackDialog::modeless(self)
    }

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

    fn held_state(&self) -> Option<HeldOk> {
        StackDialog::held_state(self)
    }

    fn canvas(&self) -> Option<egui::Id> {
        StackDialog::canvas(self)
    }

    fn json_fields(&self) -> Option<Value> {
        StackDialog::json_fields(self)
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        StackDialog::set_json_field(self, ctx, field, value)
    }

    fn show_erased(&mut self, ui: &mut Ui, stack: &DialogStack) -> Option<Box<dyn Any + Send>> {
        self.show(ui, stack).map(|result| Box::new(result) as Box<dyn Any + Send>)
    }

    fn take_child_erased(&mut self) -> Option<Box<dyn AnyStackDialog>> {
        self.take_child().map(|ChildDialog(child)| child)
    }

    fn release(&mut self, ctx: &Context) {
        StackDialog::cancelled(self, ctx);
    }

    fn cancelled(&self) -> Box<dyn Any + Send> {
        Box::new(DialogResult::<D::Result>::Cancel)
    }

    fn preview_color(&mut self, key: &str, color: Option<craft_theme::Srgba>) {
        StackDialog::preview_color(self, key, color);
    }

    fn take_color_request(&mut self) -> Option<ColorRequest> {
        StackDialog::take_color_request(self)
    }

    fn answer_color(&mut self, key: &str, color: craft_theme::Srgba) {
        StackDialog::answer_color(self, key, color);
    }

    fn set_json_fields(&mut self, ctx: &Context, fields: &[(String, Value)]) -> Result<(), String> {
        StackDialog::set_json_fields(self, ctx, fields)
    }

    fn can_reset(&self) -> bool {
        StackDialog::can_reset(self)
    }

    fn reset(&mut self, ctx: &Context) -> bool {
        StackDialog::reset(self, ctx)
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
/// - Every dialog on the stack is one the host holds: [`run`](DialogHost::run) shows each at its place on the
///   stack, so it draws and takes input there.
/// - Automation: [`get`](DialogHost::get) and [`get_mut`](DialogHost::get_mut) reach the open
///   dialog's value for code that knows its type. A script that doesn't goes by id:
///   [`open_dialogs`](DialogHost::open_dialogs) lists every open dialog with its
///   [`json_fields`](StackDialog::json_fields), [`set_json_field`](DialogHost::set_json_field) sets one,
///   and [`confirm_by_id`](DialogHost::confirm_by_id) and [`cancel_by_id`](DialogHost::cancel_by_id) close it,
///   whatever its type.
#[derive(Default)]
pub struct DialogHost {
    stack: DialogStack,
    open: Vec<OpenedDialog>,
    queued: Vec<(StartupStage, Box<dyn AnyStackDialog>)>,
    results: Vec<StoredResult>,
    /// Dialogs another of their type replaced, waiting for the pass's `run` to run their
    /// [`StackDialog::cancelled`] (`open` has no `Context`).
    replaced: Vec<OpenedDialog>,
    /// The number the last open got ([`open_serial`](DialogHost::open_serial)).
    opens: u64,
    /// Dialogs [`close_by_id`](DialogHost::close_by_id) asked to close, closed by the next `run`
    /// (which has the `Context` a close needs).
    closing: Vec<DialogId>,
}

/// Owns an opening's identity from the time it joins the stack until its result is stored.
struct OpenedDialog {
    dialog: Box<dyn AnyStackDialog>,
    serial: u64,
}

impl Deref for OpenedDialog {
    type Target = dyn AnyStackDialog;

    fn deref(&self) -> &Self::Target {
        self.dialog.as_ref()
    }
}

impl DerefMut for OpenedDialog {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.dialog.as_mut()
    }
}

/// One open dialog as automation sees it ([`DialogHost::open_dialogs`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct OpenDialog {
    /// The id the `_by_id` calls take.
    pub id: DialogId,
    /// The dialog it was opened from, for a child.
    pub parent: Option<DialogId>,
    /// [`StackDialog::key`].
    pub key: Option<String>,
    /// [`StackDialog::json_fields`].
    pub fields: Option<Value>,
}

/// A finished dialog's answer, with the key the dialog had.
struct StoredResult {
    type_id: TypeId,
    id: DialogId,
    serial: u64,
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
    /// Whether its open stack holds a dialog that is not modeless; current state, so a dialog opened this pass counts.
    /// A queued startup dialog is not shown yet, so it does not count.
    pub fn shows_modal(&self) -> bool {
        self.open.iter().any(|dialog| !dialog.modeless())
    }

    /// An empty host with no open or queued dialogs.
    pub fn new() -> Self {
        Self::default()
    }

    /// The routing stack: every open dialog in opening order. The top one takes the input.
    pub fn stack(&self) -> &DialogStack {
        &self.stack
    }

    /// Opens `dialog` on top. Its id goes on the stack now; the dialog shows from the next
    /// [`run`](DialogHost::run), open: a dialog of the same id that closed itself before starts open again, since the
    /// host forgets a removed dialog's open state (`overlay::end_stack_dialog`). Returns the id.
    pub fn open<D: StackDialog>(&mut self, dialog: D) -> DialogId {
        self.open_with_parent(Box::new(dialog), None)
    }

    /// Opens `dialog` unless a dialog of its type is already open; the first keeps the input and
    /// `dialog` is returned in `Err`. Use it for prompts that must not replace each other (a
    /// delete confirmation open when the changed-on-disk question arrives).
    pub fn open_unless_open<D: StackDialog>(&mut self, dialog: D) -> Result<DialogId, D> {
        if self.is_open::<D>() { Err(dialog) } else { Ok(self.open(dialog)) }
    }

    /// [`open_unless_open`](Self::open_unless_open) from inside a pass: a dialog it opens takes the Escape that arrived
    /// in this pass, so the focused component beneath (a canvas's frame part, say) gets no Cancel. Plain `open` has no
    /// `Context`, and the dialog shows from the next pass, after the components beneath have read the key.
    pub fn open_unless_open_in<D: StackDialog>(&mut self, ctx: &Context, dialog: D) -> Result<DialogId, D> {
        let opened = self.open_unless_open(dialog);
        if opened.is_ok() {
            crate::runtime::overlay::take_keys_for_new_dialog(ctx);
        }
        opened
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

    /// Runs every open dialog of the host for this pass in stack order, each at its place on the stack; removes the
    /// ones that finished and stores their results; opens the next queued startup modal when the stack is empty.
    /// Call once per pass, after the app's own UI. Each pass of a frame shows every dialog again and applies only its
    /// own input. A dialog opened during the run shows from the next pass. A dialog the host removes, here or by a
    /// confirm, cancel or replacing open, ends its shown life at once, so its next open is a first pass.
    pub fn run(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.release_replaced(&ctx);
        for id in std::mem::take(&mut self.closing) {
            self.cancel_by_id(&ctx, id);
        }
        self.open_next_queued();
        let ids: Vec<DialogId> = self.stack.ids().collect();
        for (place, id) in ids.into_iter().enumerate() {
            // A dialog that finished earlier in the run took the dialogs over it off the stack.
            if !self.stack.contains(id) {
                continue;
            }
            let Some(at) = self.position_by_id(id) else { continue };
            let stack = &self.stack;
            let dialog = &mut self.open[at];
            let shown = crate::runtime::overlay::stack_place(&ctx, place, id, || dialog.show_erased(ui, stack));
            // The child is taken after every show, so a request is never left for a later pass; a dialog that just
            // finished would close its child with it, so its request is dropped.
            let child = dialog.take_child_erased();
            let Some(result) = shown else {
                if let Some(child) = child {
                    // The child is not on this run's `ids`, so it shows from the next pass, as any open during the run.
                    self.open_with_parent(child, Some(id));
                }
                continue;
            };
            self.remove_tree(id);
            let dialog = self.open.remove(at);
            crate::runtime::overlay::end_stack_dialog(&ctx, id);
            self.store_result(&dialog, result);
            self.cancel_orphans(Some(&ctx));
        }
        if self.open_next_queued() {
            crate::runtime::frame::request_repaint(&ctx);
        }
    }

    /// Takes `id` and the dialogs opened over it off the stack; [`cancel_orphans`](DialogHost::cancel_orphans) then
    /// closes the ones over it.
    fn remove_tree(&mut self, id: DialogId) {
        self.stack.remove_tree(id);
    }

    /// The result of the last `D` that finished or was cancelled, once. `None` when no result
    /// waits: it was taken already, or no `D` has finished. Opening a `D` drops an untaken result
    /// of the type; if that `open` replaced a `D` that was still open, the replaced one's
    /// `Cancel` waits instead. Results are per type, so the app tells two `D` apart by what the
    /// result carries (a key, a tag). A `D` opened with an instance ([`StackDialog::instance`])
    /// keeps its result apart, for [`take_keyed`](DialogHost::take_keyed) and
    /// [`take_by_id`](DialogHost::take_by_id).
    pub fn take<D: StackDialog>(&mut self) -> Option<DialogResult<D::Result>> {
        let at = self.results.iter().rposition(|stored| stored.type_id == TypeId::of::<D>() && stored.instance.is_none())?;
        self.take_at::<D>(at).map(|(result, _)| result)
    }

    /// The result of the last `D` that finished or was cancelled, whatever its instance, with the
    /// key the dialog had ([`StackDialog::key`]), once: the key tells which command it was, a
    /// `Cancel` after a replace-by-open included. `None` key for a dialog without one.
    pub fn take_keyed<D: StackDialog>(&mut self) -> Option<(DialogResult<D::Result>, Option<String>)> {
        let at = self.results.iter().rposition(|stored| stored.type_id == TypeId::of::<D>())?;
        self.take_at::<D>(at)
    }

    /// [`take`](DialogHost::take) for the dialog `id` names, among several `D` of different
    /// instances.
    pub fn take_by_id<D: StackDialog>(&mut self, id: DialogId) -> Option<DialogResult<D::Result>> {
        let at = self.results.iter().rposition(|stored| stored.type_id == TypeId::of::<D>() && stored.id == id)?;
        self.take_at::<D>(at).map(|(result, _)| result)
    }

    /// Takes the result of the opening identified by this host's [`open_serial`](Self::open_serial).
    /// A replacement cancel uses the serial of the dialog being replaced. As with [`take`](Self::take), a later
    /// opening or result for the same type and instance can displace an untaken result.
    pub fn take_by_open_serial<D: StackDialog>(&mut self, serial: u64) -> Option<DialogResult<D::Result>> {
        let at = self.results.iter().rposition(|stored| stored.type_id == TypeId::of::<D>() && stored.serial == serial)?;
        self.take_at::<D>(at).map(|(result, _)| result)
    }

    fn take_at<D: StackDialog>(&mut self, at: usize) -> Option<(DialogResult<D::Result>, Option<String>)> {
        let stored = self.results.remove(at);
        let result = stored.result.downcast::<DialogResult<D::Result>>().ok()?;
        Some((*result, stored.key))
    }

    /// Answers the open `D` with OK, as its default button would (for automation: no pointer, no
    /// pass): runs what OK runs, closes the dialog and its children, and keeps the answer for
    /// [`take`](DialogHost::take). `false` when no `D` is open, OK is disabled, `D` has no
    /// programmatic OK, or `D` holds its OK: the answer is filed for the app
    /// ([`held_state`](DialogHost::held_state) says `Waiting`), and a `confirm` after the app accepted it
    /// closes the dialog with it.
    pub fn confirm<D: StackDialog>(&mut self, ctx: &Context) -> bool {
        match self.position::<D>() {
            Some(at) => self.confirm_at(ctx, at),
            None => false,
        }
    }

    /// Where the held OK of the open dialog `id` names stands ([`StackDialog::held_state`]); `None` when no such dialog
    /// is open or its OK holds nothing now.
    pub fn held_state(&self, id: DialogId) -> Option<HeldOk> {
        self.open.iter().find(|dialog| dialog.id() == id)?.held_state()
    }

    /// The canvas of the most recently opened dialog that shows the document on one ([`StackDialog::canvas`]): where
    /// the app's tool input in document points goes while it is open. `None` when no open dialog has one.
    pub fn canvas(&self) -> Option<egui::Id> {
        self.open.iter().rev().find_map(|dialog| dialog.canvas())
    }

    /// [`confirm`](DialogHost::confirm) for the open dialog `id` names, whatever its type.
    pub fn confirm_by_id(&mut self, ctx: &Context, id: DialogId) -> bool {
        match self.position_by_id(id) {
            Some(at) => self.confirm_at(ctx, at),
            None => false,
        }
    }

    /// Refuses (returns `false`) while a child is open over the dialog: the child's answer comes first, as the
    /// disabled parent shows on screen.
    fn confirm_at(&mut self, ctx: &Context, at: usize) -> bool {
        if self.stack.has_child(self.open[at].id()) {
            return false;
        }
        let Some(result) = self.open[at].confirm_erased(ctx) else { return false };
        let id = self.open[at].id();
        self.remove_tree(id);
        let dialog = self.open.remove(at);
        crate::runtime::overlay::end_stack_dialog(ctx, id);
        self.store_result(&dialog, result);
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

    /// Passes a child picker's live colour to the open dialog `id` names ([`StackDialog::preview_color`]); does
    /// nothing when no dialog has that id.
    pub fn preview_color(&mut self, id: DialogId, key: &str, color: Option<craft_theme::Srgba>) {
        if let Some(dialog) = self.open.iter_mut().find(|dialog| dialog.id() == id) {
            dialog.preview_color(key, color);
        }
    }

    /// [`cancel`](DialogHost::cancel) for the open dialog `id` names, whatever its type.
    pub fn cancel_by_id(&mut self, ctx: &Context, id: DialogId) -> bool {
        match self.position_by_id(id) {
            Some(at) => self.cancel_at(ctx, at),
            None => false,
        }
    }

    /// Asks for the open dialog `id` names to close as a cancel, for a caller with no `Context`
    /// (a service that drops the request the dialog answers). The close lands at the next
    /// [`run`](DialogHost::run); asking again before then, or for a dialog that is not open, does
    /// nothing more.
    pub fn close_by_id(&mut self, id: DialogId) {
        if self.position_by_id(id).is_some() && !self.closing.contains(&id) {
            self.closing.push(id);
        }
    }

    fn cancel_at(&mut self, ctx: &Context, at: usize) -> bool {
        let id = self.open[at].id();
        self.remove_tree(id);
        let mut dialog = self.open.remove(at);
        crate::runtime::overlay::end_stack_dialog(ctx, id);
        dialog.release(ctx);
        self.store_result(&dialog, dialog.cancelled());
        self.cancel_orphans(Some(ctx));
        true
    }

    /// The open dialogs in opening order (the top one last), with their fields.
    pub fn open_dialogs(&self) -> Vec<OpenDialog> {
        self.open
            .iter()
            .map(|dialog| OpenDialog { id: dialog.id(), parent: self.stack.parent(dialog.id()), key: dialog.key(), fields: dialog.json_fields() })
            .collect()
    }

    /// Sets `field` of the open dialog `id` names ([`StackDialog::set_json_field`]), whatever its type.
    pub fn set_json_field(&mut self, ctx: &Context, id: DialogId, field: &str, value: Value) -> Result<(), String> {
        match self.open.iter_mut().find(|dialog| dialog.id() == id) {
            Some(dialog) => {
                let answer = dialog.set_json_field(ctx, field, value);
                if answer.is_ok() {
                    ctx.request_repaint();
                }
                answer
            }
            None => Err("no such dialog".to_owned()),
        }
    }

    /// The top-most open dialog's pending [`ColorRequest`] with its id; the app opens a colour picker over it and
    /// answers through [`answer_color`](DialogHost::answer_color).
    pub fn take_color_request(&mut self) -> Option<(DialogId, ColorRequest)> {
        self.open.iter_mut().rev().find_map(|dialog| {
            let id = dialog.id();
            dialog.take_color_request().map(|request| (id, request))
        })
    }

    /// Writes the picked `color` into colour row `key` of the open dialog `id`.
    pub fn answer_color(&mut self, id: DialogId, key: &str, color: craft_theme::Srgba) {
        if let Some(dialog) = self.open.iter_mut().find(|dialog| dialog.id() == id) {
            dialog.answer_color(key, color);
        }
    }

    /// Sets every field of `fields` on the open dialog `id` or none ([`StackDialog::set_json_fields`]).
    pub fn set_json_fields(&mut self, ctx: &Context, id: DialogId, fields: &[(String, Value)]) -> Result<(), String> {
        match self.open.iter_mut().find(|dialog| dialog.id() == id) {
            Some(dialog) => {
                let answer = dialog.set_json_fields(ctx, fields);
                if answer.is_ok() {
                    ctx.request_repaint();
                }
                answer
            }
            None => Err("no such dialog".to_owned()),
        }
    }

    /// Runs the reset of the open dialog `id`; `false` for an unknown id or a dialog without a Reset.
    pub fn reset_by_id(&mut self, ctx: &Context, id: DialogId) -> bool {
        let Some(dialog) = self.open.iter_mut().find(|dialog| dialog.id() == id) else { return false };
        let done = dialog.reset(ctx);
        if done {
            ctx.request_repaint();
        }
        done
    }

    /// Whether the open dialog `id` has a Reset now; `false` for an unknown id.
    pub fn can_reset_by_id(&self, id: DialogId) -> bool {
        self.open.iter().find(|dialog| dialog.id() == id).is_some_and(|dialog| dialog.can_reset())
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

    /// The first open `D` with `id`. Other dialog types with the same ID are ignored.
    pub fn get_by_id<D: StackDialog>(&self, id: DialogId) -> Option<&D> {
        self.open[self.position_by_id_of::<D>(id)?].as_any().downcast_ref()
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
        let at = self.position_by_id_of::<D>(id)?;
        self.open[at].as_any_mut().downcast_mut()
    }

    /// The live `D` of this opening, while it is still open on this host.
    pub fn get_by_open_serial<D: StackDialog>(&self, serial: u64) -> Option<&D> {
        self.open.iter().find(|dialog| dialog.serial == serial)?.as_any().downcast_ref()
    }

    /// [`get_by_open_serial`](Self::get_by_open_serial), mutably.
    pub fn get_mut_by_open_serial<D: StackDialog>(&mut self, serial: u64) -> Option<&mut D> {
        self.open.iter_mut().find(|dialog| dialog.serial == serial)?.as_any_mut().downcast_mut()
    }

    /// The most recently opened `D`.
    fn position<D: StackDialog>(&self) -> Option<usize> {
        self.open.iter().rposition(|dialog| dialog.result_type() == TypeId::of::<D>())
    }

    fn position_by_id(&self, id: DialogId) -> Option<usize> {
        self.open.iter().position(|dialog| dialog.id() == id)
    }

    fn position_by_id_of<D: StackDialog>(&self, id: DialogId) -> Option<usize> {
        self.open.iter().position(|dialog| dialog.id() == id && dialog.result_type() == TypeId::of::<D>())
    }

    fn store_result(&mut self, dialog: &OpenedDialog, result: Box<dyn Any + Send>) {
        let (type_id, instance) = (dialog.result_type(), dialog.instance());
        self.results.retain(|stored| stored.type_id != type_id || stored.instance != instance);
        self.results.push(StoredResult { type_id, id: dialog.id(), serial: dialog.serial, instance, key: dialog.key(), result });
    }

    /// The newest live opening of `id`: a number distinct for every open on this host, the same while the dialog
    /// stays open. A dialog's id is the same at every open of its key, so the serial distinguishes one opening from
    /// the next. `None` when `id` is not open.
    pub fn open_serial(&self, id: DialogId) -> Option<u64> {
        self.open.iter().rfind(|dialog| dialog.id() == id).map(|dialog| dialog.serial)
    }

    /// Like [`open_serial`](Self::open_serial), restricted to the dialog type `D`.
    /// Use [`get_by_open_serial`](Self::get_by_open_serial) to access this exact opening after capturing its serial.
    pub fn open_serial_by_id<D: StackDialog>(&self, id: DialogId) -> Option<u64> {
        self.open.iter().rfind(|dialog| dialog.id() == id && dialog.result_type() == TypeId::of::<D>()).map(|dialog| dialog.serial)
    }

    fn open_with_parent(&mut self, dialog: Box<dyn AnyStackDialog>, parent: Option<DialogId>) -> DialogId {
        let (type_id, instance) = (dialog.result_type(), dialog.instance());
        self.results.retain(|stored| stored.type_id != type_id || stored.instance != instance);
        if let Some(at) = self.open.iter().position(|open| open.result_type() == type_id && open.instance() == instance) {
            self.remove_tree(self.open[at].id());
            let old = self.open.remove(at);
            self.store_result(&old, old.cancelled());
            self.replaced.push(old);
            self.cancel_orphans(None);
        }
        let id = dialog.id();
        self.opens += 1;
        self.stack.push(id, parent);
        self.open.push(OpenedDialog { dialog, serial: self.opens });
        id
    }

    /// Closes the open dialogs the stack no longer holds (children of a dialog that closed), with
    /// `Cancel`. Without a `Context` their [`StackDialog::cancelled`] waits for the next `run`.
    fn cancel_orphans(&mut self, ctx: Option<&Context>) {
        let (kept, orphans): (Vec<_>, Vec<_>) = std::mem::take(&mut self.open).into_iter().partition(|dialog| self.stack.contains(dialog.id()));
        self.open = kept;
        for mut dialog in orphans {
            self.store_result(&dialog, dialog.cancelled());
            match ctx {
                Some(ctx) => {
                    crate::runtime::overlay::end_stack_dialog(ctx, dialog.id());
                    dialog.release(ctx);
                }
                None => self.replaced.push(dialog),
            }
        }
    }

    /// Runs [`StackDialog::cancelled`] on the dialogs `open` closed without a `Context`.
    fn release_replaced(&mut self, ctx: &Context) {
        for mut dialog in std::mem::take(&mut self.replaced) {
            crate::runtime::overlay::end_stack_dialog(ctx, dialog.id());
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
    /// are blocked meanwhile (`any_modal` answers for the frame).
    pub fn is_empty(&self) -> bool {
        self.open.is_empty() && self.queued.is_empty()
    }
}
