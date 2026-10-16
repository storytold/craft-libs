//! Shared Windows Installer UI, rendered as checked-in WiX sources.
//!
//! Apps keep their installation policy, existing component identities and custom actions. Release
//! builds consume snapshots, so they need neither a sibling checkout nor a published crate.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// The suite's installer profiles. Options reflect the existing Windows packages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsApp {
    Cadcraft,
    Deckcraft,
    Designcraft,
    Effectcraft,
    Figcraft,
    Filmcraft,
    Gridcraft,
    Lightcraft,
    Pdfcraft,
    Photocraft,
    Soundcraft,
    Vectorcraft,
    Wordcraft,
    Launcher,
}

impl WindowsApp {
    /// Every Windows MSI consumer, including the launcher.
    pub const ALL: [Self; 14] = [
        Self::Cadcraft,
        Self::Deckcraft,
        Self::Designcraft,
        Self::Effectcraft,
        Self::Figcraft,
        Self::Filmcraft,
        Self::Gridcraft,
        Self::Lightcraft,
        Self::Pdfcraft,
        Self::Photocraft,
        Self::Soundcraft,
        Self::Vectorcraft,
        Self::Wordcraft,
        Self::Launcher,
    ];

    /// Repository name, used by the generator's `--app` option.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Cadcraft => "cadcraft",
            Self::Deckcraft => "deckcraft",
            Self::Designcraft => "designcraft",
            Self::Effectcraft => "effectcraft",
            Self::Figcraft => "figcraft",
            Self::Filmcraft => "filmcraft",
            Self::Gridcraft => "gridcraft",
            Self::Lightcraft => "lightcraft",
            Self::Pdfcraft => "pdfcraft",
            Self::Photocraft => "photocraft",
            Self::Soundcraft => "soundcraft",
            Self::Vectorcraft => "vectorcraft",
            Self::Wordcraft => "wordcraft",
            Self::Launcher => "craft-launcher",
        }
    }

    /// Whether the existing installer offers a destination folder picker.
    pub const fn has_folder_picker(self) -> bool {
        matches!(self, Self::Effectcraft | Self::Figcraft | Self::Lightcraft | Self::Photocraft | Self::Vectorcraft | Self::Launcher)
    }

    /// Public property already used by the app's desktop shortcut component.
    pub const fn desktop_property(self) -> Option<&'static str> {
        match self {
            Self::Cadcraft | Self::Effectcraft | Self::Gridcraft => Some("DESKTOPSHORTCUT"),
            Self::Pdfcraft | Self::Vectorcraft | Self::Launcher => Some("INSTALLDESKTOPSHORTCUT"),
            _ => None,
        }
    }

    /// App-owned launch action; retained only where the old installer offered it.
    pub const fn launch_action(self) -> Option<&'static str> {
        match self {
            Self::Figcraft => Some("LaunchFigcraft"),
            Self::Lightcraft => Some("LaunchLightcraft"),
            Self::Photocraft => Some("LaunchPhotocraft"),
            Self::Vectorcraft => Some("LaunchVectorcraft"),
            Self::Wordcraft => Some("LaunchWordcraft"),
            Self::Launcher => Some("LaunchArtCraftLauncher"),
            _ => None,
        }
    }
}

impl std::str::FromStr for WindowsApp {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL.into_iter().find(|app| app.name() == name).ok_or_else(|| format!("unknown Windows app: {name}"))
    }
}

const UI: &str = include_str!("../templates/installer-ui.wxs");
const FOLDER: &str = include_str!("../templates/folder.wxs.in");
const LICENSE: &str = include_str!("../templates/license.wxs.in");
const FOLDER_INSTALL: &str = include_str!("../templates/folder-install.wxs.in");
const SETTINGS: &str = include_str!("../templates/settings.wxs.in");
// WiX's existing path validator is retained for the apps that previously used it.
const PATH_VALID: &str = "WIXUI_DONTVALIDATEPATH OR WIXUI_INSTALLDIR_VALID = &quot;1&quot;";
const LAUNCH_ALLOWED: &str = "NOT Installed AND NOT REMOVE AND NOT REINSTALL AND NOT CraftInstallerOperation AND ACTION &lt;&gt; &quot;ADMIN&quot;";

fn path_validation(order: u8) -> String {
    format!(
        r#"          <Publish Event="DoAction" Value="WixUIValidatePath_$(sys.BUILDARCHSHORT)" Order="{order}" Condition="NOT WIXUI_DONTVALIDATEPATH" />
          <Publish Event="SpawnDialog" Value="CraftInvalidFolder" Order="{}" Condition="NOT WIXUI_DONTVALIDATEPATH AND WIXUI_INSTALLDIR_VALID &lt;&gt; &quot;1&quot;" />"#,
        order + 1
    )
}

/// Renders the common UI with an app's existing folder, shortcut and launch capabilities.
///
/// Desktop choices are explicit 1/0 values before costing, including the elevated execute
/// sequence. Folder selection follows CostFinalize. Success, failure and cancellation have
/// separate screens; full-UI removal reports uninstall completion and never offers launch.
/// Settings uses an app-owned ARP entry selecting full UI; its quiet command selects `/qn`.
/// Basic (`/qb`, `/passive`) and silent (`/qn`) installs and removals remain unattended.
pub fn windows_ui(app: WindowsApp) -> String {
    let folder = app.has_folder_picker();
    let validate_path = folder && app != WindowsApp::Effectcraft;
    let license = app == WindowsApp::Wordcraft;
    let shield = if app == WindowsApp::Launcher { "no" } else { "yes" };
    let folder_install = FOLDER_INSTALL.trim_end();
    let initial_folder_install = format!("{folder_install}\n");
    let mut extra = if folder {
        // EffectCraft locks the destination during upgrades; initially focus an enabled control.
        FOLDER
            .replace("@@INSTALL_BEFORE_FOLDER@@", if app == WindowsApp::Effectcraft { &initial_folder_install } else { "" })
            .replace("@@INSTALL_AFTER_FOLDER@@", if app == WindowsApp::Effectcraft { "" } else { folder_install })
    } else if license {
        LICENSE.to_owned()
    } else {
        String::new()
    };
    extra = extra
        .replace("@@SHIELD@@", shield)
        .replace("@@PATH_VALID@@", if validate_path { PATH_VALID } else { "1" })
        .replace("@@INSTALL_VALIDATION@@", &if validate_path { path_validation(2) } else { String::new() })
        .replace("@@BROWSE_VALIDATION@@", &if validate_path { path_validation(3) } else { String::new() })
        .replace("@@FOLDER_LOCK@@", if app == WindowsApp::Effectcraft { "DisableCondition=\"WIX_UPGRADE_DETECTED\"" } else { "" })
        .replace("@@UPGRADE_NOTE@@", if app == WindowsApp::Effectcraft { r#"        <Control Id="UpgradeNote" Type="Text" X="20" Y="160" Width="330" Height="40" Hidden="yes" ShowCondition="WIX_UPGRADE_DETECTED" Text="This update uses your existing installation folder." />"# } else { "" });
    let (mut shortcut_setup, mut shortcut_control, mut shortcut_events) = if let Some(property) = app.desktop_property() {
        // A private checkbox avoids the MSI convention that an unchecked box clears a public
        // property. Explicit zero preserves opt-out across AppSearch and elevation.
        (format!(r#"    <SetProperty Id="CraftDesktopShortcutChoice" Value="1" Before="InstallWelcome" Sequence="ui"
        Condition="UILevel = 5 AND NOT Installed AND NOT REMOVE AND NOT REINSTALL AND {property} = &quot;1&quot;" />"#),
         r#"        <Control Id="DesktopShortcut" Type="CheckBox" X="20" Y="125" Width="330" Height="20" Property="CraftDesktopShortcutChoice" CheckBoxValue="1" Text="Create a &amp;desktop shortcut" />"#.to_owned(),
         format!(r#"          <Publish Property="{property}" Value="1" Order="1" Condition="CraftDesktopShortcutChoice = &quot;1&quot;" />
          <Publish Property="{property}" Value="0" Order="2" Condition="NOT CraftDesktopShortcutChoice" />"#))
    } else {
        (String::new(), String::new(), String::new())
    };
    if app == WindowsApp::Effectcraft {
        // The elevated execute sequence must retain the explicit choice made in the wizard.
        shortcut_events.push_str(
            r#"
          <Publish Property="CRAFTDESKTOPSHORTCUTSELECTED" Value="1" Order="3" />"#,
        );
    }
    if app == WindowsApp::Pdfcraft {
        // As with desktop choices, publish explicit values so an unchecked option survives elevation.
        shortcut_setup.push_str(
            r#"
    <SetProperty Id="CraftPdfContextMenuChoice" Value="1" Before="InstallWelcome" Sequence="ui"
        Condition="UILevel = 5 AND NOT Installed AND NOT REMOVE AND NOT REINSTALL AND INSTALLPDFCONTEXTMENU = &quot;1&quot;" />"#,
        );
        shortcut_control.push_str(r#"
        <Control Id="PdfContextMenu" Type="CheckBox" X="20" Y="153" Width="330" Height="20" Property="CraftPdfContextMenuChoice" CheckBoxValue="1" Text="Add &quot;&amp;Edit with PdfCraft&quot; to the right-click menu of PDF files" />"#);
        shortcut_events.push_str(
            r#"
          <Publish Property="INSTALLPDFCONTEXTMENU" Value="1" Order="3" Condition="CraftPdfContextMenuChoice = &quot;1&quot;" />
          <Publish Property="INSTALLPDFCONTEXTMENU" Value="0" Order="4" Condition="NOT CraftPdfContextMenuChoice" />"#,
        );
    }
    let (launch_control, launch_event) = if let Some(action) = app.launch_action() {
        (r#"        <Control Id="Launch" Type="CheckBox" X="20" Y="155" Width="330" Height="20" Property="WIXUI_EXITDIALOGOPTIONALCHECKBOX" CheckBoxValue="1" Text="[WIXUI_EXITDIALOGOPTIONALCHECKBOXTEXT]" HideCondition="Installed OR REMOVE OR REINSTALL OR CraftInstallerOperation OR ACTION = &quot;ADMIN&quot;" />"#.to_owned(),
         format!(r#"          <Publish Event="DoAction" Value="{action}" Order="1" Condition="WIXUI_EXITDIALOGOPTIONALCHECKBOX = &quot;1&quot; AND {LAUNCH_ALLOWED}" />"#))
    } else {
        (String::new(), String::new())
    };
    let welcome_event = if license {
        r#"          <Publish Event="NewDialog" Value="InstallLicense" Order="99" />"#
    } else {
        r#"          <Publish Event="EndDialog" Value="Return" Order="99" />"#
    };
    let rendered = UI.replace("@@APP@@", app.name())
        .replace("@@SETTINGS@@", &SETTINGS
            .replace("@@APP@@", app.name())
            .replace("@@REGISTRY_ROOT@@", if app == WindowsApp::Launcher { "HKCU" } else { "HKLM" })
            .replace("@@EXE@@", &if app == WindowsApp::Launcher { "artcraft-launcher.exe".to_owned() } else { format!("{}.exe", app.name()) }))
        .replace("@@SHORTCUT_SETUP@@", &shortcut_setup)
        .replace("@@PATH_ACTION@@", if validate_path { r#"    <CustomActionRef Id="WixUIValidatePath_$(sys.BUILDARCHSHORT)" />"# } else { "" })
        .replace("@@EXTRA_UI@@", &extra)
        .replace("@@SHIELD@@", shield)
        .replace("@@WELCOME_SHIELD@@", if folder || license { "no" } else { shield })
        .replace("@@DESCRIPTION@@", if app == WindowsApp::Launcher { "Setup will install [ProductName] for your Windows account and add it to the Start menu. Click Next to choose its folder." } else if folder { "Setup will install [ProductName] for all users and add it to the Start menu. Click Next to choose its folder." } else if license { "Setup will install [ProductName] for all users and add it to the Start menu. Click Next to review the license." } else { "Setup will install [ProductName] for all users and add it to the Start menu. Click Install to continue." })
        .replace("@@SHORTCUT_CONTROL@@", &shortcut_control)
        .replace("@@SHORTCUT_EVENTS@@", &shortcut_events)
        .replace("@@WELCOME_BUTTON@@", if folder || license { "&amp;Next" } else { "&amp;Install" })
        .replace("@@WELCOME_EVENT@@", welcome_event)
        .replace("@@LAUNCH_CONTROL@@", &launch_control)
        .replace("@@LAUNCH_EVENT@@", &launch_event)
        .replace("@@FOLDER_SHOW@@", if folder { r#"        <Show Dialog="InstallFolder" After="InstallMaintenance" Condition="UILevel = 5 AND NOT Installed AND NOT REMOVE AND NOT REINSTALL" />"# } else { "" });
    rendered.lines().map(str::trim_end).collect::<Vec<_>>().join("\n") + "\n"
}

/// Writes a snapshot to an explicitly chosen output. Never invokes an installer.
pub fn write_windows_ui(app: WindowsApp, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    std::fs::write(path, windows_ui(app))
}

/// Checks a consumer's snapshot without modifying it.
pub fn check_windows_ui(app: WindowsApp, path: impl AsRef<std::path::Path>) -> std::io::Result<bool> {
    Ok(std::fs::read_to_string(path)? == windows_ui(app))
}
