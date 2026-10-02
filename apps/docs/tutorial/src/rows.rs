use fusor::{Signal, signal};

#[derive(Clone, PartialEq)]
pub struct Item {
    pub id: u32,
    pub title: String,
}

pub struct Row {
    item: fusor::Memo<Item>,
    note: Signal<String>,
}

impl Row {
    pub fn new(item: fusor::Memo<Item>) -> Self {
        Self {
            item,
            note: signal(String::new()),
        }
    }
}

fusor::bindings!(rows);

pub struct RowInputs {
    pub item: fusor::Memo<Item>,
}
impl fusor::dom::FromInputs for Row {
    type Error = fusor::dom::JsValue;
    type Inputs = RowInputs;
    fn from_inputs(
        inputs: Self::Inputs,
        _owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(inputs.item))
    }
}
