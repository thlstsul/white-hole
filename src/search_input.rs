use std::rc::Rc;

use dioxus::prelude::*;

use crate::{
    api::{blur, search},
    app::Browser,
};

#[component]
pub fn SearchInput(#[props(default)] class: String, keyword: Signal<String>) -> Element {
    let mut input_element = use_signal::<Option<Rc<MountedData>>>(|| None);

    let _ = use_resource(move || async move {
        let browser = use_context::<Browser>();
        let focus = browser.focus;
        if let Some(url) = input_element() {
            let _ = url.set_focus(focus()).await;
        }
    });

    let keypress = move |e: KeyboardEvent| async move {
        e.prevent_default();
        if e.key() == Key::Enter {
            let _ = search(keyword()).await;
        } else if e.key() == Key::Escape {
            blur().await;
        }
    };

    rsx! {
        label {
            class: "url input input-ghost input-neutral w-full {class}",
            onkeydown: keypress,

            svg {
                class: "h-[1em] opacity-50",
                xmlns: "http://www.w3.org/2000/svg",
                view_box: "0 0 24 24",
                g {
                    stroke_linejoin: "round",
                    stroke_linecap: "round",
                    stroke_width: "2.5",
                    fill: "none",
                    stroke: "currentColor",
                    circle {
                        cx: "11",
                        cy: "11",
                        r: "8",
                    },
                    path {
                        d: "m21 21-4.3-4.3",
                    },
                }
            }

            input {
                r#type: "search",
                placeholder: "搜索",
                value: keyword,
                oninput: move |e| {
                    keyword.set(e.value());
                },
                autocomplete: "off",
                onmounted: move |element| input_element.set(Some(element.data())),
            }

            kbd {
                class: "kbd kbd-sm",
                "CTRL",
            }
            kbd {
                class: "kbd kbd-sm",
                "L",
            }
        }
    }
}
