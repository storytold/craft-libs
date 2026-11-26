//! The Save panel calls that need `unsafe`: making a panel that may be nil, and a pop-up menu's action.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSPopUpButton, NSSavePanel};
use objc2_foundation::{NSObject, NSObjectProtocol};

/// A new Save panel, or `None` when AppKit returns nil. `+[NSSavePanel savePanel]` returns nil on some systems
/// (storytold/vectorcraft#5980), and objc2's `NSSavePanel::savePanel` panics on it.
pub fn new_save_panel(mtm: MainThreadMarker) -> Option<Retained<NSSavePanel>> {
    // SAFETY: `-init` is the initializer of a bare panel, called once on the allocation. The result is typed as
    // `Option`, so nil is `None`.
    let panel: Option<Retained<NSSavePanel>> = unsafe { msg_send![NSSavePanel::alloc(mtm), init] };
    if panel.is_some() {
        return panel;
    }
    // SAFETY: a class method that returns an autoreleased panel or nil, typed as `Option`.
    unsafe { msg_send![NSSavePanel::class(), savePanel] }
}

/// The closure a [`PopUpChoice`] calls.
pub struct Ivars {
    on_choose: Box<dyn Fn(usize)>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class implements no `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    #[name = "CraftPopUpChoice"]
    /// The target of a pop-up menu's action; see [`on_choose`].
    pub struct PopUpChoice;

    // SAFETY: NSObjectProtocol has no additional requirements.
    unsafe impl NSObjectProtocol for PopUpChoice {}

    impl PopUpChoice {
        // SAFETY: NSControl's target-action sends the pop-up button as the one object argument.
        #[unsafe(method(choose:))]
        fn choose(&self, sender: &NSPopUpButton) {
            if let Ok(index) = usize::try_from(sender.indexOfSelectedItem()) {
                (self.ivars().on_choose)(index);
            }
        }
    }
);

/// Calls `on_choose` with the index of the item the user chooses in `popup`. A control does not retain its target,
/// so the caller keeps the returned object for as long as `popup` can send its action.
pub fn on_choose(mtm: MainThreadMarker, popup: &NSPopUpButton, on_choose: impl Fn(usize) + 'static) -> Retained<PopUpChoice> {
    let this = PopUpChoice::alloc(mtm).set_ivars(Ivars { on_choose: Box::new(on_choose) });
    // SAFETY: NSObject's designated initializer, called once on the allocated subclass.
    let target: Retained<PopUpChoice> = unsafe { msg_send![super(this), init] };
    let object: &AnyObject = &target;
    // SAFETY: `choose:` takes the sender, the signature a control's action is sent with, and the caller keeps
    // `target` alive while `popup` can send it (the documentation above).
    unsafe {
        popup.setTarget(Some(object));
        popup.setAction(Some(sel!(choose:)));
    }
    target
}
