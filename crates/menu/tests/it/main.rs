//! The integration binary of craft-menu: every test is a registered trial.

#[cfg(target_os = "macos")]
mod appkit;
pub(crate) mod bar;
pub(crate) mod from_items;
pub(crate) mod heading;
pub(crate) mod keytips;
pub(crate) mod layout;
pub(crate) mod memo;
pub(crate) mod mnemonic;
pub(crate) mod native;
pub(crate) mod ribbon;
pub(crate) mod ribbon_gallery;
mod sample;
pub(crate) mod shortcuts;
pub(crate) mod title_bar;

craft_ui::registry!();

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
    #[cfg(feature = "suite")]
    let registry = craft_ui::testing::Registry::of(&[register, craft_menu::testing::register]);
    #[cfg(not(feature = "suite"))]
    let registry = craft_ui::testing::Registry::of(&[register]);
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &registry);
}
