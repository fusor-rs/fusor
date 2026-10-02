use fusor::coherence::AsyncBoundary;
use fusor::{Effect, FromInputs, Memo, OwnerHandle, Registration, Signal, effect, signal};
use fusor_async::AsyncValue;
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

#[derive(FromInputs)]
pub struct Routing {
    #[input]
    title: String,
    #[input]
    visible: Signal<bool>,
    #[input]
    cleanups: Rc<Cell<usize>>,
    #[input]
    go: Rc<dyn Fn(&str)>,
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

pub type ReadRow = Rc<dyn Fn(&OwnerHandle, Memo<Row>) -> AsyncValue<String, String, String>>;

#[derive(FromInputs)]
pub struct AsyncPanel {
    #[input]
    prefix: String,
    #[input]
    selection: Signal<u32>,
    #[input]
    read: AsyncValue<u32, String, String>,
    #[input]
    boundary: AsyncBoundary,
    #[input]
    rows: Signal<Vec<Row>>,
    #[input]
    make_read: ReadRow,
    #[input]
    starts: Rc<Cell<usize>>,
    #[input]
    cleanups: Rc<Cell<usize>>,
    #[input]
    visible: Signal<bool>,
    #[input]
    clicks: Signal<u32>,
}

pub struct AsyncRow {
    read: AsyncValue<String, String, String>,
    _effect: Effect,
    _cleanup: Registration,
}
pub struct AsyncRowInputs {
    pub row: Memo<Row>,
    pub make_read: ReadRow,
    pub starts: Rc<Cell<usize>>,
    pub cleanups: Rc<Cell<usize>>,
}
impl FromInputs for AsyncRow {
    type Inputs = AsyncRowInputs;
    type Error = &'static str;
    fn from_inputs(inputs: Self::Inputs, owner: OwnerHandle) -> Result<Self, Self::Error> {
        if inputs.row.get().label == "reject" {
            return Err("rejected async row");
        }
        Ok(Self {
            read: (inputs.make_read)(&owner, inputs.row),
            _effect: effect(move || inputs.starts.set(inputs.starts.get() + 1)),
            _cleanup: owner.on_cleanup(move || inputs.cleanups.set(inputs.cleanups.get() + 1)),
        })
    }
}

fusor::template!(backend = "memory", "ui/panel.html");
#[cfg(feature = "browser")]
fusor::template!("ui/panel.html");
