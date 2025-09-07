mod api;
mod app;
mod navigation;
mod search_input;
mod search_page;
mod settings;
mod title_bar;
mod url;
mod window_decoration;

use app::App;
use dioxus::prelude::*;
use dioxus_logger::tracing::Level;

fn main() {
    let log_level = if cfg!(debug_assertions) {
        Level::DEBUG
    } else {
        Level::ERROR
    };
    dioxus_logger::init(log_level).expect("failed to init logger");
    launch(App);
}
