//! The craft-ui story viewer: the registered stories of the linked crates, shown under a chosen appearance.

use craft_behavior::placement::Defaults;
use craft_ui::shell::app::AppSpec;
use craft_ui_gallery::WINDOW;
use craft_ui_gallery::chrome::Gallery;

const TITLE: &str = "craft-ui stories";
const WINDOW_MIN: [f32; 2] = [640.0, 480.0];

fn spec() -> AppSpec {
    AppSpec::new("craft-ui-gallery", TITLE, Defaults::new(WINDOW, WINDOW_MIN))
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    craft_ui::shell::app::run(spec(), |_| Gallery::open())
}

/// The web build: the gallery on the page's `gallery_canvas`, started by trunk's bundle. The number of stories goes
/// to the browser console, to compare with the native count.
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::{JsCast as _, JsValue};
    use eframe::web_sys::{HtmlCanvasElement, console, window};

    wasm_bindgen_futures::spawn_local(async {
        let (counts, stories) = craft_ui_gallery::linked_counts();
        let per_crate: Vec<String> = counts.iter().map(|(name, count)| format!("{name}={count}")).collect();
        console::log_1(&JsValue::from_str(&format!("craft-ui-gallery: {stories} stories; registrations by crate: {}", per_crate.join(" "))));
        let canvas = window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("gallery_canvas"))
            .and_then(|element| element.dyn_into::<HtmlCanvasElement>().ok());
        let Some(canvas) = canvas else {
            console::error_1(&JsValue::from_str("craft-ui-gallery: the page has no canvas with id gallery_canvas"));
            return;
        };
        if let Err(error) = craft_ui::shell::app::start_web(canvas, spec(), |_| Gallery::open()).await {
            console::error_2(&JsValue::from_str("craft-ui-gallery: the app did not start"), &error);
        }
    });
}
