//! The registrations `#[derive(Component)]` writes, so the suite finds every type with no hand-kept list.
//! Each registration is one `macro_rules!` of craft-ui, expanded in the user's crate and backed by `linkme`;
//! the macro's body depends on craft-ui's own `testing` feature, so a derive carries no `cfg`.

use super::{
    BodyCx, Collection, Component, Container, ItemFn, Meta, Outcome, Pattern, ShapeKind, Show as _, ShowCollection as _, ShowContainer as _,
    ShowShape, ValueOf, shape,
};
use crate::runtime::access::Role;
use crate::runtime::focus::KeyUse;
use craft_behavior::{Input, ItemSource};
use egui::{Rect, Response, Ui};
use std::any::{Any, TypeId};

/// Shows a leaf at a rect: the component, boxed. `None` when the box holds another type.
pub type LeafEntry = fn(&mut Ui, Rect, Box<dyn Any>) -> Option<Response>;

/// Shows a container at a rect with a body: the component, boxed.
pub type ContainerEntry = fn(&mut Ui, Rect, Box<dyn Any>, &mut dyn FnMut(&mut Ui, &mut BodyCx<'_>)) -> Option<Response>;

/// Shows a collection's items: the component, boxed; the source, a `Box<dyn ItemSource<Key>>`; and the item
/// closure, a `Box<ItemFn<'static, C>>`.
pub type CollectionEntry = fn(&mut Ui, Box<dyn Any>, &dyn Any, &mut dyn Any) -> Option<Response>;

/// How the suite shows an instance of a registered type, per shape.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum ShowEntry {
    /// `show_at`.
    Leaf(LeafEntry),
    /// `show_at` with a body.
    Container(ContainerEntry),
    /// `show_items`.
    Collection(CollectionEntry),
}

fn show_leaf<T: Component + Meta<Shape = shape::Leaf> + 'static>(ui: &mut Ui, rect: Rect, component: Box<dyn Any>) -> Option<Response> {
    let component = component.downcast::<T>().ok()?;
    Some(component.show_at(ui, rect).response().clone())
}

fn show_container<T: Container + 'static>(
    ui: &mut Ui,
    rect: Rect,
    component: Box<dyn Any>,
    body: &mut dyn FnMut(&mut Ui, &mut BodyCx<'_>),
) -> Option<Response> {
    let component = component.downcast::<T>().ok()?;
    Some(component.show_at(ui, rect, |ui, cx| body(ui, cx)).response().clone())
}

fn show_collection<T: Collection + 'static>(ui: &mut Ui, component: Box<dyn Any>, source: &dyn Any, item: &mut dyn Any) -> Option<Response> {
    let component = component.downcast::<T>().ok()?;
    let source = source.downcast_ref::<Box<dyn ItemSource<T::Key>>>()?;
    let item = item.downcast_mut::<Box<ItemFn<'static, T>>>()?;
    Some(component.show_items(ui, &**source, &mut **item).response().clone())
}

/// A named shared-case exemption, as `#[component(exempt(case = .., reason = ..))]` declares it.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Exemption {
    /// The shared case the type is exempt from.
    pub case: &'static str,
    /// Why the type cannot meet it.
    pub reason: &'static str,
}

impl Exemption {
    /// An exemption from `case`, for `reason`.
    pub const fn new(case: &'static str, reason: &'static str) -> Self {
        Exemption { case, reason }
    }
}

/// What the suite knows about one registered component type, without an instance: the facts the contract
/// types state and the declarations of `#[component(..)]`.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Probe {
    /// The package that declares the type, as `CARGO_PKG_NAME` spells it.
    pub crate_name: &'static str,
    /// The crate target that declares the type, as `CARGO_CRATE_NAME` spells it: the package's library crate, or
    /// an integration binary.
    pub crate_target: &'static str,
    /// The module path of the declaration.
    pub module: &'static str,
    /// The shape.
    pub shape: ShapeKind,
    /// The AccessKit role.
    pub role: Role,
    /// How input reaches the component.
    pub input: Input,
    /// The keys the focused component keeps.
    pub keys: KeyUse,
    /// How to show an instance.
    pub entry: ShowEntry,
    /// A leaf the `Canvas` body shows.
    pub canvas_control: bool,
    /// The type's `SHOWS_LABEL`: whether it paints its label as visible text.
    pub shows_label: bool,
    /// The type's `SINGLE_VALUE`: it edits one value and has no mixed state.
    pub single_value: bool,
    /// The type's `paint_outset`: how far it paints outside its rect by design.
    pub paint_outset: fn(&crate::style::UiTokens) -> f32,
    /// The type declares `#[component(overlay)]`.
    pub overlay: bool,
    /// The shared cases the type is exempt from.
    pub exempt: &'static [Exemption],
    /// The first sentence of the type's doc comment.
    pub purpose: &'static str,
    /// Makes an instance of the type, boxed, so `entry` can show a type that has no story: the type's
    /// `#[component(sample = ..)]`.
    #[cfg(feature = "testing")]
    pub sample: Option<fn() -> Box<dyn Any>>,
    /// The story that shows `sample`; the suite runs its cases over it for a type with no story of its own.
    #[cfg(feature = "testing")]
    pub sample_story: Option<fn() -> crate::testing::Story>,
    /// Resolves the style of a boxed instance of the type under a style context; `None` for an instance of
    /// another type.
    #[cfg(feature = "testing")]
    pub looks: fn(&dyn Any, &crate::style::StyleCx<'_>) -> Option<crate::testing::style::StyleLooks>,
    name: fn() -> &'static str,
    value: fn() -> bool,
}

impl Probe {
    /// The probe of the leaf `T`, declared in `module` of the crate target `crate_target` of package `crate_name`.
    pub const fn leaf<T: Component + Meta<Shape = shape::Leaf> + 'static>(
        crate_name: &'static str,
        crate_target: &'static str,
        module: &'static str,
        overlay: bool,
        exempt: &'static [Exemption],
    ) -> Self {
        Self::of::<T>(crate_name, crate_target, module, overlay, exempt, ShowEntry::Leaf(show_leaf::<T>))
    }

    /// The probe of the container `T`.
    pub const fn container<T: Container + 'static>(
        crate_name: &'static str,
        crate_target: &'static str,
        module: &'static str,
        overlay: bool,
        exempt: &'static [Exemption],
    ) -> Self {
        Self::of::<T>(crate_name, crate_target, module, overlay, exempt, ShowEntry::Container(show_container::<T>))
    }

    /// The probe of the collection `T`.
    pub const fn collection<T: Collection + 'static>(
        crate_name: &'static str,
        crate_target: &'static str,
        module: &'static str,
        overlay: bool,
        exempt: &'static [Exemption],
    ) -> Self {
        Self::of::<T>(crate_name, crate_target, module, overlay, exempt, ShowEntry::Collection(show_collection::<T>))
    }

    const fn of<T: Component + 'static>(
        crate_name: &'static str,
        crate_target: &'static str,
        module: &'static str,
        overlay: bool,
        exempt: &'static [Exemption],
        entry: ShowEntry,
    ) -> Self {
        Probe {
            crate_name,
            crate_target,
            module,
            shape: <T::Shape as ShowShape>::KIND,
            role: T::ROLE,
            input: T::INPUT,
            keys: T::KEYS,
            entry,
            canvas_control: T::CANVAS_CONTROL,
            shows_label: T::SHOWS_LABEL,
            single_value: T::SINGLE_VALUE || T::CANVAS_CONTROL,
            paint_outset: T::paint_outset,
            overlay,
            exempt,
            purpose: "",
            #[cfg(feature = "testing")]
            sample: None,
            #[cfg(feature = "testing")]
            sample_story: None,
            #[cfg(feature = "testing")]
            looks: crate::testing::style::style_looks::<T>,
            name: std::any::type_name::<T>,
            value: carries_value::<T>,
        }
    }

    /// The probe with the type's `purpose`.
    #[must_use]
    pub const fn with_purpose(mut self, purpose: &'static str) -> Self {
        self.purpose = purpose;
        self
    }

    /// The probe with the type's sample: `make` boxes an instance and `story` shows it.
    #[cfg(feature = "testing")]
    #[must_use]
    pub const fn with_sample(mut self, make: fn() -> Box<dyn Any>, story: fn() -> crate::testing::Story) -> Self {
        self.sample = Some(make);
        self.sample_story = Some(story);
        self
    }

    /// Shows the leaf `sample` in the flow of `ui`: the body of a sample story. A leaf that measures to nothing is sized
    /// by its host (a header cell, a thumbnail slot), so the story gives it a cell of the interact size.
    #[cfg(feature = "testing")]
    pub fn show_sample<T: Component + Meta<Shape = shape::Leaf> + 'static>(ui: &mut Ui, mut sample: T) -> crate::testing::StoryOutput {
        let ideal = sample.measure_in(ui, craft_behavior::Proposal::Ideal);
        if ideal.x > 0.0 && ideal.y > 0.0 {
            return sample.show(ui).response().clone().into();
        }
        let cell = Rect::from_min_size(ui.cursor().min, ui.spacing().interact_size);
        crate::runtime::egui::alloc::advance_past(ui, cell);
        sample.show_at(ui, cell).response().clone().into()
    }

    /// The type's name, with its type arguments.
    pub fn type_name(&self) -> &'static str {
        (self.name)()
    }

    /// Whether the user can change the type's value: a type whose input is `Input::Display` shows its value and
    /// takes none, so it has no invalid state and no typed or dragged edit.
    pub fn takes_input(&self) -> bool {
        self.input != Input::Display
    }

    /// Whether the component's output carries a value, so the mixed state applies to it, and the invalid state when it
    /// also takes input and is no canvas control.
    pub fn carries_value(&self) -> bool {
        (self.value)()
    }
}

fn carries_value<T: Component + 'static>() -> bool {
    TypeId::of::<ValueOf<T>>() != TypeId::of::<super::NoValue>()
}

/// What a registered pattern type is. The suite shows the stories of every kind the same way; the kind names the
/// host the type's stories show it through, so the components index and the gallery list a tool or a stack dialog
/// beside the components.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PatternKind {
    /// A composition of components (`#[derive(Pattern)]`).
    Composition,
    /// A canvas tool: its stories show it on a canvas (`#[pattern(tool)]`, a `CanvasTool`).
    Tool,
    /// A stack dialog: its stories show its dialog (`#[pattern(dialog)]`, a `StackDialog`).
    Dialog,
}

impl PatternKind {
    /// The kind as the components index prints it.
    pub const fn label(self) -> &'static str {
        match self {
            PatternKind::Composition => "Pattern",
            PatternKind::Tool => "Tool",
            PatternKind::Dialog => "Dialog",
        }
    }
}

/// What the suite knows about one registered pattern type: where it lives and what it is.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct PatternProbe {
    /// The package that declares the type, as `CARGO_PKG_NAME` spells it.
    pub crate_name: &'static str,
    /// The crate target that declares the type, as `CARGO_CRATE_NAME` spells it.
    pub crate_target: &'static str,
    /// The module path of the declaration.
    pub module: &'static str,
    /// The first sentence of the type's doc comment.
    pub purpose: &'static str,
    /// What the type is.
    pub kind: PatternKind,
    name: fn() -> &'static str,
}

impl PatternProbe {
    /// The probe of the pattern `T`, declared in `module` of the crate target `crate_target` of package `crate_name`.
    pub const fn of<T: Pattern + 'static>(crate_name: &'static str, crate_target: &'static str, module: &'static str, purpose: &'static str) -> Self {
        PatternProbe { crate_name, crate_target, module, purpose, kind: PatternKind::Composition, name: std::any::type_name::<T> }
    }

    /// The probe of the pattern `T` of kind `kind`. The module that owns a kind's trait wraps it in the constructor
    /// `#[pattern(..)]` names, bounded by that trait: `canvas::tool_probe`, `dialog::dialog_probe`.
    pub const fn kinded<T: Pattern + 'static>(
        kind: PatternKind,
        crate_name: &'static str,
        crate_target: &'static str,
        module: &'static str,
        purpose: &'static str,
    ) -> Self {
        PatternProbe { kind, ..Self::of::<T>(crate_name, crate_target, module, purpose) }
    }

    /// The type's name, with its type arguments.
    pub fn type_name(&self) -> &'static str {
        (self.name)()
    }
}

/// One entry of the suite's registration list.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum Registration {
    /// A component type, as `#[derive(Component)]` or `register_contract!` registers it.
    Component(Probe),
    /// A pattern type, as `#[derive(Pattern)]` registers it.
    Pattern(PatternProbe),
}

impl Registration {
    /// The registration of a component type.
    pub const fn component(probe: Probe) -> Self {
        Registration::Component(probe)
    }

    /// The registration of a pattern type.
    pub const fn pattern(probe: PatternProbe) -> Self {
        Registration::Pattern(probe)
    }

    /// The package the registered item belongs to.
    pub fn crate_name(&self) -> &'static str {
        match self {
            Registration::Component(probe) => probe.crate_name,
            Registration::Pattern(probe) => probe.crate_name,
        }
    }

    /// The crate target the registered item belongs to.
    pub fn crate_target(&self) -> &'static str {
        match self {
            Registration::Component(probe) => probe.crate_target,
            Registration::Pattern(probe) => probe.crate_target,
        }
    }

    /// The probe, for a component registration.
    pub fn probe(&self) -> Option<&Probe> {
        match self {
            Registration::Component(probe) => Some(probe),
            Registration::Pattern(_) => None,
        }
    }

    /// The probe, for a pattern registration.
    pub fn pattern_probe(&self) -> Option<&PatternProbe> {
        match self {
            Registration::Pattern(probe) => Some(probe),
            Registration::Component(_) => None,
        }
    }
}

#[cfg(feature = "testing")]
inventory::collect!(Registration);

/// Every registration in the running binary, by crate, module and type name.
#[doc(hidden)]
#[cfg(feature = "testing")]
pub fn registrations() -> Vec<&'static Registration> {
    let mut all: Vec<_> = inventory::iter::<Registration>().collect();
    all.sort_by_key(|registration| match registration {
        Registration::Component(probe) => (probe.crate_name, probe.crate_target, probe.module, probe.type_name()),
        Registration::Pattern(probe) => (probe.crate_name, probe.crate_target, probe.module, probe.type_name()),
    });
    all
}

/// Registers the component type `$ty` of shape `$shape` (`leaf`, `container` or `collection`) with its declarations:
/// `$overlay`, the `($case, $reason)` exemptions, the optional `$sample` constructor (`fn() -> $ty`, leaves only)
/// and the `$purpose` line of its doc comment.
#[doc(hidden)]
#[macro_export]
#[cfg(feature = "testing")]
macro_rules! __register_contract {
    ($ty:ty, $shape:ident, $overlay:expr, [$(($case:expr, $reason:expr)),*], [], $purpose:expr) => {
        const _: () = {
            $crate::__register_probe!($ty, $shape, $overlay, [$(($case, $reason)),*], $purpose,);
        };
    };
    ($ty:ty, $shape:ident, $overlay:expr, [$(($case:expr, $reason:expr)),*], [$sample:path], $purpose:expr) => {
        const _: () = {
            fn make_sample() -> ::std::boxed::Box<dyn ::std::any::Any> {
                ::std::boxed::Box::new($sample())
            }
            fn show_sample(ui: &mut $crate::__private::egui::Ui, _: &mut $crate::testing::StoryData) -> $crate::testing::StoryOutput {
                $crate::contract::Probe::show_sample::<$ty>(ui, $sample())
            }
            fn sample_story() -> $crate::testing::Story {
                $crate::testing::Story::new("sample", show_sample).covers(&[$crate::testing::Required::Default])
            }
            $crate::__register_probe!($ty, $shape, $overlay, [$(($case, $reason)),*], $purpose, .with_sample(make_sample, sample_story));
        };
    };
}

/// The registration static of `__register_contract!`; `$with` is the probe's further builder calls.
#[doc(hidden)]
#[macro_export]
#[cfg(feature = "testing")]
macro_rules! __register_probe {
    ($ty:ty, $shape:ident, $overlay:expr, [$(($case:expr, $reason:expr)),*], $purpose:expr, $($with:tt)*) => {
        const EXEMPT: &[$crate::contract::Exemption] = &[$($crate::contract::Exemption::new($case, $reason)),*];
        $crate::__private::inventory::submit! {
            $crate::contract::Registration::component(
                $crate::contract::Probe::$shape::<$ty>(env!("CARGO_PKG_NAME"), env!("CARGO_CRATE_NAME"), module_path!(), $overlay, EXEMPT)
                    .with_purpose($purpose)
                    $($with)*,
            )
        }
    };
}

/// Registers the pattern type `$ty` with the `$purpose` line of its doc comment. `$probe` is the path of the
/// `PatternProbe` constructor of its kind: `PatternProbe::of`, `canvas::tool_probe` or `dialog::dialog_probe`.
#[doc(hidden)]
#[macro_export]
#[cfg(feature = "testing")]
macro_rules! __register_pattern {
    ($ty:ty, $purpose:expr) => {
        $crate::__register_pattern!($ty, $purpose, $crate::contract::PatternProbe::of);
    };
    ($ty:ty, $purpose:expr, $($probe:tt)+) => {
        $crate::__private::inventory::submit! {
            $crate::contract::Registration::pattern(
                $($probe)+::<$ty>(env!("CARGO_PKG_NAME"), env!("CARGO_CRATE_NAME"), module_path!(), $purpose),
            )
        }
    };
}

/// Registers the pattern type `$ty`; without the `testing` feature there is nothing to register.
#[doc(hidden)]
#[macro_export]
#[cfg(not(feature = "testing"))]
macro_rules! __register_pattern {
    ($ty:ty, $purpose:expr $(, $($probe:tt)+)?) => {};
}

/// Registers the component type `$ty`; without the `testing` feature there is nothing to register.
#[doc(hidden)]
#[macro_export]
#[cfg(not(feature = "testing"))]
macro_rules! __register_contract {
    ($ty:ty, $shape:ident, $overlay:expr, [$(($case:expr, $reason:expr)),*], [$($sample:path)?], $purpose:expr) => {};
}
