//! Printer discovery and the chosen queue.

use craft_print::ui::{Destination, Discover, Feedback, Printer, State};
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

/// A rediscovery keeps the chosen queue while it exists, blocks the job while printers are being found, and replaces a
/// queue that is gone. Save as PDF waits for no discovery and no rediscovery replaces it.
#[trial]
fn discovery_preserves_choice_and_blocks_stale_or_removed_queues() {
    let ctx = egui::Context::default();
    let mut state = State::default();
    let printer = |name: &str| Destination::Printer(name.into());
    state.settings.destination = printer("Chosen");
    let discover = finds(vec![queue("Default", true), queue("Chosen", false)]);
    assert!(state.refresh(&discover));
    assert!(!state.ready(), "a printer waits for the discovery");
    state.poll(&ctx);
    assert_eq!(state.settings.destination, printer("Chosen"));
    assert!(state.ready());
    let empty = finds(Vec::new());
    assert!(state.refresh(&empty));
    assert!(!state.ready());
    state.poll(&ctx);
    assert!(!state.ready());
    assert_eq!(state.message, Some(Feedback::NoPrinters));
    assert_eq!(state.settings.destination, printer(""), "a printer that is no longer listed is replaced");
    assert!(state.refresh(&discover));
    state.poll(&ctx);
    assert_eq!(state.settings.destination, printer("Default"));

    state.settings.destination = Destination::File;
    assert!(state.refresh(&empty));
    assert!(state.ready(), "Save as PDF does not wait for the discovery");
    state.poll(&ctx);
    assert_eq!(state.settings.destination, Destination::File, "a discovery does not replace Save as PDF");
    assert_eq!(state.message, Some(Feedback::NoPrinters));
    assert!(state.ready());
    let (_job, outcome) = channel();
    assert!(state.begin(outcome).is_ok());
    assert!(!state.ready(), "Save as PDF waits while a job runs");
}
