use super::*;
use craft_ui::testing::trial;

fn findings(libdbus: Option<bool>, file_chooser_portal: Option<bool>, zenity: bool) -> Findings {
    Findings { libdbus, file_chooser_portal, zenity }
}

/// The probe speaks only when no backend can show a dialog: anything it could not tell counts as present, so a
/// working system stays silent.
#[trial]
fn a_dialog_is_explained_only_when_no_backend_can_show_one() {
    let silent = [
        findings(Some(true), Some(true), false),
        findings(Some(false), Some(true), true),
        findings(Some(true), Some(false), true),
        findings(None, None, false),
        findings(Some(true), None, false),
        findings(None, Some(true), false),
    ];
    for found in silent {
        assert_eq!(problem(found), None, "{found:?}");
    }
    assert_eq!(problem(findings(Some(false), Some(true), false)).as_deref(), Some("libdbus is missing"));
    assert_eq!(problem(findings(Some(true), Some(false), false)).as_deref(), Some("no desktop portal offers a file chooser"));
    assert_eq!(problem(findings(None, Some(false), false)).as_deref(), Some("no desktop portal offers a file chooser"));
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
#[trial]
fn the_interface_list_says_whether_a_file_chooser_is_offered() {
    let xml = |name: &str, quote: char| format!("<node><interface name={quote}{name}{quote}><method name=\"OpenFile\"/></interface></node>");
    assert!(interface_listed(&xml("org.freedesktop.portal.FileChooser", '"')));
    assert!(interface_listed(&xml("org.freedesktop.portal.FileChooser", '\'')));
    assert!(!interface_listed(&xml("org.freedesktop.portal.FileChooser2", '"')), "a longer name is another interface");
    assert!(!interface_listed(&xml("org.freedesktop.portal.Settings", '"')));
    assert!(!interface_listed(""));
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
#[trial]
fn dbus_errors_for_a_missing_interface_mean_absent() {
    use zbus::fdo::Error as Fdo;
    let absent = [
        zbus::Error::FDO(Box::new(Fdo::ServiceUnknown("gone".into()))),
        zbus::Error::FDO(Box::new(Fdo::NameHasNoOwner("gone".into()))),
        zbus::Error::FDO(Box::new(Fdo::UnknownObject("gone".into()))),
        zbus::Error::FDO(Box::new(Fdo::InvalidArgs("No such interface".into()))),
    ];
    for error in &absent {
        assert_eq!(classify_error(error), Some(false), "{error}");
    }
    let method = |name: &'static str| {
        let name = zbus::names::OwnedErrorName::try_from(name).expect("a valid error name");
        zbus::Error::MethodError(name, None, zbus::message::Message::method_call("/", "Ping").expect("a call").build(&()).expect("a message"))
    };
    assert_eq!(classify_error(&method("org.freedesktop.DBus.Error.UnknownInterface")), Some(false));
    // Anything else says nothing about the interface, so the probe counts it as present.
    assert_eq!(classify_error(&zbus::Error::FDO(Box::new(Fdo::AccessDenied("no".into())))), None);
    assert_eq!(classify_error(&method("org.freedesktop.DBus.Error.AccessDenied")), None);
}
