//! Join a document on a live relay, as a native peer.
//!
//! The smallest thing that exercises the whole path: `NativeHub` over
//! `rust_socketio`, the relay verifying an HMAC the PHP app produced, the
//! sequence coming from Valkey, and the room fanning an edit out.
//!
//! ```sh
//! cargo run -p craft-collab --features native-hub --example join -- \
//!     --relay http://relay.example.com:9444 --token "$TOKEN" --display Alice --submit
//! ```
//!
//! Run two at once with different tokens to watch one peer's edit arrive at the
//! other. The token is read from `--token` or `CRAFTCOLLAB_TOKEN`, so it need
//! not appear in a shell history.

use std::time::{Duration, Instant};

use craft_collab::native::NativeHub;
use craft_collab::{Effect, Hub, Session};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut relay = "http://127.0.0.1:9444".to_owned();
    let mut token = std::env::var("CRAFTCOLLAB_TOKEN").unwrap_or_default();
    let mut display = "peer".to_owned();
    let mut submit = false;
    let mut lock = false;
    let mut seconds = 10u64;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--relay" => relay = args.next().unwrap_or_default(),
            "--token" => token = args.next().unwrap_or_default(),
            "--display" => display = args.next().unwrap_or_default(),
            "--seconds" => seconds = args.next().and_then(|s| s.parse().ok()).unwrap_or(10),
            "--submit" => submit = true,
            "--lock" => lock = true,
            other => return Err(format!("unknown flag: {other}").into()),
        }
    }
    if token.is_empty() {
        return Err("no token. Pass --token or set CRAFTCOLLAB_TOKEN.".into());
    }

    // The document is whatever the token says, so the session's id is cosmetic
    // until the relay answers Joined.
    let doc = "pending";
    let mut session = Session::new(doc);

    println!("connecting to {relay}");
    // No `join` is sent here. The relay joins a peer into its document from the claims
    // carried in the connection handshake, not from a client event, and registers no
    // handler for "join" at all. Sending it anyway used to race the server: socketioxide
    // completes a namespace CONNECT by spawning a task rather than finishing it inline, so
    // a client that writes its very next packet immediately could arrive before that task
    // had registered the socket. The server read that as a protocol violation, closed the
    // transport, and rust_socketio's default reconnect silently opened a second session a
    // few tens of milliseconds later. That showed up as `on_connect` firing twice for one
    // client, with two different socket ids. Waiting for the relay's own "joined" message
    // before sending anything is enough to avoid the race, because by the time "joined" is
    // sent, the socket's registration has already completed on the server.
    let mut hub = NativeHub::connect(&relay, &token).await?;
    println!("connected as {display:?}, waiting for the relay to join {display:?} in");

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut acted = false;

    while Instant::now() < deadline {
        for msg in hub.poll() {
            // The first Joined tells us the real document id, so the session is
            // rebuilt around it before anything is ordered.
            if let craft_collab::ServerMsg::Joined { doc, peer, seq, roster, locks } = &msg {
                println!("joined {doc} as {peer} at seq {seq}");
                println!("  roster: {:?}", roster.iter().map(|p| &p.display).collect::<Vec<_>>());
                println!("  locks:  {locks:?}");
                session = Session::new(doc.clone());
            }

            for effect in session.handle(msg) {
                match effect {
                    Effect::Apply { command, params } => println!("APPLY   {command} {params}"),
                    Effect::Confirmed { local, seq } => println!("OK      local {local} ordered at seq {seq}"),
                    Effect::Replace { seq, .. } => println!("REPLACE from seq {seq}"),
                    Effect::LockHeld { scope } => println!("LOCK    held {scope}"),
                    Effect::LockRefused { scope, holder } => println!("LOCK    {scope} refused, {holder} has it"),
                    Effect::LockDropped { scope } => println!("LOCK    dropped {scope}"),
                    Effect::RemoteLocks { locks } => println!("LOCKS   {locks:?}"),
                    Effect::Roster { roster } => {
                        println!("ROSTER  {:?}", roster.iter().map(|p| &p.display).collect::<Vec<_>>())
                    }
                    Effect::Notice { detail } => println!("NOTICE  {detail}"),
                    Effect::Send { msg } => {
                        println!("RESYNC  asking the relay");
                        hub.send(msg)?;
                    }
                }
            }
        }

        // Act once, after the join has settled, so the output reads in order.
        if !acted && session.peer().is_some() {
            acted = true;
            if lock {
                println!("asking for a lock on B2");
                hub.send(session.request_lock("B2"))?;
            }
            if submit {
                let msg = session.submit("cell.set", serde_json::json!({ "cell": "B2", "input": "7" }), Some("B2".into()));
                println!("submitting cell.set B2=7");
                hub.send(msg)?;
            }
        }

        if !hub.connected() {
            println!("relay closed the connection");
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    println!("done. applied through seq {}, {} edits unconfirmed", session.applied(), session.inflight());
    Ok(())
}
