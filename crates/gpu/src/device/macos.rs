//! macOS: wgpu's indirect-call validation, left out on Intel Macs.

/// wgpu compiles its indirect-call validation compute shaders inside `request_device`. On old
/// Intel Macs (HD Graphics 5000 on Big Sur, #2652) Metal's Intel
/// compiler aborts on them, the device is lost and eframe exits before the window opens. Craft apps
/// issue no indirect draws or dispatches, so Intel Macs skip them; every other GPU keeps wgpu's
/// default. Same fix as PhotoCraft's storytold/PhotoCraft#3171.
///
/// `x86_64_build` is the build's architecture. An x86_64 build also runs on Apple silicon under Rosetta
/// (#5199), so it skips only when the machine itself (`apple_silicon_host`) isn't Apple silicon.
pub(crate) fn skips_indirect_validation(x86_64_build: bool, apple_silicon_host: bool) -> bool {
    x86_64_build && !apple_silicon_host
}

/// Whether the machine is Apple silicon, also when this process is translated by Rosetta:
/// `hw.optional.arm64` describes the hardware, not the process. Assumes not (an Intel Mac) when
/// the system can't tell.
#[cfg(target_os = "macos")]
fn runs_on_apple_silicon() -> bool {
    std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.optional.arm64"])
        .output()
        .is_ok_and(|out| out.status.success() && out.stdout.trim_ascii() == b"1")
}

/// Leaves wgpu's indirect-call validation out of the instance eframe creates on an Intel Mac
/// ([`skips_indirect_validation`]). `WGPU_VALIDATION_INDIRECT_CALL=1` still turns it back on.
#[cfg(target_os = "macos")]
pub(crate) fn configure(setup: &mut egui_wgpu::WgpuSetupCreateNew) {
    let x86_64_build = cfg!(target_arch = "x86_64");
    // An arm64 build runs only on Apple silicon: only an x86_64 build asks the system.
    let apple_silicon_host = !x86_64_build || runs_on_apple_silicon();
    if !skips_indirect_validation(x86_64_build, apple_silicon_host) {
        return;
    }
    let flags = &mut setup.instance_descriptor.flags;
    *flags = flags.difference(egui_wgpu::wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL).with_env();
}
