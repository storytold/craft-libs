use super::*;
use crate as spool;

mod windows_native;

#[test]
fn spooler_arguments_and_printer_list() {
    let out = "printer Office_Laser is idle.  enabled since Thu Oct  1 09:00:00 2026\nprinter Label_Writer disabled since …\nsystem default destination: Office_Laser\n";
    assert_eq!(
        parse_lpstat(out),
        [
            spool::Printer { name: "Office_Laser".into(), default: true, two_sided: None },
            spool::Printer { name: "Label_Writer".into(), default: false, two_sided: None }
        ]
    );
    assert!(parse_lpstat("lpstat: No destinations added.\nno system default destination\n").is_empty());
    let job = Job {
        printer: Some("Office_Laser".into()),
        copies: 3,
        collate: false,
        duplex: Some(Duplex::LongEdge),
        grayscale: true,
        title: "memo.pdf".into(),
        options: Vec::new(),
        ..Job::default()
    };
    assert_eq!(
        lp_args(&job).join(" "),
        "-d Office_Laser -n 3 -t memo.pdf -o collate=false -o sides=two-sided-long-edge -o print-color-mode=monochrome -o fit-to-page=false"
    );
    assert_eq!(lp_args(&Job::default())[0], "-n", "no -d: the default printer");
}

#[test]
fn lpstat_output_is_untranslated() {
    // A localized lpstat (here Polish) is unreadable to parse_lpstat...
    assert!(parse_lpstat("drukarka Office_Laser jest bezczynna.\ndomyślny cel systemowy: Office_Laser\n").is_empty());
    // ...so the command must force the C locale, including the SOFTWARE switch macOS CUPS needs.
    let cmd = spool::lpstat_command();
    let envs: Vec<_> = cmd.get_envs().map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned()))).collect();
    for key in ["LC_ALL", "LANG"] {
        assert!(envs.contains(&(key.into(), Some("C".into()))), "{key}=C missing: {envs:?}");
    }
    assert!(envs.iter().any(|(k, v)| k == "SOFTWARE" && v.as_deref().is_some_and(|v| !v.is_empty())), "SOFTWARE missing: {envs:?}");
}

#[test]
fn driverless_printers_join_the_list_without_a_queue() {
    // A driverless network printer (IPP, discovered on the LAN) has no queue, so `lpstat -p`
    // never lists it — yet GTK's print dialog shows it and `lp` prints to it through a temporary
    // queue. Only `lpstat -e` names it, and the Print dialog used to show Save as PDF alone.
    let queues = "printer Office_Laser is idle.  enabled since Thu Oct  1 09:00:00 2026\nsystem default destination: Office_Laser\n";
    assert_eq!(
        spool::printers_parsed(queues, Some("Basement_Color\nOffice_Laser\n")),
        [
            spool::Printer { name: "Office_Laser".into(), default: true, two_sided: None },
            spool::Printer { name: "Basement_Color".into(), default: false, two_sided: None },
        ],
        "queues first, then the driverless destination; the queue listed twice is not doubled"
    );
    // An old spooler without `-e` (CUPS < 1.7) keeps its queues; only the driverless names are lost.
    assert_eq!(spool::printers_parsed(queues, None), [spool::Printer { name: "Office_Laser".into(), default: true, two_sided: None }]);
    // The default can itself be driverless (`lpoptions -d` names a destination no queue exists
    // for): it keeps its marker, so the dialog preselects the user's default, not the first queue.
    assert_eq!(
        spool::printers_parsed("system default destination: Basement_Color\n", Some("Basement_Color\n")),
        [spool::Printer { name: "Basement_Color".into(), default: true, two_sided: None }],
        "a default destination without a queue is still the default"
    );
    assert!(spool::parse_lpstat_e("\n \n").is_empty(), "no destinations is empty, not [\"\"]");
    assert_eq!(spool::parse_lpstat_e("A\r\nB \n"), ["A".to_string(), "B".to_string()], "CRLF and stray spaces are trimmed");
}

/// Set on the child process when a test runs this test binary as a stand-in for `lp`.
const STAND_IN_LP: &str = "CRAFT_STAND_IN_LP";

/// Not a test of its own: [`stand_in_lp`] re-runs this binary with only this test selected, and it
/// then plays `lp`. It records its arguments and stdin in the folder named by [`STAND_IN_LP`], or,
/// when the folder is `refuse`, exits without reading stdin, the way `lp` refuses an unknown
/// printer.
#[test]
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

#[test]
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
    let sent = spool::submit_via(stand_in_lp(record.as_os_str()), &pdf, &job);
    let args = std::fs::read_to_string(record.join("args"));
    let stdin = std::fs::read(record.join("stdin"));
    let _ = std::fs::remove_dir_all(&record);
    assert!(sent.is_ok(), "{sent:?}");
    assert_eq!(args.unwrap(), "-n 1 -t memo.pdf -o collate=true -o fit-to-page=false", "no file argument: lp reads the job from stdin");
    assert!(stdin.unwrap() == pdf, "lp receives the whole job on stdin");
    let predictable = std::env::temp_dir().join(format!("craft-print-{}", std::process::id()));
    assert!(!predictable.exists(), "nothing is created at {predictable:?}, a name other local users can predict");
}

#[test]
fn a_refused_print_job_reports_the_spoolers_message() {
    // `lp` refuses an unknown printer without reading the job; the user sees why, not a broken pipe.
    let pdf = vec![b'%'; 3 << 20];
    let err = spool::submit_via(stand_in_lp("refuse".as_ref()), &pdf, &Job::default());
    assert_eq!(err, Err(Error::Spool("lp: The printer or class does not exist.".into())));
}

#[test]
fn the_spoolers_reply_drops_the_file_count_of_a_job_sent_on_stdin() {
    assert_eq!(
        spool::job_message(
            b"request id is Office_Laser-12 (0 file(s))
"
        ),
        "request id is Office_Laser-12"
    );
    assert_eq!(
        spool::job_message(
            b"request id is Office_Laser-13 (1 file(s))
"
        ),
        "request id is Office_Laser-13 (1 file(s))"
    );
    assert_eq!(spool::job_message(b""), "");
}

#[test]
fn windows_printers_carry_one_default_and_keep_their_names() {
    // Windows marks the system default with a `*` no queue name may contain; names hold spaces,
    // parentheses and non-ASCII letters, and the lines end with CRLF.
    let printers = parse_windows_printers("*Microsoft Print to PDF\r\nHP LaserJet 400 (Copy 2)\r\nКопир Canon iR\r\n\r\n*XPS via USB\r\n");
    assert_eq!(printers[0], Printer { name: "Microsoft Print to PDF".into(), default: true, two_sided: None });
    assert_eq!(printers[1], Printer { name: "HP LaserJet 400 (Copy 2)".into(), default: false, two_sided: None });
    assert_eq!(printers[2], Printer { name: "Копир Canon iR".into(), default: false, two_sided: None });
    // A spooler marking two defaults: the first still wins, so the dialog has one to preselect.
    assert_eq!(printers[3], Printer { name: "XPS via USB".into(), default: false, two_sided: None });
    assert_eq!(parse_windows_printers("\r\n"), Vec::new());
    let sided = parse_windows_printers("*Office PCL\t2\r\nOffice PS\t1\r\nTab\tName\t2\r\n");
    assert_eq!(sided[0], Printer { name: "Office PCL".into(), default: true, two_sided: Some(true) });
    assert_eq!(sided[1], Printer { name: "Office PS".into(), default: false, two_sided: Some(false) });
    assert_eq!(sided[2], Printer { name: "Tab\tName".into(), default: false, two_sided: Some(true) });
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

#[test]
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

#[test]
fn printer_option_defaults_and_job_arguments() {
    let current =
        spool::parse_lpoptions("InputSlot/Paper tray: AutoSelect *Tray2 ManualFeed\nEFRaster/Print queue action: *False Hold\nbroken line\n");
    assert_eq!(current, [("InputSlot".to_string(), "Tray2".to_string()), ("EFRaster".to_string(), "False".to_string())]);
    let job = Job {
        options: vec![
            ("InputSlot".into(), "Tray2".into()),
            ("EFMediaType".into(), "Heavy1".into()),
            ("Duplex".into(), "DuplexTumble".into()),
            ("Bad Key".into(), "x".into()),
            ("EFRaster".into(), "a=b".into()),
        ],
        ..Job::default()
    };
    let args = lp_args(&job).join(" ");
    assert!(args.ends_with("-o fit-to-page=false -o InputSlot=Tray2 -o EFMediaType=Heavy1"), "{args}");
    assert!(spool::printer_options("../../etc/passwd").is_empty(), "not a queue name");
}

#[test]
fn command_previews_do_not_require_pdf_files() {
    let job = Job { printer: Some("Office (east)".into()), copies: 2, ..Job::default() };
    let preview = command_preview(&job).unwrap();
    if backend() == Some(Backend::Cups) {
        assert_eq!(preview[0], "lp");
        assert_eq!(preview[1..], lp_args(&job));
    } else {
        assert_eq!(
            preview,
            ["xps-print", "printer=Office (east)", "copies=2", "collate=true", "duplex=(queue default)", "scaling=exact", "color=color"]
        );
    }
}

#[test]
fn queue_duplex_defaults_and_explicit_scaling_reach_the_spooler() {
    let default_args = lp_args(&Job::default());
    assert!(!default_args.iter().any(|a| a.starts_with("sides=")));
    let simplex = lp_args(&Job { duplex: Some(Duplex::Off), ..Job::default() });
    assert!(simplex.iter().any(|a| a == "sides=one-sided"));
    let shrink = lp_args(&Job { scaling: Scaling::ShrinkToFit, ..Job::default() });
    assert!(shrink.iter().any(|a| a == "print-scaling=auto-fit"));
    assert!(!shrink.iter().any(|a| a.starts_with("fit-to-page=")));
}
