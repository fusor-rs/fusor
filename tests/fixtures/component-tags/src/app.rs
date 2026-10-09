//! State stays in an ordinary Rust module; generated impls can see private fields.
use crate::widgets::{Badge, Button, Counter as CounterAlias, Empty, FragmentControls, Panel, Row};
use fusor::prelude::*;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;

#[derive(Clone)]
struct Controls {
    count: Signal<i32>,
    visible: Signal<bool>,
    key: Signal<u32>,
    panel_key: Signal<u32>,
    fail: Signal<bool>,
}

thread_local! {
    static CONTROLS: RefCell<Option<Controls>> = const { RefCell::new(None) };
    static METRICS: RefCell<[u32; 3]> = const { RefCell::new([0; 3]) };
    static EVENT_TRACE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}
pub(crate) fn record(index: usize) {
    METRICS.with(|metrics| metrics.borrow_mut()[index] += 1);
}

struct App {
    count: Signal<i32>,
    visible: Signal<bool>,
    key: Signal<u32>,
    panel_key: Signal<u32>,
    fail: Signal<bool>,
    title: &'static str,
    text: Signal<String>,
    choice: Signal<String>,
    pulse: Signal<u32>,
    _event_effects: Vec<fusor::Effect>,
    fragment: Content,
    fragment_visible: Signal<bool>,
}

impl App {
    fn new() -> Self {
        let controls = Controls {
            count: signal(0),
            visible: signal(true),
            key: signal(0),
            panel_key: signal(0),
            fail: signal(false),
        };
        CONTROLS.with(|current| *current.borrow_mut() = Some(controls.clone()));
        let text = signal("initial".to_owned());
        let choice = signal("a".to_owned());
        let pulse = signal(0);
        let event_effects = vec![
            event_effect("text", text.clone()),
            event_effect("select", choice.clone()),
            event_effect("pulse", pulse.clone()),
        ];
        let fragment_visible = signal(true);
        let visible = fragment_visible.clone();
        let count = controls.count.clone();
        let fragment = Content::new(move |_| FragmentControls {
            visible: visible.clone(),
            count: count.clone(),
        });
        Self {
            fragment,
            fragment_visible,
            count: controls.count,
            visible: controls.visible,
            key: controls.key,
            panel_key: controls.panel_key,
            fail: controls.fail,
            title: "caller title",
            text,
            choice,
            pulse,
            _event_effects: event_effects,
        }
    }

    fn input(&self) {
        trace(format!("input:{}", self.text.get()));
        self.pulse.set(1);
        self.pulse.set(2);
    }

    fn change(&self) {
        trace(format!("change:{}", self.choice.get()));
        self.pulse.set(3);
        self.pulse.set(4);
    }
}

fn trace(value: String) {
    EVENT_TRACE.with(|trace| trace.borrow_mut().push(value));
}

fn event_effect<T: Clone + std::fmt::Display + 'static>(
    label: &'static str,
    value: Signal<T>,
) -> fusor::Effect {
    effect(move || trace(format!("{label}:{}", value.get())))
}

#[wasm_bindgen]
pub fn take_event_trace() -> String {
    EVENT_TRACE.with(|trace| std::mem::take(&mut *trace.borrow_mut()).join("|"))
}

#[wasm_bindgen]
pub fn set_count(value: i32) {
    CONTROLS.with(|current| current.borrow().as_ref().unwrap().count.set(value));
}
#[wasm_bindgen]
pub fn show(value: bool) {
    CONTROLS.with(|current| current.borrow().as_ref().unwrap().visible.set(value));
}
#[wasm_bindgen]
pub fn reset(key: u32, fail: bool) {
    CONTROLS.with(|current| {
        let current = current.borrow();
        let controls = current.as_ref().unwrap();
        batch(|| {
            controls.fail.set(fail);
            controls.key.set(key);
        });
    });
}
#[wasm_bindgen]
pub fn reset_panel(value: u32) {
    CONTROLS.with(|current| current.borrow().as_ref().unwrap().panel_key.set(value));
}
#[wasm_bindgen]
pub fn metrics() -> Vec<u32> {
    METRICS.with(|metrics| metrics.borrow().to_vec())
}

#[wasm_bindgen]
pub fn fragment_contracts() -> Result<Vec<u32>, JsValue> {
    let parent = fusor::Owner::new();
    let content = Content::new(|_| FragmentControls {
        visible: signal(true),
        count: signal(42),
    });
    let mut child = content.prepare(&parent.handle())?;
    let mut result = vec![
        u32::from(child.root().is_err()),
        u32::from(child.select("button").is_err()),
        u32::from(child.is_detached()),
    ];
    let container = fusor::dom::document()?.create_element("div")?;
    child.attach(&container)?;
    result.extend([
        u32::from(child.is_detached()),
        container.child_element_count(),
    ]);
    let moved = std::cell::RefCell::new(Some(child));
    let reused = Content::from_prepared(move |_| {
        Ok(moved.borrow_mut().take().expect("one preparation attempt"))
    });
    result.push(u32::from(reused.prepare(&parent.handle()).is_err()));
    result.push(container.child_element_count());
    Ok(result)
}
#[wasm_bindgen]
pub fn unmount() -> Result<(), JsValue> {
    fusor::dom::application::unmount()
}

fusor::template!("web/index.html");
