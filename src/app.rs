use dioxus::prelude::*;
use dioxus_logger::tracing;
use futures_util::StreamExt as _;
use tauri_sys::event::listen;

use crate::{
    api::{BrowserState, get_state},
    search_page::SearchPage,
    title_bar::TitleBar,
};

const CSS: Asset = asset!("/assets/tailwind.css");

#[derive(Clone)]
pub struct Browser {
    pub icon_url: Memo<String>,
    pub title: Memo<String>,
    pub url: Memo<String>,
    pub maximized: Memo<bool>,
    pub loading: Memo<bool>,
    pub can_back: Memo<bool>,
    pub can_forward: Memo<bool>,
    pub focus: Memo<bool>,
    pub incognito: Memo<bool>,
    pub darkreader: Memo<bool>,
}

pub fn use_browser() -> Browser {
    use_context::<Browser>()
}

#[component]
pub fn App() -> Element {
    #[cfg(not(debug_assertions))]
    document::eval(r#"document.addEventListener("contextmenu", e => e.preventDefault())"#);

    let mut browser_state = use_signal(BrowserState::default);

    let icon_url = use_memo(move || browser_state.read().icon_url.clone());
    let title = use_memo(move || browser_state.read().title.clone());
    let url = use_memo(move || browser_state.read().url.clone());
    let maximized = use_memo(move || browser_state.read().maximized);
    let loading = use_memo(move || browser_state.read().loading);
    let can_back = use_memo(move || browser_state.read().can_back);
    let can_forward = use_memo(move || browser_state.read().can_forward);
    let focus = use_memo(move || browser_state.read().focus);
    let incognito = use_memo(move || browser_state.read().incognito);
    let darkreader = use_memo(move || browser_state.read().darkreader);
    use_context_provider(|| Browser {
        icon_url,
        title,
        url,
        maximized,
        loading,
        focus,
        can_back,
        can_forward,
        incognito,
        darkreader,
    });

    use_hook(|| {
        spawn(async move {
            if let Ok(state) = get_state().await {
                browser_state.set(state);
            }
            let Ok(mut events) = listen::<BrowserState>("state-changed").await else {
                return;
            };

            tracing::info!("listening for state-changed event");
            while let Some(event) = events.next().await {
                browser_state.set(event.payload);
            }
        })
    });

    rsx! {
        InnerApp {}
    }
}

#[component]
fn InnerApp() -> Element {
    let focus = use_browser().focus;

    rsx! {
        document::Stylesheet { href: CSS }

        if focus() {
            SearchPage {}
        } else {
            TitleBar {}
        }
    }
}
