//! `NoticeCenter` (20.3): posts the app's notices (saved, exported, couldn't open) to the toast queue of
//! `runtime::toast`, which `ToastStack` shows, and reports how each one ended.

use crate::contract::{NoValue, Outcome, Politeness};
use crate::runtime::env::{Label, WindowKey, WordSet};
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
    /// The lines under the title, shown as bullet lines.
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
    /// How far the job has got, between 0 and 1: the card shows a progress bar and the percentage.
    #[prop(quantity = "fraction done")]
    progress: Option<f32>,
    /// The notice's own action; activating it ends the notice as `NoticeEnd::Action`.
    #[prop(into)]
    action: Option<Label>,
    /// The message under the title: a title and a message on one card.
    #[prop(into)]
    message: Option<String>,
}

impl PostedNotice {
    /// A notice titled `title` in the neutral tone.
    pub fn new(title: impl Into<String>) -> Self {
        PostedNotice {
            title: title.into(),
            lines: Vec::new(),
            tone: Tone::Neutral,
            dismiss_key: None,
            sticky: false,
            progress: None,
            action: None,
            message: None,
        }
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
    /// The window's card limit removed the notice to make room for a newer one.
    Dropped(ToastId),
    /// The notice's own action ([`PostedNotice::action`]) was activated.
    Action(ToastId),
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

/// A notice the center posted and has not heard the end of.
#[derive(Clone)]
struct Tracked {
    id: ToastId,
    /// The notice's dismiss key: its card then ends with a "Don't Show Again" action.
    key: Option<String>,
    /// The notice has its own action, the card's first.
    action: bool,
}

impl Tracked {
    /// What the app hears of the card's `end`. The card's actions are the notice's own action, if any, then "Don't
    /// Show Again", if it has a dismiss key.
    fn end(&self, end: ToastEnd) -> NoticeEnd {
        let id = self.id;
        match (end, &self.key) {
            (ToastEnd::Action(0), _) if self.action => NoticeEnd::Action(id),
            (ToastEnd::Action(_), Some(key)) => NoticeEnd::Silenced { id, key: key.clone() },
            (ToastEnd::Action(_) | ToastEnd::Dismissed, _) => NoticeEnd::Closed(id),
            (ToastEnd::Expired, _) => NoticeEnd::Expired(id),
            (ToastEnd::Dropped, _) => NoticeEnd::Dropped(id),
        }
    }
}

/// The notices the center posted that have not ended.
#[derive(Clone, Default)]
struct Posted(Vec<Tracked>);

fn posted_slot() -> egui::Id {
    unique("craft_ui.notice_center")
}

/// The app's way to post notices to a window. It holds only the window and its timeout: the queue, the timers, the
/// hover pause and the announcement are the toast service's. So is the cap of `toast::MAX_CARDS` cards: a new notice
/// always shows, and makes room by dropping the oldest earlier card that would leave by itself (no action, a timeout),
/// else the oldest without progress; a progress notice is never dropped. `show` reports a dropped notice as
/// `NoticeEnd::Dropped`. A body posts to its shell's window (`BodyCx::shell`). A notice's own action comes first on its
/// card and a dismiss key's "Don't Show Again" last; `show` reports how posted notices ended.
#[derive(Clone, Debug, PartialEq, crate::contract::Pattern, crate::contract::Props)]
pub struct NoticeCenter {
    /// The window whose toasts show the notices.
    #[prop]
    window: WindowKey,
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
    /// A center for the app's first window whose notices hide after the toast service's default time.
    pub fn new() -> Self {
        NoticeCenter { window: WindowKey::main(), timeout: Some(toast::TIMEOUT) }
    }

    /// Posts `notice` to the center's window in the running viewport; `None` when its dismiss key is in `silenced`, and
    /// in a measuring run, which posts nothing and tracks nothing (the real run posts it). A notice with progress has no
    /// timer.
    pub fn post(&self, ctx: &Context, notice: &PostedNotice, silenced: &[String]) -> Option<ToastId> {
        if notice.dismiss_key.as_ref().is_some_and(|key| silenced.contains(key)) {
            return None;
        }
        if !crate::runtime::measure::effect(ctx, "notices::post") {
            return None;
        }
        let politeness = if notice.tone == Tone::Bad { Politeness::Assertive } else { Politeness::Polite };
        let timeout = if notice.sticky { None } else { self.timeout };
        let message = notice.message.clone().map_or(Label::Empty, Label::from);
        let mut card = Toast::new(message).title(notice.title.clone()).lines(notice.lines.clone());
        card = card.tone(notice.tone).timeout(timeout).politeness(politeness);
        if let Some(fraction) = notice.progress {
            card = card.progress(fraction);
        }
        if let Some(action) = &notice.action {
            card = card.action(ToastAction::new(action.clone()));
        }
        if notice.dismiss_key.is_some() {
            card = card.action(ToastAction::new(dont_show_again()));
        }
        let id = toast::post(ctx, &self.window, card);
        with_session(ctx, posted_slot(), |posted: &mut Posted| {
            posted.0.push(Tracked { id, key: notice.dismiss_key.clone(), action: notice.action.is_some() });
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

    /// Sets how far the job of notice `id` has got, `fraction` between 0 and 1, and its message; the center keeps
    /// tracking the id.
    pub fn set_progress(&self, ctx: &Context, id: ToastId, fraction: f32, message: Option<Label>) {
        toast::set_progress(ctx, id, fraction, message);
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
            posted.0.retain(|tracked| {
                let Some(end) = toast::end(&ctx, tracked.id) else { return true };
                ends.push(tracked.end(end));
                false
            });
        });
        NoticeCenterOutput { response: ui.response(), ends }
    }
}

/// The "Don't Show Again" action's label: the word of `common.dontShowAgain`.
fn dont_show_again() -> Label {
    CommonWords::word(DONT_SHOW_AGAIN).map_or(Label::Empty, Label::Word)
}
