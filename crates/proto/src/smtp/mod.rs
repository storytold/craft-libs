// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! An SMTP client (RFC 5321) with STARTTLS (RFC 3207) and AUTH PLAIN / LOGIN / XOAUTH2 (RFC 4954).

use std::io::{Read, Write};

use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::i18n::{Arg, Text};
use crate::line::Wire;
use crate::said;
use crate::{ProtoError, Secrets, for_trace, tidy, xoauth2};

/// Most lines one reply may have.
const MAX_REPLY_LINES: usize = 256;
/// Longest command line without its line ending (RFC 5321 §4.5.3.1.4).
const MAX_COMMAND: usize = 510;
/// Longest address (RFC 5321 §4.5.3.1.3 allows 256; some servers take more).
const MAX_ADDRESS: usize = 320;
/// Most recipients one message may have.
const MAX_RECIPIENTS: usize = 10_000;

/// An SMTP session on a stream.
pub struct Smtp<S> {
    wire: Wire<S>,
    ext: Extensions,
    secrets: Secrets,
}

/// What the server offered in its answer to EHLO.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extensions {
    pub starttls: bool,
    pub auth_plain: bool,
    pub auth_login: bool,
    /// The server takes an OAuth access token instead of a password (SASL XOAUTH2).
    pub auth_xoauth2: bool,
    /// The largest message the server takes; `Some(0)` when it names no limit.
    pub size: Option<u64>,
    pub eight_bit_mime: bool,
}

/// Neither the stream nor anything secret is shown.
impl<S> std::fmt::Debug for Smtp<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Smtp").field("extensions", &self.ext).finish_non_exhaustive()
    }
}

/// One reply: `250-first`, `250 last`.
struct Reply {
    code: u16,
    lines: Vec<String>,
}

impl Reply {
    fn class(&self) -> u16 {
        self.code / 100
    }

    /// `550 5.1.1 No such user`, as the server wrote it.
    fn text(&self) -> String {
        format!("{} {}", self.code, self.lines.join(" "))
    }
}

impl<S: Read + Write> Smtp<S> {
    /// Read the 220 greeting.
    pub fn connect(stream: S) -> Result<Self, ProtoError> {
        let mut smtp = Self::resume(stream);
        let greeting = smtp.reply()?;
        if greeting.code == 220 { Ok(smtp) } else { Err(smtp.refused(&greeting)) }
    }

    /// Continue on a stream that already greeted (after the caller wrapped it in TLS).
    pub fn resume(stream: S) -> Self {
        Smtp { wire: Wire::new(stream), ext: Extensions::default(), secrets: Secrets::default() }
    }

    /// What the last [`hello`](Self::hello) learned.
    pub fn extensions(&self) -> &Extensions {
        &self.ext
    }

    /// The stream the session runs on, for its owner to set up between commands (how long a
    /// socket waits, say). Reading from it or writing to it would take the session apart.
    pub fn get_mut(&mut self) -> &mut S {
        self.wire.get_mut()
    }

    /// EHLO (falling back to HELO); remembers and returns what the server offers.
    pub fn hello(&mut self, client_name: &str) -> Result<Extensions, ProtoError> {
        // A name that could break the command line is not sent.
        let name = if is_word(client_name) { client_name } else { "localhost" };
        let reply = self.command(&format!("EHLO {name}"))?;
        if reply.code == 250 {
            self.ext = extensions(&reply.lines);
            return Ok(self.ext.clone());
        }
        if reply.class() != 5 {
            return Err(self.refused(&reply));
        }
        // A server from before 1995 (or one hiding its extensions).
        let reply = self.command(&format!("HELO {name}"))?;
        if reply.code != 250 {
            return Err(self.refused(&reply));
        }
        self.ext = Extensions::default();
        Ok(self.ext.clone())
    }

    /// Send STARTTLS and hand the stream back for the caller to wrap; call `resume` + `hello` afterwards.
    pub fn starttls(mut self) -> Result<S, ProtoError> {
        let reply = self.command("STARTTLS")?;
        if reply.code != 220 {
            return Err(self.refused(&reply));
        }
        // Anything after the go-ahead was slipped in before encryption started (RFC 3207 §6).
        if self.wire.has_pending() {
            return Err(ProtoError::Protocol(said!("the server sent data before the secure connection started")));
        }
        Ok(self.wire.into_inner())
    }

    /// AUTH PLAIN when offered, else AUTH LOGIN. 535 and friends → `ProtoError::Auth`.
    pub fn auth(&mut self, user: &str, password: &str) -> Result<(), ProtoError> {
        if user.contains('\0') || password.contains('\0') {
            return Err(ProtoError::Auth(said!("the user name or password has a character that can't be sent")));
        }
        self.secrets.add(password);
        if self.ext.auth_plain {
            let payload = STANDARD.encode(format!("\0{user}\0{password}"));
            self.secrets.add(&payload);
            let line = format!("AUTH PLAIN {payload}");
            let reply = if line.len() <= MAX_COMMAND {
                self.secret_command(&line, "AUTH PLAIN ***")?
            } else {
                // Too long for one line: wait for the server to ask (RFC 4954 §4).
                let ask = self.command("AUTH PLAIN")?;
                if ask.code != 334 {
                    return Err(self.sign_in_failed(&ask));
                }
                self.secret_command(&payload, "***")?
            };
            return self.signed_in(&reply);
        }
        if self.ext.auth_login {
            let ask = self.command("AUTH LOGIN")?;
            if ask.code != 334 {
                return Err(self.sign_in_failed(&ask));
            }
            let ask = self.secret_command(&STANDARD.encode(user), "***")?;
            if ask.code != 334 {
                return Err(self.sign_in_failed(&ask));
            }
            let encoded = STANDARD.encode(password);
            self.secrets.add(&encoded);
            let reply = self.secret_command(&encoded, "***")?;
            return self.signed_in(&reply);
        }
        Err(ProtoError::Auth(said!("the server offers no way to sign in with a password")))
    }

    /// AUTH XOAUTH2: an OAuth access token in place of a password. A server that doesn't offer
    /// it isn't shown the token. 535 and friends → `ProtoError::Auth`.
    pub fn auth_xoauth2(&mut self, user: &str, token: &str) -> Result<(), ProtoError> {
        let payload = xoauth2(user, token)?;
        if !self.ext.auth_xoauth2 {
            return Err(ProtoError::Auth(said!("the server offers no way to sign in with an access token")));
        }
        self.secrets.add(token);
        self.secrets.add(&payload);
        let line = format!("AUTH XOAUTH2 {payload}");
        let reply = if line.len() <= MAX_COMMAND {
            self.secret_command(&line, "AUTH XOAUTH2 ***")?
        } else {
            // Too long for one line, as tokens mostly are: wait for the server to ask (RFC 4954 §4).
            let ask = self.command("AUTH XOAUTH2")?;
            if ask.code != 334 {
                return Err(self.sign_in_failed(&ask));
            }
            self.secret_command(&payload, "***")?
        };
        match reply.code {
            235 => Ok(()),
            334 => {
                // A refusal comes in two parts: the reasons (JSON in base64), and, once an empty
                // line has acknowledged them, the verdict.
                let verdict = self.command("")?;
                match verdict.code {
                    235 => Ok(()),
                    334 => {
                        let _ = self.command("*");
                        Err(ProtoError::Auth(said!("the server asked for more than a user name and access token")))
                    }
                    _ => Err(self.sign_in_failed(&verdict)),
                }
            }
            _ => Err(self.sign_in_failed(&reply)),
        }
    }

    /// MAIL FROM, RCPT TO (every recipient must be accepted), DATA with CRLF normalisation and dot-stuffing.
    pub fn send(&mut self, from: &str, recipients: &[&str], message: &[u8]) -> Result<(), ProtoError> {
        // Nothing is sent unless every address is safe to put on a command line.
        if !from.is_empty() && !is_address(from) {
            return Err(bad_address(from));
        }
        if recipients.is_empty() {
            return Err(ProtoError::Protocol(said!("the message has no recipients")));
        }
        if recipients.len() > MAX_RECIPIENTS {
            return Err(ProtoError::Protocol(said!("the message has too many recipients")));
        }
        if let Some(bad) = recipients.iter().find(|r| !is_address(r)) {
            return Err(bad_address(bad));
        }
        let (data, size) = data_block(message);
        let mut mail = format!("MAIL FROM:<{from}>");
        if self.ext.size.is_some() {
            mail.push_str(&format!(" SIZE={size}"));
        }
        if self.ext.eight_bit_mime && !message.is_ascii() {
            mail.push_str(" BODY=8BITMIME");
        }
        let reply = self.command(&mail)?;
        if reply.class() != 2 {
            return Err(self.refused(&reply));
        }
        for rcpt in recipients {
            let reply = self.command(&format!("RCPT TO:<{rcpt}>"))?;
            if reply.class() != 2 {
                let refused = match self.refused(&reply) {
                    // (The address, then what the server said of it: no words of MailCraft's to say.)
                    ProtoError::Rejected(text) => {
                        ProtoError::Rejected(Text::new("{rcpt}: {text}", vec![("rcpt", Arg::Plain(rcpt.to_string())), ("text", Arg::said(text))]))
                    }
                    other => other,
                };
                return Err(self.give_up(refused));
            }
        }
        let reply = self.command("DATA")?;
        if reply.code != 354 {
            let refused = self.refused(&reply);
            return Err(self.give_up(refused));
        }
        log::trace!("C: ({size} bytes)");
        self.wire.write(&data)?;
        self.wire.flush()?;
        let reply = self.reply()?;
        if reply.class() != 2 {
            return Err(self.refused(&reply));
        }
        Ok(())
    }

    /// QUIT. A server that hangs up without answering has quit all the same.
    pub fn quit(mut self) -> Result<(), ProtoError> {
        match self.command("QUIT") {
            Ok(reply) if reply.class() == 2 => Ok(()),
            Ok(reply) => Err(self.refused(&reply)),
            // A server that hangs up instead of saying goodbye has quit all the same.
            Err(ProtoError::Io(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Forget the message being composed, so the session can go on; hands the error back.
    fn give_up(&mut self, error: ProtoError) -> ProtoError {
        let _ = self.command("RSET");
        error
    }

    fn command(&mut self, line: &str) -> Result<Reply, ProtoError> {
        self.secret_command(line, line)
    }

    /// Send `line`; the log gets `shown` instead.
    fn secret_command(&mut self, line: &str, shown: &str) -> Result<Reply, ProtoError> {
        log::trace!("C: {}", for_trace(&self.secrets.hide(shown)));
        let mut out = Vec::with_capacity(line.len() + 2);
        out.extend_from_slice(line.as_bytes());
        out.extend_from_slice(b"\r\n");
        self.wire.write(&out)?;
        self.wire.flush()?;
        self.reply()
    }

    fn reply(&mut self) -> Result<Reply, ProtoError> {
        let mut reply = Reply { code: 0, lines: Vec::new() };
        for n in 0..MAX_REPLY_LINES {
            let raw = self.wire.read_line()?;
            let line = String::from_utf8_lossy(&raw);
            log::trace!("S: {}", for_trace(&self.secrets.hide(&line)));
            // Three digits, then `-` (more lines follow), a space or nothing (the last line).
            let code = line.get(..3).filter(|d| d.bytes().all(|b| b.is_ascii_digit())).and_then(|d| d.parse::<u16>().ok());
            let rest = line.get(3..).unwrap_or("");
            let more = match (code, rest.chars().next()) {
                (Some(code), Some('-')) if n == 0 || code == reply.code => true,
                (Some(code), Some(' ') | None) if n == 0 || code == reply.code => false,
                _ => return Err(ProtoError::Protocol(said!("\"{line}\" is not an SMTP reply", line = tidy(&self.secrets.hide(&line))))),
            };
            reply.code = code.unwrap_or(0);
            reply.lines.push(rest.get(1..).unwrap_or("").to_string());
            if !more {
                return Ok(reply);
            }
        }
        Err(ProtoError::Protocol(said!("the server's reply has too many lines")))
    }

    /// The error for a reply that isn't the one hoped for.
    fn refused(&self, reply: &Reply) -> ProtoError {
        // Hidden before it is shortened: a secret cut in half would no longer be recognised.
        let text = tidy(&self.secrets.hide(&reply.text()));
        if matches!(reply.class(), 4 | 5) { ProtoError::Rejected(text.into()) } else { ProtoError::Protocol(text.into()) }
    }

    fn sign_in_failed(&mut self, reply: &Reply) -> ProtoError {
        match self.refused(reply) {
            ProtoError::Rejected(text) => ProtoError::Auth(text),
            other => other,
        }
    }

    fn signed_in(&mut self, reply: &Reply) -> Result<(), ProtoError> {
        match reply.code {
            235 => Ok(()),
            334 => {
                // Another question we have no answer to: cancel the exchange (RFC 4954 §4).
                let _ = self.command("*");
                Err(ProtoError::Auth(said!("the server asked for more than a user name and password")))
            }
            _ => Err(self.sign_in_failed(reply)),
        }
    }
}

/// The extensions named in an EHLO reply (its first line is the server's name).
fn extensions(lines: &[String]) -> Extensions {
    let mut ext = Extensions::default();
    for line in lines.iter().skip(1) {
        let upper = line.to_ascii_uppercase();
        // `AUTH=PLAIN LOGIN` is how some old servers spell `AUTH PLAIN LOGIN`.
        let mut words = upper.split([' ', '\t', '=']).filter(|w| !w.is_empty());
        match words.next() {
            Some("STARTTLS") => ext.starttls = true,
            Some("8BITMIME") => ext.eight_bit_mime = true,
            Some("SIZE") => ext.size = Some(words.next().and_then(|n| n.parse().ok()).unwrap_or(0)),
            Some("AUTH") => {
                for mechanism in words {
                    match mechanism {
                        "PLAIN" => ext.auth_plain = true,
                        "LOGIN" => ext.auth_login = true,
                        "XOAUTH2" => ext.auth_xoauth2 = true,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    ext
}

/// Visible ASCII without spaces: safe as one word of a command line.
fn is_word(s: &str) -> bool {
    !s.is_empty() && s.len() <= 255 && s.bytes().all(|b| b.is_ascii_graphic())
}

/// An address that can go between `<` and `>` as it is.
fn is_address(s: &str) -> bool {
    is_word(s) && s.len() <= MAX_ADDRESS && !s.contains(['<', '>'])
}

fn bad_address(address: &str) -> ProtoError {
    ProtoError::Protocol(said!("\"{address}\" can't be used as an address", address = tidy(address)))
}

/// The message as DATA wants it: every line ending CRLF, a leading dot doubled, the closing
/// `.` line added. Also returns the size without the added dots and closing line (RFC 1870).
fn data_block(message: &[u8]) -> (Vec<u8>, usize) {
    let mut out = Vec::with_capacity(message.len().saturating_add(message.len() / 32).saturating_add(8));
    let mut dots = 0usize;
    let mut line_start = true;
    let mut bytes = message.iter().copied().peekable();
    while let Some(b) = bytes.next() {
        match b {
            b'\r' => {
                bytes.next_if_eq(&b'\n');
                out.extend_from_slice(b"\r\n");
                line_start = true;
            }
            b'\n' => {
                out.extend_from_slice(b"\r\n");
                line_start = true;
            }
            b'.' if line_start => {
                out.extend_from_slice(b"..");
                dots += 1;
                line_start = false;
            }
            _ => {
                out.push(b);
                line_start = false;
            }
        }
    }
    if !line_start {
        out.extend_from_slice(b"\r\n");
    }
    let size = out.len().saturating_sub(dots);
    out.extend_from_slice(b".\r\n");
    (out, size)
}

#[cfg(test)]
mod tests;
