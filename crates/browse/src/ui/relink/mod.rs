//! `LinkList` (the Links panel), `StatusBadge` and `RelinkDialog` (Link Media).
//!
//! ```ignore
//! let out = LinkList::new("Links", &links, monitor.statuses(), Bind::bound(&mut state.links)).show(ui);
//! if out.settings_changed { settings.links = state.links.settings.clone() }
//! for event in out.events {
//!     match event {
//!         LinkListEvent::Relink(keys) => relink_with_file_pick(keys),        // one file: Files::pick, no dialog
//!         LinkListEvent::RelinkToFolder(keys) => relink_to_folder(keys),      // docpath::links::folder_matches in a job
//!         LinkListEvent::FindMissing => dialogs.open(RelinkDialog::new(missing_rows(), files.clone(), RelinkOptions::default())),
//!         LinkListEvent::GoTo(key) => go_to(key),
//!         _ => {}
//!     }
//! }
//! if let Some(DialogResult::Ok(outcome)) = dialogs.take::<RelinkDialog<LinkId>>() { doc.relink(outcome.linked) }   // one undo step
//! ```

mod badge;
mod dialog;
mod list;
#[cfg(feature = "screen-testing")]
mod stories;

pub use badge::StatusBadge;
pub use dialog::{CandidateChecker, RelinkCriterion, RelinkDialog, RelinkOptions, RelinkOutcome, RelinkRow, RelinkWords};
pub use list::{LinkFilter, LinkList, LinkListEvent, LinkListOutput, LinkListSettings, LinkListState, LinkListWords, LinkSort};
