//! The integration binary of craft-menu: every test is a registered trial.

#[cfg(target_os = "macos")]
mod appkit;
mod layout;
mod mnemonic;
mod native;
mod sample;
mod shortcuts;

#[cfg(target_os = "macos")]
const APPKIT_CHILD: &str = "--craft-menu-appkit-child";

#[cfg(target_os = "macos")]
#[craft_ui::testing::trial]
fn mac_appkit() {
    // AppKit must own the process main thread. The parent remains a normal filterable trial.
    let status = std::process::Command::new(std::env::current_exe().expect("menu test executable"))
        .arg(APPKIT_CHILD)
        .status()
        .expect("start AppKit test process");
    assert!(status.success(), "AppKit test process exited with {status}");
}

fn main() {
    #[cfg(target_os = "macos")]
    if std::env::args_os().skip(1).eq([std::ffi::OsString::from(APPKIT_CHILD)]) {
        return appkit::run();
    }
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_menu::testing::registered());
}
