//! Printer discovery and the chosen queue.

use craft_print::ui::{Discover, Feedback, Printer, State};
use craft_ui::testing::trial;
use std::sync::mpsc::channel;

fn queue(name: &str, default: bool) -> Printer {
    Printer::new(name).is_default(default)
}

/// A discovery that finds `printers` at once.
fn finds(printers: Vec<Printer>) -> Discover {
    Box::new(move || {
        let (sender, receiver) = channel();
        sender.send(printers.clone()).expect("the receiver is alive");
        receiver
    })
}

/// A rediscovery keeps the chosen queue while it exists, blocks the job while printers are being found, and drops a
/// queue that is gone.
#[trial]
fn discovery_preserves_choice_and_blocks_stale_or_removed_queues() {
    let ctx = egui::Context::default();
    let mut state = State::default();
    state.settings.printer = "Chosen".into();
    let discover = finds(vec![queue("Default", true), queue("Chosen", false)]);
    assert!(state.refresh(&discover));
    assert!(!state.ready());
    state.poll(&ctx);
    assert_eq!(state.settings.printer, "Chosen");
    assert!(state.ready());
    let empty = finds(Vec::new());
    assert!(state.refresh(&empty));
    assert!(!state.ready());
    state.poll(&ctx);
    assert!(!state.ready());
    assert_eq!(state.message, Some(Feedback::NoPrinters));
    assert!(state.settings.printer.is_empty());
    assert!(state.refresh(&discover));
    state.poll(&ctx);
    assert_eq!(state.settings.printer, "Default");
}
