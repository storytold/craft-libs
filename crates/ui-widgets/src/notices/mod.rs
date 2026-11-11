//! `Notice` (inline, card and centred layouts), the settings notices and the notice center. The toast queue is
//! `runtime::toast`, drawn by `ToastStack` through the contract's `ToastHost`.

mod center;
mod notice;
mod settings;
mod toasts;
mod words;

#[cfg(feature = "testing")]
mod stories;

pub use center::{NoticeCenter, NoticeCenterOutput, NoticeEnd, PostedNotice};
pub use notice::{Notice, NoticeAction, NoticeIcon, NoticeLayout, NoticeOutput, NoticeStyle};
pub use settings::{SaveResult, SettingsLoad, SettingsNoticeState, SettingsNotices, SettingsNoticesOutput, SettingsStatus};
pub use toasts::ToastStack;
pub use words::NoticeWords;
