use fusor::{FromInputs, prelude::*};
use fusor_router::browser::declarative::Navigation;
use wasm_bindgen::JsValue;
#[derive(FromInputs)]
pub struct Dashboard {
    #[local(init = signal(0))]
    count: Signal<i32>,
}
#[derive(FromInputs)]
struct Overview;
#[derive(FromInputs)]
struct Settings {
    #[local(init = signal(0))]
    count: Signal<i32>,
}
#[derive(FromInputs)]
struct Missing;
pub struct FilterInputs {}
struct Filter {
    location: Derived<fusor_router::AppUrl>,
}
impl fusor::dom::FromInputs for Filter {
    type Inputs = FilterInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(_: FilterInputs, owner: OwnerHandle) -> Result<Self, JsValue> {
        Ok(Self {
            location: Navigation::from_owner(&owner)
                .ok_or_else(|| JsValue::from_str("missing navigation"))?
                .location(),
        })
    }
}
fusor::template!("web/components/dashboard.html");

#[derive(FromInputs)]
pub struct Team {
    #[input]
    slug: String,
    #[local(init = signal(0))]
    count: Signal<i32>,
}
