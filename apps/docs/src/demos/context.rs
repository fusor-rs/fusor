use fusor::{ContextKey, OwnerHandle, Signal, signal};
use std::rc::Rc;
use wasm_bindgen::JsValue;

struct Accent;
impl ContextKey for Accent {
    type Value = Signal<String>;
}

pub struct Context {
    accent: Signal<String>,
}

impl Context {
    pub fn new(owner: OwnerHandle) -> Result<Self, JsValue> {
        let accent = signal("Forest".into());
        owner
            .provide::<Accent>(accent.clone())
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(Self { accent })
    }
}

struct Panel;

struct Badge {
    accent: Rc<Signal<String>>,
}

impl Badge {
    fn new(owner: OwnerHandle) -> Result<Self, JsValue> {
        let accent = owner
            .context::<Accent>()
            .ok_or_else(|| JsValue::from_str("Accent provider is missing"))?;
        Ok(Self { accent })
    }
}

fusor::bindings!(context);

struct PanelInputs {}
impl fusor::dom::FromInputs for Panel {
    type Inputs = PanelInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        _inputs: Self::Inputs,
        _owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self)
    }
}

struct BadgeInputs {}
impl fusor::dom::FromInputs for Badge {
    type Inputs = BadgeInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        _inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Self::new(owner)
    }
}
