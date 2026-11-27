//! Tracked inputs: the framework inputs a view can read outside its props (the modifiers, the pointer, its presses, the
//! window's facts, the keyboard layout, the keymap, a drag, the focus, the open layers, the toast queues, the keys held,
//! the events sent to the focus, the requests to collections), each with a generation that moves when its value
//! changes. The accessor that reads one notes it into the records open around the running
//! code (`replay::note_input`); a memoized view's replay is refused when an input its run read has another generation
//! now (`runtime::lazy::settled`), so a change runs only the views that read it, and a view absent while it changed
//! runs when it is shown again.
//!
//! The scroll view a nested `Lazy` child decides by is noted too, as `Scroll`; it has no generation: the record keeps
//! the view it ran with, and a replay needs the same view.

use crate::runtime::egui::pass;
use crate::runtime::frame::PerViewport;
use egui::Context;
use std::any::Any;

/// A framework input a view reads outside its props.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Input {
    /// The modifiers held (`Cx::modifiers`, `pointer::mods`).
    Modifiers,
    /// The pointer's frame (`pointer::frame`, `pointer::with_pass_pointer`).
    Pointer,
    /// The pointer's presses and releases, an outside press the layer stack absorbed included (`pointer::primary_press`,
    /// `overlay::absorbed_press`): a layer's dismissal reads them, so a press anywhere runs the views that show layers,
    /// and a pointer move does not.
    Press,
    /// The keys held, Caps Lock and whether a key arrived (`keys::input::down`, `keyed`, `lock_keys::caps_lock`).
    Keys,
    /// The platform and window facts (`EnvView::shell`) and the viewport's safe rect (`areas::safe_rect`): its
    /// content rect and the visible part of its monitor.
    Shell,
    /// The keyboard layout (`EnvView::keyboard_layout`).
    Keyboard,
    /// The keymap: the chords commands show (`commands::chord`, `bindings`, `holder`, a resolved command's chord).
    Keymap,
    /// A drag session and its payloads (`drag::active`, `started`).
    Drag,
    /// Where key focus is (`focus::focused_rec`, `focus_in_scope`, `commands::focus_within`).
    Focus,
    /// The key, text, IME, clipboard and AccessKit action events of the frame: a run that ends holding focus reads them
    /// (`replay::close`), so any such event runs the views the focus is in, and an idle frame replays them.
    Sent,
    /// The layers shown last frame (`overlay::is_open`, `any_open`, `top`, `family`, `entry`).
    Overlays,
    /// The cards queued in the running viewport's toast queues (`toast::cards`): a post, a close or an expiry runs the
    /// views that show them.
    Toasts,
    /// The requests to collections from outside them (`items::request_rename`), read by the item step that takes
    /// them, so a pending request runs the view that takes it.
    Requests,
    /// The view of the scroll area a nested `Lazy` child decides by. No generation: compared by value.
    Scroll,
}

impl Input {
    pub(crate) const ALL: [Input; 14] = [
        Input::Modifiers,
        Input::Pointer,
        Input::Press,
        Input::Keys,
        Input::Shell,
        Input::Keyboard,
        Input::Keymap,
        Input::Drag,
        Input::Focus,
        Input::Sent,
        Input::Overlays,
        Input::Toasts,
        Input::Requests,
        Input::Scroll,
    ];

    fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// The inputs a run read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Inputs(u16);

impl Inputs {
    pub(crate) fn insert(&mut self, input: Input) {
        self.0 |= input.bit();
    }

    pub(crate) fn extend(&mut self, other: Inputs) {
        self.0 |= other.0;
    }

    pub(crate) fn contains(self, input: Input) -> bool {
        self.0 & input.bit() != 0
    }

    /// The inputs in the set that have a generation.
    pub(crate) fn generational(self) -> impl Iterator<Item = Input> {
        Input::ALL.into_iter().filter(move |input| *input != Input::Scroll && self.contains(*input))
    }
}

/// The generations of the inputs a run read, taken when it started.
pub(crate) type Seen = [u64; Input::ALL.len()];

/// A viewport's generations, and the last value each observed input had.
#[derive(Default)]
struct Generations {
    gens: Seen,
    last: [Option<Box<dyn Any + Send + Sync>>; Input::ALL.len()],
}

impl crate::runtime::frame::Stored for Generations {}

fn with<R>(ctx: &Context, f: impl FnOnce(&mut Generations) -> R) -> R {
    PerViewport::<Generations>::with(ctx, pass::current_viewport(ctx), f)
}

/// Moves the generation of `input` in the running viewport: its value changed.
pub(crate) fn bump(ctx: &Context, input: Input) {
    with(ctx, |generations| generations.gens[input as usize] += 1);
}

/// Notes the value `input` has now in the running viewport, and moves its generation when it differs from the last
/// one noted.
pub(crate) fn observe<T: PartialEq + Send + Sync + 'static>(ctx: &Context, input: Input, value: T) {
    with(ctx, |generations| {
        let slot = &mut generations.last[input as usize];
        if slot.as_ref().and_then(|last| last.downcast_ref::<T>()) != Some(&value) {
            if slot.is_some() {
                generations.gens[input as usize] += 1;
            }
            *slot = Some(Box::new(value));
        }
    });
}

/// The generation `input` has now. The inputs a pass can change while views run (keys, drag, focus, open layers, toast
/// queues) are observed here, and so are the events sent to the focus, which the key stage counts; the pointer, its
/// presses and the modifiers by the pointer's begin hook, the window facts and the layout where the frame's root env is built, the
/// keymap by the command board, the requests by each request.
pub(crate) fn generation(ctx: &Context, input: Input) -> u64 {
    match input {
        Input::Keys => observe(ctx, input, crate::runtime::keys::keys_state(ctx)),
        Input::Sent => observe(ctx, input, crate::runtime::keys::sent(ctx)),
        Input::Drag => observe(ctx, input, crate::runtime::drag::state(ctx)),
        Input::Focus => observe(ctx, input, crate::runtime::focus::focus_state(ctx)),
        Input::Overlays => observe(ctx, input, crate::runtime::overlay::open_state(ctx)),
        Input::Toasts => observe(ctx, input, crate::runtime::toast::queued(ctx)),
        Input::Keymap => return crate::runtime::commands::keymap_built(ctx),
        Input::Modifiers | Input::Pointer | Input::Press | Input::Shell | Input::Keyboard | Input::Requests | Input::Scroll => {}
    }
    with(ctx, |generations| generations.gens[input as usize])
}

/// The generation of every input now, for a run that starts.
pub(crate) fn seen(ctx: &Context) -> Seen {
    let mut seen = Seen::default();
    for input in Input::ALL {
        seen[input as usize] = generation(ctx, input);
    }
    seen
}

/// Whether an input of `read` has another generation now than in `seen`.
pub(crate) fn moved(ctx: &Context, read: Inputs, seen: &Seen) -> bool {
    read.generational().any(|input| generation(ctx, input) != seen[input as usize])
}
