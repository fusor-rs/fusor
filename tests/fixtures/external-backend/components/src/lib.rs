use fusor::{FromInputs, OwnerHandle, Registration, Signal, signal};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: u32,
    pub label: String,
    pub children: Vec<u32>,
}

#[derive(FromInputs)]
pub struct Panel {
    #[input]
    title: String,
    #[input]
    visible: Signal<bool>,
    #[input]
    rows: Signal<Vec<Row>>,
    #[input]
    cleanups: Rc<Cell<usize>>,
    #[input]
    number: Signal<i32>,
    #[input]
    checked: Signal<bool>,
    #[input]
    observed: Signal<i32>,
    #[local(init = signal(0))]
    count: Signal<i32>,
}

pub struct Child {
    label: String,
    visible: Signal<bool>,
    clicks: Signal<u32>,
    _cleanup: Registration,
}
pub struct ChildInputs {
    pub label: String,
    pub visible: Signal<bool>,
    pub cleanups: Rc<Cell<usize>>,
}

#[derive(FromInputs)]
pub struct Frame {
    #[input]
    title: &'static str,
}
impl FromInputs for Child {
    type Inputs = ChildInputs;
    type Error = &'static str;
    fn from_inputs(inputs: Self::Inputs, owner: OwnerHandle) -> Result<Self, Self::Error> {
        if inputs.label.is_empty() {
            return Err("child label must not be empty");
        }
        let cleanups = inputs.cleanups;
        let cleanup = owner.on_cleanup(move || cleanups.set(cleanups.get() + 1));
        Ok(Self {
            label: inputs.label,
            visible: inputs.visible,
            clicks: signal(0),
            _cleanup: cleanup,
        })
    }
}

fusor::template!(backend = "memory", "ui/panel.html");
#[cfg(feature = "browser")]
fusor::template!("ui/panel.html");
