use fusor::{FromInputs, JsInputs, Signal, signal};
mod properties;

struct App {
    value: Signal<f64>,
    other: Signal<f64>,
    show: Signal<bool>,
    observed: Signal<String>,
    typed: Signal<String>,
    _cleanup: fusor::Registration,
}

impl App {
    fn new(owner: fusor::OwnerHandle) -> Self {
        let value = signal(1.0);
        // Parent cleanup runs before the child's JavaScript cleanup. The whole
        // disposed tree must already reject publication into child callbacks.
        let cleanup_value = value.clone();
        let cleanup = owner.on_cleanup(move || cleanup_value.set(99.0));
        Self {
            value,
            other: signal(10.0),
            show: signal(true),
            observed: signal(String::new()),
            typed: signal(String::new()),
            _cleanup: cleanup,
        }
    }
    fn u32_detail(&self, event: web_sys::Event) {
        self.typed.set(
            fusor::js::event_detail::<u32>(&event)
                .map_or_else(|_| "error".into(), |value| format!("ok:{value}")),
        );
    }
    fn list_detail(&self, event: web_sys::Event) {
        self.typed.set(
            fusor::js::event_detail::<Option<Vec<i32>>>(&event)
                .map_or_else(|_| "error".into(), |value| format!("ok:{value:?}")),
        );
    }
}

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn stop() -> Result<(), wasm_bindgen::JsValue> {
    fusor::dom::application::unmount()
}

mod components {
    use super::*;

    #[derive(FromInputs, JsInputs)]
    struct Probe {
        #[input]
        #[js]
        value: Signal<f64>,
        #[input]
        other: Signal<f64>,
        #[input]
        show: Signal<bool>,
        #[input]
        observed: Signal<String>,
        #[local(init = signal("ordinary field".to_owned()))]
        #[js]
        __proto__: Signal<String>,
        #[local(init = signal(true))]
        #[js]
        constructor: Signal<bool>,
    }

    #[derive(FromInputs)]
    struct NoInputs;

    #[derive(FromInputs, JsInputs)]
    struct EmptyInputs;

    impl Probe {
        fn from_js(&self, event: web_sys::Event) {
            let message = fusor::js::event_detail::<String>(&event).unwrap();
            self.observed.set(format!("{message}:{}", self.other.get()));
            if message == "rewrite" {
                self.value.set(3.0);
            }
            if message == "dispose" {
                self.show.set(false);
            }
        }
    }

    fusor::template!("web/index.html");
}
