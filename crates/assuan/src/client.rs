//! An Assuan client: one command at a time, its answer read to its `OK` or `ERR`, inquiries
//! answered on the way.

use std::io::{Read, Write};

use crate::line::{Line, Wire, parse};
use crate::{Error, MAX_DATA, MAX_INQUIRIES, MAX_INQUIRY_DATA, MAX_LINE, MAX_LINES, MAX_STATUS};

/// A status line (`S <keyword> <arguments>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The keyword, as sent (printable ASCII).
    pub keyword: String,
    /// The arguments, unescaped (a status line's arguments are text; bytes that aren't UTF-8 are
    /// replaced).
    pub args: String,
}

/// The answer to a command that the server ended with `OK`.
#[derive(Default, PartialEq, Eq)]
pub struct Response {
    /// The `D` lines' data, joined and unescaped. Wiped when the response is dropped.
    pub data: Vec<u8>,
    /// The status lines, in order (at most [`MAX_STATUS`](crate::MAX_STATUS)).
    pub status: Vec<Status>,
    /// What followed `OK`.
    pub ok: String,
}

impl Response {
    /// The arguments of the first status line with `keyword`.
    pub fn status(&self, keyword: &str) -> Option<&str> {
        self.status.iter().find(|s| s.keyword == keyword).map(|s| s.args.as_str())
    }
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The data may be a session key: only its size is shown.
        f.debug_struct("Response")
            .field("data", &format_args!("<{} bytes>", self.data.len()))
            .field("status", &self.status)
            .field("ok", &self.ok)
            .finish()
    }
}

impl Drop for Response {
    fn drop(&mut self) {
        crate::wipe(&mut self.data);
    }
}

/// An inquiry from the server (`INQUIRE <keyword> <arguments>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inquiry {
    pub keyword: String,
    pub args: String,
}

/// How an inquiry is answered.
#[derive(PartialEq, Eq)]
pub enum Answer {
    /// These bytes (sent as `D` lines and `END`; none is an empty answer).
    Data(Vec<u8>),
    /// `CAN`: the inquiry is refused, and the server ends the command with an error.
    Cancel,
}

impl std::fmt::Debug for Answer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Answer::Data(d) => write!(f, "Data(<{} bytes>)", d.len()),
            Answer::Cancel => f.write_str("Cancel"),
        }
    }
}

impl Drop for Answer {
    fn drop(&mut self) {
        if let Answer::Data(d) = self {
            crate::wipe(d);
        }
    }
}

/// An Assuan connection, from the client's side.
pub struct Client<S> {
    wire: Wire<S>,
    /// An earlier command failed in a way that leaves the conversation out of step.
    broken: bool,
}

impl<S> std::fmt::Debug for Client<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("broken", &self.broken).finish_non_exhaustive()
    }
}

/// Whether `command` can go on one line as it is.
fn check_command(command: &[u8]) -> Result<(), Error> {
    if command.is_empty() {
        return Err(Error::Refused("an empty command".into()));
    }
    if command.iter().any(|b| matches!(b, b'\n' | b'\r' | 0)) {
        return Err(Error::Refused("a line break in a command".into()));
    }
    if command.len() >= MAX_LINE {
        return Err(Error::Refused("a command longer than a line".into()));
    }
    Ok(())
}

impl<S: Read + Write> Client<S> {
    /// Take over a fresh connection: the server's greeting is read (its `OK`). An `ERR` instead
    /// is the server's refusal.
    pub fn connect(stream: S) -> Result<Self, Error> {
        let mut client = Client { wire: Wire::new(stream), broken: false };
        client.read_answer(&mut |_| Answer::Cancel, None)?;
        Ok(client)
    }

    /// Send `command` and read its answer; an inquiry is refused (`CAN`).
    pub fn command(&mut self, command: &str) -> Result<Response, Error> {
        self.transact(command, &mut |_| Answer::Cancel)
    }

    /// Send `command` (one line, without its line feed) and read its answer, with `inquire`
    /// answering each inquiry the server makes. `Err(Error::Server)` is the server's `ERR`; the
    /// connection can be used again after it. After any other error it can't
    /// ([`Error::Broken`]).
    pub fn transact(&mut self, command: &str, inquire: &mut dyn FnMut(&Inquiry) -> Answer) -> Result<Response, Error> {
        self.send(command)?;
        self.read_answer(inquire, None)
    }

    /// As [`Client::command`], with each status line handed to `on_status` as it comes instead of
    /// kept in the response, so that an answer may have more of them than
    /// [`MAX_STATUS`](crate::MAX_STATUS) (a line for each key, say). The answer's lines are still
    /// capped at [`MAX_LINES`](crate::MAX_LINES).
    pub fn command_with_status(&mut self, command: &str, on_status: &mut dyn FnMut(Status)) -> Result<Response, Error> {
        self.send(command)?;
        self.read_answer(&mut |_| Answer::Cancel, Some(on_status))
    }

    fn send(&mut self, command: &str) -> Result<(), Error> {
        if self.broken {
            return Err(Error::Broken);
        }
        check_command(command.as_bytes())?;
        self.broken = true;
        self.wire.write_line(command.as_bytes())
    }

    /// Read lines to the end of an answer. The connection stays marked broken unless the answer
    /// was read whole.
    fn read_answer(&mut self, inquire: &mut dyn FnMut(&Inquiry) -> Answer, mut on_status: Option<&mut dyn FnMut(Status)>) -> Result<Response, Error> {
        self.broken = true;
        let mut response = Response::default();
        let mut inquiries = 0usize;
        // An inquiry refused on this side because its answer was too big: said when the
        // server has ended the command.
        let mut refused: Option<Error> = None;
        for _ in 0..MAX_LINES {
            let mut line = self.wire.read_line()?;
            let parsed = parse(&line);
            crate::wipe(&mut line);
            match parsed? {
                Line::Comment => {}
                Line::Data(mut data) => {
                    let fits = response.data.len().saturating_add(data.len()) <= MAX_DATA;
                    if fits {
                        crate::extend_wiped(&mut response.data, &data);
                    }
                    crate::wipe(&mut data);
                    if !fits {
                        return Err(Error::TooMuch);
                    }
                }
                Line::Status(keyword, args) => {
                    let status =
                        Status { keyword: String::from_utf8_lossy(&keyword).into_owned(), args: String::from_utf8_lossy(&args).into_owned() };
                    if let Some(on_status) = on_status.as_mut() {
                        on_status(status);
                        continue;
                    }
                    if response.status.len() >= MAX_STATUS {
                        return Err(Error::TooMuch);
                    }
                    response.status.push(status);
                }
                Line::Inquire(keyword, args) => {
                    inquiries = inquiries.saturating_add(1);
                    if inquiries > MAX_INQUIRIES {
                        return Err(Error::TooMuch);
                    }
                    let inquiry =
                        Inquiry { keyword: String::from_utf8_lossy(&keyword).into_owned(), args: String::from_utf8_lossy(&args).into_owned() };
                    match inquire(&inquiry) {
                        Answer::Data(ref data) if data.len() > MAX_INQUIRY_DATA => {
                            refused = Some(Error::Refused("an answer to the agent's inquiry too big to send".into()));
                            self.wire.write_line(b"CAN")?;
                        }
                        Answer::Data(ref data) => self.wire.write_data(data)?,
                        Answer::Cancel => self.wire.write_line(b"CAN")?,
                    }
                }
                Line::Ok(text) => {
                    self.broken = false;
                    if let Some(e) = refused {
                        return Err(e);
                    }
                    response.ok = crate::tidy(&text, 200);
                    return Ok(response);
                }
                Line::Err(e) => {
                    self.broken = false;
                    return Err(refused.unwrap_or(Error::Server(e)));
                }
            }
        }
        Err(Error::TooMuch)
    }

    /// The stream, for its owner to set up (how long a socket waits, say). Reading from it or
    /// writing to it here would take the conversation apart.
    pub fn get_mut(&mut self) -> &mut S {
        self.wire.get_mut()
    }

    /// The stream back, the conversation over.
    pub fn into_inner(self) -> S {
        self.wire.into_inner()
    }
}
