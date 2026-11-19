//! A paste the platform delivers as a clipboard event while no focused component lists Paste fires the app's Paste
//! command with the event's content, so the app pastes what the event held in the pass that runs the command.

use craft_ui::runtime::frame::take_output;
use craft_ui::testing::{Frames, trial};
use egui::{Event, Ui};

/// The content used to be dropped with the event, and the platform's offer of it lapsed a pass before the command ran,
/// so the app's paste found nothing.
#[trial]
fn an_app_paste_carries_the_content_of_its_clipboard_event() {
    let mut frames = Frames::new();
    let mut show = |_: &mut Ui| {};
    frames.idle(2, &mut show);
    take_output(&frames.ctx);
    let mut pasted = Vec::new();
    frames.frame(vec![Event::Paste("pasted words".into())], &mut show);
    for _ in 0..2 {
        pasted.extend(take_output(&frames.ctx).fired.into_iter().filter_map(|fired| fired.pasted?.text));
        frames.idle(1, &mut show);
    }
    pasted.extend(take_output(&frames.ctx).fired.into_iter().filter_map(|fired| fired.pasted?.text));
    assert_eq!(pasted.iter().map(|text| &**text).collect::<Vec<_>>(), ["pasted words"], "the Paste command carries the event's text, once");
}
