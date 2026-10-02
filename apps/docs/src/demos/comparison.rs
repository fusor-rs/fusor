use fusor::{OwnerHandle, Signal, signal};
use fusor_async::{
    AsyncBoundary, AsyncValue, BoundaryStatus, CancellationToken, Resource, ResourceState, browser,
    fetch::{self, FetchError},
};
use gloo_timers::future::TimeoutFuture;

type ProductResource = Resource<&'static str, String, FetchError>;

pub struct Comparison {
    selection: Signal<&'static str>,
    price: ProductResource,
    stock: ProductResource,
    view: AsyncBoundary,
}

// Both approaches use this exact loader. Each side makes independent requests.
async fn load(
    product: &'static str,
    field: &'static str,
    cancel: CancellationToken,
) -> Result<String, FetchError> {
    // Artificial latency makes the publication order visible in this demo.
    TimeoutFuture::new(if field == "price" { 300 } else { 1_800 }).await;
    fetch::get_text(&format!("/docs/demo-data/{product}-{field}.txt"), &cancel)
        .await
        .map(|value| value.trim().to_owned())
}

fn independent(
    owner: &OwnerHandle,
    selection: Signal<&'static str>,
    field: &'static str,
) -> ProductResource {
    browser::resource(
        owner,
        move || Some(selection.get()),
        move |key, cancel| load(key, field, cancel),
    )
}

impl Comparison {
    pub fn new(owner: OwnerHandle) -> Self {
        let selection = signal("Notebook");
        Self {
            price: independent(&owner, selection.clone(), "price"),
            stock: independent(&owner, selection.clone(), "stock"),
            selection,
            view: AsyncBoundary::coherent(),
        }
    }

    fn async_status(&self) -> &'static str {
        let price = self.price.get();
        let stock = self.stock.get();
        if matches!(price, ResourceState::Error { .. })
            || matches!(stock, ResourceState::Error { .. })
        {
            return "Couldn't load. Reset the demo to retry.";
        }
        match (&price, &stock) {
            (ResourceState::Ready(_), ResourceState::Ready(_)) => "Up to date",
            (ResourceState::Ready(_), ResourceState::Loading { .. }) => {
                "Price ready. Stock loading…"
            }
            (ResourceState::Loading { .. }, ResourceState::Ready(_)) => {
                "Stock ready. Price loading…"
            }
            _ => "Loading…",
        }
    }

    fn coherent_status(&self) -> String {
        match self.view.status() {
            BoundaryStatus::Ready => "Up to date".into(),
            BoundaryStatus::Error(error) | BoundaryStatus::Faulted(error) => {
                format!("Couldn't load: {error}. Reset the demo to retry.")
            }
            _ => "Waiting for both…".into(),
        }
    }
}

fn value(resource: &ProductResource) -> String {
    resource.with(|state| {
        state
            .data()
            .map_or_else(|| "—".into(), |data| data.value.to_string())
    })
}

fn provenance(resource: &ProductResource) -> String {
    resource.with(|state| {
        state.data().map_or_else(String::new, |data| {
            if matches!(state, ResourceState::Ready(_)) {
                data.key.to_owned()
            } else {
                format!("Still showing {}", data.key)
            }
        })
    })
}

struct CoherentField {
    data: AsyncValue<&'static str, String, FetchError>,
    label: &'static str,
}

impl CoherentField {
    fn new(
        owner: OwnerHandle,
        selection: Signal<&'static str>,
        field: &'static str,
        label: &'static str,
    ) -> Self {
        Self {
            data: browser::read(
                &owner,
                move || selection.get(),
                move |key, cancel| load(key, field, cancel),
            ),
            label,
        }
    }
}

fusor::bindings!(comparison);

struct CoherentFieldInputs {
    pub selection: Signal<&'static str>,
    pub field: &'static str,
    pub label: &'static str,
}
impl fusor::dom::FromInputs for CoherentField {
    type Inputs = CoherentFieldInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(
            owner,
            inputs.selection,
            inputs.field,
            inputs.label,
        ))
    }
}
