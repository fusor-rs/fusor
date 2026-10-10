#[cfg(target_arch = "wasm32")]
fusor_islands::export!(
    fusor_islands::browser::Unit::new().entry::<catalog_types::Cart, catalog_views::CartView>(
        |_, props| catalog_views::CartView::new(props)
    )
);
fusor::bindings!(app);
include!(env!("FUSOR_MODULE"));

// Consumer fixture for the public Rust control API. Polling then dropping is
// exactly what an abandoned application future does; no JS cancellation shim.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn exercise_waiter(dispose_owner: bool) -> Result<(), wasm_bindgen::JsValue> {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    let owner = fusor::Owner::new();
    let handle =
        fusor_islands::browser::get::<catalog_types::Designer>(&owner.handle(), "designer")
            .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))?;
    owner.commit();
    let mut future = std::pin::pin!(handle.activate());
    if !matches!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ) {
        return Err(wasm_bindgen::JsValue::from_str(
            "expected deferred activation",
        ));
    }
    if dispose_owner {
        owner.dispose();
    }
    Ok(())
}

/// Exercise a completed request through the public typed Rust facade.
/// `action` is `prefetch`, `activate` or `retry`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub async fn exercise_control(action: String) -> Result<(), wasm_bindgen::JsValue> {
    let owner = fusor::Owner::new();
    let target =
        fusor_islands::browser::get::<catalog_types::Designer>(&owner.handle(), "designer")
            .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))?;
    owner.commit();
    let result = match action.as_str() {
        "prefetch" => target.prefetch().await,
        "activate" => target.activate().await,
        "retry" => target.retry().await,
        _ => return Err(wasm_bindgen::JsValue::from_str("unknown action")),
    };
    result.map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
    // Dropping this caller leaves an activated target owned by its host.
}

/// Read the designer's status through the typed Rust facade.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn exercise_status() -> Result<String, wasm_bindgen::JsValue> {
    let owner = fusor::Owner::new();
    fusor_islands::browser::get::<catalog_types::Designer>(&owner.handle(), "designer")
        .and_then(|target| target.status())
        .map(|status| format!("{status:?}"))
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
}

/// Round-trip messages through `emit` and `listen`: an exact u64, a payload of
/// the wrong type, and a message sent after the listening scope dropped.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn exercise_messages() -> Result<(), wasm_bindgen::JsValue> {
    use catalog_types::Designer;
    use fusor_islands::browser::{emit, listen};
    use std::{cell::RefCell, rc::Rc};
    let host = fusor::dom::document()?.create_element("div")?;
    let received = Rc::new(RefCell::new(Vec::new()));
    let mut scope = fusor::dom::Scope::new(host.clone());
    let log = received.clone();
    listen::<Designer, u64>(&mut scope, &host, "picked", move |value| {
        log.borrow_mut().push(value);
    })?;
    emit::<Designer, _>(&host, "picked", &u64::MAX)?;
    emit::<Designer, _>(&host, "picked", &"not a number")?;
    drop(scope);
    emit::<Designer, _>(&host, "picked", &1_u64)?;
    match received.borrow().as_slice() {
        [Ok(u64::MAX), Err(_)] => Ok(()),
        other => Err(wasm_bindgen::JsValue::from_str(&format!(
            "unexpected messages: {other:?}"
        ))),
    }
}

#[cfg(target_arch = "wasm32")]
struct DisposingPreview;
#[cfg(target_arch = "wasm32")]
impl fusor::dom::Component for DisposingPreview {
    const TEMPLATE_HASH: &'static str = "disposing-preview";
    fn mount(self) -> Result<fusor::dom::Scope, wasm_bindgen::JsValue> {
        Ok(fusor::dom::Scope::new(
            fusor::dom::document()?.create_element("p")?,
        ))
    }
    fn prepare_component(
        parent: Option<&fusor::OwnerHandle>,
        make: fusor::dom::ComponentFactory<'_, Self>,
    ) -> Result<fusor::dom::Scope, wasm_bindgen::JsValue> {
        let owner = std::rc::Rc::new(parent.map(fusor::Owner::child).unwrap_or_default());
        let mut scope = make(owner.handle())?.mount()?;
        scope.prepare_owner(Some(&owner.handle()));
        let element = scope.root()?.clone();
        let dispose = owner.clone();
        let activation = scope.owner().on_activate(move || {
            element.remove();
            dispose.dispose();
        });
        scope.retain_state(activation);
        owner.commit();
        Ok(scope)
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub async fn exercise_preview_restoration(
    host: fusor_islands::browser::IslandElement,
) -> Result<(), wasm_bindgen::JsValue> {
    use catalog_types::{Designer, DesignerProps};
    use fusor::dom::Component;
    use fusor_islands::{Island, attributes};
    let document = fusor::dom::document()?;
    let props = fusor_islands::encode(&DesignerProps {
        product_id: 1,
        title: "preview".into(),
    })
    .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))?;
    host.set_attribute(attributes::SCHEMA, Designer::SCHEMA)?;
    host.set_attribute(attributes::HASH, DisposingPreview::TEMPLATE_HASH)?;
    let initial = document.create_element("p")?;
    host.append_child(&initial)?;
    let payload = document.create_element("script")?;
    payload.set_attribute("type", "application/json")?;
    payload.set_attribute(attributes::PROPS, "")?;
    payload.set_text_content(Some(&props));
    host.append_child(&payload)?;
    let unit = fusor_islands::browser::Unit::new()
        .entry::<Designer, DisposingPreview>(|_, _| DisposingPreview);
    let activation = unit.activate(Designer::NAME, &host, &props, "restoration")?;
    wasm_bindgen_futures::JsFuture::from(activation)
        .await
        .map(|_| ())
}
