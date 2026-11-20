# craft-ssip

A focused Speech Dispatcher SSIP wire layer: sentence-as-data encoding, final response codes,
and a bounded incremental reply reader. No sockets, process launch, cancellation commands,
voice discovery, retry or playback scheduling: those policies remain with the application.

Derived from Skaywalker's WordCraft SSIP implementation in
`crates/engine/src/speech/ssip.rs` at `e9750c5cb75d577a4669059d7bf6f9874e945839`
(MIT OR Apache-2.0). WordCraft #2937 (`65d7999eddebf120ca9fde523b01156b3e1f5fce`)
adds voice selection on this same core; this extraction does not duplicate those application changes.

Unlike the original reader, a complete line that becomes overlong in the final read is refused.
The 4096-byte limit counts bytes before LF, including an optional CR. Reply buffering holds
at most that limit plus one 512-byte read. Invalid UTF-8 is replaced, preserving WordCraft behavior.
Message input is bounded by the caller; ASCII three-digit response codes are enforced.
Caller-provided `Read` implementations must obey Rust's Read contract; callers remain responsible
for transport timeouts, cancellation and an overall deadline between `next_line` calls.

This is not a complete SSIP client and does not establish WordCraft consumer cutover.
