//! Where focus and scroll go after a navigation shows new content.
use crate::AppUrl;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, FocusOptions, HtmlElement, Window};

/// Focus the new content's `[autofocus]` element, else its first `h1`,
/// making it focusable if needed and without scrolling.
pub(super) fn focus_content(root: &Element) -> Result<(), JsValue> {
    let target = match root.query_selector("[autofocus]")? {
        Some(target) => Some(target),
        None => root.query_selector("h1")?,
    };
    let Some(node) = target
        .as_ref()
        .and_then(|node| node.dyn_ref::<HtmlElement>())
    else {
        return Ok(());
    };
    if !node.has_attribute("tabindex") && node.tab_index() < 0 {
        node.set_attribute("tabindex", "-1")?;
    }
    let options = FocusOptions::new();
    options.set_prevent_scroll(true);
    node.focus_with_options(&options)
}

/// Scroll the URL's fragment target into view, else to the top.
/// Missing targets and non-UTF-8 fragment IDs use the same top fallback.
pub(super) fn scroll_to(window: &Window, url: &AppUrl) {
    let target = (!url.fragment.is_empty())
        .then(|| {
            percent_encoding::percent_decode_str(&url.fragment)
                .decode_utf8()
                .ok()
        })
        .flatten()
        .and_then(|id| window.document()?.get_element_by_id(&id));
    match target {
        Some(target) => target.scroll_into_view(),
        None => window.scroll_to_with_x_and_y(0.0, 0.0),
    }
}
