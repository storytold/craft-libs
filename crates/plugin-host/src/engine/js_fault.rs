//! Map a JavaScript exception from a browser's WebAssembly to a [`Trap`].
//!
//! Browsers give a trap as an `Error` with a `name` and an English `message`, and no code. The
//! message text differs per browser and per release, so this match is a best effort. A trap it
//! cannot place is `Trap::GuestFault` with the browser's own text. The two traps that carry
//! meaning for the host are found by substring: `unreachable` and a stack overflow.

use super::Trap;

/// The trap for a JS exception named `name` with text `message`.
///
/// - V8: `RuntimeError: unreachable`, `RangeError: Maximum call stack size exceeded`.
/// - SpiderMonkey: `RuntimeError: unreachable executed`, `InternalError: too much recursion`.
pub fn classify_js_error(name: &str, message: &str) -> Trap {
    let text = message.to_ascii_lowercase();
    if name == "RuntimeError" && text.contains("unreachable") {
        return Trap::Unreachable;
    }
    let overflow = text.contains("call stack size exceeded") || text.contains("too much recursion");
    if overflow && matches!(name, "RangeError" | "InternalError") {
        return Trap::StackOverflow;
    }
    Trap::GuestFault(format!("{name}: {message}"))
}
