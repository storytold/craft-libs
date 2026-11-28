//! A settings portal and an Xfconf service that a test serves itself, over a socket pair in zbus's peer-to-peer mode:
//! no bus daemon and no desktop. It answers `ReadOne`, the legacy `Read` (one variant more) and `GetProperty` from a
//! table the test changes while it runs, and sends the signals the test asks for.

use futures_lite::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use zbus::zvariant::{OwnedValue, Value};

/// What the fake answers for a key.
pub(crate) enum Answer {
    Value(OwnedValue),
    /// A D-Bus error by name.
    Error(&'static str),
}

/// The keys the fake knows, shared with the test; a key it doesn't know answers the portal's `NotFound`.
#[derive(Clone, Default)]
pub(crate) struct Keys(Arc<Mutex<HashMap<(String, String), Answer>>>);

impl Keys {
    pub(crate) fn set(&self, namespace: &str, key: &str, answer: Answer) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).insert((namespace.to_owned(), key.to_owned()), answer);
    }
}

/// A signal for the fake to send.
pub(crate) enum Emit {
    /// The portal's `SettingChanged(namespace, key, value)`.
    SettingChanged(&'static str, &'static str, OwnedValue),
    /// Xfconf's `PropertyChanged(channel, property, value)`.
    PropertyChanged(&'static str, &'static str, OwnedValue),
    /// Xfconf's `PropertyRemoved(channel, property)`.
    PropertyRemoved(&'static str, &'static str),
    /// The bus's `NameOwnerChanged(name, old, new)`.
    OwnerChanged(&'static str, &'static str, &'static str),
}

/// A client connection and the fake's end of it.
pub(crate) async fn pair() -> Option<(zbus::Connection, zbus::Connection)> {
    let (ours, theirs) = std::os::unix::net::UnixStream::pair().ok()?;
    let server = zbus::connection::Builder::async_io_unix_stream(theirs).server(zbus::Guid::generate()).ok()?.p2p().build();
    let client = zbus::connection::Builder::async_io_unix_stream(ours).p2p().build();
    let (server, client) = futures_lite::future::zip(server, client).await;
    Some((client.ok()?, server.ok()?))
}

enum Next {
    Call(zbus::Result<zbus::Message>),
    Emit(Emit),
}

/// Answers calls from `keys` and sends what `emits` brings, until the client is gone; with `emits` closed it goes on
/// answering.
pub(crate) async fn serve(server: zbus::Connection, keys: Keys, emits: async_channel::Receiver<Emit>) {
    let mut calls = zbus::MessageStream::from(&server);
    loop {
        let next = futures_lite::future::or(async { calls.next().await.map(Next::Call) }, async {
            match emits.recv().await {
                Ok(emit) => Some(Next::Emit(emit)),
                Err(_) => std::future::pending().await,
            }
        })
        .await;
        let _ = match next {
            Some(Next::Call(Ok(call))) => answer(&server, &keys, &call).await,
            Some(Next::Emit(emit)) => send(&server, emit).await,
            Some(Next::Call(Err(_))) => continue,
            None => return,
        };
    }
}

async fn answer(server: &zbus::Connection, keys: &Keys, call: &zbus::Message) -> zbus::Result<()> {
    let header = call.header();
    if header.message_type() != zbus::message::Type::MethodCall {
        return Ok(());
    }
    let member = header.member().map(|member| member.to_string()).unwrap_or_default();
    let Ok((namespace, key)) = call.body().deserialize::<(String, String)>() else { return Ok(()) };
    let found = {
        let keys = keys.0.lock().unwrap_or_else(PoisonError::into_inner);
        match keys.get(&(namespace, key)) {
            Some(Answer::Value(value)) => Ok(value.try_clone().map_err(zbus::Error::from)?),
            Some(Answer::Error(name)) => Err(*name),
            None => Err("org.freedesktop.portal.Error.NotFound"),
        }
    };
    match found {
        Ok(value) if member == "Read" => server.reply(&header, &Value::Value(Box::new(Value::from(value)))).await,
        Ok(value) => server.reply(&header, &value).await,
        Err(name) => server.reply_error(&header, name, &"the fake has no such value").await,
    }
}

async fn send(server: &zbus::Connection, emit: Emit) -> zbus::Result<()> {
    const NONE: Option<&str> = None;
    match emit {
        Emit::SettingChanged(namespace, key, value) => {
            server
                .emit_signal(NONE, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings", "SettingChanged", &(namespace, key, value))
                .await
        }
        Emit::PropertyChanged(channel, property, value) => {
            server.emit_signal(NONE, "/org/xfce/Xfconf", "org.xfce.Xfconf", "PropertyChanged", &(channel, property, value)).await
        }
        Emit::PropertyRemoved(channel, property) => {
            server.emit_signal(NONE, "/org/xfce/Xfconf", "org.xfce.Xfconf", "PropertyRemoved", &(channel, property)).await
        }
        // A bus sends its own signals as `org.freedesktop.DBus`, a unique name to zbus, so a rule for them matches only
        // that sender.
        Emit::OwnerChanged(name, old, new) => {
            let signal = zbus::Message::signal("/org/freedesktop/DBus", "org.freedesktop.DBus", "NameOwnerChanged")?
                .sender("org.freedesktop.DBus")?
                .build(&(name, old, new))?;
            server.send(&signal).await
        }
    }
}

/// `value` as the owned value a key holds.
pub(crate) fn owned<'a>(value: impl Into<Value<'a>>) -> OwnedValue {
    OwnedValue::try_from(value.into()).expect("a value without file descriptors")
}

/// Waits up to five seconds for `done`, checking every 20 ms.
pub(crate) fn eventually(done: impl Fn() -> bool) -> bool {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < until {
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    done()
}
