use fusor::{JsInputs, Signal, signal};

#[derive(JsInputs)]
pub struct ThreeJs {
    #[js]
    bloom: Signal<f64>,
    #[js]
    palette: Signal<String>,
    #[js]
    paused: Signal<bool>,
    #[js]
    selected: Signal<u32>,
    status: Signal<String>,
}

impl ThreeJs {
    pub fn new() -> Self {
        Self {
            bloom: signal(72.0),
            palette: signal("aurora".into()),
            paused: signal(false),
            selected: signal(0),
            status: signal("Preparing your garden…".into()),
        }
    }

    fn selected(&self, event: web_sys::Event) {
        if let Ok(index) = fusor::js::event_detail::<u32>(&event) {
            if (1..=24).contains(&index) {
                self.selected.set(index);
            }
        }
    }

    fn report(&self, event: web_sys::Event) {
        if let Ok(message) = fusor::js::event_detail::<String>(&event) {
            self.status.set(message);
        }
    }
}

fusor::template!("web/demos/threejs.html");

pub struct ThreeJsInputs {}

impl fusor::dom::FromInputs for ThreeJs {
    type Inputs = ThreeJsInputs;

    fn from_inputs(
        _inputs: Self::Inputs,
        _owner: fusor::OwnerHandle,
    ) -> Result<Self, wasm_bindgen::JsValue> {
        Ok(Self::new())
    }
}
