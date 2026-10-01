use fusor::prelude::*;

pub struct Counter {
    count: Signal<i32>,
    clicks: Signal<i32>,
}

pub struct CounterInputs {
    pub count: Signal<i32>,
}

impl FromInputs for Counter {
    type Error = fusor::dom::JsValue;
    type Inputs = CounterInputs;

    fn from_inputs(
        inputs: Self::Inputs,
        _owner: OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self {
            count: inputs.count,
            clicks: signal(0),
        })
    }
}

fusor::template!("web/components/counter.html");
