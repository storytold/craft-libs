//! Input methods for text typed onto canvases.
//!
//! egui's `TextEdit` handles its own composition. A canvas that takes text (a type tool, a cell
//! being typed into, a caption on a timeline) uses one [`Composer`] instead:
//!
//! ```no_run
//! # fn insert(_: &str) {} fn press(_: egui::Key, _: egui::Modifiers) {}
//! # fn paste(_: &str) {} fn copy() {} fn cut() {} fn delete_around(_: usize, _: usize) {}
//! # fn delete_whole_chars_around(_: usize, _: usize) {}
//! # fn caret_rect() -> egui::Rect { egui::Rect::ZERO }
//! # let ctx = egui::Context::default(); let mut composer = craft_ui::ime::Composer::new();
//! # let editing = true;
//! use craft_ui::ime::Edit;
//! // Every pass, before shortcuts and other keyboard handling:
//! for edit in composer.update(&ctx, editing) {
//!     match edit {
//!         // The editor paints marked text from `composer.preedit()`.
//!         Edit::Preedit { .. } => {}
//!         Edit::Insert(text) => insert(&text),
//!         Edit::Key { key, modifiers, .. } => press(key, modifiers),
//!         Edit::LineBreak { modifiers } => press(egui::Key::Enter, modifiers),
//!         Edit::Paste(text) => paste(&text),
//!         Edit::Copy => copy(),
//!         Edit::Cut => cut(),
//!         Edit::DeleteSurrounding { before_chars, after_chars } => delete_around(before_chars, after_chars),
//!         // Whole chars inside the byte counts; `craft_ui::ime::surrounding_chars` counts them.
//!         Edit::DeleteSurroundingBytes { before_bytes, after_bytes } => delete_whole_chars_around(before_bytes, after_bytes),
//!     }
//! }
//! // After laying out the text, in screen points:
//! composer.set_caret(&ctx, caret_rect());
//! // Paint `composer.preedit()` at the caret, underlined.
//! ```
//!
//! The app applies every edit; the composer changes no text. Keys and clipboard events it hands
//! over as edits also stay in the input, so the app takes each from one of the two.
//!
//! When the app ends a composition itself, [`Composer::end`] returns the marked text to insert
//! and [`Composer::discard`] drops it (the edit was cancelled, or the editor already keeps it).
//! [`Composer::interrupted`] says when the platform IME should drop its marked text, for hosts
//! that talk to it directly.
//!
//! The composer covers what the apps each handled on their own:
//!
//! - Events stay in the order the platform sent them, so a Telex Backspace that replaces a
//!   letter lands between the right two inserts.
//! - Keys, text and clipboard events belong to the IME while it composes. Enter, Escape,
//!   Backspace, Delete and Space that end a composition are its too, once: Enter that confirms a
//!   word doesn't also start a line or submit, Backspace that empties the marked text doesn't
//!   delete the character before it, and Escape that cancels doesn't leave the editor.
//! - Text committed with no composition (IBus and fcitx send plain letters this way) leaves the
//!   keys after it to the editor; only the Space that typed a committed space is dropped.
//! - A bare line break committed by the IME confirms the marked text instead of breaking the line,
//!   and interrupts the IME as a click does. With nothing composing, a line break sent as text or
//!   committed arrives as [`Edit::LineBreak`], and the line break typed by a pressed Enter is
//!   dropped, so each Enter arrives once.
//! - Empty preedits outside a composition (Wayland sends them on every state change) are ignored.
//! - A click while composing keeps the marked text as typed and tells the IME to drop its copy;
//!   a late commit of the same text is dropped. Giving up the keyboard does the same.
//! - Text that repeats the commit just made, as the text of the key the IME took (Apple Korean's
//!   Space), is typed once. A key the editor gets in between, or a new pass, ends that.
//! - Cmd/Ctrl chords don't type their letter if the host sends it; Alt and AltGr still type.
//! - The candidate window is placed at the caret, clamped into the window.
//!
//! Platform bugs below egui (lost commits on macOS, the Korean first key, X11 focus) belong in
//! winit and are fixed there; the composer doesn't work around them.

mod composer;

pub use composer::{Composer, Edit, Preedit, Processed, surrounding_chars};

#[cfg(test)]
mod tests;
