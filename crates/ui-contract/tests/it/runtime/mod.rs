<<<<<<< HEAD
mod control_click;
mod cursor;
mod elapsed;
mod env;
mod frame;
mod latch;
mod memory;
mod paste;
mod pen;
mod pointer;
mod wheel;
mod window_move;
mod windows;
mod words;
=======
pub(crate) mod alt_code;
pub(crate) mod chord_fallthrough;
// `clipboard.rs` is platform-w3's trial (F-28); this file is shell-w1's, so its line is written here.
pub(crate) mod clipboard;
pub(crate) mod control_click;
pub(crate) mod cursor;
pub(crate) mod derived;
pub(crate) mod elapsed;
pub(crate) mod env;
pub(crate) mod focus;
pub(crate) mod frame;
pub(crate) mod latch;
pub(crate) mod memory;
pub(crate) mod pen;
pub(crate) mod pointer;
pub(crate) mod theme_app;
pub(crate) mod wheel;
pub(crate) mod window_move;
pub(crate) mod windows;
pub(crate) mod words;
>>>>>>> origin/next

pub(crate) mod screenshot;
