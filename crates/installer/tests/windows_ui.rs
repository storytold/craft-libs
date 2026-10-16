use craft_installer::{WindowsApp, windows_ui};
use roxmltree::Document;

#[test]
fn full_ui_has_distinct_terminal_screens_in_both_sequences() {
    let source = windows_ui(WindowsApp::Designcraft);
    let doc = Document::parse(&source).unwrap();
    for sequence in ["InstallUISequence", "AdminUISequence"] {
        let sequence = doc.descendants().find(|n| n.has_tag_name(sequence)).unwrap();
        let endings: Vec<_> = sequence.children().filter(|n| n.attribute("OnExit").is_some()).collect();
        assert_eq!(endings.len(), 3);
        for (outcome, dialog) in [("success", "InstallComplete"), ("cancel", "InstallCancelled"), ("error", "InstallFailed")] {
            let show = endings.iter().find(|n| n.attribute("OnExit") == Some(outcome)).unwrap();
            assert_eq!(show.attribute("Condition"), Some("UILevel = 5"), "basic and silent installs must stay unattended");
            assert_eq!(show.attribute("Dialog"), Some(dialog));
            let dialog = doc.descendants().find(|n| n.has_tag_name("Dialog") && n.attribute("Id") == Some(dialog)).unwrap();
            let finish = dialog.children().find(|n| n.attribute("Id") == Some("Finish")).unwrap();
            assert_eq!(finish.attribute("Default"), Some("yes"));
            assert_eq!(finish.attribute("Cancel"), Some("yes"));
            assert!(finish.children().any(|n| n.attribute("Event") == Some("EndDialog") && n.attribute("Value") == Some("Return")));
        }
    }
}

#[test]
fn removal_and_repair_do_not_display_the_install_message() {
    let source = windows_ui(WindowsApp::Designcraft);
    let doc = Document::parse(&source).unwrap();
    let dialog = doc.descendants().find(|n| n.has_tag_name("Dialog") && n.attribute("Id") == Some("InstallComplete")).unwrap();
    let control = |id| dialog.children().find(|n| n.attribute("Id") == Some(id)).unwrap();
    assert_eq!(
        control("Description").attribute("HideCondition"),
        Some("REMOVE = \"ALL\" OR REINSTALL OR CraftInstallerOperation OR ACTION = \"ADMIN\"")
    );
    assert_eq!(control("Removed").attribute("Hidden"), Some("yes"));
    assert_eq!(control("Removed").attribute("ShowCondition"), Some("REMOVE = \"ALL\" OR CraftInstallerOperation = \"remove\""));
    assert!(control("Removed").attribute("Text").unwrap().contains("was uninstalled"));
    assert_eq!(
        control("Repaired").attribute("ShowCondition"),
        Some("(REINSTALL OR CraftInstallerOperation = \"repair\") AND NOT REMOVE AND CraftInstallerOperation <> \"remove\" AND ACTION <> \"ADMIN\"")
    );
    assert!(control("Repaired").attribute("Text").unwrap().contains("was repaired"));
}

#[test]
fn every_authored_dialog_target_resolves_and_msi_support_dialogs_exist() {
    let source = windows_ui(WindowsApp::Designcraft);
    let doc = Document::parse(&source).unwrap();
    let exists = |id| doc.descendants().any(|n| n.has_tag_name("Dialog") && n.attribute("Id") == Some(id));
    for node in doc.descendants() {
        if node.has_tag_name("Show") {
            assert!(exists(node.attribute("Dialog").unwrap()));
        }
        if matches!(node.attribute("Event"), Some("SpawnDialog" | "NewDialog")) {
            assert!(exists(node.attribute("Value").unwrap()));
        }
    }
    for id in ["CraftInstallerErrorDlg", "FilesInUse", "MsiRMFilesInUse"] {
        assert!(exists(id), "missing Windows Installer support dialog {id}");
    }
    let error = doc.descendants().find(|n| n.has_tag_name("Dialog") && n.attribute("Id") == Some("CraftInstallerErrorDlg")).unwrap();
    let icon = error.children().find(|n| n.attribute("Id") == Some("ErrorIcon")).unwrap();
    assert_eq!(icon.attribute("Type"), Some("Icon"));
    assert_eq!(icon.attribute("FixedSize"), Some("yes"));
    assert_eq!(icon.attribute("Text"), Some("CraftInstallerErrorIcon"));
    assert!(doc.descendants().any(|n| n.has_tag_name("Binary")
        && n.attribute("Id") == Some("CraftInstallerErrorIcon")
        && n.attribute("SourceFile") == Some("$(var.IconPath)")));
    assert_eq!(icon.attribute("IconSize"), Some("32"));
    assert!(doc.descendants().any(|n| n.has_tag_name("EnsureTable") && n.attribute("Id") == Some("ListBox")));
    // Dialogs stay separate from the Settings registration components.
    let ui = doc.descendants().find(|n| n.has_tag_name("UI")).unwrap();
    for node in ui.descendants() {
        assert!(!matches!(node.tag_name().name(), "Component" | "RegistryValue" | "CustomAction" | "File"));
    }
}

#[test]
fn all_profiles_preserve_the_existing_options_and_launch_guards() {
    // Independent inventory of the released installers: folder, desktop choice, launch choice.
    let expected = [
        (WindowsApp::Cadcraft, false, Some("DESKTOPSHORTCUT"), None),
        (WindowsApp::Deckcraft, false, None, None),
        (WindowsApp::Designcraft, false, None, None),
        (WindowsApp::Effectcraft, true, Some("DESKTOPSHORTCUT"), None),
        (WindowsApp::Figcraft, true, None, Some("LaunchFigcraft")),
        (WindowsApp::Filmcraft, false, None, None),
        (WindowsApp::Gridcraft, false, Some("DESKTOPSHORTCUT"), None),
        (WindowsApp::Lightcraft, true, None, Some("LaunchLightcraft")),
        (WindowsApp::Pdfcraft, false, Some("INSTALLDESKTOPSHORTCUT"), None),
        (WindowsApp::Photocraft, true, None, Some("LaunchPhotocraft")),
        (WindowsApp::Soundcraft, false, None, None),
        (WindowsApp::Vectorcraft, true, Some("INSTALLDESKTOPSHORTCUT"), Some("LaunchVectorcraft")),
        (WindowsApp::Wordcraft, false, None, Some("LaunchWordcraft")),
        (WindowsApp::Launcher, true, Some("INSTALLDESKTOPSHORTCUT"), Some("LaunchArtCraftLauncher")),
    ];
    assert_eq!(expected.len(), WindowsApp::ALL.len());
    for (app, folder, desktop, launch) in expected {
        let source = windows_ui(app);
        assert!(!source.contains("@@"), "unexpanded template in {}", app.name());
        let doc = Document::parse(&source).unwrap();
        let dialog = |id| doc.descendants().find(|n| n.has_tag_name("Dialog") && n.attribute("Id") == Some(id));
        assert_eq!(dialog("InstallFolder").is_some(), folder, "{} folder option", app.name());
        let complete = dialog("InstallComplete").unwrap();
        let first_focusable = complete
            .children()
            .find(|n| {
                n.has_tag_name("Control") && matches!(n.attribute("Type"), Some("PushButton" | "CheckBox")) && n.attribute("TabSkip") != Some("yes")
            })
            .unwrap();
        assert_eq!(first_focusable.attribute("Id"), Some("Finish"), "repair/removal must not initially focus the hidden Launch control");
        if app.has_folder_picker() {
            let folder_dialog = doc.descendants().find(|n| n.has_tag_name("Dialog") && n.attribute("Id") == Some("InstallFolder")).unwrap();
            let first = folder_dialog
                .children()
                .find(|n| matches!(n.attribute("Type"), Some("PathEdit" | "PushButton")) && n.attribute("TabSkip") != Some("yes"))
                .unwrap();
            assert_eq!(first.attribute("Id"), Some(if app == WindowsApp::Effectcraft { "Install" } else { "Folder" }));
        }
        let control = |id| doc.descendants().find(|n| n.has_tag_name("Control") && n.attribute("Id") == Some(id));
        assert_eq!(control("DesktopShortcut").is_some(), desktop.is_some());
        assert_eq!(control("Launch").is_some(), launch.is_some());
        let welcome = doc.descendants().find(|n| n.has_tag_name("Show") && n.attribute("Dialog") == Some("InstallWelcome")).unwrap();
        assert_eq!(welcome.attribute("Before"), Some("CostInitialize"));
        // Repair and Remove publish feature events: the selection manager must already exist.
        let maintenance = doc.descendants().find(|n| n.has_tag_name("Show") && n.attribute("Dialog") == Some("InstallMaintenance")).unwrap();
        assert_eq!(maintenance.attribute("After"), Some("CostFinalize"));
        for (button, operation) in [("Repair", "repair"), ("Remove", "remove")] {
            assert!(control(button).unwrap().children().any(|n| n.attribute("Property") == Some("CraftInstallerOperation")
                && n.attribute("Value") == Some(operation)
                && n.attribute("Order") == Some("1")));
        }
        if let Some(property) = desktop {
            for (value, condition) in [("1", "CraftDesktopShortcutChoice = \"1\""), ("0", "NOT CraftDesktopShortcutChoice")] {
                assert!(doc.descendants().any(|n| n.has_tag_name("Publish")
                    && n.attribute("Property") == Some(property)
                    && n.attribute("Value") == Some(value)
                    && n.attribute("Condition") == Some(condition)));
            }
        }
        if let Some(action) = launch {
            let event = doc.descendants().find(|n| n.has_tag_name("Publish") && n.attribute("Value") == Some(action)).unwrap();
            assert_eq!(event.parent().unwrap().attribute("Id"), Some("Finish"));
            let condition = event.attribute("Condition").unwrap();
            for guard in ["WIXUI_EXITDIALOGOPTIONALCHECKBOX = \"1\"", "NOT Installed", "NOT REMOVE", "NOT REINSTALL", "ACTION <> \"ADMIN\""] {
                assert!(condition.contains(guard), "{} launch missing {guard}", app.name());
            }
            assert!(control("Launch").unwrap().attribute("HideCondition").unwrap().contains("REMOVE"));
        }
        if folder {
            let show = doc.descendants().find(|n| n.has_tag_name("Show") && n.attribute("Dialog") == Some("InstallFolder")).unwrap();
            assert_eq!(show.attribute("After"), Some("InstallMaintenance"));
            assert!(show.attribute("Condition").unwrap().contains("NOT REMOVE AND NOT REINSTALL"));
            let edit = control("Folder").unwrap();
            assert_eq!(edit.attribute("DisableCondition"), if app == WindowsApp::Effectcraft { Some("WIX_UPGRADE_DETECTED") } else { None });
            assert_eq!(source.contains("WixUIValidatePath_"), app != WindowsApp::Effectcraft);
        }
        assert_eq!(dialog("InstallLicense").is_some(), app == WindowsApp::Wordcraft);
        let repair = control("Repair").unwrap();
        assert_eq!(repair.attribute("ElevationShield"), Some(if app == WindowsApp::Launcher { "no" } else { "yes" }));
        for node in doc.descendants() {
            if node.has_tag_name("Show") {
                assert!(node.attribute("Condition").unwrap().contains("UILevel = 5"));
                assert!(dialog(node.attribute("Dialog").unwrap()).is_some());
            }
            if matches!(node.attribute("Event"), Some("SpawnDialog" | "NewDialog")) {
                assert!(dialog(node.attribute("Value").unwrap()).is_some());
            }
            assert!(!matches!(node.tag_name().name(), "CustomAction" | "File"));
        }
        for sequence in ["InstallUISequence", "AdminUISequence"] {
            let sequence = doc.descendants().find(|n| n.has_tag_name(sequence)).unwrap();
            assert_eq!(sequence.children().filter(|n| n.attribute("OnExit").is_some()).count(), 3);
        }
    }
}

#[test]
fn settings_removal_uses_full_ui_and_keeps_quiet_removal_separate() {
    for app in WindowsApp::ALL {
        let source = windows_ui(app);
        let doc = Document::parse(&source).unwrap();
        let group = doc.descendants().find(|n| n.attribute("Id") == Some("CraftInstallerSettings")).unwrap();
        let entry = group.children().find(|n| n.attribute("Id") == Some("CraftInstallerSettingsEntry")).unwrap();
        let key = entry.children().find(|n| n.has_tag_name("RegistryKey")).unwrap();
        assert_eq!(key.attribute("Root"), Some(if app == WindowsApp::Launcher { "HKCU" } else { "HKLM" }));
        // A stable key path keeps component identity valid across ProductCode changes.
        assert_eq!(key.attribute("Key"), Some(format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Craft.{}", app.name()).as_str()));
        assert_eq!(key.attribute("ForceDeleteOnUninstall"), Some("yes"));
        let value = |name| key.children().find(|n| n.attribute("Name") == Some(name)).unwrap();
        assert_eq!(value("DisplayName").attribute("KeyPath"), Some("yes"));
        assert_eq!(
            value("DisplayName").attribute("Value"),
            Some("[ProductName] (Craft Apps)"),
            "legacy Launcher must not mistake the Settings alias for an MSI product"
        );
        assert_eq!(value("UninstallString").attribute("Value"), Some("\"[$(var.CraftMsiSystemFolder)]msiexec.exe\" /x [ProductCode] /qf /norestart"));
        assert_eq!(
            value("QuietUninstallString").attribute("Value"),
            Some("\"[$(var.CraftMsiSystemFolder)]msiexec.exe\" /x [ProductCode] /qn /norestart")
        );
        assert_eq!(value("ModifyPath").attribute("Value"), Some("\"[$(var.CraftMsiSystemFolder)]msiexec.exe\" /i [ProductCode] /qf /norestart"));
        assert!(!key.children().any(|n| n.attribute("Name") == Some("WindowsInstaller")));
        assert!(
            doc.descendants()
                .any(|n| n.has_tag_name("Property") && n.attribute("Id") == Some("ARPSYSTEMCOMPONENT") && n.attribute("Value") == Some("1"))
        );
        let policy = group.children().find(|n| n.attribute("Id") == Some("CraftInstallerSettingsNoModify")).unwrap();
        assert_eq!(policy.attribute("Condition"), Some("ARPNOMODIFY = \"1\""));
        // Registration contains no new executable payload or privileged custom action.
        assert!(!group.descendants().any(|n| matches!(n.tag_name().name(), "File" | "CustomAction")));
    }
}

#[test]
fn pdf_context_menu_choice_preserves_default_and_opt_out_through_elevation() {
    for app in WindowsApp::ALL {
        let source = windows_ui(app);
        let doc = Document::parse(&source).unwrap();
        let control = doc.descendants().find(|n| n.has_tag_name("Control") && n.attribute("Id") == Some("PdfContextMenu"));
        assert_eq!(control.is_some(), app == WindowsApp::Pdfcraft);
        if let Some(control) = control {
            assert_eq!(control.attribute("Property"), Some("CraftPdfContextMenuChoice"));
            assert_eq!(control.attribute("CheckBoxValue"), Some("1"));
            assert!(doc.descendants().any(|n| n.has_tag_name("SetProperty")
                && n.attribute("Id") == Some("CraftPdfContextMenuChoice")
                && n.attribute("Condition").unwrap().contains("INSTALLPDFCONTEXTMENU = \"1\"")));
            for (value, condition) in [("1", "CraftPdfContextMenuChoice = \"1\""), ("0", "NOT CraftPdfContextMenuChoice")] {
                assert!(doc.descendants().any(|n| n.has_tag_name("Publish")
                    && n.attribute("Property") == Some("INSTALLPDFCONTEXTMENU")
                    && n.attribute("Value") == Some(value)
                    && n.attribute("Condition") == Some(condition)));
            }
        }
    }
}
