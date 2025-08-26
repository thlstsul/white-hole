use dioxus::prelude::*;
use dioxus_logger::tracing;
use futures_util::StreamExt as _;
use serde::Deserialize;
use tauri_sys::{core::invoke_result, event::listen};

use crate::{search_page::SearchPage, title_bar::TitleBar};

const CSS: Asset = asset!("/assets/styles.css");

#[component]
pub fn App() -> Element {
    let mut browser_state = use_signal(BrowserState::default);

    let icon_url = use_memo(move || browser_state.read().icon_url.clone());
    let title = use_memo(move || browser_state.read().title.clone());
    let url = use_memo(move || browser_state.read().url.clone());
    let maximized = use_memo(move || browser_state.read().maximized);
    let loading = use_memo(move || browser_state.read().loading);
    let can_back = use_memo(move || browser_state.read().can_back);
    let can_forward = use_memo(move || browser_state.read().can_forward);
    let focus = use_memo(move || browser_state.read().focus);
    use_context_provider(|| Browser {
        icon_url,
        title,
        url,
        maximized,
        loading,
        focus,
        can_back,
        can_forward,
    });

    spawn(async move {
        if let Ok(state) = invoke_result::<BrowserState, String>("get_state", &()).await {
            browser_state.set(state);
        }
        let Ok(mut events) = listen::<BrowserState>("state-changed").await else {
            return;
        };

        tracing::info!("listening for state-changed event");
        while let Some(event) = events.next().await {
            browser_state.set(event.payload);
        }
    });

    rsx! {
        InnerApp {}
    }
}

#[component]
pub fn InnerApp() -> Element {
    let browser = use_context::<Browser>();
    let focus = browser.focus;

    rsx! {
        document::Stylesheet { href: CSS }

        if focus() {
            SearchPage {}
        } else {
            TitleBar {}
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct BrowserState {
    icon_url: String,
    title: String,
    url: String,
    maximized: bool,
    loading: bool,
    can_back: bool,
    can_forward: bool,
    focus: bool,
}

#[derive(Clone)]
pub struct Browser {
    pub icon_url: Memo<String>,
    pub title: Memo<String>,
    pub url: Memo<String>,
    pub maximized: Memo<bool>,
    pub loading: Memo<bool>,
    pub focus: Memo<bool>,
    pub can_back: Memo<bool>,
    pub can_forward: Memo<bool>,
}
