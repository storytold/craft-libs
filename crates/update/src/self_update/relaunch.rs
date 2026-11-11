//! Starting the launcher again after this process exits, including through the host Flatpak.

use super::detect::clear_pretend_version;
#[cfg(target_os = "linux")]
use super::detect::pretend_var;
use super::*;

/// A [`Restart`] that starts `sh`, which waits for `pid` to exit and then runs `command`.
/// Detached, so it outlives us. Nothing starts before the app's exit is certain: a close the user
/// cancels leaves no helper counting down its wait.
#[cfg(unix)]
pub(super) fn relaunch_after_exit(app: &App, pid: u32, command: &[&std::ffi::OsStr]) -> Restart {
    relaunch_with(app, pid, command, process::spawn_detached)
}

/// [`relaunch_after_exit`], with `spawn` starting the helper.
#[cfg(unix)]
pub(super) fn relaunch_with(
    app: &App,
    pid: u32,
    command: &[&std::ffi::OsStr],
    spawn: impl FnOnce(std::process::Command) -> Result<(), String> + Send + 'static,
) -> Restart {
    let cmd = helper_command(app, pid, command);
    Restart::later(move || spawn(cmd).map_err(|e| format!("couldn't schedule the restart ({e})")))
}

#[cfg(unix)]
pub(super) fn helper_command(app: &App, pid: u32, command: &[&std::ffi::OsStr]) -> std::process::Command {
    let mut cmd = process::command("/bin/sh");
    cmd.arg("-c")
        .arg(r#"pid=$1; shift; i=0; while kill -0 "$pid" 2>/dev/null && [ $i -lt 600 ]; do sleep 0.1; i=$((i+1)); done; exec "$@""#)
        .arg("sh")
        .arg(pid.to_string())
        .args(command);
    clear_pretend_version(&mut cmd, app);
    cmd
}

#[cfg(target_os = "linux")]
pub(super) fn parse_flatpak_instance_id(info: &str) -> Option<String> {
    for line in info.lines() {
        if let Some(id) = line.trim().strip_prefix("instance-id=") {
            let trimmed = id.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
pub(super) fn flatpak_instance_id() -> Option<String> {
    let info = std::fs::read_to_string("/.flatpak-info").ok()?;
    parse_flatpak_instance_id(&info)
}

#[cfg(target_os = "linux")]
pub(super) fn parse_flatpak_host_pid(ps_output: &str, target_instance: &str) -> Option<u32> {
    for line in ps_output.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(inst), Some(pid)) = (parts.next(), parts.next())
            && inst == target_instance
        {
            return pid.parse().ok();
        }
    }
    None
}

#[cfg(target_os = "linux")]
pub(super) fn flatpak_host_pid() -> Option<u32> {
    let instance_id = flatpak_instance_id()?;
    let out = process::capture_flatpak(&["ps", "--columns=instance,pid"], std::time::Duration::from_secs(3))?;
    parse_flatpak_host_pid(&out, &instance_id)
}

/// A [`Restart`] that starts a host helper, which waits for this Flatpak instance to exit, then
/// relaunches it via Flatpak on the host. Nothing starts before the app's exit is certain.
#[cfg(target_os = "linux")]
pub(super) fn relaunch_flatpak_after_exit(app: &App) -> Result<Restart, String> {
    let host_pid = flatpak_host_pid();
    let instance_id = flatpak_instance_id();
    // Each wait clause polls until the old process is gone, then execs the relaunch command.
    // If the old process is still alive after 60 s the helper gives up WITHOUT launching a second
    // copy, which is the safe choice: the user can relaunch manually once the old one exits.
    let (wait_clause, target_arg) = match (host_pid, instance_id) {
        (Some(pid), _) => (
            r#"pid=$1; shift; i=0; while kill -0 "$pid" 2>/dev/null; do [ $i -ge 600 ] && exit 1; sleep 0.1; i=$((i+1)); done; exec "$@""#,
            pid.to_string(),
        ),
        (None, Some(id)) => (
            r#"id=$1; shift; i=0; while flatpak ps --columns=instance 2>/dev/null | grep -q "^$id$"; do [ $i -ge 600 ] && exit 1; sleep 0.1; i=$((i+1)); done; exec "$@""#,
            id,
        ),
        // Without either, starting it again could only meet the old copy still running.
        (None, None) => return Err(format!("{} is updated; quit and reopen it to start the new version", app.name)),
    };
    let script = format!("({wait_clause}) &");
    let cmd = flatpak_restart_command(app, &script, &target_arg);
    Ok(Restart::later(move || process::spawn_detached(cmd).map_err(|e| format!("couldn't schedule the Flatpak restart ({e})"))))
}

#[cfg(target_os = "linux")]
pub(super) fn flatpak_restart_command(app: &App, script: &str, target: &str) -> std::process::Command {
    let app_id = app.bundle_id();
    // flatpak run reapplies persistent overrides; removing inherited variables alone is insufficient.
    let global_unset = format!("--unset-env={PRETEND_VAR}");
    let app_unset = format!("--unset-env={}", pretend_var(app));
    let mut cmd = process::command("flatpak-spawn");
    cmd.args(["--host", "sh", "-c", script, "sh", target, "flatpak", "run", &global_unset, &app_unset, &app_id]);
    clear_pretend_version(&mut cmd, app);
    cmd
}
