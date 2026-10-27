# `ScanJob`: one scan, stable across attempts

`Registry::acquire` is one blocking call that returns pages or an error. A person, though, presses Scan
once and expects one document: twenty sheets from the feeder, however many times the paper jams, the feeder
runs dry, or the link drops on the way. `ScanJob` (`craft_scan::job`) is the record of that single
acquisition. It is the scanning counterpart of the print side's `PrintJob`, kept as a separate, plain object:
printing is push-based (submit, then watch a remote job), scanning is pull-based (pages arrive while the call
runs and must survive its failure).

Everything is in memory and caller-driven. There is no thread, no timer, no clock (every call that changes
the job takes a `craft_job::Timestamp`), no async, and no I/O: **a `ScanJob` does not run `acquire`**. The app
does, on its own worker thread. The job plans the attempt, records what comes back, and says what may happen
next. It builds a `craft_job::JobView`, the same record a UI shows for a print job.

## 1. The loop

```text
let mut job = ScanJob::new(settings, ScanJobConfig::default().with_scanner(&details));

loop {
    if spill_between_attempts { spill(job.take_pages()?); written = 0; }   // optional: take_pages() empties job.pages(), so the cursor starts again
    let plan = job.begin_attempt(now())?;               // or begin_attempt_with(now(), Continuation::Append)
    if plan.kept_pages == 0 {                           // nothing kept: a first attempt, or a start over
        remove_page_files(); written = 0;               // Restart emptied job.pages(): the cursor starts again too
    }
    let worker = spawn(move || craft_scan::acquire(&plan.settings));   // plan.settings is Send
    while !worker.is_finished() {
        let new = job.record_progress(&plan.progress);  // moves arrived pages into the job; returns how many
        write(&job.pages()[written..]); written = job.pages().len(); // optional: the bytes are borrowed while it runs
        publish(job.snapshot());                        // cheap: no page bytes
        if user_pressed_stop { job.cancel(now()); }     // requests plan.cancel
        park_timeout(POLL);                             // 50-250 ms: a poll per UI refresh, not a busy loop
    }
    // A worker that panicked returned nothing: record it as a failed attempt, then hand over acquire's result.
    let result = worker.join().unwrap_or_else(|_| Err(ScanError::Failed("the scan worker stopped".into())));
    job.finish_attempt(result, now())?;
    write(&job.pages()[written..]); written = job.pages().len();   // pages the last poll never saw
    publish(job.snapshot());                            // the state finish_attempt reached: the last one subscribers see

    match job.retry_advice() {
        RetryAdvice::RetryNow => continue,                      // first attempt, glass fallback
        RetryAdvice::RetryAfter(wait) => { sleep(wait); continue }   // busy scanner, nothing kept
        _ => break,        // done, or a person decides (Interrupted / NeedsAttention), or CheckDeviceFirst
    }
}
// Interrupted / NeedsAttention: ask. Continue -> begin_attempt_with(.., Append). Start over -> Restart.
// Accept what is there -> finish(..). Stop -> cancel(..). Terminal: take_pages() and use them.
```

**Threads.** The job stays on the controlling thread (the GUI's UI thread, the CLI's main thread). The worker
gets only `plan.settings` (it is `Send`: settings plus the two tokens) and returns the `Result` of `acquire`.
The controlling thread polls `record_progress`, forwards a stop request (the GUI's button, the CLI's Ctrl-C
handler setting a flag the main loop turns into `job.cancel(now())`), and calls `finish_attempt` when the worker
joins. The loop waits between polls (an interval of a UI refresh, or the worker's own wake-up if the app has
one): without a wait it would spin a core and publish the same snapshot thousands of times while a scan runs without
new pages.

`finish_attempt` changes the state and can add pages the last poll never saw (the worker may end between two polls).
So after it a writer flushes `job.pages()[written..]` once more, and the loop publishes `job.snapshot()` once more:
the publishes inside the poll loop never carry the state `finish_attempt` reached. `written` indexes `job.pages()`,
which is empty again after a `Restart` (the plan then says `kept_pages == 0`): the writer replaces the old page files
and starts `written` at 0, or the next slice would index past the end. `take_pages()` empties `job.pages()` in the
same way, but the taken pages still count as kept (an `Append` plan says `kept_pages > 0`), so the `kept_pages == 0`
test does not catch it: an app that spills pages between attempts sets `written = 0` itself whenever it takes them.

The job itself is never shared across threads and needs no lock. A short-lived CLI run is exactly this loop:
one process, one job, each page written to the job's output folder once `record_progress` has counted it (read
from `job.pages()`; `take_pages()` refuses while the attempt runs, so a CLI that writes only at the end calls it
after `finish_attempt`), a small journal the app writes, and an exit code from `exit_class()` (section 11).

`begin_attempt_with` is only ever called on the person's say-so (or the app's, on their behalf). The job never
decides, on its own, to scan on after pages were kept.

## 2. States

The lifecycle is the job's own; it is separate from anything the scanner reports.

| State | Meaning | Leaves by |
|---|---|---|
| `Preparing` | created; no attempt has begun | `begin_attempt`, `cancel` |
| `Acquiring` | an attempt is running on the app's worker | `finish_attempt`, `device_cancelled`; `cancel` only requests the stop |
| `Interrupted` | the last attempt stopped for a **known cause** before the job was done. Pages are kept. `retry_advice()` says whether the app may go on by itself | `begin_attempt_with(Append \| Restart)`, `finish`, `cancel` |
| `NeedsAttention` | the last attempt ended and it is **not known** what the scanner did beyond what was reported. Nothing is resent | the same exits as `Interrupted` |
| `Terminal(Completed)` | every page that was wanted arrived (or the feeder emptied normally) | none |
| `Terminal(CompletedWithWarnings)` | fewer pages than wanted, accepted (`finish`) or delivered | none |
| `Terminal(Failed)` | stopped by a failure nothing in the job can cure | `begin_attempt` (a manual start over; history kept) |
| `Terminal(Cancelled)` | this side stopped it (`cancel`) | none |
| `Terminal(CancelledByDevice)` | the scanner ended it (`device_cancelled`) | none |

`JobView::state` is that, reduced to `craft_job::JobState`: `Preparing` is `Pending`; `Acquiring` is `Processing`;
`Interrupted` is `NeedsAttention` unless the advice is an automatic retry, then `Pending`; `NeedsAttention` is
`NeedsAttention`; `Completed` is `Completed`; `CompletedWithWarnings` is `CompletedWithErrors` (pages are
missing); `Failed` is `Aborted` (craft-job's word for "stopped by a failure"); `Cancelled` and `CancelledByDevice`
are both `Cancelled`, and the reason `cancelled_by_device` tells them apart.

## 3. Pages so far

"Pages so far" is first-class. The snapshot lists every kept page (number, the attempt that delivered it, source,
format, width, height, dpi, bytes) and the total, plus the expected count when known (`ScanSettings::pages`, not
for hardware duplex, where sides are not sheets; `None` means "the whole feeder"). It never holds a page's
bytes: `snapshot()` clones a few small fields and is meant to be called every frame. The bytes live in the job
until `take_pages()`; `pages()` lets a preview read them without taking.

Pages reach the job two ways, and are never counted twice. While the worker runs, the app calls
`record_progress(&plan.progress)` (or `absorb_pages(vec)` if it drained the token itself). When the worker
returns, `finish_attempt` takes whatever the token still holds, then merges the result: the result's pages
replace this attempt's pages when it has at least as many (backends return all pages, `Incomplete` carries them),
otherwise the pages already in hand are kept. A backend that fails with a plain error after pushing pages
through the token therefore loses nothing, as long as the token was the way the pages travelled.

`take_pages()` is refused while an attempt runs. A pages-taken job can still `Append`: taken pages count as kept.

## 4. What survives which failure

| Failure | Pages kept | Next state | Advice |
|---|---|---|---|
| jam, cover open, multi-feed, attention, device fault (any time) | yes | `Interrupted` | `DoNotRetry`: a person acts first |
| feeder empty, pages in hand, **whole feeder wanted** or wanted count reached | yes | `Completed` | none (this is how a stack ends, per `after_transfer`) |
| feeder empty, pages in hand, **count short** (mid-job reload) | yes | `Interrupted` | `DoNotRetry`; the person reloads and chooses `Append` |
| feeder empty, **no pages**, request was `Auto`, glass exists | n/a | `Interrupted`, attempt `FellBackToGlass` | `RetryNow`; next plan has `Source::Glass` |
| feeder empty, no pages, feeder requested | n/a | `Interrupted` | `DoNotRetry` |
| busy, **no pages** (the only automatic retry) | n/a | `Interrupted` while the budget lasts, then `Failed` | `RetryAfter`, then `DoNotRetry` |
| busy, pages kept | yes | `Interrupted` | `DoNotRetry` (paper position unknown) |
| link dropped (`Offline`), with or without pages | if any | `NeedsAttention` | `CheckDeviceFirst` |
| unclassified error (`Other`) **after pages, or mid-page** | yes | `NeedsAttention` | `CheckDeviceFirst` |
| unclassified error, no pages | n/a | `Failed` | `DoNotRetry` |
| the backend reports `InterruptKind::Cancelled` | yes | `Terminal(CancelledByDevice)` | `DoNotRetry` |
| validation failure, `Unsupported`, no source | n/a | `Failed` | `DoNotRetry` |
| `cancel()` | yes | `Terminal(Cancelled)` | `DoNotRetry` |
| `device_cancelled()` | yes | `Terminal(CancelledByDevice)` | `DoNotRetry` |

"Reading a failure" is three questions, answered from the existing `InterruptKind`, the page count and whether a
page was part-way across (`ProgressToken::in_page`):

* **transient** (`Busy` only): the scanner said it is busy, so it did nothing, and waiting may cure it.
* **a person can cure it** (`CoverOpen`, `Jam`, `PaperProblem`, `FeederEmpty`, `MultiFeed`, `Attention`,
  `DeviceError`, `BusyOrFeederEmpty`): the job waits in `Interrupted`.
* **unknown outcome** (`Offline` always, `Other` after pages or mid-page): the request may have reached the
  scanner, which may have moved the carriage or fed sheets, even with no page in hand. `RetrySafety::OutcomeUnknown`; the job waits in `NeedsAttention`.

The automatic part is exactly `craft_job::advise`, fed `retryable = transient && nothing kept` and the safety above,
under `ScanJobConfig::retry` (default `RetryPolicy::default()`; `RetryPolicy::none()` turns it off). Its attempt count
is the failures in a row that delivered no page, so a large ADF scan interrupted ten times does not use up the
budget. Only `Safe` failures are ever automatic; `Idempotent` is never produced because a second `acquire` is never
harmless.

## 5. Continuation

When pages are kept, the next attempt must say what to do with them. There is no default, because where the paper
stopped is not knowable: the sheet that jammed may or may not have been scanned, and a person may have taken the
stack out and put it back.

* **`Append`**: keep the pages, ask for what is missing (`expected - kept`; the whole feeder when the total is
  unknown). The person has put the remaining sheets in. `NothingLeft` if the wanted count is already reached
  (use `finish`).
* **`Restart`**: discard the kept pages (if any) and the retry count, and scan the whole job again. Pages already
  `take_pages()`d are the app's.
* With nothing kept, the two differ only in the retry count, which `Restart` resets; so does any start from
  `Failed`, which is always a person's. An automatic retry (`begin_attempt` from `Interrupted`) keeps it. With
  nothing kept, `begin_attempt` (no argument) is enough. With pages kept, `begin_attempt` returns
  `ContinuationRequired`.
* **`Resume`** (continue after the last good page without feeding again) does not exist. It would need a backend
  that can say it can; none can. `Continuation` is `#[non_exhaustive]` so it can be added.

## 6. `Auto`

The job leaves `Source::Auto` to the backend (it reads a fresh status at acquire time) and records what the
transfer actually used (`Acquired::source`, else `ProgressToken::source`). For the fallback rule it needs a guess
at what `Auto` resolved to: `Source::resolve(feeder_ready, has_feeder, has_glass)` over what the job knows
(`ScanJobConfig::with_scanner` or `with_capabilities`, refreshed by `observe_status`). Network scanner drivers
report no tray state, so `feeder_ready` is `None` there and the guess is the feeder, which is also what the drivers
do; an empty feeder is then learned from the failure. When `auto_fallback_to_glass` says so (Auto resolved to the
feeder, glass exists, **no page at all in hand**, the failure is `FeederEmpty` or `BusyOrFeederEmpty`), the attempt
is recorded as `FellBackToGlass`, the next plan asks for `Source::Glass` with one page, and the expected total
becomes one. A jam, a cover, a plain busy and any shortfall after pages stay errors, as the rule already says.

## 7. Cancel

`cancel(at)` is the client's stop. Between attempts the job ends `Cancelled` at once. While an attempt runs it
requests the attempt's `CancelToken` (the worker's transfer stops at the backend's next checkpoint) and the job
ends `Cancelled` when `finish_attempt` reports the return, whatever it was: a user who pressed Stop does not get a
"completed" job even if the last page raced in. Kept pages stay in the job.

`InterruptKind::Cancelled` is the scanner's: the Cancel key on the panel, or the device dropping the job. A backend
that can tell returns it in a `ScanError` (also inside `Incomplete`, with the pages) and `finish_attempt` ends the job
`Terminal(CancelledByDevice)`: a different end because the app must not say "you cancelled it" and a retry is not
implied. If this side asked to stop first, `Cancelled` wins. `device_cancelled(at)` stays for a backend that learns it
out of band (a status poll while the transfer is running). The kind was added to `InterruptKind`, which is now
`#[non_exhaustive]`; its wire string is `cancelled`, appended after `device_error`.

## 8. Idempotency, honestly

A printed document has a name the app can tag. A scan request does not: `ScanSettings` has no job-name field, WIA
and the inbox WSD scan driver have no job tag to set, SANE has no job list. eSCL has `ScanJobs` and WS-Scan has job
status, where a reconcile could look; **none of that is implemented here**.

So: the job carries a `Fingerprint` (title, creation time, a window) and `JobView::tag` exists, but
`ScanJobConfig::tag_support` is `TagSupport::None`, and so is `Backend::tag_support()`, a default method added so a
future eSCL or WS-Scan backend can say otherwise (`Registry::tag_support(id)` routes it). No tag is generated or sent.
When the outcome of an acquire is unknown the answer is `CheckDeviceFirst`: someone looks at the scanner.

Say it plainly: **a duplicate `acquire` is not free.** It moves the carriage and pulls paper through the
feeder. Sheets that were already scanned but whose pages were lost are ejected and cannot be scanned again by
repeating the call; they are only scanned again if a person puts them back. That is why an unknown outcome is never
retried by the job, and why `Append` is the person's decision.

## 9. `device_view`

`device_view(&Scanner, Option<HandlingStatus>, Option<Source>) -> craft_job::DeviceView` shows a scanner the same
way as a printer: capabilities, a `PaperPathView` (feeder loaded, capacity, glass, source in use) and one `Indicator`
per status bit.

* `feeder_ready` is an `Option<bool>` and becomes a `Tri` as given (`None` is `NotReported`).
* The other bits are plain `bool`s that mean "set only when reported". A status that is entirely clear and leaves
  the feeder unreported (`HandlingStatus::default()`) is what a driver that reports nothing looks like (the inbox
  WSD scan driver), so **every** indicator is `NotReported`, never `Off`. `None` (the backend cannot preflight) is
  the same. Once any bit says something, the others are read as given, except the **ready** bits (see next).
* The ready bits `flat_ready` and `duplex_ready` depend on the scanner's `Connection`. On `Usb` a clear ready bit is
  `Off`. On a network scanner (`Wsd`, `Other`) or `Unknown`, a clear ready bit is `NotReported` even when other bits are
  set, because those drivers publish no usable tray or ready state; a set bit is a positive reading (`On`) everywhere.
  `feeder_ready` does not depend on the connection: it is taken as given, and an explicit `Some(false)` stays `Off`.
  Fault bits (jam, covers, multi-feed, attention, lamp) are not ready bits and are read as given.
* Indicators for hardware the scanner lacks (no feeder, flatbed or duplexer) are left out unless the status asserts
  them. The lamp bit is an error flag: `LampError` is `On` or "no error reported".
* `From<Connection> for DeviceConnection`: `Usb` to `Usb`, `Wsd` to `Network`, `Other` to `Other`, `Unknown` to
  `Unknown`, as in craft-job's README. `DeviceConnection::Local` has no scan counterpart.

### The source actually used

`finish_attempt` records the source the backend reports in `Acquired` (never `Auto`) as `snapshot().source_in_use`
and `AttemptRecord::source_used`; for an error with no report it keeps the plan's guess. When the plan was the
feeder (asked for, or `Auto` that guessed it) and the backend reports the glass, the backend fell back inside the
call (an `Auto` whose feeder gave no pages): `AttemptRecord::fell_back` is `true` (serde: additive, defaults to
`false` in older journals) and `JobView::reasons` carries `AUTO_FELL_BACK_REASON` ("auto: used the glass because the
feeder gave no pages") while that attempt is the latest. This is not the job's own retry on the glass after
`AttemptOutcome::FellBackToGlass`, whose attempt asks for the glass.

## 10. Limits

* No persistence. A `ScanJob` lives in the process that made it; page bytes are held in memory (the app may
  `take_pages()` and spill to disk between attempts). Types are plain data; with the optional `serde` feature the snapshot and journal types also serialise
(section 12).
* No reconcile: see section 8.
* `Resume` and a device-side job handle are not there (sections 5, 8).
* The job id in `JobView` is `<backend>:local-<n>`, a process-unique number. There is no device job to name.
* Paper position is never inferred. Whether a jammed sheet was scanned is the person's call.
* `Offline` is always an unknown outcome, even with no page: the job cannot tell a refused connection from one that
  dropped after the scanner started, so it never retries it by itself. The price is one question to the person after
  a plain "scanner unreachable". Only `Busy` is retried automatically.

## 10a. Page length in the job

`ScanSettings::page_length` (`driver` by default, `auto`, `infer`) is part of the request, so every attempt's
`AttemptPlan::settings` carries it unchanged and a continuation asks for the same length rule. Each kept page's
`PageInfo::length` records how that page got its length (`source` `driver`, `device`, `size` or `inferred`), the
length in thousandths of an inch, and `approximate` (true only for `inferred`). The journal writes it under each page
(`"length":{"source":"inferred","length_mils":10730,"approximate":true}`); a journal from before the field reads as
`driver`. The job never decides a length itself; the backend does, see the README section "Page length". Honest
limits: inference was measured on one scanner, is opt-in, and is refused (the page stays as delivered, `driver`)
when the image gives no clear boundary.

## 11. Continuing in a later process, and exit codes

A CLI run is short-lived, so it cannot keep a `ScanJob` between `scan` and `scan --continue DIR`. The job is not
serialisable (it holds tokens and page bytes); the app keeps the pages as files and a small journal of what the
snapshot said, and seeds a new job from it:

```text
let job = ScanJob::continuing(original_settings, config, kept_page_infos);   // Interrupted, N pages counted
let plan = job.begin_attempt_with(now(), Continuation::Append)?;             // asks for pages - N
```

`settings` is the **original** request (`pages` is the total, 40), `kept` is the `PageInfo` of the pages already on
disk (it is renumbered 1..n; attempt numbering continues after the highest attempt in it). The job starts
`Interrupted` with advice `DoNotRetry`, so `begin_attempt` without a continuation is refused and `Append` or
`Restart` must be named; `finish` accepts what is there. The view reads "12 of 40". `take_pages()` returns only
pages received since, and `NothingLeft` is returned when everything wanted is already kept. With an empty `kept` it
is `ScanJob::new`.

For the terminal, `ScanJobSnapshot::summary_line()` gives `Archive: interrupted, 12 of 40 pages, 1 attempt, jam`
(plain ASCII) and `exit_class()` (also on `ScanJobState`) gives an `ExitClass` with stable names: `completed`,
`needs_attention` (any state that is not an end, and a partial stack), `failed`, `cancelled` (both cancels).

## 12. Journal and the optional `serde` feature

`craft-scan = { features = ["serde"] }` (off by default; it also turns on `craft-job/serde`) derives
`Serialize` and `Deserialize` on `ScanJobSnapshot`, `ScanJobJournal`, `PageInfo`, `AttemptRecord`, `AttemptFailure`,
`AttemptOutcome`, `InFlight`, `ScanJobState`, `Outcome`, `ExitClass`, `Continuation`, `LocalJobId`, `ScanErrorInfo`,
and on the contract enums, **with the wire strings they always had** (`Source`, `Color`, `Intent`, `PageSize`,
`ImageFormat`, `Compression`, `Connection`, `InterruptKind`, `Phase`, `ListDetails`; `Source` also reads its
aliases). `ScanJob` itself is deliberately not serialisable: it holds tokens and page bytes.

**The journal.** `snapshot.into_journal()` returns a `ScanJobJournal` (plain data even without the feature):
`version`, `title`, `device`, `state`, `expected_pages`, `attempts_total`, the kept `pages` (metadata only),
`last_error` and `warnings`. The CLI writes it as JSON beside the page files after every attempt (and
`record_progress`, if it wants to survive a crash). A later `scan --continue DIR` reads it and calls
`ScanJob::continuing_from(settings, config, journal)`:

* the job is `NeedsAttention` (advice `CheckDeviceFirst`) if the journal's state was `NeedsAttention`, or if the
  journal was cut off mid-scan (`acquiring`, or `preparing` with attempts made: `ScanJobJournal::effective_state`),
  with or without kept pages; else `Interrupted` (advice `DoNotRetry`) if pages are kept or any attempt is on record,
  so the job reads as the journal did; only a journal with no attempts gives a fresh `Preparing` job. The last
  error, warnings and attempt count carry over;
* `settings` is the app's own request (device, resolution, colour, ...); the journal does not carry it. If its
  `pages` is `0` and the journal knows the wanted count, that count is used, so the continuation needs no `--pages`;
* the app maps `Continuation::Append` to "the person put the remaining sheets in" and asks.
Without the feature `ScanJob::continuing(settings, config, pages)` does the same from pages the app tracked itself.

**What is deliberately not persisted**: page bytes (the app owns the files), the `ProgressToken` and `CancelToken`,
the `ScanSettings`, the retry budget and the jitter seed (a new process starts a new budget), and the attempt
timeline of earlier runs beyond the count and the last error.

**Forward compatibility.** Names are snake_case. Unknown fields are ignored and fields an older writer lacked are
defaulted. A page `format` this version does not know reads as `bin`; an unknown `InterruptKind` reads as `other`
(craft-job's `Unknown` fallbacks cover job and device state). Other enums fail on an unknown value rather than
guess. `version` is written as `ScanJobJournal::VERSION` (1); additions keep it, a change that cannot be read by
an older reader raises it. A reader that meets a `version` newer than its own does not trust `state`: the job
seeds as an unknown outcome (`NeedsAttention`, `CheckDeviceFirst`) so nothing is resent on a guess. Durations and timestamps are milliseconds (craft-job README, "Wire format").

