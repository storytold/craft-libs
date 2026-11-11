//! EnvView, the part of Env that component code sees.

use super::app_values::EnvValue;
use super::label::Label;
use super::units::{PointSize, UnitWords, Units};
use super::values::{MotionPolicy, Scale};
use super::{ItemFacts, Scoped};
use crate::runtime::commands::CommandChain;
use craft_host::Host;
use craft_host::target::ShellInfo;
use craft_i18n::{AgeStyle, Direction, WordSet};
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
    /// The height, icon side, padding and gap of a control of `size`.
    pub(crate) fn control(&self, size: craft_theme::ControlSize) -> craft_behavior::ControlMetrics {
        self.scoped.control(size)
    }

    /// The facts the collection recorded for the item `id`.
    #[doc(hidden)]
    pub fn facts_of(&self, id: Id) -> Option<&ItemFacts> {
        self.scoped.facts_of(id)
    }

    /// Whether wheel takers in this scope take the wheel only while focused or armed.
    pub(crate) fn wheel_gated(&self) -> bool {
        self.scoped.wheel_gated
    }

    /// The innermost scrolling body around the component.
    pub(crate) fn scroll_owner(&self) -> Option<Id> {
        self.scoped.scroll_owner
    }

    /// The view of the innermost scroll area around the scope.
    pub(crate) fn lazy_view(&self) -> Option<crate::runtime::lazy::View> {
        self.scoped.lazy
    }

    /// The roving group of the innermost body with `ContainerProps.roving`.
    pub(crate) fn roving(&self) -> Option<Id> {
        self.scoped.roving
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
        &self.scoped.shell
    }

    /// The keyboard layout: which character each key position types, for text that names a key.
    pub fn keyboard_layout(&self) -> &'a craft_keymap::Layout {
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
    }

    /// How long ago `seconds` was, in the scope's language and digits: "Just now", "3 minutes ago", "Yesterday".
    pub fn relative_age(&self, seconds: u64, style: AgeStyle) -> String {
        craft_i18n::relative_age(&self.scoped.words, &self.scoped.format, seconds, style)
    }

    /// `label` with a `Word` replaced by its text in the scope's language: what domain code that draws a `Label`
    /// itself shows.
    pub fn resolve(&self, label: &Label) -> Label {
        label.resolved(&self.scoped.words)
    }
}
