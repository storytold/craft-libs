#![cfg(not(target_arch = "wasm32"))]

#[story(Widget)]
fn view() {}

#[trial]
fn changes() {}
#[path = "other.rs"]
mod other;

#[path = "."]
mod inline_alias {
    mod nested;
}
