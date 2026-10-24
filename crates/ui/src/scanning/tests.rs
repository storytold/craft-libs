use super::*;
use std::sync::mpsc::{Sender, channel};

type Senders = std::rc::Rc<std::cell::RefCell<Vec<Sender<Result<Choices, String>>>>>;

const BUSY: &str = "The scanner is busy. Close any other program that is using it, then try again.";

/// A scanner with the given id, name and connection.
fn dev(id: &str, name: &str, connection: Connection) -> Scanner {
    Scanner { id: id.into(), name: name.into(), connection }
}

/// A scanner with the given id and an unknown connection.
fn item(id: &str) -> Scanner {
    dev(id, &format!("Scanner {id}"), Connection::Unknown)
}

/// The same device listed twice: over the network (with an address tag) and over USB.
fn pair() -> Vec<Scanner> {
    vec![dev("w", "Example MFC-100 [001122334455]", Connection::Network), dev("u", "Example MFC-100", Connection::Usb)]
}

/// Choices of a fully capable scanner: feeder, glass, duplex, every colour mode and a resolution list.
fn rich() -> Choices {
    Choices {
        feeder: presence(Some(true)),
        glass: presence(Some(true)),
        colors: Capability::from_option_list(Some(&[Color::Gray, Color::Color, Color::BlackAndWhite][..])),
        dpi: Capability::from_option_list(Some(&[75, 150, 200, 300, 600, 1200][..])),
        max_pages: Capability::from_option_range(Some((1, 50))),
        all_pages: presence(Some(true)),
        duplex: presence(Some(true)),
        detect_length: Capability::Unknown,
    }
}

/// A resolution capability listing exactly `list`.
fn dpis(list: &[u32]) -> Capability<u32> {
    Capability::from_option_list(Some(list))
}

/// A colour capability listing exactly `list`.
fn colors(list: &[Color]) -> Capability<Color> {
    Capability::from_option_list(Some(list))
}

/// Runs a complete listing on `state` with `result`.
fn list(state: &mut State, result: Result<Vec<Scanner>, String>) {
    let generation = state.begin_list().unwrap();
    state.finish_list(generation, result.map_err(ListFailure::Backend));
}

/// Selects scanner `id` and finishes its choices read with `result`; true when a read was started.
fn select(state: &mut State, id: &str, result: Result<Choices, String>) -> bool {
    state.settings.scanner = id.into();
    match state.want_details(id) {
        Some(generation) => {
            state.finish_details(generation, id, result);
            true
        }
        None => false,
    }
}

#[test]
fn list_ok_selects_the_first_and_lists_everything() {
    let mut state = State::default();
    list(&mut state, Ok(vec![item("a"), item("b")]));
    assert_eq!(state.settings.scanner, "a");
    assert_eq!(state.scanners().len(), 2);
    assert!(!state.loading() && state.feedback().is_none());
    assert_eq!(state.selected().map(|s| s.id.as_str()), Some("a"));
}

#[test]
fn list_empty_clears_the_selection_and_reports_it() {
    let mut state = State::default();
    state.settings.scanner = "gone".into();
    list(&mut state, Ok(Vec::new()));
    assert!(state.settings.scanner.is_empty());
    assert_eq!(state.feedback(), Some(Feedback::NoScanners));
    assert!(state.selected().is_none() && state.choices().is_none());
    assert!(state.want_details("").is_none(), "no scanner, nothing to load");
}

#[test]
fn list_error_is_reported_and_keeps_the_previous_list() {
    let mut state = State::default();
    list(&mut state, Ok(vec![item("a")]));
    list(&mut state, Err("scanning needs a driver".into()));
    assert_eq!(state.feedback(), Some(Feedback::DiscoveryFailed("scanning needs a driver".into())));
    assert_eq!(state.scanners().len(), 1);
    assert_eq!(state.settings.scanner, "a");
    assert!(!state.loading());
    list(&mut state, Ok(vec![item("a")]));
    assert!(state.feedback().is_none(), "a good listing clears the failure");
}

#[test]
fn only_one_listing_runs_at_a_time_and_stale_results_are_ignored() {
    let mut state = State::default();
    let first = state.begin_list().unwrap();
    assert!(state.begin_list().is_none());
    assert_eq!(state.feedback(), Some(Feedback::FindingScanners));
    state.finish_list(first + 1, Ok(vec![item("a")]));
    assert!(state.scanners().is_empty() && state.loading());
    state.finish_list(first, Ok(vec![item("a")]));
    assert_eq!(state.scanners().len(), 1);
}

#[test]
fn default_prefers_usb_over_its_network_twin_only_when_nothing_is_remembered() {
    assert_eq!(default_scanner_id(&pair()).as_deref(), Some("u"));
    let lone = vec![dev("w", "Example [1]", Connection::Network), dev("u", "Other", Connection::Usb)];
    assert_eq!(default_scanner_id(&lone).as_deref(), Some("w"));
    assert_eq!(default_scanner_id(&[]), None);

    let mut state = State::default();
    state.settings.scanner = "w".into();
    list(&mut state, Ok(pair()));
    assert_eq!(state.settings.scanner, "w", "a remembered scanner that is still listed is kept");
    let mut state = State::default();
    list(&mut state, Ok(pair()));
    assert_eq!(state.settings.scanner, "u");
    let mut state = State::default();
    state.settings.scanner = "gone".into();
    list(&mut state, Ok(pair()));
    assert_eq!(state.settings.scanner, "u", "a vanished selection falls back to the default rule");
    let mut state = State::default();
    state.settings.scanner = "gone".into();
    list(&mut state, Ok(vec![item("a"), item("b")]));
    assert_eq!(state.settings.scanner, "a");
}

#[test]
fn base_names_ignore_a_trailing_bracket_suffix_and_case() {
    assert_eq!(base_name("Example [3c2a]"), "Example");
    assert_eq!(base_name("Example"), "Example");
    assert_eq!(base_name("[x]"), "[x]");
    assert_eq!(base_name("A [b] c"), "A [b] c");
    assert!(same_base_name("EXAMPLE mfc [1]", "example MFC"));
    assert!(!same_base_name(" [1]", " [2]"), "empty base names never match");
}

#[test]
fn same_printer_note_goes_both_ways_and_ignores_non_matches() {
    let refs = pair();
    assert_eq!(same_printer_note(&refs, &refs[0]), Some(SamePrinter::AlsoUsb));
    assert_eq!(same_printer_note(&refs, &refs[1]), Some(SamePrinter::AlsoNetwork));
    let other = vec![dev("a", "Example MFC", Connection::Usb), dev("b", "Epson", Connection::Network), dev("c", "EXAMPLE mfc", Connection::Usb)];
    assert_eq!(same_printer_note(&other, &other[0]), None, "a different connection is required");
    assert_eq!(same_printer_note(&other, &other[1]), None);
    let unknown = vec![dev("a", "Example", Connection::Unknown), dev("b", "example [1]", Connection::Network)];
    assert_eq!(same_printer_note(&unknown, &unknown[0]), None);
    assert_eq!(same_printer_note(&unknown, &unknown[1]), None);
}

#[test]
fn details_ok_gives_the_real_choices_and_clamps_the_settings() {
    let mut state = State::default();
    list(&mut state, Ok(vec![item("a")]));
    state.settings.dpi = 301;
    state.settings.duplex = true;
    assert!(select(&mut state, "a", Ok(Choices { duplex: presence(Some(false)), dpi: dpis(&[100, 200]), ..rich() })));
    assert_eq!(state.settings.dpi, 200);
    assert!(!state.settings.duplex);
    assert!(state.choices().is_some() && !state.checking() && state.feedback().is_none());
}

#[test]
fn details_loading_offers_no_choices_and_does_not_reload() {
    let mut state = State::default();
    list(&mut state, Ok(vec![item("a")]));
    assert!(state.want_details("a").is_some());
    assert!(state.checking());
    assert_eq!(state.feedback(), Some(Feedback::Checking));
    assert!(state.choices().is_none());
    assert!(state.want_details("a").is_none(), "already loading");
}

#[test]
fn busy_scanner_stays_listed_and_selected_with_conservative_choices_and_can_retry() {
    let mut state = State::default();
    list(&mut state, Ok(vec![item("a"), item("b")]));
    assert!(select(&mut state, "a", Err(BUSY.into())));
    assert_eq!(state.scanners().len(), 2);
    assert_eq!(state.settings.scanner, "a");
    assert_eq!(state.details_error(), Some(BUSY));
    assert_eq!(state.feedback(), Some(Feedback::DetailsFailed(BUSY.into())));
    assert_eq!(state.choices(), Some(Choices::default()));
    assert!(state.choices().is_some_and(|c| c.offers(Source::Auto) && c.offers(Source::Feeder) && c.offers(Source::Glass) && c.duplex.is_enabled()));
    let generation = state.retry_details().unwrap();
    assert!(state.checking());
    state.finish_details(generation, "a", Ok(rich()));
    assert!(state.details_error().is_none());
    assert!(state.retry_details().is_none(), "a ready scanner is not retried");
}

#[test]
fn retry_without_a_failure_does_nothing() {
    let mut state = State::default();
    list(&mut state, Ok(vec![item("a")]));
    assert!(state.retry_details().is_none());
}

#[test]
fn stale_details_are_ignored() {
    let mut state = State::default();
    list(&mut state, Ok(vec![item("a"), item("b")]));
    let slow = state.want_details("a").unwrap();
    let fast = state.want_details("b").unwrap();
    state.settings.scanner = "b".into();
    state.finish_details(slow, "a", Ok(rich()));
    assert!(state.checking(), "still waiting for b");
    state.finish_details(slow, "b", Ok(rich()));
    assert!(state.checking(), "same id, older generation");
    state.finish_details(fast, "b", Ok(rich()));
    assert!(matches!(state.details(), DetailsState::Ready(_)));
}

#[test]
fn reselecting_reloads_and_an_unchanged_selection_does_not() {
    let mut state = State::default();
    list(&mut state, Ok(vec![item("a"), item("b")]));
    assert!(select(&mut state, "a", Ok(rich())));
    assert!(select(&mut state, "b", Ok(rich())));
    assert!(state.want_details("b").is_none());
    assert!(state.want_details("a").is_some(), "switching back reads a again");
}

#[test]
fn refresh_resets_the_details() {
    let mut state = State::default();
    list(&mut state, Ok(vec![item("a")]));
    assert!(select(&mut state, "a", Err(BUSY.into())));
    list(&mut state, Ok(vec![item("a")]));
    assert_eq!(*state.details(), DetailsState::Idle);
    assert!(state.choices().is_none());
    assert!(state.want_details("a").is_some());
}

#[test]
fn workers_deliver_through_channels_and_dropped_workers_are_handled() {
    let ctx = egui::Context::default();
    let mut state = State::default();
    let (sender, receiver) = channel();
    let slot = std::cell::RefCell::new(Some(receiver));
    let discover: Discover = Box::new(move || slot.borrow_mut().take().unwrap_or_else(|| channel().1));
    assert!(state.refresh(&discover));
    assert!(!state.refresh(&discover), "a running listing is not restarted");
    state.poll(&ctx);
    assert!(state.loading());
    sender.send(Ok(vec![item("a")])).unwrap();
    state.poll(&ctx);
    assert!(!state.loading());
    assert_eq!(state.settings.scanner, "a");

    let dropped: Discover = Box::new(|| channel().1);
    assert!(state.refresh(&dropped));
    state.poll(&ctx);
    assert_eq!(state.feedback(), Some(Feedback::WorkerStopped));
    assert!(!state.loading());

    let senders = Senders::default();
    let kept = senders.clone();
    let load: LoadChoices = Box::new(move |_| {
        let (sender, receiver) = channel();
        kept.borrow_mut().push(sender);
        receiver
    });
    state.ensure_details(&load);
    assert!(state.checking());
    state.ensure_details(&load);
    assert_eq!(senders.borrow().len(), 1, "one load per selection");
    senders.borrow_mut().clear();
    state.poll(&ctx);
    assert_eq!(*state.details(), DetailsState::Failed(String::new()));
    assert!(state.choices().is_some(), "a stopped worker still leaves the scanner usable");
    assert!(state.retry(&load));
    assert!(!state.retry(&load), "retry only while failed");
    if let Some(sender) = senders.borrow().first() {
        sender.send(Ok(rich())).unwrap();
    }
    state.poll(&ctx);
    assert!(matches!(state.details(), DetailsState::Ready(_)));
}

#[test]
fn a_stale_receiver_is_dropped_when_the_selection_changes() {
    let ctx = egui::Context::default();
    let mut state = State::default();
    list(&mut state, Ok(vec![item("a"), item("b")]));
    let senders = Senders::default();
    let kept = senders.clone();
    let load: LoadChoices = Box::new(move |_| {
        let (sender, receiver) = channel();
        kept.borrow_mut().push(sender);
        receiver
    });
    state.ensure_details(&load);
    state.settings.scanner = "b".into();
    state.ensure_details(&load);
    let both = senders.borrow();
    assert_eq!(both.len(), 2);
    assert!(both[0].send(Ok(rich())).is_err(), "the first worker's receiver is gone");
    both[1].send(Ok(Choices { dpi: dpis(&[100]), ..rich() })).unwrap();
    drop(both);
    state.poll(&ctx);
    assert_eq!(state.settings.dpi, 100);
}

#[test]
fn presets_apply_against_choices() {
    let choices = rich();
    let mut s = Settings::default();
    s.apply_preset(Preset::DocumentText, &choices);
    assert_eq!((s.color, s.dpi, s.preset), (Color::Gray, 300, Some(Preset::DocumentText)));
    s.apply_preset(Preset::ColorDocument, &choices);
    assert_eq!((s.color, s.dpi), (Color::Color, 300));
    s.apply_preset(Preset::BlackAndWhite, &choices);
    assert_eq!((s.color, s.dpi), (Color::BlackAndWhite, 300));
    s.apply_preset(Preset::Photo, &choices);
    assert_eq!((s.color, s.dpi), (Color::Color, 600));
}

#[test]
fn preset_resolution_choices_follow_what_is_listed() {
    let mut s = Settings::default();
    s.apply_preset(Preset::DocumentText, &Choices { dpi: dpis(&[100, 200]), ..Choices::default() });
    assert_eq!(s.dpi, 200);
    s.apply_preset(Preset::DocumentText, &Choices { dpi: dpis(&[150, 1200]), ..Choices::default() });
    assert_eq!(s.dpi, 150, "first listed when neither document value exists");
    s.apply_preset(Preset::Photo, &Choices { dpi: dpis(&[150, 400, 1200]), ..Choices::default() });
    assert_eq!(s.dpi, 400);
    s.apply_preset(Preset::Photo, &Choices { dpi: dpis(&[100, 500, 1200]), ..Choices::default() });
    assert_eq!(s.dpi, 500, "highest up to 600");
    s.apply_preset(Preset::Photo, &Choices { dpi: dpis(&[1200, 2400]), ..Choices::default() });
    assert_eq!(s.dpi, 1200);
    s.dpi = 123;
    s.apply_preset(Preset::Photo, &Choices::default());
    assert_eq!(s.dpi, 123, "no list leaves the resolution alone");
}

#[test]
fn a_missing_colour_mode_falls_back_to_the_first_available() {
    let choices = Choices { colors: colors(&[Color::Gray, Color::BlackAndWhite]), ..Choices::default() };
    let mut s = Settings::default();
    s.apply_preset(Preset::ColorDocument, &choices);
    assert_eq!(s.color, Color::Gray);
    let choices = Choices { colors: colors(&[Color::BlackAndWhite, Color::Gray]), ..Choices::default() };
    s.apply_preset(Preset::Photo, &choices);
    assert_eq!(s.color, Color::BlackAndWhite);
    assert!(!Preset::Photo.offered(&choices) && Preset::BlackAndWhite.offered(&choices));
    assert!(Preset::Photo.offered(&Choices::default()), "unknown colour modes offer every preset");
}

#[test]
fn clamping_brings_settings_into_the_choices() {
    let choices = Choices {
        feeder: presence(Some(true)),
        glass: presence(Some(false)),
        colors: colors(&[Color::BlackAndWhite, Color::Color]),
        dpi: dpis(&[200, 600]),
        max_pages: Capability::from_option_range(Some((1, 5))),
        all_pages: presence(Some(false)),
        duplex: presence(Some(false)),
        detect_length: Capability::Unknown,
    };
    let mut s = Settings { source: Source::Auto, color: Color::Gray, dpi: 300, pages: Pages::All, duplex: true, ..Settings::default() };
    s.clamp(&choices);
    assert_eq!(s.source, Source::Auto, "auto stays: it resolves to the feeder");
    assert_eq!(choices.resolve(s.source), Source::Feeder);
    assert_eq!(s.color, Color::BlackAndWhite);
    assert_eq!(s.dpi, 200);
    assert_eq!(s.pages, Pages::Count(1));
    assert!(!s.duplex);
    s.pages = Pages::Count(40);
    s.clamp(&choices);
    assert_eq!(s.pages, Pages::Count(5));
    s.pages = Pages::Count(0);
    s.clamp(&choices);
    assert_eq!(s.pages, Pages::Count(1));
    let before = s.clone();
    s.clamp(&Choices::default());
    assert_eq!(s, before, "an all-unknown set changes nothing");
    s.source = Source::Glass;
    s.clamp(&choices);
    assert_eq!(s.source, Source::Auto, "an unsupported glass cannot stay chosen");
    let mut s = Settings { pages: Pages::Count(u32::MAX), ..Settings::default() };
    s.clamp(&Choices::default());
    assert_eq!(s.pages, Pages::Count(MAX_PAGES));
}

#[test]
fn the_device_default_leads_the_list_and_is_what_an_unlisted_value_falls_back_to() {
    let choices = Choices { dpi: Capability::list_default_first(&[75, 150, 300], Some(&150)), ..Choices::default() };
    assert_eq!(choices.dpi_options(), vec![150, 75, 300], "default first");
    let mut s = Settings { dpi: 301, ..Settings::default() };
    s.clamp(&choices);
    assert_eq!(s.dpi, 150);
    s.dpi = 75;
    s.clamp(&choices);
    assert_eq!(s.dpi, 75, "a listed value stays");
    let choices = Choices { colors: Capability::list_default_first(&[Color::Gray, Color::Color], Some(&Color::Color)), ..Choices::default() };
    assert_eq!(choices.color_options(), vec![Color::Color, Color::Gray]);
    let mut s = Settings { color: Color::BlackAndWhite, ..Settings::default() };
    s.clamp(&choices);
    assert_eq!(s.color, Color::Color);
}

#[test]
fn a_published_range_offers_the_common_values_inside_it_and_moves_a_value_in() {
    let choices = Choices { dpi: Capability::from_option_range(Some((100, 400))), ..Choices::default() };
    assert_eq!(choices.dpi_options(), vec![100, 150, 200, 300]);
    let mut s = Settings { dpi: 600, ..Settings::default() };
    s.clamp(&choices);
    assert_eq!(s.dpi, 400);
    let odd = Choices { dpi: Capability::from_option_range(Some((110, 130))), ..Choices::default() };
    assert_eq!(odd.dpi_options(), vec![110, 130], "no common value inside: the ends");
}

#[test]
fn unknown_options_offer_the_common_values_and_unsupported_ones_offer_nothing() {
    let unknown = Choices::default();
    assert_eq!(unknown.dpi_options(), COMMON_DPI.to_vec());
    assert_eq!(unknown.color_options(), vec![Color::Gray, Color::Color, Color::BlackAndWhite]);
    let none = Choices { dpi: Capability::Unsupported, colors: Capability::Unsupported, ..Choices::default() };
    assert!(none.dpi_options().is_empty() && none.color_options().is_empty());
    let empty = Choices { dpi: Capability::from_option_list(Some(&[])), ..Choices::default() };
    assert!(empty.dpi.is_unsupported(), "a published empty list is unsupported, not unknown");
    let mut s = Settings::default();
    s.apply_preset(Preset::Photo, &none);
    assert_eq!((s.dpi, s.color), (300, Color::Gray), "unsupported options are left alone");
    assert!(!Preset::Photo.offered(&none));
}

#[test]
fn feeder_controls_apply_unless_glass_is_chosen_or_the_feeder_is_unsupported() {
    let choices = rich();
    assert!(choices.feeder_in_play(Source::Auto) && choices.feeder_in_play(Source::Feeder));
    assert!(!choices.feeder_in_play(Source::Glass));
    let no_glass = Choices { glass: presence(Some(false)), ..rich() };
    assert!(no_glass.feeder_in_play(Source::Auto), "auto is the feeder when there is no glass");
    let no_feeder = Choices { feeder: presence(Some(false)), ..rich() };
    assert!(!no_feeder.feeder_in_play(Source::Auto) && !no_feeder.feeder_in_play(Source::Feeder));
    assert_eq!(no_feeder.resolve(Source::Auto), Source::Glass, "auto falls to the glass");
    assert!(no_feeder.offers(Source::Auto) && no_feeder.offers(Source::Glass) && !no_feeder.offers(Source::Feeder));
    let unknown = Choices { feeder: Capability::Unknown, ..rich() };
    assert!(unknown.feeder_in_play(Source::Auto) && unknown.offers(Source::Feeder), "unknown is enabled");
    let neither = Choices { feeder: Capability::Unsupported, glass: Capability::Unsupported, ..rich() };
    assert!(!neither.offers(Source::Auto) && !neither.offers(Source::Glass));
}

/// The test wording.
fn labels() -> Labels<'static> {
    Labels {
        scanner: "Scanner",
        select_scanner: "Choose",
        refresh: "Neu laden",
        refresh_hint: "Lists scanners",
        source: "Quelle",
        source_auto: "Automatisch",
        source_feeder: "Einzug",
        source_glass: "Glas",
        color: "Farbe",
        color_gray: "Grau",
        color_color: "Farbig",
        color_black_and_white: "Schwarzweiss",
        resolution: "Aufloesung",
        dpi: "dpi",
        pages: "Seiten",
        all_pages: "Alle Seiten",
        duplex: "Duplex",
        duplex_both_sides: "Beidseitig",
        usb: "USB",
        network: "Netzwerk",
        checking: "Pruefe",
        try_again: "Nochmal",
        preset_text: "Text",
        preset_color_document: "Farbdokument",
        preset_black_and_white: "SW",
        preset_photo: "Foto",
        also_usb: "auch USB",
        also_network: "auch Netzwerk",
        unsupported_reason: "Nicht vorhanden.",
        unconfirmed_note: "Nicht bestaetigt.",
        reasons: Reasons { feeder: Some("Kein Einzug."), duplex: Some("Keine Duplexeinheit."), ..Reasons::default() },
        length: LengthLabels {
            heading: "Seitenlaenge",
            driver: "Treiberstandard",
            match_size: "Wie Seitengroesse",
            detect: "Erkennen (Scanner)",
            infer: "Aus Bild erkennen (ungefaehr)",
            no_page_size: "Zuerst eine Seitengroesse waehlen.",
            no_detection: "Dieser Scanner erkennt die Laenge nicht.",
            infer_note: "Aus dem Bild geschaetzt, kann falsch sein.",
        },
        warning_feeder_empty: "Einzug leer.",
        warning_cover_open: "Abdeckung offen.",
        warning_feeder_ready_but_empty: "Einzug bereit, aber leer?",
    }
}

#[test]
fn labels_build_tags_and_notes_without_changing_identity() {
    let labels = labels();
    assert_eq!(labels.device_label(&dev("1", "Scan X", Connection::Usb)), "Scan X  · USB");
    assert_eq!(labels.device_label(&dev("1", "Scan X", Connection::Network)), "Scan X  · Netzwerk");
    assert_eq!(labels.device_label(&dev("1", "Scan X", Connection::Unknown)), "Scan X");
    assert_eq!(labels.note(SamePrinter::AlsoUsb), "auch USB");
    assert_eq!(labels.note(SamePrinter::AlsoNetwork), "auch Netzwerk");
}

/// Discovery and choices workers that answer at once: the pair of scanners, with choices only for the USB one.
fn workers() -> (Discover, LoadChoices) {
    let discover: Discover = Box::new(|| {
        let (sender, receiver) = channel();
        sender.send(Ok(pair())).unwrap();
        receiver
    });
    let load: LoadChoices = Box::new(|id| {
        let (sender, receiver) = channel();
        let result = if id == "u" { Ok(rich()) } else { Err(BUSY.to_string()) };
        sender.send(result).unwrap();
        receiver
    });
    (discover, load)
}

/// Runs one frame of the widget with `events` and returns its response and output.
fn frame(ctx: &egui::Context, state: &mut State, discover: &Discover, load: &LoadChoices, events: Vec<egui::Event>) -> (Response, egui::FullOutput) {
    let mut response = Response::default();
    let output = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(260.0, 520.0))), events, ..Default::default() },
        |ui| response = state.show(ui, egui::Id::new("scan"), &labels(), discover, load),
    );
    let mut output = output;
    output.textures_delta.clear();
    (response, output)
}

#[test]
fn show_lists_selects_loads_details_and_exposes_labelled_controls() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (discover, load) = workers();
    let mut state = State::default();
    let (first, _) = frame(&ctx, &mut state, &discover, &load, Vec::new());
    assert!(first.changed, "the default selection is a change");
    assert_eq!(state.settings.scanner, "u");
    let (_, output) = frame(&ctx, &mut state, &discover, &load, Vec::new());
    assert!(matches!(state.details(), DetailsState::Ready(_)));
    let (_, output2) = frame(&ctx, &mut state, &discover, &load, Vec::new());
    let tree = output2.platform_output.accesskit_update.or(output.platform_output.accesskit_update).unwrap();
    let has = |text: &str| tree.nodes.iter().any(|(_, node)| node.label() == Some(text) || node.value() == Some(text));
    for text in ["Scanner", "Quelle", "Aufloesung", "Farbe", "Seiten", "Alle Seiten", "Beidseitig", "Neu laden", "Foto", "Einzug"] {
        assert!(has(text), "missing {text}");
    }
    assert_eq!(state.settings.scanner, "u", "drawing alone never changes the selection again");
}

#[test]
fn an_unsupported_until_empty_option_is_shown_disabled_not_hidden() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (discover, _) = workers();
    let load: LoadChoices = Box::new(|_| {
        let (sender, receiver) = channel();
        sender.send(Ok(Choices { all_pages: Capability::Unsupported, ..rich() })).unwrap();
        receiver
    });
    let mut state = State::default();
    let mut tree = None;
    for _ in 0..3 {
        let (_, output) = frame(&ctx, &mut state, &discover, &load, Vec::new());
        tree = output.platform_output.accesskit_update.or(tree);
    }
    let tree = tree.expect("an accessibility tree");
    let all = tree.nodes.iter().find(|(_, node)| node.label() == Some("Alle Seiten")).map(|(_, node)| node);
    let all = all.expect("the until-empty checkbox is still there");
    assert!(all.is_disabled(), "shown, but disabled");
}

#[test]
fn show_never_loads_details_for_an_unselected_scanner() {
    let ctx = egui::Context::default();
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let log = asked.clone();
    let discover: Discover = Box::new(|| {
        let (sender, receiver) = channel();
        sender.send(Ok(vec![item("a"), item("b"), item("c")])).unwrap();
        receiver
    });
    let load: LoadChoices = Box::new(move |id| {
        log.borrow_mut().push(id.to_string());
        let (sender, receiver) = channel();
        sender.send(Ok(rich())).unwrap();
        receiver
    });
    let mut state = State::default();
    for _ in 0..4 {
        frame(&ctx, &mut state, &discover, &load, Vec::new());
    }
    assert_eq!(*asked.borrow(), vec!["a".to_string()]);
}

#[test]
fn failed_details_offer_a_retry_button_and_report_it() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (discover, load) = workers();
    let mut state = State::default();
    state.settings.scanner = "w".into();
    for _ in 0..3 {
        frame(&ctx, &mut state, &discover, &load, Vec::new());
    }
    assert_eq!(state.details_error(), Some(BUSY));
    let (_, output) = frame(&ctx, &mut state, &discover, &load, Vec::new());
    let tree = output.platform_output.accesskit_update.unwrap();
    let retry = tree.nodes.iter().find(|(_, node)| node.label() == Some("Nochmal")).unwrap().1.clone();
    let bounds = retry.bounds().unwrap();
    let center = egui::pos2(((bounds.x0 + bounds.x1) / 2.0) as f32, ((bounds.y0 + bounds.y1) / 2.0) as f32);
    let pointer =
        |pressed| egui::Event::PointerButton { pos: center, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&ctx, &mut state, &discover, &load, vec![egui::Event::PointerMoved(center), pointer(true)]);
    let (response, _) = frame(&ctx, &mut state, &discover, &load, vec![pointer(false)]);
    assert!(response.retry);
    assert!(state.checking());
}

// ---- three-state options ------------------------------------------------------------------------

/// The request for `choices` after clamping `settings` to them.
fn request_of(choices: &Choices, mut settings: Settings) -> Request {
    settings.clamp(choices);
    choices.request(&settings)
}

#[test]
fn a_request_sends_supported_values_hints_for_unknown_and_nothing_for_unsupported() {
    let settings = Settings { source: Source::Feeder, color: Color::Color, dpi: 300, pages: Pages::Count(3), duplex: true, ..Settings::default() };

    let r = request_of(&rich(), settings.clone());
    assert_eq!(r.source, Sent::Value(Source::Feeder));
    assert_eq!((r.color, r.dpi), (Sent::Value(Color::Color), Sent::Value(300)));
    assert_eq!((r.pages, r.duplex), (Sent::Value(Pages::Count(3)), Sent::Value(true)));

    let r = request_of(&Choices::default(), settings.clone());
    assert_eq!(r.source, Sent::Hint(Source::Feeder));
    assert_eq!((r.color.clone(), r.dpi.clone()), (Sent::Hint(Color::Color), Sent::Hint(300)));
    assert_eq!((r.pages.clone(), r.duplex.clone()), (Sent::Hint(Pages::Count(3)), Sent::Hint(true)));
    assert!(r.color.is_hint() && r.dpi.value() == Some(&300), "a hint still carries its value");

    let nothing = Choices {
        feeder: Capability::Unsupported,
        colors: Capability::Unsupported,
        dpi: Capability::Unsupported,
        duplex: Capability::Unsupported,
        ..rich()
    };
    let r = request_of(&nothing, settings);
    assert_eq!(r.source, Sent::Value(Source::Glass), "a feeder choice falls back to the glass");
    assert_eq!((r.color, r.dpi, r.pages, r.duplex), (Sent::Omitted, Sent::Omitted, Sent::Omitted, Sent::Omitted));
}

#[test]
fn each_option_is_omitted_when_unsupported_independently() {
    let base = Settings { source: Source::Feeder, pages: Pages::Count(2), duplex: true, ..Settings::default() };
    let r = request_of(&Choices { duplex: Capability::Unsupported, ..rich() }, base.clone());
    assert_eq!((r.duplex, r.dpi.is_hint()), (Sent::Omitted, false));
    let r = request_of(&Choices { dpi: Capability::Unsupported, ..rich() }, base.clone());
    assert!(r.dpi == Sent::Omitted && r.duplex == Sent::Value(true));
    let r = request_of(&Choices { max_pages: Capability::Unsupported, ..rich() }, base.clone());
    assert_eq!(r.pages, Sent::Omitted);
    let r = request_of(&Choices { all_pages: Capability::Unsupported, ..rich() }, Settings { pages: Pages::All, ..base.clone() });
    assert_eq!(r.pages, Sent::Value(Pages::Count(1)), "clamped to a count the device does accept");
    let r = request_of(&Choices { colors: colors(&[Color::BlackAndWhite]), ..rich() }, base.clone());
    assert_eq!(r.color, Sent::Value(Color::BlackAndWhite), "never a value the device did not list");
    let r = request_of(&rich(), Settings { source: Source::Glass, ..base });
    assert_eq!((r.pages, r.duplex), (Sent::Omitted, Sent::Omitted), "feeder options do not apply to the glass");
}

#[test]
fn a_range_value_is_moved_into_the_range_when_sent() {
    let choices = Choices { dpi: Capability::from_option_range(Some((100, 400))), ..Choices::default() };
    let r = choices.request(&Settings { dpi: 600, ..Settings::default() });
    assert_eq!(r.dpi, Sent::Value(400));
    let r = rich().request(&Settings { source: Source::Feeder, pages: Pages::Count(500), ..Settings::default() });
    assert_eq!(r.pages, Sent::Value(Pages::Count(50)));
}

#[test]
fn auto_without_a_feeder_is_the_glass_and_never_asks_for_the_feeder() {
    let no_feeder = Choices { feeder: Capability::Unsupported, ..rich() };
    let r = request_of(&no_feeder, Settings { source: Source::Auto, pages: Pages::All, duplex: true, ..Settings::default() });
    assert_eq!(r.source, Sent::Value(Source::Glass));
    assert_eq!((r.pages, r.duplex), (Sent::Omitted, Sent::Omitted));
    let r = request_of(&rich(), Settings { source: Source::Auto, ..Settings::default() });
    assert_eq!(r.source, Sent::Value(Source::Auto));
    assert_eq!(request_of(&Choices::default(), Settings::default()).source, Sent::Hint(Source::Auto));
}

/// Workers that list one scanner and answer its choices read with `choices`.
fn workers_for(choices: Choices) -> (Discover, LoadChoices) {
    let discover: Discover = Box::new(|| {
        let (sender, receiver) = channel();
        sender.send(Ok(vec![item("a")])).unwrap();
        receiver
    });
    let load: LoadChoices = Box::new(move |_| {
        let (sender, receiver) = channel();
        sender.send(Ok(choices.clone())).unwrap();
        receiver
    });
    (discover, load)
}

/// Draw `choices` (with `warnings`) and return the accessibility tree of the last frame.
fn tree_of(choices: Choices, warnings: Vec<Warning>) -> egui::accesskit::TreeUpdate {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (discover, load) = workers_for(choices);
    let mut state = State::default();
    state.set_warnings(warnings);
    let mut last = None;
    for _ in 0..4 {
        let (_, output) = frame(&ctx, &mut state, &discover, &load, Vec::new());
        last = output.platform_output.accesskit_update.or(last);
    }
    last.unwrap()
}

/// The accessibility nodes whose label or value is `text`.
fn text_nodes(tree: &egui::accesskit::TreeUpdate, text: &str) -> Vec<egui::accesskit::Node> {
    tree.nodes.iter().filter(|(_, node)| node.label() == Some(text) || node.value() == Some(text)).map(|(_, node)| node.clone()).collect()
}

/// Whether some node is labelled `text`.
fn present(tree: &egui::accesskit::TreeUpdate, text: &str) -> bool {
    !text_nodes(tree, text).is_empty()
}

/// Whether the control named `text` is disabled; panics when there is no such control.
fn disabled(tree: &egui::accesskit::TreeUpdate, text: &str) -> bool {
    let nodes = text_nodes(tree, text);
    assert!(!nodes.is_empty(), "no control named {text}");
    nodes.iter().any(|node| node.is_disabled())
}

#[test]
fn an_unsupported_feeder_is_disabled_with_its_reason_and_the_duplex_and_page_rows_stay_visible() {
    let tree = tree_of(Choices { feeder: Capability::Unsupported, duplex: Capability::Unsupported, ..rich() }, Vec::new());
    assert!(disabled(&tree, "Einzug"), "the feeder source is shown, disabled");
    assert!(!disabled(&tree, "Glas") && !disabled(&tree, "Automatisch"), "auto and glass stay available");
    assert!(present(&tree, "Kein Einzug."), "the reason is visible");
    assert!(present(&tree, "Beidseitig") && disabled(&tree, "Beidseitig"), "never silently hidden");
    assert!(present(&tree, "Alle Seiten") && disabled(&tree, "Alle Seiten"));
    assert!(!present(&tree, "Nicht bestaetigt."), "everything else is confirmed");
}

#[test]
fn a_missing_duplex_unit_disables_only_duplex_and_says_why() {
    let tree = tree_of(Choices { duplex: Capability::Unsupported, ..rich() }, Vec::new());
    assert!(disabled(&tree, "Beidseitig") && present(&tree, "Keine Duplexeinheit."));
    assert!(!disabled(&tree, "Alle Seiten") && !disabled(&tree, "Einzug"));
}

#[test]
fn unknown_options_are_enabled_with_a_quiet_note() {
    let tree = tree_of(Choices::default(), Vec::new());
    for text in ["Einzug", "Glas", "Beidseitig", "Alle Seiten", "Foto"] {
        assert!(present(&tree, text) && !disabled(&tree, text), "{text} stays usable");
    }
    assert!(present(&tree, "Nicht bestaetigt."));
    assert!(!present(&tree, "Nicht vorhanden."), "unknown is not unsupported");
    let confirmed = tree_of(rich(), Vec::new());
    assert!(!present(&confirmed, "Nicht bestaetigt.") && !present(&confirmed, "Nicht vorhanden."), "a fully published scanner has no notes");
}

#[test]
fn unsupported_colour_and_resolution_are_disabled_with_the_general_reason() {
    let tree = tree_of(Choices { colors: Capability::Unsupported, dpi: Capability::Unsupported, ..rich() }, Vec::new());
    assert!(present(&tree, "Nicht vorhanden."));
    assert!(disabled(&tree, "Foto"), "a preset needs colour modes; shown disabled, not hidden");
}

#[test]
fn status_warnings_are_drawn_and_never_disable_anything() {
    let warnings = vec![Warning::FeederEmpty, Warning::CoverOpen, Warning::FeederReadyButEmpty];
    let calm = tree_of(rich(), Vec::new());
    let warned = tree_of(rich(), warnings);
    for text in ["Einzug leer.", "Abdeckung offen.", "Einzug bereit, aber leer?"] {
        assert!(present(&warned, text) && !present(&calm, text), "{text}");
    }
    for text in ["Einzug", "Glas", "Automatisch", "Beidseitig", "Alle Seiten", "Foto", "Text"] {
        assert_eq!(disabled(&warned, text), disabled(&calm, text), "{text} is unaffected by status");
        assert!(!disabled(&warned, text));
    }
    // And the request is the same with or without warnings: status is not capability.
    let mut state = State::default();
    state.set_warnings(vec![Warning::CoverOpen]);
    assert_eq!(state.warnings(), [Warning::CoverOpen]);
    let settings = Settings { source: Source::Feeder, ..Settings::default() };
    assert_eq!(rich().request(&settings), request_of(&rich(), settings));
}

// ---- page length ---------------------------------------------------------------------------------

/// Each page-length option with its availability.
fn lengths(choices: &Choices, chosen: bool) -> Vec<(PageLength, LengthAvailability)> {
    choices.length_choices(chosen).iter().map(|c| (c.value, c.availability)).collect()
}

#[test]
fn the_page_length_options_follow_the_three_state_vocabulary() {
    use LengthAvailability::{Available, Unavailable, Unconfirmed};
    use LengthReason::{NoDetection, NoPageSize};
    // Detection not published: enabled, quietly unconfirmed. Match page size needs a page size.
    let unknown = rich();
    assert_eq!(
        lengths(&unknown, false),
        [
            (PageLength::Driver, Available),
            (PageLength::MatchSize, Unavailable(NoPageSize)),
            (PageLength::Detect, Unconfirmed),
            (PageLength::Infer, Available),
        ]
    );
    assert_eq!(lengths(&unknown, true)[1], (PageLength::MatchSize, Available));
    // The scanner published its answer: present is enabled and confirmed; absent is disabled with a reason.
    let present = Choices { detect_length: presence(Some(true)), ..rich() };
    assert_eq!(lengths(&present, true)[2], (PageLength::Detect, Available));
    let absent = Choices { detect_length: presence(Some(false)), ..rich() };
    assert_eq!(lengths(&absent, true)[2], (PageLength::Detect, Unavailable(NoDetection)));
    // Image inference and the driver default never depend on anything.
    for choices in [&unknown, &present, &absent, &Choices::default()] {
        for chosen in [false, true] {
            let all = lengths(choices, chosen);
            assert_eq!(all[0].1, Available);
            assert_eq!(all[3].1, Available);
        }
    }
}

#[test]
fn an_option_that_became_unavailable_falls_back_to_the_driver_default() {
    let absent = Choices { detect_length: presence(Some(false)), ..rich() };
    assert_eq!(absent.length_or_default(PageLength::Detect, true), PageLength::Driver);
    assert_eq!(absent.length_or_default(PageLength::MatchSize, false), PageLength::Driver);
    assert_eq!(absent.length_or_default(PageLength::MatchSize, true), PageLength::MatchSize);
    assert_eq!(absent.length_or_default(PageLength::Infer, false), PageLength::Infer);
    let mut s = Settings { page_length: PageLength::MatchSize, page_size_chosen: false, ..Settings::default() };
    s.clamp(&rich());
    assert_eq!(s.page_length, PageLength::Driver, "the page size was cleared, so the match goes");
    let mut s = Settings { page_length: PageLength::Detect, ..Settings::default() };
    s.clamp(&absent);
    assert_eq!(s.page_length, PageLength::Driver);
    let mut s = Settings { page_length: PageLength::Infer, ..Settings::default() };
    s.clamp(&absent);
    assert_eq!(s.page_length, PageLength::Infer);
}

#[test]
fn the_request_carries_the_page_length_only_for_the_feeder_and_marks_unconfirmed_ones_as_hints() {
    let feeder =
        |length: PageLength, chosen: bool| Settings { source: Source::Feeder, page_length: length, page_size_chosen: chosen, ..Settings::default() };
    let unknown = rich();
    assert_eq!(unknown.request(&feeder(PageLength::Driver, false)).page_length, Sent::Value(PageLength::Driver));
    assert_eq!(unknown.request(&feeder(PageLength::Detect, false)).page_length, Sent::Hint(PageLength::Detect));
    assert_eq!(unknown.request(&feeder(PageLength::Infer, false)).page_length, Sent::Value(PageLength::Infer));
    assert_eq!(unknown.request(&feeder(PageLength::MatchSize, true)).page_length, Sent::Value(PageLength::MatchSize));
    assert_eq!(unknown.request(&feeder(PageLength::MatchSize, false)).page_length, Sent::Omitted);
    let present = Choices { detect_length: presence(Some(true)), ..rich() };
    assert_eq!(present.request(&feeder(PageLength::Detect, false)).page_length, Sent::Value(PageLength::Detect));
    let absent = Choices { detect_length: presence(Some(false)), ..rich() };
    assert_eq!(absent.request(&feeder(PageLength::Detect, false)).page_length, Sent::Omitted);
    let glass = Settings { source: Source::Glass, page_length: PageLength::Infer, ..Settings::default() };
    assert_eq!(unknown.request(&glass).page_length, Sent::Omitted, "the bed has a fixed length");
    assert_eq!(PageLength::default(), PageLength::Driver);
}

#[test]
fn the_length_labels_name_reasons_and_notes() {
    let l = labels().length;
    assert_eq!(l.option(PageLength::MatchSize), "Wie Seitengroesse");
    assert_eq!(l.reason(LengthReason::NoPageSize), "Zuerst eine Seitengroesse waehlen.");
    assert_eq!(l.reason(LengthReason::NoDetection), "Dieser Scanner erkennt die Laenge nicht.");
    // The image guess is always shown with its note; an unconfirmed detection with the quiet one.
    assert_eq!(l.note_for(PageLength::Infer, LengthAvailability::Available, "Nicht bestaetigt."), "Aus dem Bild geschaetzt, kann falsch sein.");
    assert_eq!(l.note_for(PageLength::Detect, LengthAvailability::Unconfirmed, "Nicht bestaetigt."), "Nicht bestaetigt.");
    assert_eq!(l.note_for(PageLength::Detect, LengthAvailability::Available, "Nicht bestaetigt."), "");
    assert_eq!(l.note_for(PageLength::Driver, LengthAvailability::Available, "Nicht bestaetigt."), "");
}

/// Whether a node is labelled `text`.
fn labelled(tree: &egui::accesskit::TreeUpdate, text: &str) -> bool {
    tree.nodes.iter().any(|(_, node)| node.label() == Some(text) || node.value() == Some(text))
}

#[test]
fn the_page_length_row_is_drawn_for_the_feeder_and_hidden_for_the_bed() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (discover, load) = workers();
    let mut state = State::default();
    state.settings.page_size_chosen = true;
    state.settings.source = Source::Feeder;
    let mut tree = None;
    for _ in 0..4 {
        let (_, output) = frame(&ctx, &mut state, &discover, &load, Vec::new());
        tree = output.platform_output.accesskit_update.or(tree);
    }
    let tree = tree.expect("a frame with a tree");
    assert!(labelled(&tree, "Seitenlaenge"), "the caption");
    assert!(labelled(&tree, "Treiberstandard"), "the current choice");
    assert_eq!(state.settings.page_length, PageLength::Driver);

    let mut bed = State::default();
    bed.settings.source = Source::Glass;
    let mut tree = None;
    for _ in 0..4 {
        let (_, output) = frame(&ctx, &mut bed, &discover, &load, Vec::new());
        tree = output.platform_output.accesskit_update.or(tree);
    }
    assert!(!labelled(&tree.expect("tree"), "Seitenlaenge"), "no page-length row on the bed");
}

#[test]
fn drawing_resets_a_choice_the_scanner_or_the_page_size_rules_out() {
    let ctx = egui::Context::default();
    let (discover, load) = workers();
    let mut state = State::default();
    state.settings.source = Source::Feeder;
    state.settings.page_length = PageLength::MatchSize;
    state.settings.page_size_chosen = false;
    for _ in 0..4 {
        frame(&ctx, &mut state, &discover, &load, Vec::new());
    }
    assert_eq!(state.settings.page_length, PageLength::Driver);
    state.settings.page_size_chosen = true;
    state.settings.page_length = PageLength::MatchSize;
    for _ in 0..2 {
        frame(&ctx, &mut state, &discover, &load, Vec::new());
    }
    assert_eq!(state.settings.page_length, PageLength::MatchSize, "kept while a page size is chosen");
}
