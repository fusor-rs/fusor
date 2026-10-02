use fusor::{OwnerHandle, Signal};
use fusor_async::{
    AsyncValue,
    browser,
    fetch::{self, FetchError},
};

pub struct Price {
    quote: AsyncValue<String, String, FetchError>,
}

impl Price {
    pub fn new(owner: OwnerHandle, product: Signal<String>) -> Self {
        Self {
            quote: browser::read(
                &owner,
                move || product.get(),
                |key, cancel| async move {
                    fetch::get_text(&format!("/data/price/{key}.txt"), &cancel).await
                },
            ),
        }
    }
}

pub struct Stock {
    stock: AsyncValue<String, String, FetchError>,
}

impl Stock {
    pub fn new(owner: OwnerHandle, product: Signal<String>) -> Self {
        Self {
            stock: browser::read(
                &owner,
                move || product.get(),
                |key, cancel| async move {
                    fetch::get_text(&format!("/data/stock/{key}.txt"), &cancel).await
                },
            ),
        }
    }
}

fusor::bindings!(pricing);

pub struct PriceInputs {
    pub product: Signal<String>,
}
impl fusor::dom::FromInputs for Price {
    type Error = fusor::dom::JsValue;
    type Inputs = PriceInputs;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(owner, inputs.product))
    }
}

pub struct StockInputs {
    pub product: Signal<String>,
}
impl fusor::dom::FromInputs for Stock {
    type Error = fusor::dom::JsValue;
    type Inputs = StockInputs;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(owner, inputs.product))
    }
}
