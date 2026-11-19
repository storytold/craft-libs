//! Dialog shells and prompts: `Dialog`, `DialogFrame`, `BlockingOverlay`, `Prompt` with its
//! configurations (`Alert`, `StandardPrompt`, `UnsavedChangesDialog`, `RecoveryDialog`) and the values an app routes
//! its dialogs by (`DialogStack`, `DialogHost`). Form dialogs build on the shells.

pub(crate) mod alert;
pub(crate) mod blocking;
#[doc(hidden)]
pub mod frame;
mod held;
mod host;
pub(crate) mod modal;
pub(crate) mod prompt;
pub(crate) mod prompts;
pub(crate) mod recovery;
mod serde_fields;
mod stack;
#[cfg(feature = "suite")]
#[doc(hidden)]
pub mod stories;
#[cfg(feature = "suite")]
pub(crate) mod trials;
pub(crate) mod unsaved;
mod words;

pub use alert::{Alert, AlertOutput, HostedAlert};
pub use blocking::{BlockingOverlay, BlockingOverlayStyle};
pub use frame::{DialogFrame, DialogFrameStyle, DialogOutput, FooterCx, FooterHooks};
pub use held::{HeldAnswer, HeldOk};
#[doc(hidden)]
pub use host::dialog_probe;
pub use host::{DialogHost, DialogResult, FieldRefusal, OpenDialog, StackDialog, StartupStage};
pub use modal::{ButtonPlace, Dialog, DialogStyle};
pub use prompt::{Prompt, PromptOutput};
pub use prompts::{PromptAnswer, PromptChoice, PromptKind, StandardPrompt, StandardPromptOutput};
pub use recovery::{RecoveredDocument, RecoveryAnswer, RecoveryDialog, RecoveryOutput};
pub use serde_fields::{serde_fields, set_serde_field};
pub use stack::{DialogId, DialogStack, dialog_id};
pub use unsaved::{UnsavedAnswer, UnsavedChangesDialog, UnsavedChangesOutput};
pub use words::DialogWords;
