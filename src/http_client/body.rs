use dioxus::prelude::*;

#[component]
pub fn BodyArea(
    #[props(default)] class: String,
    value: Signal<String>,
    #[props(default)] editable: bool,
) -> Element {
    rsx! {
        textarea {
            class: "body-area textarea textarea-ghost textarea-neutral {class}",
            readonly: !editable,
            oninput: move |e| {
                value.set(e.value());
            },
            value,
        }
    }
}
