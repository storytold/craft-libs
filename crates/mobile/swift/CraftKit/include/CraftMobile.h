// The C interface of craft-mobile (src/ios.rs), linked from the app's static library.
// Call everything on the main thread. Calls before craft_start or after craft_stop do nothing.
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// Requests from the app, delivered after frames. `text` and `data` are valid only during the
// call: copy them. kind 0: pick one file, 1: pick files (text: allowed extensions, comma-joined,
// empty for any the app declares); 2: export (text: suggested name, data: the file);
// 3: panel state (text: a Snapshot as JSON); 4: copy text (text); 5: copy image (text "W×H",
// data RGBA8 unpremultiplied); 6: open URL (text); 7: keyboard (text "1" show, "0" hide);
// 8: write in place (text: the location, a line break, the file name; data: the file), answered
// with craft_write_done; 9: busy (text "1" when the app starts work the user is waiting for that
// must finish even in the background, "0" when it's done): hold background time for that span and,
// while the scene isn't active, keep calling craft_frame (it runs the app without drawing);
// 10: paste (the user asked to: read the clipboard and answer with craft_paste_text, craft_paste_image
// or, when it still holds the image the app copied last, craft_paste_own; text first while a text
// field is focused; empty text when there is nothing to paste); 11: share (text: file name, data:
// the file) through the share sheet; 12: show the Photos picker (chosen photos open with
// craft_open_file, as copies).
typedef void (*CraftHandler)(void *context, uint32_t kind, const uint8_t *text, size_t text_len, const uint8_t *data, size_t data_len);

// Where requests go; `context` comes back unchanged and is never dereferenced by Rust.
void craft_set_handler(void *context, CraftHandler handler);

// Start the app drawing into `layer`, a CAMetalLayer that must stay alive until craft_stop.
// Generated per app by craft_mobile::export!. Sizes in pixels; scale is pixels per point.
bool craft_start(void *layer, uint32_t width_px, uint32_t height_px, float scale);
void craft_stop(void);
void craft_resize(uint32_t width_px, uint32_t height_px, float scale);
// Call on every display refresh: draws only when something changed. Returns whether it drew.
bool craft_frame(void);
// phase: 0 began, 1 moved, 2 ended, 3 cancelled. Position in points; force 0...1, or 0 if none.
void craft_touch(uint64_t id, uint32_t phase, float x, float y, float force);

// A file the system handed over (Files, Share, drag and drop). `location` is where the shell can
// write it back (its URL as a string), or empty (null, 0) for a copy.
void craft_open_file(const uint8_t *name, size_t name_len, const uint8_t *data, size_t data_len, const uint8_t *location, size_t location_len);
// Answer the open picker: add each chosen file, then craft_pick_done (none added = cancelled).
void craft_pick_add(const uint8_t *name, size_t name_len, const uint8_t *data, size_t data_len, const uint8_t *location, size_t location_len);
void craft_pick_done(void);
// A write in place (request 8) to `location` ended: `error` empty (null, 0) when the file was
// written, else why it wasn't.
void craft_write_done(const uint8_t *location, size_t location_len, const uint8_t *error, size_t error_len);
// A native control was used: an action as JSON ({"action":"undo"}, ...). False if malformed.
bool craft_action(const uint8_t *json, size_t len);

// Where the app keeps its own data (Application Support). Call before craft_start.
void craft_set_data_dir(const uint8_t *path, size_t len);

// Keyboards. `name` is egui's key name ("A", "Enter", "ArrowLeft"); modifiers: 1 shift,
// 2 control, 4 option, 8 command. Text typed (on-screen keyboard, dictation) goes to craft_text.
void craft_key(const uint8_t *name, size_t len, bool pressed, uint32_t modifiers);
void craft_text(const uint8_t *text, size_t len);
// Paste what the user asked to paste, read from the clipboard.
void craft_paste_text(const uint8_t *text, size_t len);
void craft_paste_image(const uint8_t *rgba, size_t len, uint32_t width, uint32_t height);
// The clipboard still holds the image the app copied last (request 5): the app pastes its own copy.
void craft_paste_own(void);

// Trackpads and mice, in points. `inside` false when the pointer left the view; scroll deltas
// positive move content right and down.
void craft_hover(float x, float y, bool inside);
void craft_scroll(float x, float y, float dx, float dy);
void craft_secondary_click(float x, float y);

// Stylus contact, sent instead of craft_touch: pressure 0...1, altitude radians from the screen
// (pi/2 upright), azimuth radians clockwise from +x, eraser when the eraser end is in use.
void craft_stylus(uint64_t id, uint32_t phase, float x, float y, float pressure, float altitude, float azimuth, bool eraser);

// Life cycle: leaving the screen (the app writes recovery data; call inside a background task),
// back on screen, and the OS short of memory.
void craft_suspend(void);
void craft_resume(void);
void craft_memory_warning(void);
// The device's power state: thermal is ProcessInfo.ThermalState's raw value (0 nominal, 1 fair,
// 2 serious, 3 critical); low_power whether Low Power Mode is on. After craft_start and on changes.
void craft_power(uint32_t thermal, bool low_power);
