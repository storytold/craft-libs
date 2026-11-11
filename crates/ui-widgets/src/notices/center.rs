//! `NoticeCenter` (20.3): posts the app's notices (saved, exported, couldn't open) to the toast queue of
//! `runtime::toast`, which `ToastStack` shows, and reports how each one ended.

use crate::contract::{NoValue, Outcome, Politeness};
use crate::runtime::env::{Label, WordSet};
use crate::runtime::memory::{unique, with_session};
use crate::runtime::toast::{self, Toast, ToastAction, ToastEnd, ToastId};
use crate::runtime::words::CommonWords;
use craft_theme::variant::Tone;
use egui::{Context, Response, Ui};

/// The id of the "Don't Show Again" word, which the toast card resolves in the viewport's language.
const DONT_SHOW_AGAIN: &str = "common.dontShowAgain";

/// Posted notices the center still waits to hear the end of; the oldest go first.
const TRACKED: usize = 32;

/// One notice, already translated and filled in.
#[derive(Clone, Debug, PartialEq, crate::contract::Props)]
#[non_exhaustive]
pub struct PostedNotice {
    /// The title line.
    #[prop(into)]
    title: String,
    /// The lines under the title.
    #[prop(into)]
    lines: Vec<String>,
    /// The tone the notice reads in; a failure is announced at once.
    #[prop]
    tone: Tone,
    /// A preference key. A notice whose key the app has stored in `silenced` is not posted.
    #[prop(into)]
    dismiss_key: Option<String>,
    /// Stays until dismissed, whatever the center's timeout (an error the user must read).
    #[prop(flag)]
    sticky: bool,
}

impl PostedNotice {
    /// A notice titled `title` in the neutral tone.
    pub fn new(title: impl Into<String>) -> Self {
        PostedNotice { title: title.into(), lines: Vec::new(), tone: Tone::Neutral, dismiss_key: None, sticky: false }
    }
}

/// Why a posted notice left the screen, for the app to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NoticeEnd {
    /// The user closed the notice.
    Closed(ToastId),
    /// The timeout passed.
    Expired(ToastId),
    /// "Don't Show Again": the app stores `key` as silenced.
    #[non_exhaustive]
    Silenced {
        /// The notice.
        id: ToastId,
        /// The notice's dismiss key.
        key: String,
    },
}

/// What `NoticeCenter::show` reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct NoticeCenterOutput {
    /// The pattern's response; it paints nothing.
    pub response: Response,
    /// The notices that ended since the last pass.
    pub ends: Vec<NoticeEnd>,
}

impl Outcome for NoticeCenterOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The notices the center posted that have not ended, with their dismiss keys.
#[derive(Clone, Default)]
struct Posted(Vec<(ToastId, Option<String>)>);

fn posted_slot() -> egui::Id {
    unique("craft_ui.notice_center")
}

/// The app's way to post notices. It holds only its timeout: the queue, the timers, the cap of three cards, the
/// hover pause and the announcement are the toast service's. A notice with a dismiss key shows a "Don't Show Again"
/// action; `show` reports how posted notices ended.
#[derive(Clone, Debug, PartialEq, crate::contract::Pattern, crate::contract::Props)]
pub struct NoticeCenter {
    /// Seconds before a notice hides, from the app's notification duration.
    #[prop(quantity = "seconds")]
    timeout: Option<f32>,
}

impl Default for NoticeCenter {
    fn default() -> Self {
        NoticeCenter::new()
    }
}

impl NoticeCenter {
    /// A center whose notices hide after the toast service's default time.
    pub fn new() -> Self {
        NoticeCenter { timeout: Some(toast::TIMEOUT) }
    }

    /// Posts `notice` to the running viewport; `None` when its dismiss key is in `silenced`.
    pub fn post(&self, ctx: &Context, notice: &PostedNotice, silenced: &[String]) -> Option<ToastId> {
        if notice.dismiss_key.as_ref().is_some_and(|key| silenced.contains(key)) {
            return None;
        }
        let politeness = if notice.tone == Tone::Bad { Politeness::Assertive } else { Politeness::Polite };
        let timeout = if notice.sticky { None } else { self.timeout };
        let mut card = if notice.lines.is_empty() {
            Toast::new(notice.title.clone())
        } else {
            let card = Toast::new(notice.lines.join("\n"));
            if notice.title.is_empty() { card } else { card.title(notice.title.clone()) }
        };
        card = card.tone(notice.tone).timeout(timeout).politeness(politeness);
        if notice.dismiss_key.is_some() {
            card = card.action(ToastAction::new(dont_show_again()));
        }
        let id = toast::post(ctx, card);
        with_session(ctx, posted_slot(), |posted: &mut Posted| {
            posted.0.push((id, notice.dismiss_key.clone()));
            if posted.0.len() > TRACKED {
                posted.0.remove(0);
            }
        });
        Some(id)
    }

    /// Keeps notice `id` until it is closed, whatever the timeout (an error the user must read).
    pub fn keep(&self, ctx: &Context, id: ToastId) {
        toast::keep(ctx, id);
    }

    /// Removes a notice the app no longer needs (the job it reported on was cancelled).
    pub fn remove(&self, ctx: &Context, id: ToastId) {
        toast::dismiss(ctx, id);
    }

    /// Collects how the notices posted through any center ended since the last pass.
    pub fn show(self, ui: &mut Ui) -> NoticeCenterOutput {
        let ctx = ui.ctx().clone();
        let mut ends = Vec::new();
        with_session(&ctx, posted_slot(), |posted: &mut Posted| {
            posted.0.retain(|(id, key)| {
                let Some(end) = toast::end(&ctx, *id) else { return true };
                ends.push(match (end, key) {
                    (ToastEnd::Action(_), Some(key)) => NoticeEnd::Silenced { id: *id, key: key.clone() },
                    (ToastEnd::Action(_) | ToastEnd::Dismissed, _) => NoticeEnd::Closed(*id),
                    (ToastEnd::Expired, _) => NoticeEnd::Expired(*id),
                });
                false
            });
        });
        NoticeCenterOutput { response: ui.response(), ends }
    }
}

/// The "Don't Show Again" action's label: the word of `common.dontShowAgain`.
fn dont_show_again() -> Label {
    CommonWords::WORDS.iter().find(|word| word.id == DONT_SHOW_AGAIN).copied().map_or(Label::Empty, Label::Word)
}
