use catalog_types::CartProps;
use fusor::{Signal, signal};
use fusor_std::forms::TextField;

pub struct CartView {
    product_id: u64,
    title: String,
    quantity: Signal<String>,
    accepted: Signal<bool>,
    note: TextField<String>,
    size: Signal<String>,
    wrap: Signal<String>,
    submissions: Signal<u32>,
    input_id: String,
    lines: Signal<Vec<u32>>,
}
impl CartView {
    pub fn new(props: CartProps) -> Self {
        Self {
            lines: signal(vec![1, 2]),
            product_id: props.product_id,
            title: props.title,
            quantity: signal(props.quantity),
            accepted: signal(false),
            note: TextField::new(String::new()),
            size: signal("medium".into()),
            wrap: signal("plain".into()),
            submissions: signal(0),
            input_id: format!("quantity-{}", props.product_id),
        }
    }
}
struct Note {
    value: Signal<String>,
}
impl Note {
    fn new() -> Self {
        Self {
            value: signal("Native child".into()),
        }
    }
}
struct Line {
    item: fusor::Memo<u32>,
    draft: Signal<String>,
}
impl Line {
    fn new(item: fusor::Memo<u32>) -> Self {
        Self {
            item,
            draft: signal("Native row".into()),
        }
    }
}
fusor::bindings!(app);
include!(env!("FUSOR_MODULE"));

struct LineInputs {
    pub item: fusor::Memo<u32>,
}
impl fusor::dom::FromInputs for Line {
    type Inputs = LineInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        inputs: Self::Inputs,
        _owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(inputs.item))
    }
}

struct NoteInputs {}
impl fusor::dom::FromInputs for Note {
    type Inputs = NoteInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        _inputs: Self::Inputs,
        _owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new())
    }
}

#[derive(fusor::FromInputs)]
struct NoteLabel {
    #[input]
    text: &'static str,
}

#[derive(fusor::FromInputs)]
struct NoteContainer;
