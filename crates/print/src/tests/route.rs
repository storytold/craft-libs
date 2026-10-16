//! The routing between the transports: `lpstat`/`lp` when the CUPS command-line tools are
//! there, then IPP to the reachable server, then the portal. Every environment-dependent route
//! runs in a child process under a controlled `PATH` and a session bus pointed at a socket that
//! does not exist (the environment is process state, and changing it here is forbidden), so no
//! test can reach this host's spooler, CUPS server or portal. A reachable CUPS server cannot be
//! faked — `cups_reachable` probes fixed paths — so the routes that follow it skip when one is
//! really reachable; ipp.rs's fake-server tests are the oracle for the IPP transport itself.

use super::*;
use crate as spool;

/// Set on the child process that re-runs a routing test under the stand-in commands.
const ROUTE_CHILD: &str = "CRAFT_ROUTE_CHILD";

/// A folder of stand-in spooler commands: `name` → the script body under a `#!/bin/sh` shebang.
fn stand_in_dir(commands: &[(&str, &str)]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    use std::os::unix::fs::PermissionsExt;
    for (name, body) in commands {
        let path = dir.path().join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
    }
    let bin = dir.path().to_path_buf();
    (dir, bin)
}

/// Re-runs `name` as a child with `bin` as its `PATH` and the session bus pointed at a socket
/// that does not exist, so routing cannot open this host's print dialog.
fn route_child(name: &str, bin: &std::path::Path) -> std::process::Output {
    let dead_bus = std::env::temp_dir().join(format!("craft-print-no-session-bus-{}", std::process::id()));
    let _ = std::fs::remove_file(&dead_bus);
    std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("PATH", bin)
        .env(ROUTE_CHILD, "1")
        .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", dead_bus.display()))
        .output()
        .unwrap()
}

/// Runs `scenario` when this binary was re-run as a routing child; otherwise runs it as a child
/// under the stand-in environment and asserts that it passed.
fn routed(name: &str, bin: &std::path::Path, scenario: impl FnOnce()) {
    if std::env::var_os(ROUTE_CHILD).is_some() {
        scenario();
        return;
    }
    let out = route_child(name, bin);
    assert!(out.status.success(), "the routing child failed:\n{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

#[test]
fn spawnable_needs_an_executable_file_on_the_search_path() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, bin) = stand_in_dir(&[("lp", ":")]);
    assert!(spool::spawnable_in(bin.as_os_str(), "lp"), "the stand-in is spawnable");
    std::fs::set_permissions(dir.path().join("lp"), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(!spool::spawnable_in(bin.as_os_str(), "lp"), "a file without the execute bits is not");
    std::fs::create_dir(dir.path().join("lpstat")).unwrap();
    assert!(!spool::spawnable_in(bin.as_os_str(), "lpstat"), "a directory is not");
    assert!(!spool::spawnable_in(bin.as_os_str(), "lpoptions"), "a name the path lacks is not");
    assert!(!spool::spawnable_in("".as_ref(), "lp"), "an empty search path spawns nothing");
}

#[test]
fn lpstat_that_answers_wins_over_the_fallbacks() {
    let (_dir, bin) = stand_in_dir(&[(
        "lpstat",
        r#"if [ "$1" = "-e" ]; then echo "Office_Laser"; else echo "printer Office_Laser is idle."; echo "system default destination: Office_Laser"; fi"#,
    )]);
    routed("tests::route::lpstat_that_answers_wins_over_the_fallbacks", &bin, || {
        let printers = spool::printers();
        assert_eq!(printers.len(), 1, "{printers:?}");
        assert_eq!(printers[0], spool::Printer { name: "Office_Laser".into(), default: true, two_sided: None });
    });
}

#[test]
fn lpstat_that_answers_nothing_falls_back_to_the_server() {
    if spool::ipp::cups_reachable() {
        // A real server is reachable: the fallback would ask it, and its queues are not this
        // test's business (ipp.rs tests the server answers). Skip rather than depend on the host.
        return;
    }
    let (_dir, bin) = stand_in_dir(&[(
        "lpstat",
        r#"if [ "$1" = "-e" ]; then exit 0; fi; echo "no system default destination"; echo "lpstat: Scheduler is not running." >&2; exit 1"#,
    )]);
    routed("tests::route::lpstat_that_answers_nothing_falls_back_to_the_server", &bin, || {
        assert!(spool::printers().is_empty(), "a stopped scheduler with no reachable server has no printers");
    });
}

#[test]
fn without_lpstat_the_list_stays_empty_without_a_server() {
    if spool::ipp::cups_reachable() {
        return;
    }
    let (_dir, bin) = stand_in_dir(&[]);
    routed("tests::route::without_lpstat_the_list_stays_empty_without_a_server", &bin, || {
        assert!(spool::printers().is_empty(), "a sandbox without the tools and without a server has no printers");
    });
}

#[test]
fn print_jobs_go_to_lp_when_it_is_there() {
    let (_dir, bin) = stand_in_dir(&[("lp", "PATH=/usr/bin:/bin; export PATH; cat >/dev/null; echo \"request id is Route-1\"")]);
    routed("tests::route::print_jobs_go_to_lp_when_it_is_there", &bin, || {
        let sent = spool::submit_with_progress(b"%PDF-1.7 synthetic routing job\n", &Job::default(), &mut |_, _| {});
        assert_eq!(sent, Ok("request id is Route-1".to_string()), "the stand-in lp spooled the job");
    });
}

#[test]
fn without_a_transport_the_job_is_refused() {
    if spool::ipp::cups_reachable() {
        return;
    }
    let (_dir, bin) = stand_in_dir(&[]);
    routed("tests::route::without_a_transport_the_job_is_refused", &bin, || {
        let err = spool::submit_with_progress(b"%PDF-1.7\n", &Job::default(), &mut |_, _| {}).unwrap_err();
        assert_eq!(err, Error::Spool(spool::PRINTING_UNAVAILABLE.into()), "no spooler, no server, no portal");
    });
}

#[test]
fn the_ipp_preview_describes_the_print_job() {
    let job = Job {
        printer: Some("Office_Laser".into()),
        copies: 2,
        duplex: Some(Duplex::LongEdge),
        grayscale: true,
        scaling: Scaling::ShrinkToFit,
        ..Job::default()
    };
    assert_eq!(
        spool::ipp_preview(&job),
        ["ipp", "printer=Office_Laser", "copies=2", "collate=true", "duplex=two-sided-long-edge", "scaling=shrink-to-fit", "color=monochrome"]
    );
    assert_eq!(
        spool::ipp_preview(&Job::default()),
        ["ipp", "printer=(default)", "copies=1", "collate=true", "duplex=(queue default)", "scaling=exact", "color=color"]
    );
}

#[test]
fn the_portal_preview_points_at_its_dialog() {
    assert_eq!(spool::portal_preview(), ["portal", "printer=(dialog)"]);
}

#[test]
fn the_preview_names_lp_when_it_would_run() {
    let (_dir, bin) = stand_in_dir(&[("lp", ":")]);
    routed("tests::route::the_preview_names_lp_when_it_would_run", &bin, || {
        let preview = spool::command_preview(&Job::default()).unwrap();
        assert_eq!(preview[0], "lp", "{preview:?}");
        assert_eq!(preview[1..], lp_args(&Job::default()));
    });
}

#[test]
fn the_preview_refuses_when_no_transport_exists() {
    if spool::ipp::cups_reachable() {
        return;
    }
    let (_dir, bin) = stand_in_dir(&[]);
    routed("tests::route::the_preview_refuses_when_no_transport_exists", &bin, || {
        assert_eq!(spool::command_preview(&Job::default()), Err(Error::Spool(spool::PRINTING_UNAVAILABLE.into())));
    });
}
