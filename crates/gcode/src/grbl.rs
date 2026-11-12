//! Talking to a Grbl controller: the real-time command bytes and the parsers of what it answers.
//! No I/O happens here; a sender writes the bytes and hands the replies in.
//!
//! Facts from the Grbl v1.1 interface documentation
//! (<https://github.com/gnea/grbl/wiki/Grbl-v1.1-Interface>): a line is answered with `ok` or
//! `error:<n>`, a fault with `ALARM:<n>`, and `?` with a status report `<State|…>`.

/// Real-time command: ask for a status report ([`parse_status`] reads it).
pub const STATUS_QUERY: u8 = b'?';
/// Real-time command: feed hold (pause).
pub const FEED_HOLD: u8 = b'!';
/// Real-time command: cycle start (resume).
pub const CYCLE_START: u8 = b'~';
/// Real-time command: soft reset (Ctrl-X). Cancels the job.
pub const SOFT_RESET: u8 = 0x18;
/// The line that switches the laser or spindle off; send it when a job is abandoned mid-stream.
pub const TOOL_OFF: &str = "M5";

/// Most bytes of a status report [`parse_status`] looks at. Grbl's own lines are well under 100
/// bytes.
pub const MAX_REPLY_LEN: usize = 256;

/// The machine state a status report starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Idle,
    Run,
    Hold,
    Jog,
    Alarm,
    Door,
    Check,
    Home,
    Sleep,
    /// A state word this crate does not know, as sent.
    Other(String),
}

/// The first `MAX_REPLY_LEN` bytes of `reply`, cut at a character boundary.
fn capped(reply: &str) -> &str {
    let mut end = reply.len().min(MAX_REPLY_LEN);
    while !reply.is_char_boundary(end) {
        end -= 1;
    }
    reply.get(..end).unwrap_or("")
}

/// The state of a status report (`<Idle|MPos:…>`, `<Hold:0|…>`). `None` when `reply` is not a
/// status report.
pub fn parse_status(reply: &str) -> Option<State> {
    let body = capped(reply).trim().strip_prefix('<')?;
    Some(match body.split(['|', '>', ':']).next().unwrap_or("") {
        "Idle" => State::Idle,
        "Run" => State::Run,
        "Hold" => State::Hold,
        "Jog" => State::Jog,
        "Alarm" => State::Alarm,
        "Door" => State::Door,
        "Check" => State::Check,
        "Home" => State::Home,
        "Sleep" => State::Sleep,
        other => State::Other(other.to_string()),
    })
}

/// What the controller said about the line it was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ack {
    /// `ok`: the line was accepted; send the next.
    Ok,
    /// `error:<n>` or `ALARM:<n>`: stop sending and switch the tool off.
    Rejected,
}

/// Reads what has arrived since a line was sent. `None` while the answer is still incomplete: read
/// more and ask again with everything received so far. A rejection anywhere in the reply wins over
/// an `ok`. The whole reply is searched, without copying it: unrelated lines may arrive before the
/// answer, and how much to wait for is the sender's limit to set.
pub fn parse_ack(reply: &str) -> Option<Ack> {
    if reply.contains("error") || reply.contains("ALARM") {
        Some(Ack::Rejected)
    } else if reply.contains("ok") {
        Some(Ack::Ok)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_time_bytes_are_the_documented_ones() {
        assert_eq!([STATUS_QUERY, FEED_HOLD, CYCLE_START, SOFT_RESET], *b"?!~\x18");
    }

    #[test]
    fn status_reports_give_their_state() {
        assert_eq!(parse_status("<Idle|MPos:0,0,0>"), Some(State::Idle));
        assert_eq!(parse_status("<Hold:0|MPos:0,0,0>"), Some(State::Hold));
        assert_eq!(parse_status("  <Door:3|WPos:1,2,3|FS:0,0>\r\n"), Some(State::Door));
        for (word, state) in [
            ("Run", State::Run),
            ("Jog", State::Jog),
            ("Alarm", State::Alarm),
            ("Check", State::Check),
            ("Home", State::Home),
            ("Sleep", State::Sleep),
        ] {
            assert_eq!(parse_status(&format!("<{word}>")), Some(state));
        }
        assert_eq!(parse_status("<Tool|MPos:0,0,0>"), Some(State::Other("Tool".into())));
        assert_eq!(parse_status("<"), Some(State::Other(String::new())));
        assert_eq!(parse_status("ok"), None);
        assert_eq!(parse_status(""), None);
    }

    #[test]
    fn acks_are_ok_rejected_or_not_yet() {
        assert_eq!(parse_ack("ok\r\n"), Some(Ack::Ok));
        assert_eq!(parse_ack("error:22\r\n"), Some(Ack::Rejected));
        assert_eq!(parse_ack("ALARM:1\r\n"), Some(Ack::Rejected));
        assert_eq!(parse_ack("ok\r\nerror:9\r\n"), Some(Ack::Rejected));
        assert_eq!(parse_ack("o"), None);
        assert_eq!(parse_ack(""), None);
    }

    #[test]
    fn a_status_report_is_read_only_up_to_the_cap() {
        let pad = " ".repeat(MAX_REPLY_LEN - 2);
        // An acknowledgement is found wherever it is.
        assert_eq!(parse_ack(&format!("{pad}{pad}ok")), Some(Ack::Ok));
        assert_eq!(parse_status(&format!("{pad}<Idle>")), Some(State::Other("I".into())));
        assert_eq!(parse_status(&format!("{pad}  <Idle>")), None);
        // A long unknown state word is kept only up to the cap; a cut inside a character backs up.
        assert_eq!(parse_status(&format!("<{}", "x".repeat(1000))), Some(State::Other("x".repeat(MAX_REPLY_LEN - 1))));
        assert_eq!(parse_status(&format!("<{}", "é".repeat(1000))), Some(State::Other("é".repeat(127))));
    }
}
