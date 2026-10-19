use craft_installer::{WindowsApp, windows_ui};

#[test]
fn every_profile_requests_restart_manager_shutdown_instead_of_restart_only_flags() {
    for app in WindowsApp::ALL {
        let source = windows_ui(app);
        let dialog = source.split("<Dialog Id=\"MsiRMFilesInUse\"").nth(1).unwrap().split("</Dialog>").next().unwrap();
        let ok = dialog.split("<Control Id=\"OK\"").nth(1).unwrap().split("</Control>").next().unwrap();
        assert!(ok.contains(r#"<Publish Event="RMShutdownAndRestart" Value="0" Order="1" />"#), "{} must request application shutdown", app.name());
        assert!(ok.contains(r#"<Publish Event="EndDialog" Value="Return" Order="2" />"#), "{} must return to MSI", app.name());
        assert!(!ok.contains("MSIRMSHUTDOWN"), "{} must not require restart registration before shutdown", app.name());
    }
}
