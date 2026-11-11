use super::*;
use crate as spool;
use craft_ui::testing::trial;

mod windows_native;

/// Set on the child process when a test runs this test binary as a stand-in for `lp`.
const STAND_IN_LP: &str = "CRAFT_STAND_IN_LP";

/// Not a test of its own: [`stand_in_lp`] re-runs this binary with only this test selected, and it
/// then plays `lp`. It records its arguments and stdin in the folder named by [`STAND_IN_LP`], or,
/// when the folder is `refuse`, exits without reading stdin, the way `lp` refuses an unknown
/// printer.
#[trial]
fn stand_in_for_lp() {
    let Some(record) = std::env::var_os(STAND_IN_LP) else { return };
    if record == "refuse" {
        eprintln!("lp: The printer or class does not exist.");
        std::process::exit(1);
    }
    let record = std::path::PathBuf::from(record);
    let args: Vec<String> = std::env::args().skip_while(|a| a != "--").skip(1).collect();
    let mut stdin = Vec::new();
    std::io::Read::read_to_end(&mut std::io::stdin(), &mut stdin).unwrap();
    std::fs::write(record.join("args"), args.join(" ")).unwrap();
    std::fs::write(record.join("stdin"), stdin).unwrap();
}

/// A spooler command that runs [`stand_in_for_lp`] with `record` (see there).
fn stand_in_lp(record: &std::ffi::OsStr) -> std::process::Command {
    let mut c = std::process::Command::new(std::env::current_exe().unwrap());
    c.args(["--exact", "tests::stand_in_for_lp", "--nocapture", "--"]).env(STAND_IN_LP, record);
    c
}

#[trial]
fn print_jobs_reach_lp_on_stdin_never_through_a_shared_temp_folder() {
    // The job used to be written to `<temp>/craft-print-<pid>/job-<time>.pdf` and handed to `lp`
    // by name. In a shared /tmp another local user can predict that folder, create it first (or
    // plant a symlink there) and so read every printed document or swap the file before `lp`
    // reads it. Piping the job to `lp` leaves nothing on disk.
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let record =
        std::env::temp_dir().join(format!("craft-print-test-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir(&record).unwrap();
    // Bigger than any pipe buffer, so the spooler must read while the job is still being written.
    let pdf: Vec<u8> = b"%PDF-1.7 synthetic print job\n".iter().copied().cycle().take(3 << 20).collect();
    let job = Job { title: "memo.pdf".into(), ..Job::default() };
    let sent = spool::submit_via(stand_in_lp(record.as_os_str()), &pdf, &job, &Cancel::new());
    let args = std::fs::read_to_string(record.join("args"));
    let stdin = std::fs::read(record.join("stdin"));
    let _ = std::fs::remove_dir_all(&record);
    assert!(sent.is_ok(), "{sent:?}");
    // On macOS the job, naming no tray, also asks for Auto Select (storytold/pdfcraft#3752).
    let auto = if cfg!(target_os = "macos") { " -o AP_D_InputSlot=" } else { "" };
    assert_eq!(
        args.unwrap(),
        format!("-n 1 -t memo.pdf -o collate=true -o fit-to-page=false{auto}"),
        "no file argument: lp reads the job from stdin"
    );
    assert!(stdin.unwrap() == pdf, "lp receives the whole job on stdin");
    let predictable = std::env::temp_dir().join(format!("craft-print-{}", std::process::id()));
    assert!(!predictable.exists(), "nothing is created at {predictable:?}, a name other local users can predict");
}

#[trial]
fn a_job_cancels_until_it_starts_reaching_the_spooler() {
    // Pending: cancelling wins, and the job refuses to start sending; cancelling again agrees.
    let pending = Cancel::new();
    assert!(pending.cancel(), "a pending job is never sent");
    assert_eq!(pending.start_sending(), Err(Error::Spool(CANCELLED_MESSAGE.into())));
    assert!(pending.cancel(), "still cancelled");
    // Sending: too late to cancel; a job sent as several copies keeps sending.
    let sending = Cancel::new();
    assert_eq!(sending.start_sending(), Ok(()));
    assert!(!sending.cancel(), "a job being sent completes; the system's queue cancels it");
    assert_eq!(sending.start_sending(), Ok(()), "the next copy of the same job");
    // A clone is the same token: the UI keeps one while the worker submits with the other.
    let worker = Cancel::new();
    let ui = worker.clone();
    assert!(ui.cancel());
    assert!(worker.start_sending().is_err());
}

/// A PPD shaped like a Fiery's: installable options, multi-line PostScript in the choices, Latin-1
/// labels, options the Print dialog sets itself.
const FIERY_PPD: &[u8] = b"*PPD-Adobe: \"4.3\"\n\
*LanguageEncoding: ISOLatin1\n\
*OpenGroup: InstallableOptions/Installable Options\n\
*OpenUI *EFFinisher/Finisher option: PickOne\n\
*DefaultEFFinisher: False\n\
*EFFinisher False/Not installed: \"\"\n\
*EFFinisher SingleStapler/Single stapler: \"\"\n\
*CloseUI: *EFFinisher\n\
*CloseGroup: InstallableOptions\n\
*OpenGroup: FPPaperSource/Media\n\
*OpenUI *InputSlot/Paper tray: PickOne\n\
*OrderDependency: 20.0 AnySetup *InputSlot\n\
*DefaultInputSlot: AutoSelect\n\
*InputSlot AutoSelect/Auto tray select: \"\n\
userdict /XJXEFIsetpageproperties known\n\
{ << /XJXsettrayselV2 [ 7 ] >> XJXEFIsetpageproperties } if\"\n\
*End\n\
*InputSlot ManualFeed/Bypass tray: \"\n\
{ pop 2 XJXsettrayselV2 } if\"\n\
*End\n\
*InputSlot Tray2/Tray 2: \"\"\n\
*fr.InputSlot Tray2/Bac 2: \"\"\n\
*CloseUI: *InputSlot\n\
*OpenUI *EFMediaType/Paper type: PickOne\n\
*DefaultEFMediaType: Plain\n\
*EFMediaType Plain/Plain: \"\"\n\
*EFMediaType Heavy1/Thick 1 (106\xad163 g/m\xb2): \"\"\n\
*CloseUI: *EFMediaType\n\
*CloseGroup: FPPaperSource\n\
*OpenUI *PageSize/Page size: PickOne\n\
*DefaultPageSize: A4\n\
*PageSize A4/A4: \"\"\n\
*CloseUI: *PageSize\n\
*OpenUI *EFRaster/Print queue action: PickOne\n\
*DefaultEFRaster: Bogus\n\
*EFRaster False/Print: \"\"\n\
*EFRaster Hold: \"\"\n\
*CloseUI: *EFRaster\n";

#[trial]
fn printer_options_come_from_the_ppd() {
    let options = spool::parse_ppd(&spool::ppd_text(FIERY_PPD));
    let keys: Vec<&str> = options.iter().map(|o| o.key.as_str()).collect();
    assert_eq!(keys, ["InputSlot", "EFMediaType", "EFRaster"], "no installable options, no PageSize");
    let tray = &options[0];
    assert_eq!((tray.label.as_str(), tray.group.as_str(), tray.default.as_str()), ("Paper tray", "Media", "AutoSelect"));
    assert_eq!(
        tray.choices,
        [("AutoSelect".into(), "Auto tray select".into()), ("ManualFeed".into(), "Bypass tray".into()), ("Tray2".into(), "Tray 2".into())],
        "PostScript lines and translations (*fr.InputSlot) are not choices"
    );
    assert_eq!(options[1].choices[1].1, "Thick 1 (106\u{ad}163 g/m²)", "Latin-1 labels");
    // A default that isn't one of the choices falls back to the first; a choice without a label shows its keyword.
    assert_eq!((options[2].default.as_str(), options[2].choices[1].1.as_str()), ("False", "Hold"));
    assert!(spool::parse_ppd("*OpenUI *Broken\n*Broken A/B\n").is_empty(), "unclosed or malformed blocks are skipped");
}

#[trial]
fn a_job_naming_no_tray_asks_macos_for_auto_select() {
    // storytold/pdfcraft#3752: CUPS gives a job that names no tray the PPD's default tray. A Xerox
    // queue defaulting to an empty Tray 1 kept every job waiting at the printer ("Tray 1 is
    // empty") while CUPS reported it completed; Preview printed, because the macOS Print dialog's
    // Paper Feed: Auto Select sends `AP_D_InputSlot` empty, which drops the PPD's default tray.
    let auto = "-o AP_D_InputSlot=";
    let args = |options: &[(&str, &str)], apple: bool| {
        let job = Job { options: options.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), ..Job::default() };
        spool::lp_args_for(&job, apple).join(" ")
    };
    assert_eq!(args(&[], true), format!("-n 1 -t Craft -o collate=true -o fit-to-page=false {auto}"));
    assert!(args(&[("EFMediaType", "Heavy1")], true).ends_with(&format!("-o EFMediaType=Heavy1 {auto}")), "other options are not a tray");
    // A tray the job names is the tray: CUPS would prefer it to Auto Select anyway.
    assert!(args(&[("InputSlot", "Tray3")], true).ends_with("-o fit-to-page=false -o InputSlot=Tray3"));
    assert!(!args(&[("inputslot", "Tray3")], true).contains("AP_D_InputSlot"), "CUPS option names ignore case");
    assert_eq!(args(&[("AP_D_InputSlot", "Tray3")], true).matches("AP_D_InputSlot").count(), 1, "the caller's own is not doubled");
    // Auto Select is no tray at all, and neither is a value lp can't be given.
    for value in [AUTO_SELECT, "Tray 3"] {
        let a = args(&[("InputSlot", value)], true);
        assert!(a.ends_with(auto) && !a.contains("-o InputSlot="), "{value:?}: {a}");
    }
    // `AP_D_InputSlot` is Apple's: other CUPS gets nothing new.
    assert!(!args(&[], false).contains("AP_D_InputSlot"));
    assert!(!args(&[("InputSlot", AUTO_SELECT)], false).contains("InputSlot"));
    assert_eq!(lp_args(&Job::default()), spool::lp_args_for(&Job::default(), cfg!(target_os = "macos")));
}

#[trial]
fn macos_offers_auto_select_where_the_driver_would_pick_its_default_tray() {
    // The Fiery PPD's own default tray is AutoSelect; its trays are AutoSelect, ManualFeed, Tray2.
    let options = || spool::parse_ppd(&spool::ppd_text(FIERY_PPD));
    let auto = (AUTO_SELECT.to_string(), "Auto Select".to_string());
    let applied = |current: &str, apple: bool| {
        let mut o = options();
        spool::apply_queue_defaults_for(&mut o, &spool::parse_lpoptions(current), apple);
        o
    };
    // The queue's tray is the driver's default: the job names none and lp_args asks for Auto
    // Select, so the dialog shows that, first, and Tray 2 or the driver's default stays a choice.
    for current in ["InputSlot/Paper tray: *AutoSelect ManualFeed Tray2\n", ""] {
        let o = applied(current, true);
        assert_eq!((o[0].default.as_str(), o[0].choices.first()), (AUTO_SELECT, Some(&auto)), "{current:?}");
        assert_eq!(o[0].choices.len(), 4);
    }
    // The user's own default tray (`lpoptions -o InputSlot=Tray2`): lp adds it to every job and it
    // beats Auto Select, so Auto Select would be a choice that does nothing.
    let o = applied("InputSlot/Paper tray: AutoSelect *Tray2 ManualFeed\nEFRaster/Print queue action: *Hold False\n", true);
    assert_eq!(o[0].default, "Tray2");
    assert!(!o[0].choices.contains(&auto));
    assert_eq!(o[2].default, "Hold", "the other options take the queue's defaults as before");
    // A current choice the driver doesn't offer is skipped, so the driver's default stands.
    assert_eq!(applied("InputSlot/Paper tray: *Tray9\n", true)[0].default, AUTO_SELECT);
    // CUPS reads option and choice names without regard to case: a user default spelled otherwise
    // is still Tray 2, kept in the driver's spelling, and still beats Auto Select.
    let o = applied("inputslot/Paper tray: AutoSelect *tray2\nefraster/Print queue action: *HOLD\n", true);
    assert_eq!((o[0].default.as_str(), o[0].choices.contains(&auto), o[2].default.as_str()), ("Tray2", false, "Hold"));
    // Other platforms: the queue's defaults, nothing more.
    assert_eq!(applied("", false)[0], options()[0]);
    assert_eq!(applied("InputSlot/Paper tray: AutoSelect *Tray2\n", false)[0].default, "Tray2");
    // Only the tray gets it, and a driver without one gets none.
    let mut no_tray = spool::parse_ppd(
        "*OpenUI *EFMediaType/Paper type: PickOne\n*DefaultEFMediaType: Plain\n*EFMediaType Plain/Plain: \"\"\n*CloseUI: *EFMediaType\n",
    );
    spool::apply_queue_defaults_for(&mut no_tray, &[], true);
    assert_eq!(no_tray[0].choices, [("Plain".to_string(), "Plain".to_string())]);
}

#[cfg(all(unix, not(target_arch = "wasm32")))]
mod lpstat_limits {
    use super::*;
    use craft_ui::testing::trial;
    use std::process::Command;
    use std::time::{Duration, Instant};

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.args(["-c", script]);
        c
    }

    /// Discovery that hangs only loses the driverless printers; the queues are listed.
    #[trial]
    fn discovery_that_hangs_keeps_the_queues() {
        let mut queues =
            sh("printf 'printer Office_Laser is idle.  enabled since Mon\\n\\tDescription: Office\\nsystem default destination: Office_Laser\\n'");
        let started = Instant::now();
        let found = printers_via(&mut queues, Duration::from_secs(5), &mut sh("sleep 30"), Duration::from_millis(300));
        assert_eq!(found.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Office_Laser"]);
        assert!(found[0].default);
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    }
}
