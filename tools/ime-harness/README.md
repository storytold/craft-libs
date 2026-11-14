# Linux IME harness

Automated input-method tests for egui apps on Linux, run in a container: a real IME framework
(fcitx5 or IBus) on a real display server (Xvfb or headless sway), a minimal egui app with one
`TextEdit`, and synthetic key presses. Each case types a composition and checks the text the app
ends up with. This is the "Integration" row of Track 9 in
[craft-libs#198](https://github.com/storytold/craft-libs/issues/198).

| Path | What it is |
|---|---|
| `Dockerfile` | Debian trixie with Xvfb, sway, fcitx5, IBus, the engines, xdotool, wtype, wlrctl and Rust |
| `app/` | The test app (eframe 0.37, glow renderer); its own Cargo project, outside the workspace |
| `harness/ime-harness` | Starts D-Bus, the display server and the IME, runs the app, types, checks |
| `harness/sway.config` | Config for headless sway |

## Build

With rootless docker, point the client at your daemon first:

```sh
export DOCKER_HOST=unix:///run/user/$(id -u)/docker.sock
docker build -t craft-ime-harness tools/ime-harness
```

The first build downloads about 1 GB of packages and the Rust toolchain; the image is about 2.8 GB.
Cargo's registry and target dir live in BuildKit cache mounts, so rebuilding after an app change
takes seconds.

## Run

```sh
docker run --rm craft-ime-harness list                        # backends and cases
docker run --rm craft-ime-harness all                         # every case, one line each
docker run --rm craft-ime-harness run fcitx5-x11 pinyin       # one case, with logs on stderr
```

Each case starts its own D-Bus session, display server and IME, so cases don't affect each other.
A run prints `PASS`, `FAIL` (with the events the app received), `ERROR` (setup failed) or `SKIP`.

To keep the logs (`app.log`, `fcitx5.log` or `ibus.log`, `sway.log`, `harness.log` and
`committed.txt` per case), mount a directory:

```sh
mkdir -p results
docker run --rm -e IME_HARNESS_RESULTS=/results -v "$PWD/results:/results" craft-ime-harness all
```

The container runs as its root user, which under rootless docker is you, so the files are yours.

Debugging switches, passed with `-e`:

| Variable | Effect |
|---|---|
| `IME_HARNESS_LOG_ALL=1` | The app logs every egui input event, not only key, text and IME events |
| `RUST_LOG=winit=trace` | winit's own log, in `app.log` |
| `APP_WAYLAND_DEBUG=1` | Wayland protocol trace of the app (text-input-v3 traffic), in `app.log` |
| `FCITX_WAYLAND_DEBUG=1` | Wayland protocol trace of fcitx5 (input-method-v2 traffic) |
| `FCITX5_VERBOSE='*=5'` | fcitx5 debug log, including every XIM request |

### Backends

| Backend | Display | IME | Protocol the app uses |
|---|---|---|---|
| `fcitx5-x11` | Xvfb | fcitx5 | XIM, fcitx5's default over-the-spot styles (fcitx5 draws the preedit) |
| `fcitx5-x11-onthespot` | Xvfb | fcitx5, `UseOnTheSpot=True` | XIM with preedit callbacks (the app draws the preedit) |
| `fcitx5-sway` | headless sway | fcitx5 | text-input-v3 (fcitx5 on input-method-v2) |
| `ibus-x11` | Xvfb | IBus | XIM |

winit 0.30 talks to input methods only through XIM on X11 and text-input-v3 on Wayland, so these
cover both paths egui apps use.

### Cases

| Case | Keys | Expected text |
|---|---|---|
| `pinyin` | `n i h a o space` | `你好` |
| `hangul` (2-set) | `g k s r m f space` | `한글 ` |
| `telex` (Unikey) | `t i e e s n g space` | `tiếng ` |
| `anthy` | `n i h o n g o space Return` | `日本語` |
| `pinyin-click` | click into the focused `TextEdit`, then the `pinyin` keys | `你好` |
| `pinyin-hotplug` | add and remove a pointer device, then the `pinyin` keys | `你好` |

`pinyin-click` runs on X11 only and `pinyin-hotplug` on sway only: wlrctl creates a new virtual
pointer for each command and removes it when the command exits, so on sway a click never arrives
but a pointer hotplug does.

To add a case, add the engine's keys, expected text and engine names to the functions at the top
of `harness/ime-harness`.

## Results with stock egui 0.37.0, eframe 0.37.0 and winit 0.30.13

Package versions: fcitx5 5.1.12, fcitx5-chinese-addons 5.1.8, fcitx5-hangul, fcitx5-unikey and
fcitx5-anthy 5.1.6, IBus 1.5.32, ibus-libpinyin 1.15.8, ibus-hangul 1.5.5, ibus-unikey 0.7.0~beta1,
ibus-anthy 1.5.17, sway 1.10.1, Xvfb 21.1.16.

| Case | `fcitx5-x11` | `fcitx5-x11-onthespot` | `fcitx5-sway` | `ibus-x11` |
|---|---|---|---|---|
| `pinyin` | pass | pass | pass | pass |
| `hangul` | pass | pass | pass | pass |
| `telex` | pass | pass | pass | pass |
| `anthy` | pass | pass | pass | pass |
| `pinyin-click` | **fail**: `nihao ` | **fail**: `nihao ` | n/a | pass |
| `pinyin-hotplug` | n/a | n/a | **fail**: `nihao ` | n/a |

The same matrix passes completely with winit patched to aiongg/winit `craft/linux-ime` at
1d49b2d0 (storytold/winit#5).

### Findings

1. **X11: a click into a focused text field turns fcitx5 off.** Every click on a `TextEdit` calls
   `Memory::request_focus`, which asks the backend to interrupt the composition
   (egui `widgets/text_edit/builder.rs:880`, `context.rs:3173`). egui-winit does that by calling
   `set_ime_allowed(false)` and then `set_ime_allowed(true)` (egui-winit `lib.rs:1281`). On X11
   winit 0.30.13 handles each call by destroying the input context and creating a new one
   (`x11/ime/mod.rs:197`), and does not set focus on the new one. fcitx5 keeps the active input
   method per input context by default (`ShareInputState=No`), so the new context starts with
   the keyboard layout and the next keys arrive as Latin text. With `ShareInputState=All` the case
   passes, and IBus, whose engine state is global, passes too.
2. **Wayland: a pointer or touch device going away turns the IME off.** winit 0.30.13 destroys
   its `zwp_text_input_v3` whenever the seat loses any capability (`wayland/seat/mod.rs:161`),
   and creates a new one only when a capability is added. After a pointer is removed the window
   has no text input at all, and keys reach the app untranslated. When a text input is created
   later, sway sends it no `enter` for the window that already has focus, so it stays disabled
   until focus changes. Unplugging a mouse or tablet triggers this.
3. **fcitx5's default XIM styles give the app no preedit.** fcitx5 offers on-the-spot (preedit
   callback) styles only with `UseOnTheSpot=True`. By default winit picks over-the-spot,
   receives only commits, and fcitx5 draws the preedit in its own window at the reported spot.
   With on-the-spot enabled, preedit text arrives, but the caret is always reported at 0
   (`active_range_chars: Some(0..0)` for `ni hao`), so egui draws the caret at the start of the
   preedit. IBus reports the caret at the end. Whether fcitx5 or winit's XIM caret handling is at
   fault is not settled yet.

### Pitfalls the harness works around

These are setup problems, not toolkit bugs, but each one makes a case fail in a misleading way.

- **Locale.** Under the C locale, Xlib's `Xutf8LookupString` returns nothing for CJK commits:
  fcitx5 logs the commit and the app gets no event. The harness sets `LANG=C.UTF-8`.
- **Keyboard-less seat.** Headless sway starts with no keyboard. winit creates its text input
  only when the seat gains a keyboard, after the window has focus, so the text input never gets
  `enter` (the same mechanism as finding 2). The harness holds a virtual keyboard (`wtype -s`)
  for the whole sway session.
- **Focus requests.** Every `request_focus` interrupts the composition (finding 1), so the app
  requests focus once, not every frame.
- **`fcitx5-remote` as a readiness check** starts a second fcitx5 through D-Bus activation; the
  harness waits for the `org.fcitx.Fcitx5` bus name instead.
- **Mozc** is not used: `mozc_server` does not start inside the container (fcitx5-mozc blocks
  in `ServerLauncher::StartServer`, which also blocks fcitx5's XIM server). Anthy covers Japanese.
- **ibus-hangul and ibus-anthy** start in Latin mode; the harness sets them to Hangul and
  hiragana through gsettings.

## Testing a patched egui or winit

Uncomment the `[patch.crates-io]` section in `app/Cargo.toml` and set the branches. Then update
the lock file inside the base image and rebuild:

```sh
cd tools/ime-harness
docker build --target base -t craft-ime-harness-base .
docker run --rm -v "$PWD/app:/src/app" -w /src/app craft-ime-harness-base cargo update -p winit
docker build -t craft-ime-harness:patched .
docker run --rm craft-ime-harness:patched all
```

A local checkout can't be patched in by path, because the image only sees `tools/ime-harness`.
Push the branch and use a `git` dependency, or copy the checkout into `app/` for the build.
