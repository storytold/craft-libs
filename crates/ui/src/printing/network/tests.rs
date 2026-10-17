use std::{cell::Cell, rc::Rc, sync::mpsc};

use super::*;

const PDF: &str = "application/pdf";

fn printer() -> PrinterSummary {
    PrinterSummary {
        name: "Office".into(),
        color: Some(true),
        state: "Idle".into(),
        queued_jobs: Some(0),
        reasons: vec![],
        caps: Caps {
            formats: vec![PDF.into()],
            sides: listed(&["OneSided", "TwoSidedLongEdge"]),
            orientations: listed(&["Portrait"]),
            // Not published: unknown.
            qualities: Capability::Unknown,
            media_sizes: listed(&["iso_a4_210x297mm"]),
            copies: Capability::from_option_range(Some((1, 5))),
            ..Caps::default()
        },
    }
}

/// A published list of tokens.
fn listed(tokens: &[&str]) -> Capability<String> {
    let tokens: Vec<String> = tokens.iter().map(|t| (*t).to_owned()).collect();
    Capability::from_option_list(Some(&tokens))
}

/// A draft that was queried at `192.0.2.5` and got `summary`.
fn queried_with(summary: PrinterSummary) -> Draft {
    let mut d = Draft { url: "192.0.2.5".into(), ..Default::default() };
    let g = d.begin_query();
    d.finish_query(g, &d.normalized_url(), Ok(summary));
    d
}

/// A printer whose caps are exactly `caps` (every option not set there is unknown).
fn with_caps(caps: Caps) -> PrinterSummary {
    PrinterSummary { caps: Caps { formats: vec![PDF.into()], ..caps }, ..printer() }
}

/// Answer a new query on `d` with `summary`.
fn requery(d: &mut Draft, summary: PrinterSummary) {
    let g = d.begin_query();
    d.finish_query(g, "http://192.0.2.5", Ok(summary));
}

fn queried(url: &str) -> Draft {
    let mut d = Draft { url: url.into(), ..Default::default() };
    let g = d.begin_query();
    d.finish_query(g, &d.normalized_url(), Ok(printer()));
    d
}

fn labels() -> Labels<'static> {
    Labels {
        address: "Adresse",
        address_hint: "http://example",
        address_note: "Nur lokale Adressen",
        query_printer: "Drucker abfragen",
        querying: "Frage den Drucker",
        query_first: "Zuerst abfragen",
        printer_status: "Status: {state}",
        reasons: "Gruende: {reasons}",
        no_common_format: "Kein gemeinsames Format",
        copies: "Kopien",
        sides: "Zweiseitig",
        sides_off: "Aus",
        sides_long_edge: "Lange Kante",
        sides_short_edge: "Kurze Kante",
        orientation: "Ausrichtung",
        portrait: "Hoch",
        landscape: "Quer",
        reverse_portrait: "Hoch gedreht",
        reverse_landscape: "Quer gedreht",
        quality: "Qualitaet",
        draft: "Entwurf",
        normal: "Normal",
        high: "Hoch",
        photo: "Foto",
        color: "Farbe",
        color_color: "Farbig",
        color_monochrome: "Schwarzweiss",
        resolution: "Aufloesung",
        resolution_dpi: "{dpi} dpi",
        resolution_dpi_xy: "{x} x {y} dpi",
        auto: "Automatisch",
        unsupported_copies: "Dieser Drucker druckt nur eine Kopie.",
        unsupported_sides: "Dieser Drucker druckt nur einseitig.",
        unsupported_orientation: "Ausrichtung nicht unterstuetzt.",
        unsupported_quality: "Qualitaet nicht unterstuetzt.",
        unsupported_color: "Dieser Drucker druckt nur schwarzweiss.",
        unsupported_resolution: "Aufloesung nicht unterstuetzt.",
        unconfirmed_note: "Vom Drucker nicht bestaetigt",
        confirm: "Ich bestaetige den Versand",
        sending: "Sende an {printer}",
        canceling: "Abbrechen laeuft",
        cancel: "Abbrechen",
        sent: "Gesendet an {printer}",
        accepted: "Vom Drucker angenommen: {printer}",
        stage_waiting: "Wartet",
        stage_held: "Angehalten",
        stage_printing: "Druckt",
        stage_stopped: "Drucker braucht Aufmerksamkeit",
        sheets: "Blatt: {n}",
        sheets_progress: "{done} von {total}",
        outcome_warnings: "Mit Warnungen",
        outcome_errors: "Mit Fehlern",
        outcome_canceled: "Abgebrochen",
        outcome_aborted: "Gestoppt",
        outcome_forgotten: "Vergessen",
        outcome_still_running: "Laeuft noch",
        outcome_accepted_unknown: "Gesendet, Ergebnis unbekannt",
        outcome_unknown_warning: "Moeglicherweise schon gesendet: erst am Drucker nachsehen",
        failure_policy: "Adresse abgelehnt",
        failure_transport: "Nicht erreichbar",
        failure_protocol: "Antwort unverstaendlich",
        failure_fault: "Drucker lehnt ab",
        failure_no_common_format: "Kein Format",
        failure_unsupported: "Nicht unterstuetzt",
        failure_encode: "Dokument nicht vorbereitet",
        failure_invalid: "Ungueltig",
        failure_stopped: "Unerwartet beendet",
        failure_other: "Unerwartet",
    }
}

#[test]
fn addresses_get_a_scheme_and_nothing_else() {
    assert_eq!(normalize_url(" 192.0.2.5:5357/s "), "http://192.0.2.5:5357/s");
    assert_eq!(normalize_url("http://198.51.100.2/"), "http://198.51.100.2/");
    assert_eq!(normalize_url("https://198.51.100.2/"), "https://198.51.100.2/");
    assert_eq!(normalize_url("  "), "");
}

#[test]
fn only_listed_options_are_offered() {
    let d = queried("192.0.2.5");
    assert_eq!(d.offered_sides(), [Sides::OneSided, Sides::LongEdge]);
    assert_eq!(d.offered_orientations(), [Orientation::Portrait]);
    // Not published is unknown, not unsupported: every known value is offered, as a hint.
    assert_eq!(d.offered_qualities(), Quality::ALL);
    assert_eq!(d.max_copies(), 5);
    // Nothing queried: everything unknown, so the hints are what is offered.
    let fresh = Draft::default();
    assert_eq!(fresh.offered_sides(), Sides::ALL);
    assert_eq!(fresh.offered_orientations(), Orientation::ALL);
    assert_eq!(fresh.max_copies(), UNKNOWN_MAX_COPIES);
}

#[test]
fn photo_is_a_quality_with_a_token_and_a_label() {
    assert_eq!(Quality::ALL.last().copied(), Some(Quality::Photo));
    assert_eq!(Quality::Photo.token(), "Photo");
    assert_eq!(Quality::from_token("photo"), Some(Quality::Photo));
    assert_eq!(Quality::from_token(" High "), Some(Quality::High));
    assert_eq!(Quality::from_token("Best"), None);
    assert!(Quality::ALL.iter().all(|q| Quality::from_token(q.token()) == Some(*q)));
    assert_eq!(labels().quality_label(Quality::Photo), "Foto");
}

#[test]
fn photo_is_offered_and_requested_only_for_a_printer_that_lists_it() {
    let mut photo_printer = printer();
    photo_printer.caps.qualities = listed(&["High", "Photo", "Vendor"]);
    let mut d = Draft { url: "192.0.2.5".into(), ..Default::default() };
    let g = d.begin_query();
    d.finish_query(g, &d.normalized_url(), Ok(photo_printer));
    assert_eq!(d.offered_qualities(), [Quality::High, Quality::Photo], "in the dialog's order; a vendor value is not a choice");
    d.quality = Some(Quality::Photo);
    assert_eq!(d.request(None).quality, Some(Quality::Photo));

    // Another printer that does not list it: the choice is dropped, not sent.
    let mut plain = printer();
    plain.caps.qualities = listed(&["Normal", "High"]);
    let g = d.begin_query();
    d.finish_query(g, "http://192.0.2.5", Ok(plain));
    assert_eq!((d.quality, d.request(None).quality), (None, None));
    d.quality = Some(Quality::Photo);
    assert_eq!(d.request(None).quality, None, "a stale choice is never sent");
}

#[test]
fn a_request_is_limited_to_what_the_printer_lists() {
    let mut listing = printer();
    listing.caps.qualities = listed(&["Normal", "High"]); // changed: this printer now lists qualities, so Photo is not listed
    let mut d = queried_with(listing);
    d.copies = 3;
    d.sides = Some(Sides::LongEdge);
    d.quality = Some(Quality::Photo);
    let r = d.request(Some("iso_a4_210x297mm"));
    assert_eq!((r.copies, r.sides, r.quality), (Some(3), Some(Sides::LongEdge), None));
    assert_eq!(r.media_size.as_deref(), Some("iso_a4_210x297mm"));
    assert_eq!(d.request(Some("na_letter_8.5x11in")).media_size, None, "an unlisted size is left to the printer");
    assert_eq!(d.request(None).media_size, None);
    d.copies = 1;
    assert_eq!(d.request(None).copies, None);
}

#[test]
fn choices_the_next_printer_lacks_are_dropped() {
    let mut d = queried("192.0.2.5");
    d.copies = 5;
    d.sides = Some(Sides::LongEdge);
    d.orientation = Some(Orientation::Portrait);
    let mut other = printer();
    other.caps.sides = listed(&["OneSided"]);
    other.caps.orientations = Capability::Unsupported; // published, and empty
    other.caps.copies = Capability::from_option_range(Some((1, 2)));
    let g = d.begin_query();
    d.finish_query(g, "http://192.0.2.5", Ok(other));
    assert_eq!((d.sides, d.orientation, d.copies), (None, None, 2));
}

#[test]
fn the_confirmation_is_never_default_and_never_kept() {
    let mut d = queried("192.0.2.5");
    assert!(!d.confirmed && !Draft::default().confirmed);
    assert_eq!(d.block(true, true, &[PDF]), Some(Block::Unconfirmed));
    d.confirmed = true;
    assert_eq!(d.block(true, true, &[PDF]), None);
    d.url = "192.0.2.6".into();
    d.address_edited();
    assert!(!d.confirmed && d.query == Query::Idle && d.printer().is_none());
    let mut d = queried("192.0.2.5");
    d.confirmed = true;
    d.begin_query();
    assert!(!d.confirmed);
    let mut d = queried("192.0.2.5");
    d.confirmed = true;
    d.job_started();
    assert!(!d.confirmed && d.running());
    d.confirmed = true;
    d.dialog_opened();
    assert!(!d.confirmed && d.running(), "a running job survives reopening");
    d.job_finished(JobView::Idle);
    d.confirmed = true;
    d.dialog_opened();
    assert!(!d.confirmed);
}

#[test]
fn print_waits_for_a_query_of_this_address() {
    let mut d = Draft { url: "192.0.2.5".into(), confirmed: true, ..Default::default() };
    assert_eq!(d.block(false, true, &[PDF]), Some(Block::Address));
    assert_eq!(d.block(true, true, &[PDF]), Some(Block::NotQueried));
    let g = d.begin_query();
    assert_eq!(d.block(true, true, &[PDF]), Some(Block::Busy));
    d.finish_query(g, "http://192.0.2.5", Ok(printer()));
    d.confirmed = true;
    assert_eq!(d.block(true, false, &[PDF]), Some(Block::NoPages));
    assert_eq!(d.block(true, true, &[PDF]), None);
    // A printer that takes none of the formats the host can produce.
    assert_eq!(d.block(true, true, &["image/pwg-raster"]), Some(Block::Format));
    assert_eq!(d.block(true, true, &["image/pwg-raster", "APPLICATION/PDF"]), None, "format match ignores case");
    // A printer listing no formats is left to answer for itself.
    let mut silent = printer();
    silent.caps.formats.clear();
    let g = d.begin_query();
    d.finish_query(g, "http://192.0.2.5", Ok(silent));
    d.confirmed = true;
    assert_eq!(d.block(true, true, &["image/pwg-raster"]), None);
}

#[test]
fn a_late_or_stale_answer_is_dropped() {
    let mut d = Draft { url: "192.0.2.5".into(), ..Default::default() };
    let first = d.begin_query();
    let second = d.begin_query();
    d.finish_query(first, "http://192.0.2.5", Ok(printer()));
    assert!(d.querying(), "the older answer does not end the newer query");
    d.url = "192.0.2.9".into();
    d.address_edited();
    d.finish_query(second, "http://192.0.2.5", Ok(printer()));
    assert!(d.printer().is_none());
    let g = d.begin_query();
    let failure = Failure::new(FailureKind::Transport, "timed out");
    d.finish_query(g, "http://192.0.2.9", Err(failure.clone()));
    assert_eq!(d.query, Query::Failed(failure));
}

#[test]
fn cancel_is_sent_once() {
    let mut d = queried("192.0.2.5");
    d.job_started();
    assert_eq!(d.job, JobView::Running { canceling: false });
    assert!(d.job_canceling());
    assert!(!d.job_canceling());
    d.job_finished(JobView::Idle);
    assert!(!d.job_canceling());
}

#[test]
fn outcomes_and_failures_map_from_client_names() {
    for (name, outcome) in [
        ("completed", Outcome::Completed),
        ("completed_with_warnings", Outcome::CompletedWithWarnings),
        ("completed_with_errors", Outcome::CompletedWithErrors),
        ("canceled", Outcome::Canceled),
        ("forgotten", Outcome::Forgotten),
        ("still_running", Outcome::StillRunning),
        ("anything else", Outcome::Aborted),
    ] {
        assert_eq!(Outcome::from_name(name), outcome, "{name}");
    }
    assert!(Outcome::CompletedWithWarnings.is_success() && !Outcome::Canceled.is_success());
    let l = labels();
    assert_eq!(l.outcome_text(Outcome::Completed), None);
    assert!(Outcome::from_name("canceled") != Outcome::Completed && l.outcome_text(Outcome::Canceled) == Some("Abgebrochen"));
    let kinds = [
        "policy",
        "transport",
        "connection_closed",
        "http",
        "too_large",
        "xml",
        "protocol",
        "fault",
        "invalid",
        "no_common_format",
        "unsupported",
        "encode",
        "stopped",
        "mystery",
    ];
    let headings: std::collections::BTreeSet<_> = kinds.iter().map(|k| l.failure_heading(FailureKind::from_name(k))).collect();
    assert_eq!(headings.len(), 10, "{headings:?}");
}

fn query_worker(answer: Result<PrinterSummary, Failure>) -> QueryPrinter {
    let answer = std::sync::Mutex::new(Some(answer));
    Box::new(move |_| {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut slot) = answer.lock()
            && let Some(answer) = slot.take()
        {
            let _ = tx.send(answer);
        }
        rx
    })
}

#[test]
fn state_files_a_query_answer_and_drops_a_stopped_worker() {
    let ctx = egui::Context::default();
    let mut state = State::new(Draft::for_address("192.0.2.5"));
    assert!(!state.start_query(&query_worker(Ok(printer())), false), "an address the host refused starts nothing");
    assert!(state.start_query(&query_worker(Ok(printer())), true));
    assert!(state.draft.querying());
    assert!(!state.start_query(&query_worker(Ok(printer())), true), "one query at a time");
    state.poll(&ctx);
    assert_eq!(state.draft.printer().map(|p| p.name.as_str()), Some("Office"));
    // A worker that ends without answering is a failure, not a hang.
    let silent: QueryPrinter = Box::new(|_| mpsc::channel().1);
    assert!(state.start_query(&silent, true));
    state.poll(&ctx);
    assert_eq!(state.draft.query, Query::Failed(Failure::new(FailureKind::Stopped, "")));
    assert!(state.draft.printer().is_none());
}

#[test]
fn a_job_starts_only_when_nothing_blocks_it_and_spends_the_confirmation() {
    let ctx = egui::Context::default();
    let mut state = State::new(queried("192.0.2.5"));
    let cancelled = Rc::new(Cell::new(0));
    let (tx, rx) = mpsc::channel();
    let slot = std::cell::RefCell::new(Some(rx));
    let seen = Rc::new(std::cell::RefCell::new(None));
    let (seen2, cancelled2) = (seen.clone(), cancelled.clone());
    let send: SendJob = Box::new(move |url, request| {
        *seen2.borrow_mut() = Some((url.to_owned(), request));
        let counter = cancelled2.clone();
        JobHandle::new(slot.borrow_mut().take().unwrap_or_else(|| mpsc::channel().1), Box::new(move || counter.set(counter.get() + 1)))
    });
    assert!(!state.start_job(&send, true, true, &[PDF], None), "unconfirmed");
    state.draft.confirmed = true;
    state.draft.copies = 2;
    assert!(state.start_job(&send, true, true, &[PDF], Some("iso_a4_210x297mm")));
    assert!(!state.draft.confirmed && state.draft.running());
    let sent = seen.borrow().clone();
    let (url, request) = sent.unwrap_or_default();
    assert_eq!(url, "http://192.0.2.5");
    assert_eq!((request.copies, request.media_size.as_deref()), (Some(2), Some("iso_a4_210x297mm")));
    assert!(!state.start_query(&query_worker(Ok(printer())), true), "no query while a job runs");
    assert!(state.cancel_job());
    assert!(!state.cancel_job());
    assert_eq!(cancelled.get(), 1, "the host is told once");
    let report = Report { outcome: Outcome::Canceled, printer: "Office".into(), sheets: 0, notes: vec![] };
    tx.send(Ok(report.clone())).unwrap();
    state.poll(&ctx);
    assert_eq!(state.draft.job, JobView::Finished(report));
    assert!(!state.cancel_job());
}

#[test]
fn a_job_worker_that_stops_or_fails_is_reported() {
    let ctx = egui::Context::default();
    let mut state = State::new(queried("192.0.2.5"));
    let (tx, rx) = mpsc::channel();
    let slot = std::cell::RefCell::new(Some(rx));
    let send: SendJob = Box::new(move |_, _| JobHandle::new(slot.borrow_mut().take().unwrap_or_else(|| mpsc::channel().1), Box::new(|| {})));
    state.draft.confirmed = true;
    assert!(state.start_job(&send, true, true, &[PDF], None));
    drop(tx);
    state.poll(&ctx);
    assert_eq!(state.draft.job, JobView::Failed(Failure::new(FailureKind::Stopped, "")));
    let (tx, rx) = mpsc::channel();
    let slot = std::cell::RefCell::new(Some(rx));
    let send: SendJob = Box::new(move |_, _| JobHandle::new(slot.borrow_mut().take().unwrap_or_else(|| mpsc::channel().1), Box::new(|| {})));
    state.draft.confirmed = true;
    assert!(state.start_job(&send, true, true, &[PDF], None));
    let failure = Failure::new(FailureKind::Fault, "refused");
    tx.send(Err(failure.clone())).unwrap();
    state.poll(&ctx);
    assert_eq!(state.draft.job, JobView::Failed(failure));
}

type Frames<'a> = (&'a egui::Context, &'a std::cell::RefCell<State>, &'a Validate, &'a QueryPrinter);

fn frame(f: &Frames<'_>, events: Vec<egui::Event>) -> (Response, egui::FullOutput) {
    let (ctx, state, validate, query) = *f;
    let l = labels();
    let mut response = Response::default();
    let mut output = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 480.0))), events, ..Default::default() },
        |ui| {
            let host = Host { validate, query, sheets_ok: true, formats: &[PDF] };
            response = state.borrow_mut().show(ui, egui::Id::new("net"), &host, &l);
        },
    );
    output.textures_delta.clear();
    (response, output)
}

fn center_of(output: &egui::FullOutput, label: &str) -> Option<egui::Pos2> {
    let tree = output.platform_output.accesskit_update.as_ref()?;
    let node = tree.nodes.iter().find(|(_, n)| n.label() == Some(label) || n.value() == Some(label))?.1.clone();
    let b = node.bounds()?;
    Some(egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
}

fn click(f: &Frames<'_>, at: egui::Pos2) -> Response {
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(f, vec![egui::Event::PointerMoved(at), button(true)]);
    frame(f, vec![button(false)]).0
}

#[test]
fn the_section_shows_its_controls_with_labels_and_drives_query_and_confirmation() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let state = std::cell::RefCell::new(State::new(Draft::for_address("192.0.2.5")));
    let validate: &Validate = &|url| if url.is_empty() { Err(Failure::new(FailureKind::Policy, "empty")) } else { Ok(()) };
    let query = query_worker(Ok(printer()));
    let f: Frames<'_> = (&ctx, &state, validate, &query);
    let (r, output) = frame(&f, Vec::new());
    assert_eq!(r.block, Some(Block::NotQueried));
    let at = center_of(&output, "Drucker abfragen");
    assert!(at.is_some(), "the Query button is reachable by label");
    let r = click(&f, at.unwrap_or_default());
    assert!(r.query_started);
    let (r, output) = frame(&f, Vec::new());
    assert_eq!(r.block, Some(Block::Unconfirmed));
    for label in ["Adresse", "Ich bestaetige den Versand", "Kopien", "Zweiseitig", "Ausrichtung"] {
        assert!(center_of(&output, label).is_some(), "{label} is exposed");
    }
    // Changed: an unpublished option (quality here) is unknown, not unsupported, so it is drawn.
    assert!(center_of(&output, "Qualitaet").is_some(), "an unpublished option is drawn, enabled");
    let at = center_of(&output, "Ich bestaetige den Versand").unwrap_or_default();
    let r = click(&f, at);
    assert!(r.changed);
    assert!(state.borrow().draft.confirmed);
    let (r, _) = frame(&f, Vec::new());
    assert_eq!(r.block, None, "the app may now enable Print");
    // Editing the address voids the answer and the yes.
    state.borrow_mut().draft.url.push('0');
    state.borrow_mut().draft.address_edited();
    let (r, _) = frame(&f, Vec::new());
    assert_eq!(r.block, Some(Block::NotQueried));
    assert!(!state.borrow().draft.confirmed);
}

#[test]
fn a_refused_address_shows_the_hosts_error_and_blocks_query() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let state = std::cell::RefCell::new(State::new(Draft::for_address("somewhere")));
    let validate: &Validate = &|_| Err(Failure::new(FailureKind::Policy, "not on this network"));
    let query = query_worker(Ok(printer()));
    let f: Frames<'_> = (&ctx, &state, validate, &query);
    let (r, output) = frame(&f, Vec::new());
    assert_eq!(r.block, Some(Block::Address));
    assert!(center_of(&output, "Adresse abgelehnt").is_some());
    assert!(center_of(&output, "not on this network").is_some());
    let tree = output.platform_output.accesskit_update.as_ref();
    let disabled = tree.is_some_and(|t| t.nodes.iter().any(|(_, n)| n.label() == Some("Drucker abfragen") && n.is_disabled()));
    assert!(disabled, "Query printer is disabled for a refused address");
}

#[test]
fn a_running_job_disables_editing_and_offers_cancel() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut draft = queried("192.0.2.5");
    draft.job_started();
    let state = std::cell::RefCell::new(State::new(draft));
    let validate: &Validate = &|_| Ok(());
    let query = query_worker(Ok(printer()));
    let f: Frames<'_> = (&ctx, &state, validate, &query);
    let (r, output) = frame(&f, Vec::new());
    assert_eq!(r.block, Some(Block::Busy));
    let tree = output.platform_output.accesskit_update.as_ref();
    let disabled = |label: &str| tree.is_some_and(|t| t.nodes.iter().any(|(_, n)| n.label() == Some(label) && n.is_disabled()));
    assert!(disabled("Ich bestaetige den Versand") && disabled("Drucker abfragen"));
    let at = center_of(&output, "Abbrechen").unwrap_or_default();
    let r = click(&f, at);
    assert!(r.cancel_requested);
    assert_eq!(state.borrow().draft.job, JobView::Running { canceling: true });
}

// ---- two-phase jobs: sending, accepted, status updates, result ----

struct Worker {
    result: mpsc::Sender<Result<Report, Failure>>,
    updates: mpsc::Sender<JobUpdate>,
    cancelled: Rc<Cell<u32>>,
}

/// A started job whose worker the test plays.
fn running_job() -> (State, Worker, egui::Context) {
    let ctx = egui::Context::default();
    let mut state = State::new(queried("192.0.2.5"));
    let (result_tx, result_rx) = mpsc::channel();
    let (updates_tx, updates_rx) = mpsc::channel();
    let cancelled = Rc::new(Cell::new(0));
    let counter = cancelled.clone();
    let handles = std::cell::RefCell::new(Some((result_rx, updates_rx)));
    let send: SendJob = Box::new(move |_, _| {
        let (result, updates) = handles.borrow_mut().take().unwrap_or_else(|| (mpsc::channel().1, mpsc::channel().1));
        let counter = counter.clone();
        JobHandle::new(result, Box::new(move || counter.set(counter.get() + 1))).with_updates(updates)
    });
    state.draft.confirmed = true;
    assert!(state.start_job(&send, true, true, &[PDF], None));
    (state, Worker { result: result_tx, updates: updates_tx, cancelled }, ctx)
}

fn progress(stage: Stage, done: Option<u32>, total: Option<u32>) -> Progress {
    Progress { stage, sheets_done: done, sheets_total: total, notes: Vec::new() }
}

#[test]
fn a_job_goes_from_sending_to_accepted_to_a_report_one_frame_at_a_time() {
    let (mut state, worker, ctx) = running_job();
    assert_eq!(state.draft.job, JobView::Running { canceling: false });
    state.poll(&ctx);
    assert_eq!(state.draft.job, JobView::Running { canceling: false }, "nothing arrived: nothing changed, and nothing blocked");
    // The printer accepts the document: the UI no longer says "sending".
    worker.updates.send(JobUpdate::Accepted(Progress::default())).unwrap();
    state.poll(&ctx);
    assert_eq!(state.draft.job, JobView::Accepted { canceling: false, progress: Progress::default() });
    assert!(state.draft.running(), "an accepted job is still a job in flight");
    assert_eq!(state.draft.block(true, true, &[PDF]), Some(Block::Busy));
    // Several statuses in one frame: the last one is shown.
    worker.updates.send(JobUpdate::Status(progress(Stage::Waiting, None, None))).unwrap();
    worker.updates.send(JobUpdate::Status(progress(Stage::Printing, Some(1), Some(3)))).unwrap();
    state.poll(&ctx);
    assert_eq!(state.draft.job, JobView::Accepted { canceling: false, progress: progress(Stage::Printing, Some(1), Some(3)) });
    // The result ends it.
    let report = Report { outcome: Outcome::Completed, printer: "Office".into(), sheets: 3, notes: vec![] };
    worker.result.send(Ok(report.clone())).unwrap();
    state.poll(&ctx);
    assert_eq!(state.draft.job, JobView::Finished(report.clone()));
    assert!(!state.draft.running());
    // Late updates change nothing.
    worker.updates.send(JobUpdate::Status(progress(Stage::Printing, Some(2), Some(3)))).unwrap_or_default();
    state.poll(&ctx);
    assert_eq!(state.draft.job, JobView::Finished(report));
}

#[test]
fn updates_and_the_result_in_the_same_frame_end_in_the_result() {
    let (mut state, worker, ctx) = running_job();
    worker.updates.send(JobUpdate::Accepted(Progress::default())).unwrap();
    worker.updates.send(JobUpdate::Status(progress(Stage::Printing, Some(1), Some(1)))).unwrap();
    let report = Report { outcome: Outcome::CompletedWithWarnings, printer: "Office".into(), sheets: 1, notes: vec!["low toner".into()] };
    worker.result.send(Ok(report.clone())).unwrap();
    state.poll(&ctx);
    assert_eq!(state.draft.job, JobView::Finished(report));
}

#[test]
fn a_status_before_the_acceptance_counts_as_the_acceptance_and_one_after_the_end_is_dropped() {
    let mut d = queried("192.0.2.5");
    d.job_progress(progress(Stage::Printing, None, None));
    assert_eq!(d.job, JobView::Idle, "no job: nothing to update");
    d.job_started();
    d.job_progress(progress(Stage::Printing, None, None));
    assert!(matches!(d.job, JobView::Accepted { canceling: false, ref progress } if progress.stage == Stage::Printing));
    let mut d = queried("192.0.2.5");
    d.job_started();
    d.job_canceling();
    d.job_accepted(Progress::default());
    assert!(matches!(d.job, JobView::Accepted { canceling: true, .. }), "a cancel already pressed is kept");
    d.job_accepted(progress(Stage::Printing, None, None));
    assert!(matches!(d.job, JobView::Accepted { ref progress, .. } if progress.stage == Stage::Accepted), "accepted twice changes nothing");
    d.job_finished(JobView::Failed(Failure::new(FailureKind::Other, "x")));
    d.job_progress(progress(Stage::Printing, None, None));
    assert!(matches!(d.job, JobView::Failed(_)));
}

#[test]
fn cancel_works_in_both_phases_and_the_host_is_told_once() {
    let (mut state, worker, ctx) = running_job();
    worker.updates.send(JobUpdate::Accepted(Progress::default())).unwrap();
    state.poll(&ctx);
    assert!(state.cancel_job());
    assert!(!state.cancel_job());
    assert_eq!(worker.cancelled.get(), 1);
    assert!(matches!(state.draft.job, JobView::Accepted { canceling: true, .. }));
    worker.updates.send(JobUpdate::Status(progress(Stage::Printing, Some(1), Some(2)))).unwrap();
    state.poll(&ctx);
    assert!(matches!(state.draft.job, JobView::Accepted { canceling: true, .. }), "a status does not undo a cancel");
    // And in the sending phase.
    let (mut state, worker, _ctx) = running_job();
    assert!(state.cancel_job());
    assert_eq!(worker.cancelled.get(), 1);
    worker.updates.send(JobUpdate::Accepted(Progress::default())).unwrap();
    state.poll(&egui::Context::default());
    assert!(matches!(state.draft.job, JobView::Accepted { canceling: true, .. }));
}

#[test]
fn a_worker_that_stops_after_the_acceptance_is_sent_outcome_unknown_not_a_failure() {
    let (mut state, worker, ctx) = running_job();
    worker.updates.send(JobUpdate::Accepted(Progress::default())).unwrap();
    drop(worker);
    state.poll(&ctx);
    assert_eq!(state.draft.job, JobView::Finished(Report { outcome: Outcome::AcceptedUnknown, printer: "Office".into(), sheets: 0, notes: vec![] }));
    // Before the acceptance it is still a failure: nothing is known to have been sent.
    let (mut state, worker, ctx) = running_job();
    drop(worker);
    state.poll(&ctx);
    assert_eq!(state.draft.job, JobView::Failed(Failure::new(FailureKind::Stopped, "")));
}

#[test]
fn a_printer_that_cannot_report_ends_as_accepted_unknown() {
    let (mut state, worker, ctx) = running_job();
    worker.updates.send(JobUpdate::Accepted(Progress::default())).unwrap();
    let report = Report { outcome: Outcome::AcceptedUnknown, printer: "Office".into(), sheets: 0, notes: vec![] };
    worker.result.send(Ok(report.clone())).unwrap();
    state.poll(&ctx);
    assert_eq!(state.draft.job, JobView::Finished(report));
    assert!(!Outcome::AcceptedUnknown.is_success(), "sent is not printed");
    assert_eq!(labels().outcome_text(Outcome::AcceptedUnknown), Some("Gesendet, Ergebnis unbekannt"));
}

#[test]
fn client_names_map_tolerantly_to_stages_and_outcomes() {
    for (name, stage) in [
        ("pending", Stage::Waiting),
        ("held", Stage::Held),
        ("processing", Stage::Printing),
        ("stopped", Stage::Stopped),
        ("unknown", Stage::Accepted),
        ("something new", Stage::Accepted),
    ] {
        assert_eq!(Stage::from_name(name), stage, "{name}");
    }
    assert_eq!(Outcome::from_name("accepted_unknown"), Outcome::AcceptedUnknown);
    assert_eq!(Outcome::from_name("no_status"), Outcome::AcceptedUnknown);
    assert_eq!(Outcome::from_name("completed"), Outcome::Completed, "the old names still map");
    let l = labels();
    assert_eq!(l.stage_text(Stage::Accepted, "Office"), "Vom Drucker angenommen: Office");
    assert_eq!(l.stage_text(Stage::Printing, "Office"), "Druckt");
    let f = Failure::new(FailureKind::Transport, "closed");
    assert!(!f.outcome_unknown && f.clone().with_outcome_unknown(true).outcome_unknown);
}

#[test]
fn the_accepted_phase_is_drawn_with_its_stage_progress_and_cancel() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut draft = queried("192.0.2.5");
    draft.job_started();
    draft.job_accepted(Progress { stage: Stage::Printing, sheets_done: Some(1), sheets_total: Some(4), notes: vec!["Papier fast leer".into()] });
    let state = std::cell::RefCell::new(State::new(draft));
    let validate: &Validate = &|_| Ok(());
    let query = query_worker(Ok(printer()));
    let f: Frames<'_> = (&ctx, &state, validate, &query);
    let (r, output) = frame(&f, Vec::new());
    assert_eq!(r.block, Some(Block::Busy));
    assert!(center_of(&output, "Druckt").is_some(), "the stage is worded by the app");
    assert!(center_of(&output, "Papier fast leer").is_some(), "the printer's reasons are shown");
    assert!(center_of(&output, "Sende an Office").is_none(), "no longer 'sending'");
    let tree = output.platform_output.accesskit_update.as_ref();
    assert!(
        tree.is_some_and(|t| t
            .nodes
            .iter()
            .any(|(_, n)| n.label() == Some("1 von 4") || n.value() == Some("1 von 4") || n.label().is_some_and(|l| l.contains("1 von 4")))),
        "the sheet progress is exposed"
    );
    let at = center_of(&output, "Abbrechen").unwrap_or_default();
    let r = click(&f, at);
    assert!(r.cancel_requested);
    assert!(matches!(state.borrow().draft.job, JobView::Accepted { canceling: true, .. }));
}

#[test]
fn an_accepted_job_without_status_says_accepted_and_a_failure_that_may_have_been_delivered_says_so() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut draft = queried("192.0.2.5");
    draft.job_started();
    draft.job_accepted(Progress::default());
    let state = std::cell::RefCell::new(State::new(draft));
    let validate: &Validate = &|_| Ok(());
    let query = query_worker(Ok(printer()));
    let f: Frames<'_> = (&ctx, &state, validate, &query);
    let (_, output) = frame(&f, Vec::new());
    assert!(center_of(&output, "Vom Drucker angenommen: Office").is_some());
    state.borrow_mut().draft.job_finished(JobView::Failed(Failure::new(FailureKind::Transport, "closed").with_outcome_unknown(true)));
    let (_, output) = frame(&f, Vec::new());
    assert!(center_of(&output, "Nicht erreichbar").is_some());
    assert!(center_of(&output, "Moeglicherweise schon gesendet: erst am Drucker nachsehen").is_some());
    state.borrow_mut().draft.job_finished(JobView::Failed(Failure::new(FailureKind::Transport, "refused")));
    let (_, output) = frame(&f, Vec::new());
    assert!(center_of(&output, "Moeglicherweise schon gesendet: erst am Drucker nachsehen").is_none(), "only when it may have been sent");
}

// ---- optional sheet totals ----

fn exposes(output: &egui::FullOutput, text: &str) -> bool {
    output.platform_output.accesskit_update.as_ref().is_some_and(|t| {
        t.nodes.iter().any(|(_, n)| n.label() == Some(text) || n.value() == Some(text) || n.label().is_some_and(|l| l.contains(text)))
    })
}

fn accepted_draft(expected_pages: Option<u32>, progress: Progress) -> Draft {
    let mut draft = queried("192.0.2.5");
    draft.expected_pages = expected_pages;
    draft.job_started();
    draft.job_accepted(progress);
    draft
}

#[test]
fn progress_is_indeterminate_until_a_total_is_known() {
    assert_eq!(progress(Stage::Printing, Some(2), None).fraction(), None);
    assert_eq!(progress(Stage::Printing, None, Some(4)).fraction(), None);
    assert_eq!(progress(Stage::Printing, Some(2), Some(0)).fraction(), None);
    assert_eq!(progress(Stage::Printing, Some(1), Some(4)).fraction(), Some(0.25));
    assert_eq!(progress(Stage::Printing, Some(9), Some(4)).fraction(), Some(1.0), "more than the total is full, not overfull");
    let unknown = progress(Stage::Printing, Some(2), None);
    assert_eq!(unknown.clone().with_expected_total(Some(5)).sheets_total, Some(5));
    assert_eq!(unknown.clone().with_expected_total(None).sheets_total, None);
    assert_eq!(unknown.with_expected_total(Some(0)).sheets_total, None, "zero is no total");
    assert_eq!(progress(Stage::Printing, Some(2), Some(3)).with_expected_total(Some(5)).sheets_total, Some(3), "the printer's total wins");
    assert_eq!(progress(Stage::Printing, Some(2), Some(0)).with_expected_total(Some(5)).sheets_total, Some(5), "a reported zero is not a total");
}

#[test]
fn the_expected_sheets_follow_pages_sides_and_copies() {
    let mut d = queried("192.0.2.5");
    assert_eq!(d.expected_sheets(), None, "the app has not said");
    d.expected_pages = Some(5);
    assert_eq!(d.expected_sheets(), Some(5));
    d.sides = Some(Sides::LongEdge);
    assert_eq!(d.expected_sheets(), Some(3), "two pages to a sheet, rounded up");
    d.copies = 2;
    assert_eq!(d.expected_sheets(), Some(6));
    d.expected_pages = Some(0);
    assert_eq!(d.expected_sheets(), None, "zero pages is unknown");
    d.expected_pages = Some(4);
    d.copies = 99;
    assert_eq!(d.expected_sheets(), Some(2 * 5), "copies stay within what the printer takes");
}

#[test]
fn the_request_carries_the_expected_pages_to_the_host() {
    let mut d = queried("192.0.2.5");
    assert_eq!(d.request(None).expected_pages, None);
    d.expected_pages = Some(7);
    assert_eq!(d.request(None).expected_pages, Some(7));
    d.expected_pages = Some(0);
    assert_eq!(d.request(None).expected_pages, None);
}

#[test]
fn a_job_without_a_reported_total_gets_the_expected_one_and_a_reported_total_wins() {
    let d = accepted_draft(Some(4), Progress::default());
    assert_eq!(d.job, JobView::Accepted { canceling: false, progress: Progress { sheets_total: Some(4), ..Progress::default() } });
    let mut d = accepted_draft(Some(4), progress(Stage::Printing, Some(1), None));
    d.job_progress(progress(Stage::Printing, Some(2), None));
    assert!(matches!(&d.job, JobView::Accepted { progress, .. } if progress.sheets_done == Some(2) && progress.sheets_total == Some(4)));
    d.job_progress(progress(Stage::Printing, Some(3), Some(6)));
    assert!(matches!(&d.job, JobView::Accepted { progress, .. } if progress.sheets_total == Some(6)), "the printer's total replaces the estimate");
    // Without an expectation nothing is invented.
    let d = accepted_draft(None, progress(Stage::Printing, Some(1), None));
    assert!(matches!(&d.job, JobView::Accepted { progress, .. } if progress.sheets_total.is_none()));
    // The estimate is taken when the job starts: later edits do not move it.
    let mut d = accepted_draft(Some(4), Progress::default());
    d.copies = 3;
    d.job_progress(progress(Stage::Printing, Some(1), None));
    assert!(matches!(&d.job, JobView::Accepted { progress, .. } if progress.sheets_total == Some(4)));
}

fn drawn(draft: Draft) -> egui::FullOutput {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let state = std::cell::RefCell::new(State::new(draft));
    let validate: &Validate = &|_| Ok(());
    let query = query_worker(Ok(printer()));
    let f: Frames<'_> = (&ctx, &state, validate, &query);
    frame(&f, Vec::new()).1
}

#[test]
fn an_unknown_total_shows_the_count_and_no_bar_and_a_known_one_shows_the_bar() {
    // The printer says 3 sheets done and nothing else: "Blatt: 3", and no "x von y" bar.
    let output = drawn(accepted_draft(None, progress(Stage::Printing, Some(3), None)));
    assert!(center_of(&output, "Blatt: 3").is_some(), "the count of sheets is shown");
    assert!(!exposes(&output, " von "), "there is no determinate bar without a total");
    // Nothing done yet and no total: neither.
    let output = drawn(accepted_draft(None, progress(Stage::Printing, Some(0), None)));
    assert!(!exposes(&output, "Blatt:") && !exposes(&output, " von "));
    // The app's page count turns it determinate.
    let output = drawn(accepted_draft(Some(4), progress(Stage::Printing, Some(1), None)));
    assert!(exposes(&output, "1 von 4"), "the expected total fills the bar");
    assert!(center_of(&output, "Blatt: 1").is_none(), "a bar replaces the bare count");
    // The printer's own total wins over the expectation.
    let output = drawn(accepted_draft(Some(4), progress(Stage::Printing, Some(1), Some(8))));
    assert!(exposes(&output, "1 von 8") && !exposes(&output, "1 von 4"));
}

// ---- three-state options: supported, unsupported, unknown ----

fn drawn_with(summary: PrinterSummary) -> egui::FullOutput {
    drawn(queried_with(summary))
}

/// For every node whose description is `description`: is it disabled?
fn described(output: &egui::FullOutput, description: &str) -> Vec<bool> {
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .map(|t| t.nodes.iter().filter(|(_, n)| n.description() == Some(description)).map(|(_, n)| n.is_disabled()).collect())
        .unwrap_or_default()
}

fn one_sided_only() -> PrinterSummary {
    let mut p = printer();
    p.caps.sides = listed(&["OneSided"]);
    p
}

#[test]
fn sides_without_a_two_sided_value_are_unsupported_and_the_request_never_carries_one() {
    let mut d = queried_with(one_sided_only());
    assert_eq!(d.sides_state(), Capability::Unsupported);
    assert!(d.offered_sides().is_empty());
    d.sides = Some(Sides::LongEdge);
    assert_eq!(d.request(None).sides, None, "a stale two-sided choice is never sent");
    d.sides = Some(Sides::OneSided);
    assert_eq!(d.request(None).sides, None, "nor is one-sided: the printer has no duplex option to set");
    // The dependent choice: a duplex estimate does not halve the sheets of an unsupported option.
    d.sides = Some(Sides::LongEdge);
    d.expected_pages = Some(4);
    assert_eq!(d.expected_sheets(), Some(4));
    // Cleared on a new answer.
    let mut d = queried("192.0.2.5");
    d.sides = Some(Sides::LongEdge);
    requery(&mut d, one_sided_only());
    assert_eq!(d.sides, None);
    // An explicit "no", and a list with nothing the dialog knows, are the same state.
    let mut p = printer();
    p.caps.sides = Capability::Unsupported;
    assert_eq!(queried_with(p).sides_state(), Capability::Unsupported);
    let mut p = printer();
    p.caps.sides = listed(&["Vendor"]);
    assert_eq!(queried_with(p).sides_state(), Capability::Unsupported);
}

#[test]
fn every_option_has_the_three_states() {
    // Supported: restricted to the list; the printer default is carried.
    let mut p = printer();
    p.caps.qualities = Capability::list_default_first(&["Draft".to_owned(), "High".to_owned()], Some(&"High".to_owned()));
    p.caps.color_modes = listed(&["Color", "Monochrome"]);
    p.caps.resolutions = Capability::from_option_list(Some(&[Resolution::square(300), Resolution::square(600)][..]));
    let d = queried_with(p);
    assert_eq!(d.quality_state().list(), [Quality::Draft, Quality::High]);
    assert_eq!(d.quality_state().default_value(), Some(&Quality::High));
    assert_eq!(d.offered_colors(), ColorMode::ALL);
    assert_eq!(d.offered_resolutions(), [Resolution::square(300), Resolution::square(600)]);
    assert!(d.copies_state().is_supported() && d.sides_state().is_supported());

    // Unknown: nothing published; every hint is offered.
    let d = queried_with(with_caps(Caps::default()));
    assert!(d.sides_state().is_enabled() && d.orientation_state().is_enabled() && d.quality_state().is_enabled());
    assert!(d.color_state().is_enabled() && d.resolution_state().is_enabled() && d.copies_state().is_enabled());
    assert!(!d.sides_state().is_known() && !d.color_state().is_known() && !d.copies_state().is_known());
    assert_eq!((d.offered_sides().len(), d.offered_colors().len()), (3, 2));
    assert_eq!(d.offered_resolutions(), COMMON_RESOLUTIONS);
    assert_eq!(d.max_copies(), UNKNOWN_MAX_COPIES);

    // Unsupported: published, and no.
    let unsupported = Caps {
        sides: Capability::Unsupported,
        orientations: Capability::Unsupported,
        qualities: Capability::Unsupported,
        color_modes: listed(&["Monochrome"]),
        resolutions: Capability::Unsupported,
        copies: Capability::from_option_range(Some((1, 1))),
        ..Caps::default()
    };
    let d = queried_with(with_caps(unsupported));
    assert!(d.sides_state().is_unsupported() && d.orientation_state().is_unsupported() && d.quality_state().is_unsupported());
    assert!(d.color_state().is_unsupported(), "a monochrome-only printer has no colour option");
    assert!(d.resolution_state().is_unsupported() && d.copies_state().is_unsupported(), "a maximum of one copy is not a choice");
    assert_eq!(d.max_copies(), 1);
    assert!(d.offered_qualities().is_empty() && d.offered_colors().is_empty() && d.offered_resolutions().is_empty());
}

#[test]
fn the_request_omits_unsupported_keeps_unknown_hints_and_restricts_supported() {
    let set_all = |d: &mut Draft| {
        d.copies = 3;
        d.sides = Some(Sides::ShortEdge);
        d.orientation = Some(Orientation::Landscape);
        d.quality = Some(Quality::Photo);
        d.color = Some(ColorMode::Color);
        d.resolution = Some(Resolution::square(600));
    };
    // Unknown: every hint goes.
    let mut d = queried_with(with_caps(Caps::default()));
    set_all(&mut d);
    let r = d.request(Some("na_letter_8.5x11in"));
    assert_eq!((r.copies, r.sides, r.orientation), (Some(3), Some(Sides::ShortEdge), Some(Orientation::Landscape)));
    assert_eq!((r.quality, r.color, r.resolution), (Some(Quality::Photo), Some(ColorMode::Color), Some(Resolution::square(600))));
    assert_eq!(r.media_size.as_deref(), Some("na_letter_8.5x11in"), "an unpublished media list does not drop the app's size");
    // Unsupported: nothing goes, even when the draft still holds the values.
    let none = Caps {
        sides: Capability::Unsupported,
        orientations: Capability::Unsupported,
        qualities: Capability::Unsupported,
        color_modes: Capability::Unsupported,
        media_sizes: Capability::Unsupported,
        resolutions: Capability::Unsupported,
        copies: Capability::Unsupported,
        ..Caps::default()
    };
    let mut d = queried_with(with_caps(none));
    set_all(&mut d);
    let r = d.request(Some("iso_a4_210x297mm"));
    assert_eq!((r.copies, r.sides, r.orientation, r.quality), (None, None, None, None));
    assert_eq!((r.color, r.resolution, r.media_size), (None, None, None));
    // Supported: only listed values; copies clamped into the range.
    let some = Caps {
        sides: listed(&["OneSided", "TwoSidedLongEdge"]),
        orientations: listed(&["Portrait"]),
        qualities: listed(&["Normal"]),
        color_modes: listed(&["Color", "Monochrome"]),
        media_sizes: listed(&["iso_a4_210x297mm"]),
        resolutions: Capability::from_option_list(Some(&[Resolution::square(300)][..])),
        copies: Capability::from_option_range(Some((1, 2))),
        ..Caps::default()
    };
    let mut d = queried_with(with_caps(some));
    set_all(&mut d);
    d.sides = Some(Sides::LongEdge);
    let r = d.request(Some("na_letter_8.5x11in"));
    assert_eq!((r.copies, r.sides, r.orientation, r.quality), (Some(2), Some(Sides::LongEdge), None, None));
    assert_eq!((r.color, r.resolution, r.media_size), (Some(ColorMode::Color), None, None));
}

#[test]
fn states_follow_the_query_and_a_failed_query_is_all_unknown() {
    let mut d = queried_with(one_sided_only());
    assert!(d.sides_state().is_unsupported());
    // A printer that does not publish duplex: a hint is kept and sent.
    requery(&mut d, with_caps(Caps::default()));
    assert!(!d.sides_state().is_known(), "a new answer replaces the old states");
    d.sides = Some(Sides::LongEdge);
    assert_eq!(d.request(None).sides, Some(Sides::LongEdge));
    // Back to a printer that rules it out: cleared.
    requery(&mut d, one_sided_only());
    assert_eq!(d.sides, None);
    d.color = Some(ColorMode::Color);
    let mut mono = printer();
    mono.caps.color_modes = listed(&["Monochrome"]);
    requery(&mut d, mono);
    assert_eq!(d.color, None, "colour is cleared on a black-and-white printer");
    // The details failed: nothing is known, and the old answer's states are gone.
    d.copies = 3;
    let g = d.begin_query();
    d.finish_query(g, "http://192.0.2.5", Err(Failure::new(FailureKind::Transport, "timed out")));
    assert!(d.printer().is_none());
    assert!(!d.sides_state().is_known() && !d.color_state().is_known() && !d.copies_state().is_known());
    assert_eq!(d.request(None), Request::default());
    // Another address: the answer for the first is not applied.
    let mut d = queried_with(one_sided_only());
    d.url = "192.0.2.9".into();
    d.address_edited();
    assert!(!d.sides_state().is_known());
}

#[test]
fn only_the_edges_the_printer_lists_are_offered() {
    let mut p = printer();
    p.caps.sides = listed(&["OneSided", "TwoSidedShortEdge"]);
    let d = queried_with(p);
    assert_eq!(d.offered_sides(), [Sides::OneSided, Sides::ShortEdge]);
}

#[test]
fn status_never_disables_a_control() {
    // A printer that is out of paper with a cover open still has every capability.
    let mut p = printer();
    p.state = "Stopped".into();
    p.reasons = vec!["media-empty".into(), "cover-open".into()];
    let d = queried_with(p);
    assert!(d.sides_state().is_supported() && d.copies_state().is_supported());
    assert_eq!(d.max_copies(), 5);
    let output = drawn(d);
    assert!(center_of(&output, "Gruende: media-empty, cover-open").is_some(), "the reasons are shown as a warning");
    assert!(described(&output, "Vom Drucker nicht bestaetigt").iter().all(|disabled| !disabled), "controls stay enabled");
}

#[test]
fn an_unsupported_control_is_shown_disabled_with_its_reason_as_text_and_description() {
    let output = drawn_with(one_sided_only());
    assert!(center_of(&output, "Zweiseitig").is_some(), "greyed, not hidden");
    assert!(center_of(&output, "Dieser Drucker druckt nur einseitig.").is_some(), "the reason is visible");
    assert_eq!(described(&output, "Dieser Drucker druckt nur einseitig."), [true], "the description is the reason and the control is disabled");
    assert!(described(&output, "Ausrichtung nicht unterstuetzt.").is_empty(), "a supported option has no reason");
}

#[test]
fn an_unknown_control_is_enabled_with_the_note_and_a_supported_one_is_enabled_and_restricted() {
    // printer(): quality, colour and resolution unknown; sides, orientation and copies supported.
    let output = drawn_with(printer());
    let notes = described(&output, "Vom Drucker nicht bestaetigt");
    assert_eq!(notes, [false, false, false], "quality, colour and resolution are enabled with the note");
    assert!(center_of(&output, "Vom Drucker nicht bestaetigt").is_some(), "the note is visible text");
    for label in ["Qualitaet", "Farbe", "Aufloesung", "Zweiseitig", "Ausrichtung", "Kopien"] {
        assert!(center_of(&output, label).is_some(), "{label}");
    }
    assert!(!exposes(&output, "Dieser Drucker druckt nur einseitig."));
}

#[test]
fn all_unsupported_draws_every_reason_on_a_disabled_control() {
    let none = Caps {
        sides: Capability::Unsupported,
        orientations: Capability::Unsupported,
        qualities: Capability::Unsupported,
        color_modes: listed(&["Monochrome"]),
        resolutions: Capability::Unsupported,
        copies: Capability::from_option_range(Some((1, 1))),
        ..Caps::default()
    };
    let output = drawn_with(with_caps(none));
    for reason in [
        "Dieser Drucker druckt nur eine Kopie.",
        "Dieser Drucker druckt nur einseitig.",
        "Ausrichtung nicht unterstuetzt.",
        "Qualitaet nicht unterstuetzt.",
        "Dieser Drucker druckt nur schwarzweiss.",
        "Aufloesung nicht unterstuetzt.",
    ] {
        assert_eq!(described(&output, reason), [true], "{reason}");
    }
    assert!(!exposes(&output, "Vom Drucker nicht bestaetigt"), "nothing is unknown here");
}

#[test]
fn notes_and_option_labels_are_worded_by_the_app() {
    let l = Labels { unconfirmed_note: "", ..labels() };
    assert_eq!(l.note(&Capability::<u8>::Unknown, "x"), None, "an empty note label is omitted");
    assert_eq!(l.note(&Capability::<u8>::Unsupported, "x"), Some("x"));
    assert_eq!(l.note(&Capability::from_option_list(Some(&[1u8][..])), "x"), None);
    assert_eq!(l.resolution_label(Resolution::square(600)), "600 dpi");
    assert_eq!(l.resolution_label(Resolution::new(600, 300)), "600 x 300 dpi");
    assert_eq!((l.color_label(ColorMode::Color), l.color_label(ColorMode::Monochrome)), ("Farbig", "Schwarzweiss"));
}
