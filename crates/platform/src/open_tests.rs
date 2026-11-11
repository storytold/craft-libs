//! `craft_platform::open`.

// The target is one argument after `--` (or the Windows handler), so no shell reads a `&` and no
// tool reads a leading `-` as an option.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn open_command_keeps_the_target_one_inert_argument() {
    let target = "https://x.test/?a=1&calc";
    let command = crate::open::open_command(target);
    let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    assert_ne!(command.get_program(), "cmd");
    assert_eq!(args.last().map(String::as_str), Some(target));
    if !cfg!(windows) {
        assert_eq!(args, ["--", target]);
    }
}
