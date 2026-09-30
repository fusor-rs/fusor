use wasm_bindgen::prelude::*;
#[wasm_bindgen(module = "/src/bridge.js")]
extern "C" {

    #[wasm_bindgen(catch)]
    pub(crate) fn open_runtime(
        pool: bool,
        threads: usize,
        active: usize,
        capacity: usize,
        callback: &js_sys::Function,
    ) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    pub(crate) fn command(runtime: &JsValue, frame: &str) -> Result<(), JsValue>;
    pub(crate) fn shutdown(runtime: &JsValue, error: &str);
    pub(crate) fn close_runtime(runtime: &JsValue, service: u64, timeout: f64) -> js_sys::Promise;
    pub(crate) fn ready(runtime: &JsValue) -> js_sys::Promise;
    pub(crate) fn pool_identity(runtime: &JsValue) -> String;
    pub(crate) fn generation(runtime: &JsValue) -> String;
    pub(crate) fn lease(pool: &str, retain: bool, shared: &str);
    pub(crate) fn hardware_parallelism() -> usize;
    pub(crate) fn dedicated_workers() -> bool;
    pub(crate) fn shared_memory() -> bool;
    pub(crate) fn require_pool();

    pub(crate) fn emit(id: u64, event: &str);
    pub(crate) fn reserve(id: u64) -> js_sys::Promise;
    pub(crate) fn return_credit(id: u64);

    pub(crate) fn start_worker(app: JsValue, configuration: &str);
}
