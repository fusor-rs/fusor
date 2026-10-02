use fusor::{OwnerHandle, Registration, Signal, signal};
use gloo_timers::callback::Interval;
use std::{cell::RefCell, rc::Rc};

pub struct Lifecycle {
    visible: Signal<bool>,
    ticks: Signal<u32>,
    events: Signal<Vec<String>>,
}

impl Lifecycle {
    pub fn new() -> Self {
        Self {
            visible: signal(true),
            ticks: signal(0),
            events: signal(Vec::new()),
        }
    }
}

struct TickingPanel {
    _activation: Registration,
    _cleanup: Registration,
}

fn record(events: &Signal<Vec<String>>, message: &str) {
    events.update(|items| {
        items.push(message.into());
        if items.len() > 6 {
            items.remove(0);
        }
    });
}

impl TickingPanel {
    fn new(owner: OwnerHandle, ticks: Signal<u32>, events: Signal<Vec<String>>) -> Self {
        let timer = Rc::new(RefCell::new(None));
        let active_timer = timer.clone();
        let active_events = events.clone();
        let activation = owner.on_activate(move || {
            ticks.set(0);
            record(&active_events, "Mounted → timer started");
            *active_timer.borrow_mut() = Some(Interval::new(1_000, move || {
                ticks.update(|value| *value += 1);
            }));
        });
        let cleanup = owner.on_cleanup(move || {
            timer.borrow_mut().take(); // Dropping Interval clears the browser timer.
            record(&events, "Disposed → timer stopped");
        });
        Self {
            _activation: activation,
            _cleanup: cleanup,
        }
    }
}

fusor::bindings!(lifecycle);

struct TickingPanelInputs {
    pub ticks: Signal<u32>,
    pub events: Signal<Vec<String>>,
}
impl fusor::dom::FromInputs for TickingPanel {
    type Inputs = TickingPanelInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(owner, inputs.ticks, inputs.events))
    }
}
