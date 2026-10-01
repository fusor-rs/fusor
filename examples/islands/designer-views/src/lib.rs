use catalog_types::DesignerProps;
use fusor::{OwnerHandle, Signal, signal};
use fusor_async::{
    AsyncBoundary, AsyncValue, browser,
    fetch::{self, FetchError},
};

pub struct DesignerView {
    title: String,
    selection: Signal<String>,
    boundary: AsyncBoundary,
}
impl DesignerView {
    pub fn new(props: DesignerProps) -> Self {
        Self {
            title: props.title,
            selection: signal("A".into()),
            boundary: AsyncBoundary::coherent(),
        }
    }
}
pub struct Price {
    value: AsyncValue<String, String, FetchError>,
}
pub struct Stock {
    value: AsyncValue<String, String, FetchError>,
}
fn read(
    owner: &OwnerHandle,
    selection: Signal<String>,
    kind: &'static str,
) -> AsyncValue<String, String, FetchError> {
    browser::read(
        owner,
        move || selection.get(),
        move |key, cancel| async move {
            fetch::get_text(&format!("/api/designer/{kind}/{key}"), &cancel).await
        },
    )
}
impl Price {
    fn new(owner: OwnerHandle, selection: Signal<String>) -> Self {
        Self {
            value: read(&owner, selection, "price"),
        }
    }
}
impl Stock {
    fn new(owner: OwnerHandle, selection: Signal<String>) -> Self {
        Self {
            value: read(&owner, selection, "stock"),
        }
    }
}

fusor::bindings!(app);
include!(env!("FUSOR_MODULE"));

pub struct PriceInputs {
    pub selection: Signal<String>,
}
impl fusor::dom::FromInputs for Price {
    type Error = fusor::dom::JsValue;
    type Inputs = PriceInputs;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(owner, inputs.selection))
    }
}

pub struct StockInputs {
    pub selection: Signal<String>,
}
impl fusor::dom::FromInputs for Stock {
    type Error = fusor::dom::JsValue;
    type Inputs = StockInputs;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(owner, inputs.selection))
    }
}
