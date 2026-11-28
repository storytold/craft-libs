//! xfwm4's `/general/button_layout` from Xfconf, Xfce's settings service, over D-Bus: `org.xfce.Xfconf` at
//! `/org/xfce/Xfconf`, `GetProperty(channel, property)`, and the `PropertyChanged(channel, property, value)` and
//! `PropertyRemoved(channel, property)` signals of Xfconf's public interface description. The service starts on the
//! first call (D-Bus activation) and may be replaced; a removed property is read again, which gives the system's
//! default where there is one. A sandbox reaches Xfconf only with `--talk-name=org.xfce.Xfconf`; without it the call
//! is refused and the next source answers.

use super::super::{LayoutSource, RawLayout, Shared};
use super::{RETRY, back_off};
use craft_host::caption_layout::{LayoutDesktop, LayoutReading, MAX_LAYOUT_BYTES};
use futures_lite::StreamExt;
use std::time::Duration;
use zbus::zvariant::{OwnedValue, Value};

const DEST: &str = "org.xfce.Xfconf";
const PATH: &str = "/org/xfce/Xfconf";
const IFACE: &str = "org.xfce.Xfconf";
const CHANNEL: &str = "xfwm4";
const PROPERTY: &str = "/general/button_layout";
const TIMEOUT: Duration = Duration::from_secs(2);
const SOURCE: LayoutSource = LayoutSource::DesktopService;

/// Follows xfwm4's layout on the session bus until the watcher is dropped; returns at once unless xfwm4 is the window
/// manager.
pub(super) async fn watch(shared: &Shared, desktop: LayoutDesktop) {
    if !matches!(desktop, LayoutDesktop::Xfce | LayoutDesktop::Xfwm) {
        return;
    }
    let mut retry = RETRY;
    loop {
        let connection = match zbus::connection::Builder::session() {
            Ok(builder) => bounded(builder.method_timeout(TIMEOUT).build()).await.and_then(Result::ok),
            Err(_) => None,
        };
        if let Some(connection) = connection
            && follow(shared, &connection).await
        {
            retry = RETRY;
        }
        back_off(&mut retry).await;
    }
}

/// Reads the property, then follows its signals and the service's replacement until the connection fails. Whether
/// the service answered at all.
pub(super) async fn follow(shared: &Shared, connection: &zbus::Connection) -> bool {
    let Some(mut changes) = subscribe(connection, signal_rule("PropertyChanged")).await else { return false };
    let Some(mut removals) = subscribe(connection, signal_rule("PropertyRemoved")).await else { return false };
    let Some(mut owners) = subscribe(connection, owner_rule()).await else { return false };
    let mut answered = read(shared, connection).await;
    loop {
        let event = futures_lite::future::or(
            futures_lite::future::or(async { changes.next().await.map(Event::Changed) }, async { removals.next().await.map(Event::Removed) }),
            async { owners.next().await.map(Event::Owner) },
        )
        .await;
        match event {
            Some(Event::Changed(Ok(message))) => {
                if let Ok((channel, property, value)) = message.body().deserialize::<(String, String, OwnedValue)>()
                    && (channel.as_str(), property.as_str()) == (CHANNEL, PROPERTY)
                    && let Some(reading) = reading_of(&value)
                {
                    answered = true;
                    shared.publish_caption(SOURCE, Some(reading));
                }
            }
            // Reset to its default, or a new service: what it says now.
            Some(Event::Removed(Ok(_)) | Event::Owner(Ok(_))) => answered |= read(shared, connection).await,
            _ => return answered,
        }
    }
}

enum Event {
    Changed(zbus::Result<zbus::Message>),
    Removed(zbus::Result<zbus::Message>),
    Owner(zbus::Result<zbus::Message>),
}

/// Reads the property and publishes what the service said: its value, or that it has none (no such property, a value
/// of another type, the call refused). A call that fails to get an answer changes nothing. Whether it answered.
async fn read(shared: &Shared, connection: &zbus::Connection) -> bool {
    match bounded(connection.call_method(Some(DEST), PATH, Some(IFACE), "GetProperty", &(CHANNEL, PROPERTY))).await {
        Some(Ok(message)) => {
            let reading = message.body().deserialize::<OwnedValue>().ok().as_deref().and_then(reading_of);
            shared.publish_caption(SOURCE, reading);
            true
        }
        Some(Err(zbus::Error::MethodError(..))) => {
            shared.publish_caption(SOURCE, None);
            true
        }
        _ => false,
    }
}

/// The property's value as a layout: a string, at most `MAX_LAYOUT_BYTES` long.
fn reading_of(value: &Value<'_>) -> Option<LayoutReading> {
    let Value::Str(text) = value else { return None };
    (text.len() <= MAX_LAYOUT_BYTES)
        .then(|| LayoutReading::new(SOURCE, RawLayout::Xfwm(text.as_str().trim().to_owned()), format!("Xfconf, {CHANNEL} {PROPERTY}")))
}

fn signal_rule(member: &'static str) -> zbus::Result<zbus::MatchRule<'static>> {
    Ok(zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(DEST)?
        .path(PATH)?
        .interface(IFACE)?
        .member(member)?
        .arg(0, CHANNEL)?
        .arg(1, PROPERTY)?
        .build())
}

fn owner_rule() -> zbus::Result<zbus::MatchRule<'static>> {
    Ok(zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")?
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .arg(0, DEST)?
        .build())
}

async fn subscribe(connection: &zbus::Connection, rule: zbus::Result<zbus::MatchRule<'static>>) -> Option<zbus::MessageStream> {
    bounded(zbus::MessageStream::for_match_rule(rule.ok()?, connection, Some(8))).await?.ok()
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> Option<T> {
    futures_lite::future::race(async { Some(future.await) }, async {
        async_io::Timer::after(TIMEOUT).await;
        None
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{CHANNEL, PROPERTY, follow};
    use crate::appearance::fake_bus::{Answer, Emit, Keys, eventually, owned, pair, serve};
    use crate::appearance::{RawLayout, Shared, Watcher};

    // xfwm4's layout is read from Xfconf and followed: a change of the property is taken, another property's is not,
    // and a property reset with no default left gives way to the next source.
    #[test]
    fn xfwm4s_layout_follows_xfconf() -> Result<(), Box<dyn std::error::Error>> {
        let keys = Keys::default();
        keys.set(CHANNEL, PROPERTY, Answer::Value(owned("C|HM")));
        let service = keys.clone();
        let (emit, emits) = async_channel::unbounded();
        let watcher = Watcher::spawn_shared(Shared::new(false, Box::new(|| {})), move |shared| async move {
            let Some((client, server)) = pair().await else { return };
            futures_lite::future::zip(serve(server, service, emits), follow(&shared, &client)).await;
        })
        .ok_or("worker creation failed")?;
        let layout = |watcher: &Watcher| watcher.caption_layout().map(|reading| reading.raw);
        assert!(eventually(|| layout(&watcher) == Some(RawLayout::Xfwm("C|HM".into()))), "read at start");
        emit.send_blocking(Emit::PropertyChanged(CHANNEL, "/general/theme", owned("Default")))?;
        emit.send_blocking(Emit::PropertyChanged(CHANNEL, PROPERTY, owned("O|SHMC")))?;
        assert!(eventually(|| layout(&watcher) == Some(RawLayout::Xfwm("O|SHMC".into()))), "the change is followed");
        keys.set(CHANNEL, PROPERTY, Answer::Error("org.xfce.Xfconf.Error.PropertyNotFound"));
        emit.send_blocking(Emit::PropertyRemoved(CHANNEL, PROPERTY))?;
        assert!(eventually(|| layout(&watcher).is_none()), "a removed property gives way");
        Ok(())
    }
}
