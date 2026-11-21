//! A Craft Link client for `Services::link_apps` and `Services::link_call` (native only).
//!
//! The plugin host checks the `link.call:<app>` grant and shapes the reply. This module does the
//! transport an app would otherwise write itself: scan the runtime directory for live instances,
//! connect, authenticate, say `link.hello`, send one method, and read one reply.
//!
//! A call runs on its own thread so the caller can watch the cancel flag. When the user cancels,
//! the caller returns `LinkFailure::Cancelled` at once. The thread ends when the peer answers or
//! after the link reply deadline, and its connection closes then.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

use craft_ipc::client::{Client, HelloClient};
use craft_ipc::{Discovery, InstanceRecord, LinkError, RuntimeDir, random_uuid};
use serde_json::Value;

use crate::services::{LinkApp, LinkFailure};

/// How often a waiting call looks at the cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(20);

/// Finds running Craft apps and calls them as one app.
#[derive(Clone)]
pub struct LinkClient {
    runtime: RuntimeDir,
    identity: HelloClient,
}

fn failed(error: &LinkError) -> LinkFailure {
    LinkFailure::Failed(format!("{} ({})", error, error.code()))
}

impl LinkClient {
    /// A client with an explicit `link.hello` identity.
    pub fn new(runtime: RuntimeDir, identity: HelloClient) -> LinkClient {
        LinkClient { runtime, identity }
    }

    /// A client that says it is `app` `app_version`, with this process id and a fresh instance id.
    /// `runtime` is `RuntimeDir::resolve()` in an app, or a temporary directory in a test.
    pub fn for_app(runtime: RuntimeDir, app: &str, app_version: &str) -> Result<LinkClient, LinkFailure> {
        let instance = random_uuid().map_err(|e| failed(&e))?;
        let identity = HelloClient {
            app: app.to_owned(),
            app_version: app_version.to_owned(),
            pid: std::process::id(),
            instance,
            protocol_min: 1,
            protocol_max: 1,
        };
        Ok(LinkClient { runtime, identity })
    }

    /// Live instances other than this client's own, newest first.
    fn peers(&self) -> Result<Vec<InstanceRecord>, LinkFailure> {
        let mut records = Discovery::scan(&self.runtime).map_err(|e| failed(&e))?;
        records.retain(|r| r.pid != self.identity.pid || r.app != self.identity.app);
        records.sort_by_key(|r| std::cmp::Reverse(r.started));
        Ok(records)
    }

    /// One row per running app. Where an app has several instances, the newest one speaks for it.
    pub fn apps(&self) -> Result<Vec<LinkApp>, LinkFailure> {
        let mut out: Vec<LinkApp> = Vec::new();
        for r in self.peers()? {
            if out.iter().all(|a| a.app != r.app) {
                out.push(LinkApp { app: r.app, app_version: r.app_version });
            }
        }
        out.sort_by(|a, b| a.app.cmp(&b.app));
        Ok(out)
    }

    /// Send `method` to the newest instance of `app` and wait for the reply.
    pub fn call(&self, app: &str, method: &str, params: &Value, cancel: &AtomicBool) -> Result<Value, LinkFailure> {
        if cancel.load(Ordering::Relaxed) {
            return Err(LinkFailure::Cancelled);
        }
        let record = self.peers()?.into_iter().find(|r| r.app == app).ok_or_else(|| LinkFailure::NotRunning(format!("{app} is not running")))?;
        let identity = self.identity.clone();
        let (method, params) = (method.to_owned(), params.clone());
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name(format!("link-call-{app}"))
            .spawn(move || {
                let reply = Client::connect(&record).and_then(|mut client| {
                    client.hello(&identity)?;
                    client.call(&method, params)
                });
                // The receiver is gone when the caller cancelled. Nothing is left to tell.
                let _ = tx.send(reply);
            })
            .map_err(|e| LinkFailure::Failed(format!("could not start the link thread: {e}")))?;
        loop {
            match rx.recv_timeout(CANCEL_POLL) {
                Ok(reply) => return reply.map_err(|e| failed(&e)),
                Err(RecvTimeoutError::Timeout) => {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(LinkFailure::Cancelled);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return Err(LinkFailure::Failed("the link thread ended without a reply".into())),
            }
        }
    }
}
