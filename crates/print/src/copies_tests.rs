//! photocraft#14132: a job asks for 1 to 9999 copies, as the reference print dialog allows; every backend sends at
//! most [`MOST_COPIES`] and the dialog's copies setting takes them.

use super::*;
use craft_ui::testing::trial;

/// Each row: the copies asked for and the copies sent.
const ROWS: [(u32, u32); 6] = [(1, 1), (999, 999), (1000, 1000), (9999, 9999), (10000, 9999), (0, 1)];

#[trial]
fn every_backend_sends_up_to_9999_copies() {
    assert_eq!(MOST_COPIES, 9999);
    for (asked, sent) in ROWS {
        let job = Job { copies: asked, ..Job::default() };
        let args = lp_args(&job);
        let n = args.iter().position(|arg| arg == "-n").and_then(|at| args.get(at + 1));
        assert_eq!(n, Some(&sent.to_string()), "lp -n for {asked}: {args:?}");
        assert!(windows_preview(&job, None).contains(&format!("copies={sent}")), "the Windows job for {asked}");
        let ticket = ticket::ticket_xml(&job, false);
        assert!(ticket.contains(&format!(">{sent}</psf:Value>")), "the print ticket for {asked}: {ticket}");
    }
}

/// The direct CUPS transport refuses only counts past the range, before any endpoint is tried.
#[cfg(not(target_arch = "wasm32"))]
#[trial]
fn direct_cups_takes_up_to_9999_copies() {
    let destination = cups::Destination { name: "Office".into(), is_class: false };
    let refusal = |copies: u32| match cups::submit_as_at(&[], &destination, b"%PDF", &Job { copies, ..Job::default() }, "someone") {
        Err(Error::Spool(message)) => message,
        other => panic!("{copies}: {other:?}"),
    };
    for copies in [1, 1000, 9999] {
        assert_eq!(refusal(copies), "no CUPS endpoint selected", "{copies} copies are taken");
    }
    for copies in [0, 10000] {
        assert_eq!(refusal(copies), "copies must be between 1 and 9999", "{copies} copies are refused");
    }
}

/// The print dialog's copies setting takes 1 to 9999 through automation and refuses the rest.
#[cfg(feature = "ui")]
#[trial]
fn the_dialog_copies_take_up_to_9999() {
    use craft_ui::dialog::StackDialog as _;
    use craft_ui::patterns::form_dialogs::DialogFields;
    let mut dialog = ui::HostedPrint::new(ui::State::default(), DialogFields::default(), Vec::new());
    let ctx = egui::Context::default();
    for copies in [1, 1000, 9999] {
        dialog.set_json_field(&ctx, "copies", serde_json::json!(copies)).unwrap_or_else(|e| panic!("{copies}: {e}"));
        assert_eq!(dialog.state().settings.copies, copies);
    }
    for copies in [0, 10000] {
        assert!(dialog.set_json_field(&ctx, "copies", serde_json::json!(copies)).is_err(), "{copies} is refused");
    }
}
