//! `Common`, `Props`, `Resolved` and `Tooltip` (chapter 3).

use super::{IdSource, Internal, Meta};
use crate::runtime::commands::{CommandRef, CommandScope};
use crate::runtime::env::Label;
use craft_behavior::DocKey;
use craft_theme::{ControlSize, Variant};
use egui::{AsIdSalt, Id, IdSalt};

/// What every component carries: the props the pipeline reads at step 1.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Common {
    pub(crate) id: IdSource,
    #[doc(hidden)]
    pub label: Label,
    pub(crate) enabled: Option<bool>,
    pub(crate) read_only: Option<bool>,
    pub(crate) size: Option<ControlSize>,
    pub(crate) variant: Option<Variant>,
    #[doc(hidden)]
    pub tooltip: Option<Tooltip>,
    #[doc(hidden)]
    pub description: Option<String>,
    #[doc(hidden)]
    pub selected: Option<bool>,
    pub(crate) command: Option<CommandRef>,
    /// Step 16 reads it into `Access` for the label-scope patch (`described_by` instead of `labelled_by`).
    pub(crate) keeps_name: bool,
    pub(crate) access_key: Option<char>,
    pub(crate) default_stop: bool,
    pub(crate) doc_key: Option<DocKey>,
    /// The scope a leaf is an instance of while it holds key focus.
    pub(crate) command_scope: Option<CommandScope>,
    /// The component's horizontal axis keeps its left-to-right order under a right-to-left interface.
    pub(crate) ltr_axis: bool,
}

impl Common {
    /// A `Common` with every prop unset, which the derive's constructors start from.
    pub const fn new() -> Common {
        Common {
            id: IdSource::Auto,
            label: Label::Empty,
            enabled: None,
            read_only: None,
            size: None,
            variant: None,
            tooltip: None,
            description: None,
            selected: None,
            command: None,
            keeps_name: false,
            access_key: None,
            default_stop: false,
            doc_key: None,
            command_scope: None,
            ltr_axis: false,
        }
    }
}

/// The setters every component gets from `Meta`.
pub trait Props: Meta + Sized {
    /// Salts the id with `salt`.
    fn id_salt(mut self, salt: impl AsIdSalt) -> Self {
        self.common_mut(Internal::new()).id = IdSource::Salt(IdSalt::new(salt));
        self
    }

    /// Uses `id` as the component's id.
    fn id(mut self, id: Id) -> Self {
        self.common_mut(Internal::new()).id = IdSource::Id(id);
        self
    }

    /// The visible label.
    fn label(mut self, label: impl Into<Label>) -> Self {
        self.common_mut(Internal::new()).label = label.into();
        self
    }

    /// Enables or disables the component; only `false` has an effect below an enabled scope.
    fn enabled(mut self, on: bool) -> Self {
        self.common_mut(Internal::new()).enabled = Some(on);
        self
    }

    /// Refuses every edit under this component; focus, selection and copy keep working.
    fn read_only(mut self, on: bool) -> Self {
        self.common_mut(Internal::new()).read_only = Some(on);
        self
    }

    /// The control size, over the scope's.
    fn size(mut self, size: ControlSize) -> Self {
        self.common_mut(Internal::new()).size = Some(size);
        self
    }

    /// The variant; one the component does not accept gives its default.
    fn variant(mut self, variant: impl Into<Variant>) -> Self {
        self.common_mut(Internal::new()).variant = Some(variant.into());
        self
    }

    /// The tooltip.
    fn tooltip(mut self, tooltip: impl Into<Tooltip>) -> Self {
        self.common_mut(Internal::new()).tooltip = Some(tooltip.into());
        self
    }

    /// The accessible description.
    fn description(mut self, text: impl Into<String>) -> Self {
        self.common_mut(Internal::new()).description = Some(text.into());
        self
    }

    /// Marks the component selected, outside a collection.
    fn selected(mut self, on: bool) -> Self {
        self.common_mut(Internal::new()).selected = Some(on);
        self
    }

    /// Binds the component to a command: its label, state and activation follow the command.
    fn command(mut self, command: impl Into<CommandRef>) -> Self {
        self.common_mut(Internal::new()).command = Some(command.into());
        self
    }

    /// The component's own name stays when a label scope would name it; the scope's label describes it.
    fn keeps_name(mut self, on: bool) -> Self {
        self.common_mut(Internal::new()).keeps_name = on;
        self
    }

    /// The letter that activates the component from the keyboard, reported as its AccessKit `access_key`. The
    /// component that owns the key handling reads the letter; the node only reports it.
    fn access_key(mut self, key: char) -> Self {
        self.common_mut(Internal::new()).access_key = Some(key);
        self
    }

    /// A leaf is an instance of `scope` while it holds key focus: commands of `scope` reach it and the items it
    /// shows. Containers and collections take the scope as their own `command_scope` prop.
    fn command_scope(mut self, scope: CommandScope) -> Self
    where
        Self: Meta<Shape = super::shape::Leaf>,
    {
        self.common_mut(Internal::new()).command_scope = Some(scope);
        self
    }

    /// The component's horizontal axis keeps its left-to-right order under a right-to-left interface: a time axis, a
    /// ruler, a document view. The component sits where its parent's flow puts it (mirrored); its own content flow,
    /// keys and roving order, the components inside it, and its access node do not mirror. A container declares it on
    /// the one part of its content that is such an axis, and its other parts mirror.
    fn ltr_axis(mut self) -> Self {
        self.common_mut(Internal::new()).ltr_axis = true;
        self
    }

    /// Marks the component's stop as its focus scope's default: a layer opened with `Initial::Default` focuses it,
    /// so a popover names the field it opens on.
    fn default_stop(mut self) -> Self {
        self.common_mut(Internal::new()).default_stop = true;
        self
    }

    /// The semantic key of the component.
    fn doc_key(mut self, key: DocKey) -> Self {
        self.common_mut(Internal::new()).doc_key = Some(key);
        self
    }
}

impl<T: Meta> Props for T {}

/// `Common` after step 1; it borrows locals of the pipeline frame, never the component.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Resolved<'p> {
    /// The component's id.
    pub id: Id,
    /// The label text.
    pub label: &'p str,
    /// The label as given, for `WordSetCx::label`.
    pub(crate) label_value: &'p Label,
    /// The resolved enabled state.
    pub enabled: bool,
    /// True when the instance or any scope says so; nothing below can make it writable.
    pub read_only: bool,
    /// The control size.
    pub size: ControlSize,
    /// The variant, one the component accepts.
    pub variant: Variant,
    /// `Common.selected`.
    pub selected: Option<bool>,
    /// Read by the label-scope patch: the name stays and the field label describes.
    pub(crate) keeps_name: bool,
    /// `Common.access_key`.
    pub(crate) access_key: Option<char>,
    /// `Common.default_stop`: the stop is its scope's default (`Initial::Default`).
    pub(crate) default_stop: bool,
    /// `Common.ltr_axis`.
    pub(crate) ltr_axis: bool,
    /// `Common.description`.
    pub(crate) description: Option<&'p str>,
    /// `Common.tooltip`.
    #[doc(hidden)]
    pub tooltip: Option<&'p Tooltip>,
    /// The undo adapter's key for edits of this component.
    #[doc(hidden)]
    pub doc_key: Option<DocKey>,
    /// Step 1's resolved command: the bound command as the scopes above the component see it.
    pub(crate) command: Option<&'p crate::runtime::commands::ResolvedCommand>,
}

/// A tooltip: one type for components and parts.
#[derive(Clone, Debug, PartialEq)]
pub struct Tooltip(Box<TooltipSpec>);

/// What a tooltip shows and when.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TooltipSpec {
    /// The text.
    pub text: Label,
    /// The text shown while the control is disabled; the control then stays focusable.
    pub disabled_text: Option<Label>,
    /// Where it appears.
    pub at: TooltipAt,
    /// When it appears.
    pub when: TooltipWhen,
}

/// Where a tooltip appears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TooltipAt {
    /// Below the control.
    Control,
    /// At the pointer.
    Pointer,
}

/// When a tooltip appears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TooltipWhen {
    /// After the hover delay.
    Hover,
    /// At once.
    Now,
}

impl TooltipSpec {
    /// A tooltip of `text` under the control, after the hover delay.
    pub fn new(text: impl Into<Label>) -> Self {
        TooltipSpec { text: text.into(), disabled_text: None, at: TooltipAt::Control, when: TooltipWhen::Hover }
    }

    /// The text shown while the control is disabled.
    pub fn disabled_text(mut self, text: impl Into<Label>) -> Self {
        self.disabled_text = Some(text.into());
        self
    }

    /// Where it appears.
    pub const fn at(mut self, at: TooltipAt) -> Self {
        self.at = at;
        self
    }

    /// When it appears.
    pub const fn when(mut self, when: TooltipWhen) -> Self {
        self.when = when;
        self
    }
}

impl From<TooltipSpec> for Tooltip {
    fn from(spec: TooltipSpec) -> Self {
        Tooltip(Box::new(spec))
    }
}

impl<T: Into<Label>> From<T> for Tooltip {
    fn from(text: T) -> Self {
        Tooltip(Box::new(TooltipSpec::new(text)))
    }
}

impl Tooltip {
    /// What the tooltip shows and when.
    pub fn spec(&self) -> &TooltipSpec {
        &self.0
    }
}
