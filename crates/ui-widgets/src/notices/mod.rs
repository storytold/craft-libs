//! `Notice` (inline, card and centred layouts), the settings notices and the notice center. The toast queue is
//! `runtime::toast`, drawn by `ToastStack` through the contract's `ToastHost`.

pub(crate) mod center;
pub(crate) mod notice;
pub(crate) mod settings;
pub(crate) mod toasts;
mod words;

#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use center::{NoticeCenter, NoticeCenterOutput, NoticeEnd, PostedNotice};
pub(crate) use notice::tone_icon;
pub use notice::{Notice, NoticeAction, NoticeIcon, NoticeLayout, NoticeOutput, NoticeStyle};
pub use settings::{SaveResult, SettingsLoad, SettingsNoticeState, SettingsNotices, SettingsNoticesOutput, SettingsStatus};
pub use toasts::ToastStack;
pub use words::NoticeWords;
