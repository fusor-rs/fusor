use fusor::{OwnerHandle, Registration, Signal};

pub struct Watch {
    _activation: Registration,
    _cleanup: Registration,
}

impl Watch {
    pub fn new(owner: OwnerHandle, lifecycle: Signal<String>) -> Self {
        let activated = lifecycle.clone();
        let activation = owner.on_activate(move || {
            activated.set("Panel mounted".into());
        });
        let cleanup = owner.on_cleanup(move || {
            lifecycle.set("Panel removed; cleanup ran".into());
        });
        Self {
            _activation: activation,
            _cleanup: cleanup,
        }
    }
}

fusor::bindings!(watch);

pub struct WatchInputs {
    pub lifecycle: Signal<String>,
}
impl fusor::dom::FromInputs for Watch {
    type Error = fusor::dom::JsValue;
    type Inputs = WatchInputs;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(owner, inputs.lifecycle))
    }
}
