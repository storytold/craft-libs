//! EnvView, the part of Env that component code sees.

use super::app_values::EnvValue;
use super::label::Label;
use super::units::{PointSize, UnitWords, Units, ZoomPrecision};
use super::values::{MotionPolicy, Scale};
use super::{Held, ItemFacts, Scoped};
use crate::runtime::commands::CommandChain;
use craft_host::Host;
use craft_host::target::ShellInfo;
use craft_i18n::{AgeStyle, Counted, Direction, WordSet};
use craft_i18n::{CalendarDate, DateFormat, Format};
use craft_theme::Mode;
use egui::Id;
use std::sync::Arc;

/// What component code sees: everything in `Env` except tokens, size, enabled, read-only, arena, transform
/// and `ids_stable`.
#[derive(Clone, Copy)]
pub struct EnvView<'a> {
    pub(super) scoped: &'a Scoped,
}

impl<'a> EnvView<'a> {
    /// The footer a `FooterSlot` under this scope places, if a container offers one.
    pub(crate) fn footer_slot(&self) -> Option<super::FooterSlotOf> {
        self.scoped.footer_slot
    }

    /// The height, icon side, padding, gap and field widths of a control of `size`.
    pub(crate) fn control(&self, size: craft_theme::ControlSize) -> craft_behavior::ControlMetrics {
        self.scoped.control(size)
    }

    /// Whether an enclosing body is measuring the smallest width its children can use.
    #[doc(hidden)]
    pub fn measuring_least_width(&self) -> bool {
        self.scoped.least
    }

    /// The facts the collection recorded for the item `id`.
    #[doc(hidden)]
    pub fn facts_of(&self, id: Id) -> Option<&ItemFacts> {
        self.scoped.facts_of(id)
    }

    /// How wheel takers in this scope take the wheel.
    pub(crate) fn wheel(&self) -> super::WheelScope {
        self.scoped.wheel
    }

    /// The innermost scrolling body around the component.
    pub(crate) fn scroll_owner(&self) -> Option<Id> {
        self.scoped.scroll_owner.map(|owner| owner.id)
    }

    pub(crate) fn fallback_collection(&self) -> Option<Id> {
        self.scoped.fallback_collection
    }

    /// The view of the innermost scroll area around the scope.
    pub(crate) fn lazy_view(&self) -> Option<crate::runtime::lazy::View> {
        self.scoped.lazy
    }

    /// The line length paragraphs wrap at in this scope, if a container around it sets one.
    pub(crate) fn measure(&self) -> Option<f32> {
        self.scoped.measure
    }

    /// The content-sized `Stack` column whose first sweep runs now, if the scope is a direct child of it in flow.
    pub(crate) fn column_first_sweep(&self) -> Option<Id> {
        match self.scoped.column {
            Some(super::ColumnSweep::Count(column)) => Some(column),
            _ => None,
        }
    }

    /// The inside label the scope carries ([`inside_label`](EnvView::inside_label) reads it back).
    pub(crate) fn inside_held(&self) -> Option<&'a Held> {
        self.scoped.inside_label.as_ref()
    }

    /// The surface the component sits on: the innermost container or collection surface that paints a fill, else the
    /// window's background.
    pub fn surface(&self) -> crate::style::SurfaceKind {
        self.scoped.surface
    }

    /// The flow direction: layout order, arrow keys and roving.
    pub fn direction(&self) -> Direction {
        self.scoped.direction
    }

    /// The paragraph direction of text, which only a scope that sets the interface's direction changes.
    pub fn text_direction(&self) -> Direction {
        self.scoped.text_direction
    }

    /// The light or dark mode the scope's tokens are resolved in: the app's, or the one a scope with its own tokens
    /// set.
    pub fn mode(&self) -> Mode {
        self.scoped.token_inputs.mode
    }

    /// The motion mode in effect.
    pub fn motion(&self) -> MotionPolicy {
        self.scoped.motion
    }

    /// Whether the canvas surround shows the theme's pattern (`RootEnv::canvas_pattern`).
    pub fn canvas_pattern(&self) -> bool {
        self.scoped.canvas_pattern
    }

    /// The pixels per point.
    pub fn scale(&self) -> Scale {
        self.scoped.scale
    }

    #[doc(hidden)]
    pub fn command_chain(&self) -> &CommandChain {
        &self.scoped.commands
    }

    /// The platform and the running viewport's window facts: the OS, the capabilities and the window's kind,
    /// chrome, state, insets and focus.
    pub fn shell(&self) -> &'a ShellInfo {
        crate::runtime::replay::note_input(&self.scoped.ctx, crate::runtime::inputs::Input::Shell);
        &self.scoped.shell
    }

    /// The keyboard layout: which character each key position types, for text that names a key.
    pub fn keyboard_layout(&self) -> &'a craft_keymap::Layout {
        crate::runtime::replay::note_input(&self.scoped.ctx, crate::runtime::inputs::Input::Keyboard);
        &self.scoped.keyboard
    }

    /// The app's `Host`, from `FrameInput.host`.
    pub fn host(&self) -> Option<&'a Arc<Host>> {
        self.scoped.host.as_ref()
    }

    /// A clone of the innermost app value, else `T::root()`.
    pub fn get<T: EnvValue>(&self) -> T {
        self.scoped.values.get::<T>()
    }

    /// The word set `L` in the scope's language, built once per `Words`.
    pub fn words<L: WordSet + Send + Sync + 'static>(&self) -> Arc<L> {
        self.scoped.words.set::<L>()
    }

    /// How the scope writes numbers: locale digits, separators and grouping.
    pub fn format(&self) -> &'a Format {
        &self.scoped.format
    }

    /// How the scope writes dates: the host's catalog format, else the system region's.
    pub fn dates(&self) -> Arc<DateFormat> {
        Arc::clone(&self.scoped.dates)
    }

    /// Today's date as the host supplied it with `RootEnv::today`; `None` when it gave none.
    pub fn today(&self) -> Option<CalendarDate> {
        self.scoped.today
    }

    /// The scope's number format and unit words, to read and write numbers with their unit.
    pub fn units(&self) -> Units {
        Units::new(self.scoped.format.clone(), self.words::<UnitWords>(), self.scoped.words.clone(), self.scoped.values.get::<PointSize>())
            .with_zoom_precision(self.scoped.values.get::<ZoomPrecision>())
    }

    /// How long ago `seconds` was, in the scope's language and digits: "Just now", "3 minutes ago", "Yesterday".
    pub fn relative_age(&self, seconds: u64, style: AgeStyle) -> String {
        craft_i18n::relative_age(&self.scoped.words, &self.scoped.format, seconds, style)
    }

    /// `n` things by the count word `count`, in the scope's language and digits: "1 page", "3 pages".
    pub fn counted(&self, count: Counted, n: u64) -> String {
        self.scoped.words.counted(count, n, &self.scoped.format.integer(n))
    }

    /// `label` with a `Word` replaced by its text in the scope's language: what domain code that draws a `Label`
    /// itself shows.
    pub fn resolve(&self, label: &Label) -> Label {
        label.resolved(&self.scoped.words)
    }

    /// The env this view shows, owned: what the drag preview, which paints after the region it started in, paints with.
    pub(crate) fn to_env(self) -> super::Env {
        super::Env { scoped: Arc::new(self.scoped.clone()) }
    }

    /// The scope's resolved tokens, shared: what a drop target's mark paints with.
    pub(crate) fn shared_tokens(&self) -> Arc<crate::style::UiTokens> {
        self.scoped.tokens.clone()
    }
}
