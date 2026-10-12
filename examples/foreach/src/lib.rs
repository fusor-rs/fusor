use fusor::prelude::*;
use fusor_components::{Scroller, VirtualRows};
#[derive(Clone, PartialEq)]
struct Item {
    id: u32,
    title: String,
}
fn numbered(ids: std::ops::Range<u32>) -> Vec<Item> {
    ids.map(|id| Item {
        id,
        title: format!("Row {id}"),
    })
    .collect()
}
#[derive(Clone, PartialEq)]
struct Group {
    id: u32,
    title: String,
    items: Vec<Item>,
}
struct ViewState {
    items: Signal<Vec<Item>>,
    groups: Signal<Vec<Group>>,
    live: Signal<i32>,
    long: Signal<Vec<Item>>,
    rows: VirtualRows<Item>,
    page_rows: VirtualRows<Item>,
}
impl ViewState {
    fn new() -> Self {
        let long = signal(numbered(0..1000));
        let rows = VirtualRows::new(
            {
                let long = long.clone();
                move || long.get()
            },
            |item| if item.id % 5 == 0 { 40.0 } else { 20.0 },
            Scroller::Container,
            2,
        );
        let page_rows = VirtualRows::new(|| numbered(0..300), |_| 30.0, Scroller::Page, 2);
        Self {
            long,
            rows,
            page_rows,
            live: signal(0),
            groups: signal(vec![
                Group {
                    id: 1,
                    title: "First".into(),
                    items: vec![Item {
                        id: 11,
                        title: "Nested Ada".into(),
                    }],
                },
                Group {
                    id: 2,
                    title: "Second".into(),
                    items: vec![Item {
                        id: 21,
                        title: "Nested Grace".into(),
                    }],
                },
            ]),
            items: signal(vec![
                Item {
                    id: 1,
                    title: "Ada".into(),
                },
                Item {
                    id: 2,
                    title: "Grace".into(),
                },
            ]),
        }
    }
    fn reverse(&self) {
        self.items.update(|v| v.reverse());
    }
    fn insert(&self) {
        self.items.update(|v| {
            v.insert(
                0,
                Item {
                    id: 3,
                    title: "Lin".into(),
                },
            )
        });
    }
    fn remove(&self, id: u32) {
        self.items.update(|v| v.retain(|item| item.id != id));
    }
    fn rename(&self) {
        self.items.update(|v| {
            if let Some(item) = v.iter_mut().find(|i| i.id == 1) {
                item.title = "Ada Lovelace".into()
            }
        });
    }
    fn jump(&self) {
        self.rows.scroll_to(500);
    }
    fn grow(&self) {
        self.long.update(|items| {
            let next = u32::try_from(items.len()).expect("the example list stays small");
            items.extend(numbered(next..next + 5));
        });
    }
}
fusor::template!("web/index.html");

// Never reads its index: the compiler's position-free row source.
#[derive(fusor::FromInputs)]
struct ForwardedRow {
    #[input]
    item: Memo<Item>,
    #[local(init = signal(0))]
    clicks: Signal<u32>,
}

struct TrackedRow {
    item: Memo<Item>,
    position: Memo<usize>,
    clicks: Signal<u32>,
    _activate: fusor::Registration,
    _cleanup: fusor::Registration,
}
struct TrackedRowInputs {
    item: Memo<Item>,
    position: Memo<usize>,
    live: Signal<i32>,
}
impl fusor::dom::FromInputs for TrackedRow {
    type Error = fusor::dom::JsValue;
    type Inputs = TrackedRowInputs;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        let live = inputs.live.clone();
        Ok(Self {
            item: inputs.item,
            position: inputs.position,
            clicks: signal(0),
            _activate: owner.on_activate(move || live.update(|n| *n += 1)),
            _cleanup: owner.on_cleanup(move || {
                inputs.live.update(|n| *n -= 1);
                CLEANUPS.with(|count| count.set(count.get() + 1));
            }),
        })
    }
}

// Exercise inferred application state and the managed startup lifecycle.
thread_local! {
    static CLEANUPS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static CONSTRUCTIONS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static FAIL_NEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
fn create_view(_owner: fusor::OwnerHandle) -> Result<ViewState, fusor::dom::JsValue> {
    CONSTRUCTIONS.with(|count| count.set(count.get() + 1));
    if FAIL_NEXT.with(|fail| fail.replace(false)) {
        return Err(fusor::dom::JsValue::from_str("requested startup failure"));
    }
    Ok(ViewState::new())
}
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn restart() -> Result<(), fusor::dom::JsValue> {
    __fusor_mount()
}
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn stop() -> Result<(), fusor::dom::JsValue> {
    fusor::dom::application::unmount()
}
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn fail_next() {
    FAIL_NEXT.with(|fail| fail.set(true));
}
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn constructions() -> u32 {
    CONSTRUCTIONS.with(|count| count.get())
}

#[wasm_bindgen::prelude::wasm_bindgen]
pub fn cleanups() -> u32 {
    CLEANUPS.with(|count| count.get())
}
