use fusor::Registration;
use fusor::prelude::*;
use wasm_bindgen::prelude::*;
struct App {
    count: Signal<u32>,
    visible: Signal<bool>,
    placed: Signal<bool>,
    version: Signal<u32>,
    live: Signal<i32>,
    rows: Signal<Vec<u32>>,
}
impl App {
    fn new(owner: OwnerHandle) -> Self {
        owner.provide::<Location>("caller").unwrap();
        Self {
            count: signal(0),
            visible: signal(true),
            placed: signal(true),
            version: signal(0),
            live: signal(0),
            rows: signal(vec![1, 2]),
        }
    }
}
struct Location;
impl fusor::ContextKey for Location {
    type Value = &'static str;
}
struct Panel;
struct PanelInputs {}
impl FromInputs for Panel {
    type Inputs = PanelInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(_: PanelInputs, owner: OwnerHandle) -> Result<Self, JsValue> {
        owner.provide::<Location>("panel").unwrap();
        Ok(Self)
    }
}
#[derive(FromInputs)]
struct Table;
#[derive(FromInputs)]
struct Forward;
#[derive(FromInputs)]
struct CoherentForward {
    #[input]
    placed: Signal<bool>,
}
#[derive(FromInputs)]
struct Ignore;
struct Tracked {
    clicks: Signal<u32>,
    _activate: Registration,
    _cleanup: Registration,
}
struct TrackedInputs {
    live: Signal<i32>,
}
impl FromInputs for Tracked {
    type Inputs = TrackedInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(inputs: Self::Inputs, owner: OwnerHandle) -> Result<Self, JsValue> {
        assert_eq!(*owner.context::<Location>().unwrap(), "panel");
        let live = inputs.live.clone();
        Ok(Self {
            clicks: signal(0),
            _activate: owner.on_activate(move || live.update(|n| *n += 1)),
            _cleanup: owner.on_cleanup(move || inputs.live.update(|n| *n -= 1)),
        })
    }
}
#[wasm_bindgen]
pub fn stop() -> Result<(), JsValue> {
    fusor::dom::application::unmount()
}
fusor::template!("web/index.html");
