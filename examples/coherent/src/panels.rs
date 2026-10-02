use fusor::{Effect, OwnerHandle, Signal, effect};
use fusor_async::{
    AsyncValue, browser,
    fetch::{self, FetchError},
};

fn data(
    owner: &OwnerHandle,
    selected: Signal<String>,
    kind: &'static str,
) -> AsyncValue<String, String, FetchError> {
    browser::read(
        owner,
        move || selected.get(),
        move |key, cancel| async move { fetch::get_text(&format!("/api/{kind}/{key}"), &cancel).await },
    )
}
pub struct Price {
    quote: AsyncValue<String, String, FetchError>,
}
impl Price {
    pub fn new(owner: OwnerHandle, selected: Signal<String>) -> Self {
        Self {
            quote: data(&owner, selected, "price"),
        }
    }
}
pub struct Stock {
    stock: AsyncValue<String, String, FetchError>,
}
impl Stock {
    pub fn new(owner: OwnerHandle, selected: Signal<String>) -> Self {
        Self {
            stock: data(&owner, selected, "stock"),
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct RowData {
    pub id: u32,
    pub label: String,
}
pub struct Row {
    item: fusor::Memo<RowData>,
    _effect: Effect,
}
impl Row {
    pub fn new(item: fusor::Memo<RowData>, mounts: Signal<usize>) -> Self {
        Self {
            item,
            _effect: effect(move || mounts.update(|count| *count += 1)),
        }
    }
}
fusor::bindings!(panels);

pub struct RowInputs {
    pub item: fusor::Memo<RowData>,
    pub mounts: Signal<usize>,
}
impl fusor::dom::FromInputs for Row {
    type Inputs = RowInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        inputs: Self::Inputs,
        _owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(inputs.item, inputs.mounts))
    }
}

pub struct PriceInputs {
    pub selected: Signal<String>,
}
impl fusor::dom::FromInputs for Price {
    type Inputs = PriceInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(owner, inputs.selected))
    }
}

pub struct StockInputs {
    pub selected: Signal<String>,
}
impl fusor::dom::FromInputs for Stock {
    type Inputs = StockInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(owner, inputs.selected))
    }
}

#[derive(fusor::FromInputs)]
pub struct CoherentPanel;
