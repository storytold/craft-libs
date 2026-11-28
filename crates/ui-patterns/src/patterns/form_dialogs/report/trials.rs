//! Trials of the report dialog.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use super::{HostedReport, ReportDialog, ReportGroup, ReportNote};
use crate::contract::Bind;
use crate::dialog::StackDialog as _;
use crate::runtime::env::Env;
use crate::testing::{Fixture, trial};
use craft_theme::variant::Tone;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

fn groups() -> Vec<ReportGroup> {
    vec![
        ReportGroup::new(
            "Fonts",
            vec![ReportNote::new(Tone::Warn, "Missing Arial").detail("page 2"), ReportNote::new(Tone::Bad, "Cannot read Inter")],
        ),
        ReportGroup::new("Images", vec![ReportNote::new(Tone::Good, "hero.png")]),
    ]
}

/// The copied text holds each group's heading before its notes, in order, with a detail after the text; the count line
/// counts every note; the hosted report lists the groups.
#[trial]
fn grouped_notes_copy_with_their_headings() {
    let copied = Rc::new(RefCell::new(String::new()));
    let sink = Rc::clone(&copied);
    let mut open = true;
    let mut fixture = Fixture::new(move |ui| {
        let dialog = ReportDialog::new("Report", "Checked.", vec![ReportNote::new(Tone::Info, "Imported")]).groups(groups());
        *sink.borrow_mut() = dialog.as_text(&Env::of(ui));
        dialog.opened(Bind::bound(&mut open)).show(ui);
    });
    fixture.settle();
    assert_eq!(*copied.borrow(), "Info: Imported\nFonts\nWarning: Missing Arial (page 2)\nError: Cannot read Inter\nImages\nSuccess: hero.png");
    assert!(fixture.by_label("4 notes").is_some(), "the count counts the flat note and every grouped one");

    let hosted = HostedReport::new("Report", "Checked.", Vec::new()).groups(groups());
    assert_eq!(
        hosted.json_fields(),
        Some(json!({
            "lines": [],
            "notes": [],
            "groups": [
                { "heading": "Fonts", "notes": ["Missing Arial", "Cannot read Inter"] },
                { "heading": "Images", "notes": ["hero.png"] },
            ],
        }))
    );
}
