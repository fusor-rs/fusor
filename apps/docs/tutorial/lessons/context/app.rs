use fusor::{ContextKey, OwnerHandle, Signal, signal};
use std::rc::Rc;
use wasm_bindgen::JsValue;

struct Theme;
impl ContextKey for Theme {
    type Value = Signal<String>;
}

struct App {
    theme: Signal<String>,
}

impl App {
    fn new(owner: OwnerHandle) -> Result<Self, JsValue> {
        let theme = signal("dark".into());
        owner
            .provide::<Theme>(theme.clone())
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(Self { theme })
    }
}

struct Badge {
    theme: Rc<Signal<String>>,
}

impl Badge {
    fn new(owner: OwnerHandle) -> Result<Self, JsValue> {
        let theme = owner
            .context::<Theme>()
            .ok_or_else(|| JsValue::from_str("missing Theme provider"))?;
        Ok(Self { theme })
    }
}

fusor::template!("web/index.html");

struct BadgeInputs {}
impl fusor::dom::FromInputs for Badge {
    type Error = fusor::dom::JsValue;
    type Inputs = BadgeInputs;
    fn from_inputs(
        _inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Self::new(owner)
    }
}
