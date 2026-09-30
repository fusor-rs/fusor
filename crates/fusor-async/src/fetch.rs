//! An abortable GET adapter for loaders. HTTP caching follows Fetch defaults.
//! For other methods, headers or bodies, build the request yourself and pass
//! [`CancellationToken::abort_signal`] as its signal.
use crate::CancellationToken;
use std::fmt;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{RequestInit, Response};

#[wasm_bindgen::prelude::wasm_bindgen(
    inline_js = "export async function fusorFetch(url, options) { const base = globalThis.__fusor_worker_base_url; return globalThis.fetch(base ? new URL(url, base).href : url, options); }"
)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = fusorFetch)]
    fn fetch(url: &str, options: &RequestInit) -> js_sys::Promise;
}

/// Why [`get_text`] failed.
#[derive(Debug)]
pub enum FetchError {
    /// `cancel` fired before the response body was fully read.
    Cancelled,
    /// The server answered with a non-2xx status.
    Status { url: String, status: u16 },
    /// Fetch or body reading failed in the browser.
    Js(JsValue),
}
impl From<JsValue> for FetchError {
    fn from(error: JsValue) -> Self {
        Self::Js(error)
    }
}
impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("request cancelled"),
            Self::Status { url, status } => write!(f, "GET {url}: HTTP {status}"),
            Self::Js(error) => match error.as_string() {
                Some(message) => f.write_str(&message),
                None => write!(f, "{error:?}"),
            },
        }
    }
}
impl std::error::Error for FetchError {}

/// GET and fully read a UTF-8 response body. Rejects non-2xx responses.
/// `cancel` aborts the Fetch at any point, including while the body is read.
pub async fn get_text(url: &str, cancel: &CancellationToken) -> Result<String, FetchError> {
    let result: Result<String, FetchError> = async {
        let options = RequestInit::new();
        options.set_signal(Some(&cancel.abort_signal()?));
        let response: Response = JsFuture::from(fetch(url, &options)).await?.dyn_into()?;
        if !response.ok() {
            return Err(FetchError::Status {
                url: url.to_owned(),
                status: response.status(),
            });
        }
        JsFuture::from(response.text()?)
            .await?
            .as_string()
            .ok_or_else(|| JsValue::from_str("Fetch returned a non-text body").into())
    }
    .await;
    // An aborted Fetch rejects with a DOM `AbortError`; report it as cancellation.
    result.map_err(|error| match error {
        FetchError::Js(_) if cancel.is_cancelled() => FetchError::Cancelled,
        error => error,
    })
}
