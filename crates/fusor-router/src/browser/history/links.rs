//! Which clicks the router follows itself instead of leaving to the browser.
use fusor::dom::document;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Event, HtmlAnchorElement, MouseEvent, Url};

/// A plain primary click on a router link (`data-fusor-link`) that opens in
/// this window on `origin`: its anchor and URL. Modified clicks, downloads and
/// external links are left to the browser.
pub(super) fn router_link(
    event: &Event,
    origin: &str,
) -> Result<Option<(HtmlAnchorElement, Url)>, JsValue> {
    let Some(mouse) = event.dyn_ref::<MouseEvent>() else {
        return Ok(None);
    };
    if event.default_prevented()
        || mouse.button() != 0
        || mouse.ctrl_key()
        || mouse.meta_key()
        || mouse.alt_key()
        || mouse.shift_key()
    {
        return Ok(None);
    }
    let Some(anchor) = event
        .composed_path()
        .iter()
        .find_map(|node| node.dyn_into::<HtmlAnchorElement>().ok())
    else {
        return Ok(None);
    };
    if !anchor.has_attribute(fusor::template::LINK_ATTRIBUTE)
        || anchor.has_attribute("download")
        || anchor
            .rel()
            .split_whitespace()
            .any(|part| part.eq_ignore_ascii_case("external"))
        || !opens_here(&anchor)?
    {
        return Ok(None);
    }
    let url = Url::new(&anchor.href())?;
    Ok((url.origin() == origin).then_some((anchor, url)))
}

/// Whether the anchor's target, or the document's `<base target>`, is this window.
fn opens_here(anchor: &HtmlAnchorElement) -> Result<bool, JsValue> {
    let target = if anchor.target().is_empty() {
        document()?
            .query_selector("base[target]")?
            .and_then(|base| base.get_attribute("target"))
            .unwrap_or_default()
    } else {
        anchor.target()
    };
    Ok(target.is_empty() || target.eq_ignore_ascii_case("_self"))
}
