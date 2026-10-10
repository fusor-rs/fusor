use fusor::{Memo, Signal, memo, signal};
use std::rc::Rc;
#[derive(Clone)]
struct Item {
    id: u32,
    value: Signal<u32>,
}
impl PartialEq for Item {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Item {
    fn new(id: u32) -> Self {
        Self {
            id,
            value: signal(id),
        }
    }
}
struct Model {
    rows: Signal<Vec<Item>>,
    shared: Signal<u32>,
    mode: String,
    total: Memo<u64>,
}
impl Model {
    fn new(n: u32, mode: String) -> Rc<Self> {
        let rows = signal((0..n).map(Item::new).collect::<Vec<_>>());
        let inputs = rows.get();
        let fanin = mode == "fanin";
        let total = memo(move || {
            if fanin {
                inputs.iter().map(|row| u64::from(row.value.get())).sum()
            } else {
                0
            }
        });
        Rc::new(Self {
            rows,
            shared: signal(0),
            mode,
            total,
        })
    }
    fn visible(&self) -> Vec<Item> {
        if self.mode == "fanin" {
            vec![]
        } else {
            self.rows.get()
        }
    }
}
struct App {
    model: Rc<Model>,
}
struct Row {
    item: fusor::Memo<Item>,
    model: Rc<Model>,
    computed: Memo<u32>,
}
impl Row {
    fn new(item: fusor::Memo<Item>, model: Rc<Model>) -> Self {
        let captured = item.clone();
        Self {
            item,
            model,
            computed: memo(move || captured.get().value.get() * 2),
        }
    }
}
fusor::bindings!(app);
include!(env!("FUSOR_MODULE"));

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use fusor::{
        batch,
        dom::{Component, Scope, delivery, document},
    };
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;
    thread_local! { static APP: RefCell<Option<(Rc<Model>,Scope)>> = const { RefCell::new(None) }; }
    fn model() -> Rc<Model> {
        APP.with(|app| app.borrow().as_ref().expect("mounted benchmark").0.clone())
    }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        document()?
            .document_element()
            .unwrap()
            .set_attribute("data-bench-ready", "true")
    }
    #[wasm_bindgen]
    pub fn bench_mount(n: u32, mode: String) -> Result<(), JsValue> {
        let model = Model::new(n, mode);
        let captured = model.clone();
        let mut scope =
            App::prepare_component(None, Box::new(move |_| Ok(App { model: captured })))?;
        scope.attach(&document()?.get_element_by_id("app").unwrap())?;
        scope.try_commit()?;
        APP.with(|app| app.replace(Some((model, scope))));
        Ok(())
    }
    #[wasm_bindgen]
    pub fn bench_hydrate(n: u32) -> Result<(), JsValue> {
        let model = Model::new(n, "rows".into());
        let captured = model.clone();
        let root = document()?
            .get_element_by_id("app")
            .unwrap()
            .first_element_child()
            .unwrap();
        delivery::enable();
        let scope = delivery::with_root(&root, || {
            App::prepare_component(None, Box::new(move |_| Ok(App { model: captured })))
        })?;
        scope.try_commit()?;
        APP.with(|app| app.replace(Some((model, scope))));
        Ok(())
    }
    #[wasm_bindgen]
    pub fn bench_unmount() {
        let old = APP.with(|app| app.take());
        drop(old);
        if let Ok(doc) = document() {
            doc.get_element_by_id("app").unwrap().set_text_content(None);
        }
    }
    #[wasm_bindgen]
    pub fn bench_update(index: usize, value: u32) {
        model()
            .rows
            .with(|rows| rows[index].value.clone())
            .set(value);
    }
    #[wasm_bindgen]
    pub fn bench_bulk(count: usize) {
        let rows = model().rows.get();
        batch(|| {
            for row in rows.iter().take(count) {
                row.value.update(|value| *value += 1);
            }
        });
    }
    #[wasm_bindgen]
    pub fn bench_insert(index: usize, id: u32) {
        model()
            .rows
            .update(|rows| rows.insert(index, Item::new(id)));
    }
    #[wasm_bindgen]
    pub fn bench_remove(index: usize) {
        model().rows.update(|rows| {
            rows.remove(index);
        });
    }
    #[wasm_bindgen]
    pub fn bench_swap(a: usize, b: usize) {
        model().rows.update(|rows| rows.swap(a, b));
    }
    #[wasm_bindgen]
    pub fn bench_fanout(value: u32) {
        model().shared.set(value);
    }
    #[wasm_bindgen]
    pub fn bench_fanin() {
        let rows = model().rows.get();
        batch(|| {
            for row in &rows {
                row.value.update(|value| *value += 1);
            }
        });
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub fn server_render(n: u32) -> fusor_server::Result<String> {
    use fusor_server::Render;
    Ok(App {
        model: Model::new(n, "rows".into()),
    }
    .render(&mut fusor_server::Context::new())?
    .into_string())
}

struct RowInputs {
    pub item: fusor::Memo<Item>,
    pub model: Rc<Model>,
}
impl fusor::dom::FromInputs for Row {
    type Error = fusor::dom::JsValue;
    type Inputs = RowInputs;
    fn from_inputs(
        inputs: Self::Inputs,
        _owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(inputs.item, inputs.model))
    }
}
