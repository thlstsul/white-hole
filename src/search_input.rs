use dioxus::prelude::*;

#[component]
pub fn SearchInput(
    #[props(default)] class: String,
    value: Signal<String>,
    onenter: EventHandler<()>,
    onmounted: EventHandler<MountedEvent>,
) -> Element {
    let onkeydown = move |e: KeyboardEvent| {
        if e.key() == Key::Enter {
            onenter.call(());
        }
        Ok(())
    };

    rsx! {
        label {
            class: "url input input-ghost has-[:focus]:outline-none w-full {class}",
            onkeydown,

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
                    circle { cx: "11", cy: "11", r: "8" }
                    path { d: "m21 21-4.3-4.3" }
                }
            }

            input {
                r#type: "search",
                placeholder: "搜索",
                autocomplete: "off",
                autofocus: true,
                value,
                onmounted,
                oninput: move |e| value.set(e.value()),
            }

            kbd { class: "kbd kbd-sm", "ENTER" }
        }
    }
}
