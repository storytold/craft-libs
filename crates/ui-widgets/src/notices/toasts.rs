//! The toast stack: draws the toast queue's cards as `Notice`s in `NoticeLayout::Toast`.

use crate::contract::toast::{CardOutcome, ToastHost};
use crate::contract::{Props as _, Show as _};
use crate::display::{Progress, Shade, Text};
use crate::notices::{Notice, NoticeAction, NoticeLayout};
use crate::runtime::env::{Env, WindowKey};
use crate::runtime::toast::ToastEnd;
use crate::style::TypeRole;
use egui::{Align2, Rect, Ui};

/// The cards of a window's toast queue. Whoever owns the window root calls `show` once per pass. A card with progress
/// shows a progress bar named by its title (else its message) and the percentage under its text, before its actions.
#[derive(Clone, Copy, Debug, Default, crate::contract::Pattern)]
pub struct ToastStack;

impl ToastStack {
    /// Shows every queued toast of `window` at the end-bottom corner of `body`, a rect of `ui`'s layer, through the
    /// toast host.
    pub fn show(ui: &Ui, window: &WindowKey, body: Rect) {
        Self::show_at(ui, window, body, None);
    }

    /// Shows the queue at a bottom alignment of the body, or its locale's trailing bottom corner when `None`.
    pub fn show_at(ui: &Ui, window: &WindowKey, body: Rect, alignment: Option<Align2>) {
        ToastHost::show_at(ui, window, body, alignment, &mut |ui, toast, salt| {
            let mut notice =
                Notice::new(toast.tone, toast.message.clone()).layout(NoticeLayout::Toast).lines(toast.lines.clone()).dismiss().id_salt(salt);
            if let Some(title) = &toast.title {
                notice = notice.title(title.clone());
            }
            if let Some(fraction) = toast.progress {
                // The bar and the percentage read as `StatusProgress` shows a task's progress.
                let name = toast.title.clone().unwrap_or_else(|| toast.message.clone());
                let percent = Env::of(ui).format().percent(f64::from(Progress::clamped(fraction)), 0);
                notice = notice
                    .content(Progress::new(fraction).label(name), |_| {})
                    .content(Text::new(percent).role(TypeRole::Mono).shade(Shade::Muted), |_| {});
            }
            for action in &toast.actions {
                notice = notice.action(NoticeAction::new(action.label.clone()));
            }
            let out = notice.show(ui);
            let end = match (out.action, out.dismissed) {
                (Some(taken), _) => Some(ToastEnd::Action(taken)),
                (None, true) => Some(ToastEnd::Dismissed),
                (None, false) => None,
            };
            CardOutcome::new(out.response.contains_pointer(), end)
        });
    }
}
