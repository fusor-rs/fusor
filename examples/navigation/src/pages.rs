//! Page state and data loading stay in ordinary Rust.
use crate::routes::{BASE, Page};
use fusor::prelude::*;
use fusor_async::{
    Resource, ResourceState,
    browser::resource,
    fetch::{FetchError, get_text},
};
use fusor_router::Location;
use wasm_bindgen::JsValue;

pub struct Home {
    previews: Signal<Vec<u32>>,
}
impl Default for Home {
    fn default() -> Self {
        Self {
            previews: signal(Vec::new()),
        }
    }
}
pub struct Preview {
    data: Resource<u32, String, FetchError>,
}
impl Preview {
    pub fn new(owner: OwnerHandle, item: fusor::Memo<u32>) -> Self {
        Self {
            data: resource(
                &owner,
                move || Some(item.get()),
                |id, cancel| async move { get_text(&format!("{BASE}data/{id}.txt"), &cancel).await },
            ),
        }
    }
    fn text(&self) -> String {
        self.data.with(|state| {
            state
                .data()
                .map_or_else(|| "Loading preview…".into(), |data| data.value.to_string())
        })
    }
}
pub struct NotFound;
pub struct Metadata {
    data: Resource<(), String, FetchError>,
}
impl Metadata {
    pub fn new(owner: OwnerHandle) -> Self {
        Self {
            data: resource(
                &owner,
                || Some(()),
                |_, cancel| async move { get_text(&format!("{BASE}data/5.txt"), &cancel).await },
            ),
        }
    }
    fn text(&self) -> String {
        self.data.with(|state| {
            state
                .data()
                .map_or_else(|| "Loading metadata…".into(), |data| data.value.to_string())
        })
    }
}
pub struct Article {
    pub data: Resource<(u32, String), String, FetchError>,
    pub draft: Signal<String>,
    metadata_visible: Signal<bool>,
}
impl Article {
    pub fn new(owner: OwnerHandle, location: Derived<Location<Page>>) -> Self {
        let data = resource(
            &owner,
            move || {
                let location = location.get();
                match location.route {
                    Some(Page::Article(id)) => {
                        Some((id, location.url.query_first("revision").unwrap_or_default()))
                    }
                    _ => None,
                }
            },
            |(id, revision), cancel| async move {
                let query = fusor_router::encode_query([("revision", revision.as_str())]);
                get_text(&format!("{BASE}data/{id}.txt?{query}"), &cancel).await
            },
        );
        Self {
            data,
            draft: signal(String::new()),
            metadata_visible: signal(true),
        }
    }
    pub fn status(&self) -> String {
        self.data.with(|state| match state {
            ResourceState::Idle => "Idle".into(),
            ResourceState::Loading { key, .. } => format!("Loading article {} ({})", key.0, key.1),
            ResourceState::Ready(data) => format!("Ready article {} ({})", data.key.0, data.key.1),
            ResourceState::Error { key, error, .. } => {
                format!("Article {} failed: {error}", key.0)
            }
            ResourceState::Disposed => "Disposed".into(),
        })
    }
    pub fn text(&self) -> String {
        self.data.with(|state| {
            state.data().map_or_else(String::new, |data| {
                format!("Article {} ({}):\n{}", data.key.0, data.key.1, data.value)
            })
        })
    }
}

fusor::bindings!(pages);

pub struct PreviewInputs {
    pub item: fusor::Memo<u32>,
}
impl fusor::dom::FromInputs for Preview {
    type Inputs = PreviewInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(owner, inputs.item))
    }
}

pub struct MetadataInputs {}
impl fusor::dom::FromInputs for Metadata {
    type Inputs = MetadataInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        _inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(owner))
    }
}

pub struct EmptyInputs {}
impl fusor::dom::FromInputs for Home {
    type Inputs = EmptyInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(_: Self::Inputs, _: OwnerHandle) -> Result<Self, JsValue> {
        Ok(Self::default())
    }
}
impl fusor::dom::FromInputs for NotFound {
    type Inputs = EmptyInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(_: Self::Inputs, _: OwnerHandle) -> Result<Self, JsValue> {
        Ok(Self)
    }
}
pub struct ArticleRoute {
    valid: bool,
}
fn route_location(owner: &OwnerHandle) -> Result<Derived<Location<Page>>, JsValue> {
    let location = fusor_router::browser::declarative::Navigation::from_owner(owner)
        .ok_or_else(|| JsValue::from_str("missing navigation"))?
        .location();
    Ok(derived(move || Location::<Page>::new(location.get())))
}
impl fusor::dom::FromInputs for ArticleRoute {
    type Inputs = EmptyInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(_: Self::Inputs, owner: OwnerHandle) -> Result<Self, JsValue> {
        Ok(Self {
            valid: matches!(route_location(&owner)?.get().route, Some(Page::Article(_))),
        })
    }
}
impl fusor::dom::FromInputs for Article {
    type Inputs = EmptyInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(_: Self::Inputs, owner: OwnerHandle) -> Result<Self, JsValue> {
        let location = route_location(&owner)?;
        Ok(Self::new(owner, location))
    }
}
