//! Sending the notices over SMTP with lettre (STARTTLS or TLS through rustls).

use super::config::{Security, Smtp};
use crate::Error;
use crate::mail::{Email, Mailer};
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{Message, SmtpTransport, Transport};
use std::time::Duration;

/// Sends to the administrator through the configured SMTP server.
pub struct SmtpMailer {
    from: Mailbox,
    to: Mailbox,
    transport: SmtpTransport,
}

impl SmtpMailer {
    pub fn new(smtp: &Smtp, to: &str) -> Result<SmtpMailer, Error> {
        let (from, to) = mailboxes(&smtp.from, to)?;
        let tls = |host: &str| TlsParameters::new(host.into()).map_err(|e| Error::Mail(format!("TLS for {host}: {e}")));
        let mut builder = SmtpTransport::builder_dangerous(smtp.host.trim()).port(smtp.port).timeout(Some(Duration::from_secs(30)));
        builder = match smtp.security {
            Security::Starttls => builder.tls(Tls::Required(tls(smtp.host.trim())?)),
            Security::Tls => builder.tls(Tls::Wrapper(tls(smtp.host.trim())?)),
            Security::None => builder.tls(Tls::None),
        };
        if let (Some(user), Some(password)) = (&smtp.username, &smtp.password) {
            builder = builder.credentials(Credentials::new(user.clone(), password.clone()));
        }
        Ok(SmtpMailer { from, to, transport: builder.build() })
    }
}

impl Mailer for SmtpMailer {
    fn send(&self, email: &Email) -> Result<(), Error> {
        let message = message(&self.from, &self.to, email)?;
        self.transport.send(&message).map(|_| ()).map_err(|e| Error::Mail(format!("couldn't send the email: {e}")))
    }
}

/// Parses the sender and recipient.
pub fn mailboxes(from: &str, to: &str) -> Result<(Mailbox, Mailbox), Error> {
    let from: Mailbox = from.parse().map_err(|e| Error::Mail(format!("smtp.from {from:?}: {e}")))?;
    let to: Mailbox = to.parse().map_err(|e| Error::Mail(format!("admin.email {to:?}: {e}")))?;
    Ok((from, to))
}

/// The RFC 5322 message for an [`Email`]: plain UTF-8 text; lettre encodes the headers, so a
/// version or app name can't add header lines.
pub fn message(from: &Mailbox, to: &Mailbox, email: &Email) -> Result<Message, Error> {
    Message::builder()
        .from(from.clone())
        .to(to.clone())
        .subject(email.subject.clone())
        .header(lettre::message::header::ContentType::TEXT_PLAIN)
        .body(email.body.clone())
        .map_err(|e| Error::Mail(format!("couldn't build the email: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_become_plain_text_messages_and_header_injection_is_inert() {
        let (from, to) = mailboxes("GridCraft <updates@example.org>", "admin@example.org").unwrap();
        let email = Email { subject: "GridCraft 0.6.1 is available\r\nBcc: victim@example.org".into(), body: "Hello\nworld\n".into() };
        let raw = String::from_utf8(message(&from, &to, &email).unwrap().formatted()).unwrap();
        assert!(raw.contains("From: GridCraft <updates@example.org>\r\n"), "{raw}");
        assert!(raw.contains("To: admin@example.org\r\n"), "{raw}");
        assert!(raw.contains("Content-Type: text/plain; charset=utf-8\r\n"), "{raw}");
        assert!(!raw.contains("\r\nBcc:"), "{raw}");
        assert!(mailboxes("not an address", "admin@example.org").is_err());
    }
}
