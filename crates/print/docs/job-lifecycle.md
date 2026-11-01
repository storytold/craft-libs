# Job lifecycle: submit, watch, cancel, retry

How a print job is followed after a printer has accepted it, why `Backend::submit` does not
wait for the paper, and how errors say whether sending again is safe. The README has the short
version ("Job lifecycle"); this is the reasoning.

## 1. Survey: how comparable stacks do it

Notes written from the specifications and the behaviour of the stacks as we understand them; none
of this needs a particular printer.

**CUPS (`lp`, `lpstat`, IPP).** `lp` returns as soon as the scheduler has queued the job and prints
`request id is <queue>-<n>`; it never waits for output. What happens next is a separate question you
ask later: `lpstat -o` / `lpstat -W not-completed|completed` list jobs, `cancel` / `lprm` remove one.
Underneath, IPP draws the same line. `Print-Job` (or `Create-Job` + `Send-Document`) answers when the
document has been *received*, with `job-id`, `job-state` and `job-state-reasons` in the reply.
Progress is `Get-Job-Attributes` (poll) or `Get-Jobs`; `Cancel-Job` stops a job. The job states are
pending, pending-held, processing, processing-stopped, canceled, aborted and completed, and the
terminal ones are the last three. Completed jobs fall out of the "not completed" list and are kept
in history only as long as the scheduler's `PreserveJobHistory` allows: a poller must treat "unknown
job" after a while as "finished or forgotten".

**IPP event notifications (RFC 3995, 3996).** `Create-Job-Subscription` /
`Create-Printer-Subscription` register interest in events such as `job-state-changed`; the `ippget`
delivery method is a **long poll**: `Get-Notifications` with a sequence number returns the events
since then, optionally holding the request open (`notify-wait`) until one arrives, with a
subscription lease that must be renewed. It is the right tool for a long-lived client; it needs a
second kind of request, lease bookkeeping and a transport that tolerates a held request.

**IPP Everywhere (PWG 5100.14).** Requires `Print-Job`, `Validate-Job`, `Cancel-Job`,
`Get-Job-Attributes`, `Get-Jobs` and `Get-Printer-Attributes`, and the PWG job counters
(`job-media-sheets-completed`, `job-impressions-completed`). Event notifications are optional. So
the baseline every conforming printer offers is: accept, then poll one job. That is our baseline too.

**WS-Print (WSD).** `CreatePrintJob` then `SendDocument` (the reply means the document was accepted),
then polling: `GetJobElements` for one job (state, state reasons, counters), `GetActiveJobs` and
`GetJobHistory` for lists, `CancelJob`. A finished job is dropped from the printer's tables quickly,
and asking for it then is a `ClientErrorJobIdNotFound` fault: "forgotten" again. The push option is
WS-Eventing (`Subscribe` with a callback endpoint, renewed before it expires) with six events:
`PrinterElementsChangeEvent`, `PrinterStatusSummaryEvent`, `PrinterStatusConditionEvent`,
`PrinterStatusConditionClearedEvent`, `JobStatusEvent` and `JobEndStateEvent`. Push needs an inbound
listener on the client, which an app on a desktop behind a firewall often cannot offer.

**Windows spooler.** `StartDocPrinter`/`EndDocPrinter` hands the job to the spooler and returns a job
id; afterwards `GetJob` / `EnumJobs` give `JOB_INFO` with a status bit set (spooling, printing,
paused, error, offline, paper-out, user-intervention, deleting, printed, complete, retained) and
`PagesPrinted` / `TotalPages`; `SetJob` cancels. `FindFirstPrinterChangeNotification` gives a wait
handle that is signalled on a change (a push, in-process). Completed jobs vanish from the queue unless
the printer retains them. Polling `JOB_INFO` is what most tools do; the notification handle is the
"events" variant.

**macOS.** The print system is CUPS; the app-level sessions (`PMPrintSession`) report errors after the
fact and the system print queue UI follows the job. For our purposes it is the CUPS case.

**Precedents in this repository.**

* `craft-scan`'s `acquire` is one blocking call. Progress is a `ProgressToken` of atomics the UI reads
  every frame (pages done, percent of the page in flight); cancel is a `CancelToken` that travels in the
  settings and is polled at checkpoints. Nothing blocks the UI thread because the app calls it from a
  worker, and the *UI* polls the shared state. Scanning is one device transaction; a print job is two
  (hand over, then wait for the machine), which is why submit and watch can be split here.
* `craft-ui::printing` shows what the apps already do for the spooler: a worker thread, an `mpsc`
  receiver polled with `try_recv` once per frame, `request_repaint_after` while work is pending, and a
  `Feedback` enum that the app words. `craft-print`'s own `submit_with_progress` bridges native worker
  events to a callback through a bounded `sync_channel(1)` so a slow callback cannot pile events up.
* `craft-ui::printing::network` already uses `mpsc` receivers for its query and job workers, plus a
  cancel closure.
* `craft-settings` and `craft-atomic` have no waiting or status concept that applies; nothing to reuse.

**What the comparison says.**

1. Every stack separates *accepted* from *printed*. The accept call is short and bounded; the job id it
   returns is the handle for everything after.
2. The portable baseline for "what happened to it" is polling one job. Push (IPP notifications,
   WS-Eventing, the Windows change handle) is an optimisation with real costs (leases, listeners, held
   requests) and is optional on the printer.
3. Finished jobs are forgotten; "unknown job" is a state to model, not an error to surface.
4. Some paths (`lp`, `StartDocPrinter` without keeping the id) give no handle at all. "Accepted, outcome
   unknown" is a legitimate final answer and a UI must be able to show it.
5. A sleeping loop inside the accept call is the one thing nobody does. It ties up a worker thread,
   hides cancel behind it and makes "did it print?" depend on a timer.

## 2. Design

* `Backend::submit` returns `Submitted` when the printer has accepted the document. It is bounded by
  the backend's per-request timeouts and never waits for output.
* `Backend::job_status(&JobId) -> JobStatus` is **one request**. `JobState` is Pending, Held,
  Processing, NeedsAttention (a stopped printer), Completed, CompletedWithErrors, Cancelled, Aborted or
  Unknown (a completed job the printer qualified is `Completed` with `JobStatus::warnings`); `Unknown` means "accepted, outcome unknown" and is not terminal. `Backend::cancel_job`
  cancels (and "too late" is not an error).
* `Backend::watch_support()` is `None` (cannot report: do not poll), `Poll` or `Events`
  (`recommended_poll_interval()` is the cadence). `Events` is opt-in per backend and only a wake-up for
  the poll (`Backend::wake_source`, `JobWatch::with_wake`); see `docs/events.md`.
* The registry routes jobs by the `JobId`'s backend exactly as it routes printers
  (`submit_job`, `job_status`, `cancel_job`, `watch_support`, `job_watch`).
* `JobWatch` is a caller-driven state machine: no thread, no timer. `tick(source)` makes at most one
  request and only when one is due, otherwise says when to come back; every wait is measured on an
  injectable `Clock`. It backs off while the status does not change, retries failed polls under a
  `RetryPolicy`, sends one cancel when its `CancelToken` is set, and ends in a `WatchOutcome`
  (Terminal, Forgotten, NoStatus, TimedOut, Failed). Tick it on a worker thread, not in a UI frame: a network backend blocks for up to its per-request timeout, and the UI reads the latest status the worker saw.
* `wait_for_job(registry, job, WaitPolicy)` is the blocking convenience for a CLI, built on `tick`.
  `max_wait` is finite (two minutes by default); when it runs out the job is untouched.

### Why not the alternatives

* *Blocking `submit` with a long timeout* (the first WS-Print slice): a worker is pinned for minutes,
  cancel has to be threaded through a sleep, and "the job is still running" is reported as a success.
* *A callback or a channel inside `submit`*: still blocks the call, and makes every backend own a
  thread story. A caller-driven state machine lets the caller choose (UI frame, worker loop, CLI).
* *Async runtime*: the crate has none and the apps do not need one for one request every few seconds.
* *Push first*: needs a listener and lease renewal; not available behind most desktop firewalls; and
  the printer may not offer it. `WatchSupport::Events` leaves room; the WS-Print backend now offers it
  as an opt-in wake-up on top of polling (`docs/events.md`), never instead of it.

## 3. Retry semantics

Two separate questions.

* `PrintError::is_retryable()`: is the condition transient (busy, unreachable, timed out)?
* `PrintError::retry_safety()`: would sending the same request again be safe?
  * `Safe`: nothing was sent or accepted (refused before connect, policy, a connect failure, a 429 or
    503 before the printer processed anything).
  * `OutcomeUnknown`: the request may have been delivered (the connection closed or reset after the
    request was written, a timeout while waiting for the reply to a create or send). Retrying a submit
    could print twice. The error is transient *and* unsafe; `craft_job::advise` answers `CheckDeviceFirst` for it (also when the attempts are used up), so nothing repeats it by itself,
    and a backend that can offers a *reconcile* step (look for the job before resubmitting).
  * `Idempotent`: a query or a cancel; repeating is harmless.
* `PrintError::phase()`: `Prepare` (before any traffic), `Connect`, `Request`, `Reply`, `Poll`.
* `PrintError::retry_after()`: the wait the device or backend suggests; `craft_job::advise`
  honours it (up to `max_retry_after`) on top of an exponential, jittered, capped backoff.

Neutral defaults when a backend does not say (`ErrorKind::default_retry_safety`):

| `ErrorKind`        | retryable | default safety   |
|--------------------|-----------|------------------|
| `Unsupported`      | no        | Safe             |
| `UnknownBackend`   | no        | Safe             |
| `NotFound`         | no        | Safe             |
| `InvalidRequest`   | no        | Safe             |
| `UnsupportedFormat`| no        | Safe             |
| `ConsentRequired`  | no        | Safe             |
| `Cancelled`        | no        | Safe             |
| `Unavailable`      | yes       | Safe             |
| `Busy`             | yes       | Safe             |
| `Timeout`          | yes       | OutcomeUnknown   |
| `Rejected`         | no        | Safe             |
| `Io`               | no        | Safe             |
| `Failed`           | no        | OutcomeUnknown   |

A backend overrides the default with `with_safety` where it knows better (a query's timeout is
`Idempotent`; a connection that closed after the request was written is `OutcomeUnknown`).

## 4. WS-Print: what each failure means

The WS-Print backend classifies a failure by what failed (`WsdError`) and by which operation was in
flight (`Op`): a query (`GetPrinterElements`, `GetActiveJobs`, `GetJobHistory`), a status read
(`GetJobElements` while following a job) and a cancel may be repeated; a create and a send may not.
The principle: **a failure that proves the printer did not act is `Safe`; a failure after the request
may have arrived is `OutcomeUnknown` for a create or a send and `Idempotent` for the rest.** The table
is pinned by `the_retry_table_for_every_kind` in `src/wsd/error.rs`.

| Failure | `ErrorKind` | Retryable | Phase | Safety (create or send) | Safety (query, status, cancel) | `retry_after` |
|---|---|---|---|---|---|---|
| Address or setting refused (policy, invalid, unsupported) | `InvalidRequest` | no | Prepare | Safe | Safe | none |
| No common document format | `UnsupportedFormat` | no | Prepare | Safe | Safe | none |
| Document could not be produced | `Failed` | no | Prepare | Safe | Safe | none |
| Caller cancelled | `Cancelled` | no | Prepare | Safe | Safe | none |
| Our request over the size cap | `InvalidRequest` | no | Prepare | Safe | Safe | none |
| Transport `refused` or `unreachable` (stage connect) | `Unavailable` | yes | Connect | Safe | Safe | none |
| Transport `connect_timeout` (stage connect) | `Timeout` | yes | Connect | Safe | Safe | none |
| Transport `connection_closed` (stage reply; request, if it was being written) | `Unavailable` | yes | Reply (Request) | OutcomeUnknown | Idempotent | none |
| Transport `timeout` (stage reply) | `Timeout` | yes | Reply | OutcomeUnknown | Idempotent | none |
| The caller's deadline passed before the request was sent | `Timeout` | yes | Prepare | Safe | Safe | none |
| Transport `invalid_response` (a peer that did not speak HTTP) | `Failed` | no | Reply | OutcomeUnknown | Idempotent | none |
| Transport `io` (an I/O error that fits no other kind) | `Unavailable` | yes | Reply | OutcomeUnknown | Idempotent | none |
| Transport `redirect` (refused) | `Failed` | no | Reply | OutcomeUnknown | Idempotent | none |
| HTTP 408 | `Timeout` | yes | Request | Safe | Safe | none |
| HTTP 425 | `Busy` | yes | Request | Safe | Safe | 2 s, or the printer's |
| HTTP 429 | `Busy` | yes | Request | Safe | Safe | 5 s, or the printer's |
| HTTP 503 | `Unavailable` | yes | Request | Safe | Safe | 5 s, or the printer's |
| HTTP 500, 502, 504 and other 5xx | `Unavailable` | yes | Reply | OutcomeUnknown | Idempotent | the printer's, if sent |
| HTTP 501, 505 | `Failed` | no | Reply | Safe | Safe | none |
| HTTP 3xx | `Failed` | no | Reply | OutcomeUnknown | Idempotent | none |
| HTTP 4xx other than 408, 425, 429 | `Failed` | no | Reply | Safe | Safe | none |
| Unreadable or unexpected reply, reply over the cap | `Failed` | no | Reply | OutcomeUnknown | Idempotent | none |
| Fault: not accepting jobs | `Busy` | yes | Reply | Safe | Safe | 15 s, or the printer's |
| Fault: temporary error | `Busy` | yes | Reply | Safe | Safe | 5 s, or the printer's |
| Fault: document active | `Busy` | yes | Reply | Safe | Safe | 2 s, or the printer's |
| Fault: format or compression not supported | `UnsupportedFormat` | no | Reply | Safe | Safe | none |
| Fault: job id not found | `NotFound` | no | Reply | Safe | Safe | none |
| Fault: job cancelled | `Cancelled` | no | Reply | Safe | Safe | none |
| Fault: invalid arguments and the other client errors | `InvalidRequest` | no | Reply | Safe | Safe | none |
| Fault: internal error | `Failed` | no | Reply | Safe | Safe | none |
| Fault: last document already sent | `Failed` | no | Reply | OutcomeUnknown | Idempotent | none |
| Fault: operation failed, action not supported, vendor codes | `Rejected` | no | Reply | Safe | Safe | none |

For a status read the phase of every failure that reached the wire is `Poll`. A fault is a definite
answer (the printer said no), which is why it is `Safe` even for an internal error; only "the last
document was already sent" says an earlier send did arrive.

When the printer sends an HTTP `Retry-After` (on a status error or on the reply that carried a
fault), `craft-wsd` parses and caps it and the backend uses it as `retry_after`, for the transient
kinds only. Without one the values above are defaults, not measurements: a printer that is not taking
jobs is in a mode that lasts (15 s), a temporary error clears sooner (5 s), a document still being
processed sooner still (2 s). `RetryPolicy` takes the larger of this and its own backoff.

Nothing is read from text. `craft-wsd` reports a transport failure as a typed kind (`refused`,
`unreachable`, `connect_timeout`, `timeout`, `redirect`, `connection_closed`, `invalid_response`, `io`) and the stage
the exchange reached (prepare, connect, request, reply). The stage gives the phase and, through
`craft-wsd`'s own `retry_safety(idempotent)`, the safety: nothing sent (prepare, connect) is `Safe`,
anything that may have arrived is `OutcomeUnknown` for a create or send and `Idempotent` for the
rest. HTTP 429 and 503 are a decline and `Safe` there too. What this crate adds is the print
service's own reading: 408 and 425 are declines, 501, 505 and the other 4xx are definite refusals, a
SOAP fault is a definite answer. A request the HTTP client could not build is not a transport failure
at all: `craft-wsd` reports it as `Invalid`, which is a preparation failure and `Safe`. A closed connection is a transport kind, not an error kind of
its own. `every_transport_kind_for_every_operation` pins every kind against every operation.

**Reconcile.** `WsdBackend::reconcile(printer, title)` is the answer to `OutcomeUnknown`: read
`GetActiveJobs` and `GetJobHistory` and look for jobs with that title from this backend's user name
(`Found`, `Ambiguous`, `NotFound`). WS-Print job summaries carry no creation time, so "created in the
window" is whatever the printer still lists. After a failed send with an unknown outcome the backend
deliberately does **not** cancel the job it created: cancelling could abort a job that is printing.

**One level up.** Following a remote job, retrying safely and reconciling are combined into one
object per user-visible print, `PrintJob`, with an idempotency tag that makes the reconcile exact where
the backend can carry one; see [job-tracking.md](job-tracking.md). `Registry::find_job` is the
backend-neutral form of `WsdBackend::reconcile`.
