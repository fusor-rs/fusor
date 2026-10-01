use fusor::{OwnerHandle, Signal};
use fusor_async::{
    Resource, ResourceState,
    browser::resource,
    fetch::{FetchError, get_text},
};

pub struct Reader {
    data: Resource<u32, String, FetchError>,
}

impl Reader {
    pub fn new(owner: OwnerHandle, selected_id: Signal<u32>) -> Self {
        let data = resource(
            &owner,
            move || Some(selected_id.get()),
            |id, cancel| async move { get_text(&format!("/data/{id}.txt"), &cancel).await },
        );
        Self { data }
    }

    fn status(&self) -> String {
        self.data.with(|state| match state {
            ResourceState::Idle => "No issue selected".into(),
            ResourceState::Loading { key, .. } => format!("Loading issue {key}…"),
            ResourceState::Ready(data) => format!("Loaded issue {}", data.key),
            ResourceState::Error { key, error, .. } => {
                format!("Could not load issue {key}: {error}")
            }
            ResourceState::Disposed => "Reader closed".into(),
        })
    }

    fn text(&self) -> String {
        self.data.with(|state| {
            state.data().map_or_else(String::new, |data| {
                format!("Issue {}: {}", data.key, data.value)
            })
        })
    }
}

fusor::bindings!(reader);

pub struct ReaderInputs {
    pub selected_id: Signal<u32>,
}
impl fusor::dom::FromInputs for Reader {
    type Error = fusor::dom::JsValue;
    type Inputs = ReaderInputs;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(owner, inputs.selected_id))
    }
}
