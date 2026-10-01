use fusor::{Signal, signal};

pub struct Details {
    id: u32,
    note: Signal<String>,
}

impl Details {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            note: signal(String::new()),
        }
    }
}

fusor::bindings!(details);

pub struct DetailsInputs {
    pub id: u32,
}
impl fusor::dom::FromInputs for Details {
    type Error = fusor::dom::JsValue;
    type Inputs = DetailsInputs;
    fn from_inputs(
        inputs: Self::Inputs,
        _owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(inputs.id))
    }
}
