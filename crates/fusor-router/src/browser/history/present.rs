//! Where focus and scroll go after a navigation shows new content.
use crate::AppUrl;
use wasm_bindgen::JsCast;
use web_sys::{Element, FocusOptions, HtmlElement, Window};

/// Focus the new content's `[autofocus]` element, else its first `h1`,
/// making it focusable if needed and without scrolling.
pub(super) fn focus_content(root: &Element) {
    let target = root
        .query_selector("[autofocus]")
        .ok()
        .flatten()
        .or_else(|| root.query_selector("h1").ok().flatten());
    let Some(node) = target
        .as_ref()
        .and_then(|node| node.dyn_ref::<HtmlElement>())
    else {
        return;
    };
    if !node.has_attribute("tabindex") && node.tab_index() < 0 {
        let _ = node.set_attribute("tabindex", "-1");
    }
    let options = FocusOptions::new();
    options.set_prevent_scroll(true);
    let _ = node.focus_with_options(&options);
}

/// Scroll the URL's fragment target into view, else to the top.
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
