# Push status for WS-Print: events as a wake-up, polling as the truth

`docs/job-lifecycle.md` and `docs/job-tracking.md` describe how a job is followed by asking: one
`GetJobElements` per poll, a back-off while nothing changes, a ceiling and a few staleness rules. This
note describes the optional alternative the protocol offers, WS-Eventing, what it costs, and the
small, deliberately conservative way `craft-print` uses it. Everything here is behind the `wsd`
feature, is **off unless the app configures it**, and changes no result a poll would produce.

## 1. What WS-Print eventing gives

A WS-Print service is also a WS-Eventing event source (the 2004/08 submission as Windows-era devices
speak it). A client sends `Subscribe` to the print service and names where the printer should call
back; the printer answers with a subscription manager reference that carries an identifier.

**The six events**, each a one-way SOAP message whose `wsa:Action` is the event's name:

| Event | Sent when | Carries |
|---|---|---|
| `PrinterElementsChangeEvent` | description, configuration, default ticket or capabilities changed | the changed elements |
| `PrinterStatusSummaryEvent` | the printer's state or its state reasons changed | state, primary reason, reasons |
| `PrinterStatusConditionEvent` | a device condition (paper out, cover open) was raised | the condition |
| `PrinterStatusConditionClearedEvent` | a condition went away | its id and time |
| `JobStatusEvent` | a job's state, reasons or counters changed | job id, state, reasons, counters |
| `JobEndStateEvent` | a job reached an end state | job id, end state, reasons, name, user |

**Subscription parameters.**

* *Delivery:* push only, mode `.../DeliveryModes/Push`, with a `NotifyTo` endpoint reference. Any
  reference parameters in `NotifyTo` are echoed by the printer as SOAP headers on every event, which
  is the hook for a per-subscription identifier. An optional `EndTo` receives `SubscriptionEnd`.
* *Filter:* the DPWS action dialect, a list of event action URIs. There is **no standard filter by
  job**: a subscription is per printer and per event set, and which job an event is about is only
  known from its content.
* *Expires:* a duration or a date-time, a request. The printer answers with what it grants (it may
  grant less). A subscription that is not renewed (`Renew`) before its expiry silently ends. `GetStatus`
  returns the remaining lifetime, `Unsubscribe` ends it.
* *`SubscriptionEnd`:* the printer tells the `EndTo` endpoint that it ended a subscription (delivery
  failures, shutting down, cancelling). Printers that reboot usually do not send it: they just forget.
* *Event rate:* WS-Print divides events into **CompleteState** events (`PrinterStatusSummary`,
  `JobStatus`: each carries the whole state, so only the newest matters and the printer may merge and
  space them) and **cumulative** events (conditions, `JobEndState`: every one matters and none may be
  dropped). `SetEventRate` sets the minimum spacing of the CompleteState events (a printer's default
  is typically around 30 seconds). The consequence: a progress change can be tens of seconds late
  when it arrives as an event, while an end state is prompt. `craft-print` does not call `SetEventRate`
  (it changes a printer-wide setting other clients share); it uses the events as they come.
* *Resync:* events say that something changed. The authoritative state is always asked for:
  `GetJobElements` for a job (what `Backend::job_status` is) and `GetPrinterElements` for the printer.

## 2. What it costs

Push means the **client listens**. That is the whole cost, and it is not small:

* **An inbound HTTP listener** on the client, for as long as anything is subscribed.
* **Reachability.** The printer must be able to open a TCP connection to the client. A host firewall
  that blocks inbound connections, a client on another subnet or VLAN, a NAT between them or a VPN
  all break delivery without any error on our side: the `Subscribe` succeeds, nothing ever arrives.
  A desktop app behind the default Windows firewall will usually trigger a prompt or be blocked.
* **Security.** Anyone who can reach the listener can send it a message that looks like an event. A
  listener on a LAN is an unauthenticated write surface. The rules in section 4 exist because of
  this, and the central one is: **event content is never trusted for anything but "go and look".**
* **Lifecycle.** Leases must be renewed, ended subscriptions noticed, and subscriptions removed when
  nobody needs them, or the printer carries a dead subscription until it expires (and some printers
  have very few slots).
* **Printer variance.** Real printers differ in which filters they accept, how long a lease they
  grant, whether they echo reference parameters, whether they ever send `SubscriptionEnd`. This
  version is verified against the fake printer and against `fakeprint` (a standards-faithful
  simulator), not against real devices.

## 3. The design: a wake-up, never a source of truth

The recommended hybrid, and the only mode implemented:

1. **Events are a wake-up and a latency optimisation.** A verified event only means "poll this job
   now". The state, progress and end of a job come from `Backend::job_status` (`GetJobElements`),
   exactly as when polling. The payload of an event is parsed for one thing: the job number of a job
   event, to decide *whose* watch to wake. It is never turned into a status.
2. **So a bad event cannot produce a wrong result.** A lost event costs latency, not correctness: the
   safety poll catches it. A duplicated event costs one extra poll (and is de-duplicated and
   rate-limited). A forged or garbled event costs at most one extra poll, and most are dropped before
   that. An event can never complete, fail, cancel or stall a job; only a successful poll can.
3. **Polling stays.** While a subscription is *healthy* the watch polls only as a **safety poll**
   (default 30 seconds, `WaitPolicy::safety_interval`) plus once per wake. When it is not healthy the
   watch polls at the ordinary fast cadence (2 s growing to 15 s).
4. **A subscription earns its trust.** It is *healthy* only if it was established, has not expired
   on our clock, has not been ended, and **a verified event arrived within the quiet window** (default
   2 minutes). Right after `Subscribe` nothing has arrived yet, so the watch keeps polling fast until
   the first event proves the path works (this is what makes a firewall-blocked listener harmless: the
   watch never leaves the fast cadence). After a long silence the trust is withdrawn again (a printer
   that rebooted and forgot us looks exactly like a quiet job), and the next event restores it.
5. **Fall back to fast polling** when: `Subscribe` fails (retried with back-off, 1 minute doubling to
   10 minutes), a `Renew` fails and the lease runs out, a `SubscriptionEnd` arrives, the lease expires,
   or no event arrives within the quiet window.

### Staleness

`Staleness` (job-tracking.md section 9) keeps its meaning. A **good read** is a successful
authoritative status read and nothing else; it alone resets the stall clock and ends `NoNews`. A
verified event counts as *news from the printer* for the silence rule only, and only for a bounded
time: the silence clock is measured from the later of the last good read and the last event, but the
event can postpone `NeedsAttention(NoNews)` at most to twice `silence_after` after the last good read.
A printer that sends events but whose status reads keep failing is therefore still flagged. Events
never reset the stall rule (that needs a changed reading) and never affect the ceiling.

## 4. The listener: exposure rules

* **Off unless configured.** `WsdBackend::new()` listens to nothing. `WsdBackend::with_events(NotifyConfig)`
  is the only thing that opens a socket, it is not a default and not a feature switch that flips
  behaviour for existing users.
* **Explicit bind, explicit advertise.** `NotifyConfig { bind: SocketAddr, advertise: PrinterUrl }`:
  where to listen and which URL the printers are told. There is no default for either and no
  "listen on every interface" convenience. The rules, checked before any socket is opened (by
  `NotifyConfig::new` / `parse`, and again by `WsdBackend::with_events` / `Events::start` because the
  fields are public), each refused with `ErrorKind::InvalidRequest`:

  | Address | Rule |
  |---|---|
  | `bind` is a wildcard: `0.0.0.0`, `::`, or the v4-mapped `::ffff:0.0.0.0` | **refused**. The app names the one local interface address a printer reaches it on |
  | `bind` is a concrete address that is not loopback, link-local or private (public, multicast, broadcast, documentation ranges) | refused (the numeric local-address policy of a printer URL) |
  | `bind` is a loopback, link-local or private address (a v4-mapped form is judged as its IPv4 address) | accepted |
  | `advertise` breaks the same policy (host name, public address, `https://`, ...) | refused |
  | `advertise` host equals the `bind` address | accepted (the default shape; the port follows a port-0 bind) |
  | `advertise` host differs from `bind` and is a **private** address (`10/8`, `172.16/12`, `192.168/16`, `fc00::/7`) | accepted: the app's explicit NAT or port-forward address |
  | `advertise` host differs from `bind` and is loopback or link-local | refused: it cannot be the listener |

  A wildcard bind used to be allowed "because the app said so"; it is no longer, so a listener can never
  be reachable on an interface the app did not name (a VPN adapter, a public interface), whatever the
  source rules below do. `NotifyConfig::loopback()` binds `127.0.0.1:0` for tests and local simulators.
* **One listener per backend**, shared by all its subscriptions (clones of a backend share it).
  It is the only background thread this crate starts, and it lives from `with_events` until the last
  clone of the backend is dropped or `shutdown_events()` is called (both unsubscribe, stop and join).
  There is no async runtime, and no other thread: connections are handled one at a time on the
  listener thread with short deadlines. Because one connection is served at a time, a long
  `EventTuning::connection_deadline` lets one stalled client block every event for that long; the
  default stays 3 seconds.
* **A request is accepted only if all of this holds**, checked in this order, cheapest first:
  1. the **source address** is the host of a printer we hold a subscription with (anything else is
     closed without reading a byte);
  2. a global **rate limit** (a token bucket) has room;
  3. it is a well-formed `POST` to `/n/<token>` or `/e/<token>` over HTTP/1.1 with a `Content-Length`
     within the cap (64 KiB), a SOAP content type and a header block within its caps, read within a
     total deadline (3 seconds by default, never more than 5 minutes: a longer
     `EventTuning::connection_deadline`, `Duration::MAX` included, is cut to
     `MAX_CONNECTION_DEADLINE`). No chunked bodies, no pipelining, `Connection: close`;
  4. `<token>` is the **unguessable per-subscription identifier** (two random UUIDs, 64 hex digits, from std's randomly keyed hasher), compared in
     constant time against every subscription (no early exit), **and the source address equals that
     subscription's printer**;
  5. the envelope parses (bounded XML, no DTD), carries the subscription's identifier as a
     `wse:Identifier` header (the reference parameter we put in `NotifyTo` and `EndTo`), and the
     identifier matches the token in constant time;
  6. the action is one of the types **we subscribed to**, and the body's first element is that event's
     element in the WS-Print namespace (a wrong namespace or an action/body mismatch is dropped);
  7. a message id seen recently on this subscription is a duplicate and is acknowledged but not acted on.
  Failures answer a bare status with no detail: 400 (malformed), 403 (a wrong identifier), 404 (an
  unknown path or subscription), 405 (not `POST`), 408 (the connection deadline passed), 411 (no
  length), 413 (a body over the cap), 415 (not a SOAP content type), 429 (over the rate limit) or
  431 (the header block is too large). They are counted (`EventStats`) for diagnostics. Valid events
  are answered `202 Accepted`.
* **Bounded everything:** one connection at a time; 8 KiB of headers; 64 KiB of body; a 3 second
  total deadline per connection (configurable, capped at 5 minutes, so a silent peer cannot hold the listener or block `shutdown_events()` indefinitely); a token bucket (default burst 50, 20 per second); at most 16
  subscriptions; 256 remembered message ids per subscription; wakes coalesce into one counter per
  watch, so a flood cannot grow memory.
* **Scope of a subscription.** One per printer, shared by all the jobs watched on that printer.
  Event set: `JobStatus`, `JobEndState`, `PrinterStatusSummary`, `PrinterStatusCondition` and
  `PrinterStatusConditionCleared` (not `PrinterElementsChange`: capabilities are not job news). There is
  no job filter dialect, so job events for jobs of other users on the same printer arrive too; they are
  dropped after reading the job number and wake nobody. Printer-level events wake every watch on that
  printer (a stopped printer is news for all its jobs).
* **Cleanup.** A subscription is released when the last watch that used it ends (the watch's
  `WakeReceiver` is dropped); the `Unsubscribe` is sent by the next housekeeping call, by the backend's
  shutdown or by its drop. A subscription nobody renews expires on the printer anyway: we ask for 10
  minutes, not hours.
* **Renewal at about 50% of the granted lease**, measured on the backend's clock.

### Who drives the housekeeping

The listener only receives. Everything that sends a request (`Subscribe`, `Renew`, `Unsubscribe`) is
**caller-driven**: `WsdBackend::maintain_events()` does whatever is due, one bounded request per
subscription per call, and returns at once when nothing is due or another call is already doing it.
`Backend::job_status` calls it first, so a watched job keeps its subscription alive at the safety-poll
cadence with no extra timer. That means a status poll can spend one bounded request per subscription on
housekeeping before its own `GetJobElements`: poll from a worker thread, not a UI thread, when events are
on. Without `with_events` nothing extra is sent. An app that wants tighter control can call
`maintain_events()` from its own loop as well. The clock is injectable (`with_clock`), so lease,
renewal, back-off and quiet-window behaviour are tested without sleeping.

## 5. The code

* `crate::WakeReceiver` / `WakeSender` (neutral, no dependencies): a bounded, coalescing "something
  happened" signal plus a health flag. `Backend::wake_source(&JobId)` (defaulted to `None`) hands a
  receiver to a watch; `Registry::job_watch` and `PrintJob` attach it.
* `JobWatch` takes the receiver (`with_wake`): a pending wake makes the next poll due now (at most one
  wake poll per second), a healthy receiver stretches the idle cadence to `safety_interval`, and an
  unhealthy one leaves the watch exactly as it was without events.
* `craft_print::wsd::events` (`NotifyConfig`, `Events`, `Wake`, `EventStats`): the listener, the
  subscription manager and the mapping from verified events to wakes. The `craft-wsd` types it uses
  are reached through `wsd/adapter.rs` only.
* `WsdBackend::with_events`, `maintain_events`, `shutdown_events`, `events()`, `events_status()`
  (plain data: where it listens, per-printer `subscribed` / `healthy` / watchers / seconds since the last
  event, and the counters); `watch_support()` is `Events` while at least one subscription is healthy,
  otherwise `Poll`. The configuration (`NotifyConfig::new` / `parse`), the status and the counters are
  plain owned data, with explicit start (`with_events`) and stop (`shutdown_events`), so a language
  binding can wrap them.

## 6. Verification

* In this repository, without any printer: the listener against static hostile byte vectors sent by a
  test client over a loopback socket (forged token, wrong identifier, malformed and oversized
  messages, wrong namespace, unsubscribed types, duplicates, floods, a stalled sender), the
  subscription manager against canned `craft-wsd` WS-Eventing replies from a scripted transport,
  lease, renewal, expiry, back-off and `SubscriptionEnd` flows on the injected clock, `JobWatch` and
  `PrintJob` on the trait-level fake backend, and listener shutdown.
* The fake printer is `fakeprint`. Eventing against it (the six events delivered to this listener,
  a lost event caught by the safety poll, a forged event ignored, `SubscriptionEnd`) is verified through
  the Python bindings in a separate harness, outside this repository.

## 7. Not done

* No `SetEventRate` call, no `GetStatus` health probe beyond what `Renew` tells us, no IPv6 scope
  handling, no HTTPS (the listener and the printer URLs are plain HTTP on a local network, like the
  rest of the backend), no chunked request bodies, no discovery of which of the host's addresses a
  printer can reach (the app says which address to advertise).
* Not verified against real printers or against a real firewall or NAT. Where those break delivery
  the effect is the ordinary polling behaviour, by construction of section 3.
