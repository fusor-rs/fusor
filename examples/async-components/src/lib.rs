use fusor::prelude::*;
use fusor_async::{
    AsyncValue, CancellationSource, browser,
    fetch::{self, FetchError},
};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

struct App {
    selected: Signal<String>,
    visible: Signal<bool>,
    clicked: Signal<String>,
    retained: AsyncValue<String, String, FetchError>,
    show_retained: Signal<bool>,
}
impl App {
    fn new(owner: OwnerHandle) -> Self {
        Self {
            selected: signal("A".into()),
            visible: signal(true),
            clicked: signal(String::new()),
            retained: browser::read(
                &owner,
                || "retained".to_owned(),
                |_, cancel| async move { fetch::get_text("/api/retained/value", &cancel).await },
            ),
            show_retained: signal(false),
        }
    }
}
struct ReadPanel {
    read: AsyncValue<String, String, FetchError>,
    clicked: Signal<String>,
}
struct ReadPanelInputs {
    selected: Signal<String>,
    kind: &'static str,
    clicked: Signal<String>,
}
impl FromInputs for ReadPanel {
    type Inputs = ReadPanelInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(inputs: Self::Inputs, owner: OwnerHandle) -> Result<Self, JsValue> {
        Ok(Self {
            read: browser::read(
                &owner,
                move || inputs.selected.get(),
                move |key, cancel| async move {
                    fetch::get_text(&format!("/api/{}/{key}", inputs.kind), &cancel).await
                },
            ),
            clicked: inputs.clicked,
        })
    }
}
#[derive(FromInputs)]
struct Retained {
    #[input]
    value: AsyncValue<String, String, FetchError>,
}
#[derive(FromInputs)]
struct Panel;
#[wasm_bindgen]
pub fn stop() -> Result<(), JsValue> {
    fusor::dom::application::unmount()
}

thread_local! { static PROBE: RefCell<Option<CancellationSource>> = const { RefCell::new(None) }; }
/// Test hook: GET `url` directly and describe the outcome. `cancelled` cancels
/// before the request starts; `cancel_probe` cancels one in flight.
#[wasm_bindgen]
pub async fn probe_get_text(url: String, cancelled: bool) -> String {
    let source = CancellationSource::default();
    let token = source.token();
    if cancelled {
        source.cancel();
    }
    PROBE.with(|probe| *probe.borrow_mut() = Some(source));
    let result = fetch::get_text(&url, &token).await;
    if let Some(source) = PROBE.with(|probe| probe.borrow_mut().take()) {
        source.complete();
    }
    match result {
        Ok(text) => format!("ok: {text}"),
        Err(error @ FetchError::Cancelled) => format!("cancelled: {error}"),
        Err(error @ FetchError::Status { .. }) => format!("status: {error}"),
        Err(error @ FetchError::Js(_)) => format!("js: {error}"),
    }
}
#[wasm_bindgen]
pub fn cancel_probe() {
    PROBE.with(|probe| {
        if let Some(source) = &*probe.borrow() {
            source.cancel();
        }
    });
}
fusor::template!("web/index.html");
