use fusor::prelude::*;
use fusor_async::{
    AsyncValue, browser,
    fetch::{self, FetchError},
};
use gloo_timers::future::TimeoutFuture;

#[derive(FromInputs)]
pub struct AsyncData {
    #[local(init = signal(12_u32))]
    issue: Signal<u32>,
}

struct IssueField {
    value: AsyncValue<u32, String, FetchError>,
    field: &'static str,
}

pub struct IssueFieldInputs {
    pub issue: Signal<u32>,
    pub field: &'static str,
    pub delay: u32,
}

impl FromInputs for IssueField {
    type Inputs = IssueFieldInputs;
    type Error = fusor::dom::JsValue;

    fn from_inputs(inputs: Self::Inputs, owner: OwnerHandle) -> Result<Self, fusor::dom::JsValue> {
        let field = inputs.field;
        let value = browser::read(
            &owner,
            move || inputs.issue.get(),
            move |issue, cancel| async move {
                TimeoutFuture::new(inputs.delay).await;
                fetch::get_text(&format!("./data/issues/{issue}-{field}.txt"), &cancel)
                    .await
                    .map(|value| value.trim().to_owned())
            },
        );
        Ok(Self { value, field })
    }
}

fusor::template!("web/components/async_data.html");
