//! Optional, component-local JavaScript modules and explicit Rust inputs.
//!
//! Derive [`JsInputs`] and mark `Signal<T>` fields with `#[js]`. JavaScript gets
//! read-only `get()` and `subscribe(callback)` methods. Subscribe delivers the
//! current value immediately, then one snapshot after each Rust reactive batch.
//! Supported values are bool, String, f64, i32, u32, JsValue, Option and Vec.
//! Use native CustomEvent payloads and [`event_detail`] to send values to Rust.
use crate::{Effect, OwnerHandle, Signal, dom::Scope, effect, untrack};
use js_sys::{Array, Object, Reflect};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
use web_sys::{CustomEvent, Element, Event};

#[wasm_bindgen(module = "/src/js/runtime.js")]
extern "C" {
    fn inputs_create() -> Object;
    fn input_create(read: &JsValue, observe: &JsValue) -> JsValue;
    fn input_api(state: &JsValue) -> JsValue;
    fn input_publish(state: &JsValue, value: &JsValue);
    fn input_dispose(state: &JsValue);
    fn module_create(root: &Element, inputs: &JsValue, id: &str) -> JsValue;
    #[wasm_bindgen(catch)]
    fn module_activate(state: &JsValue) -> Result<(), JsValue>;
    fn module_dispose(state: &JsValue);
}

mod sealed {
    pub trait Sealed {}
}

/// Bounded, checked JavaScript value conversion. This trait is sealed so an
/// exposed input cannot silently acquire arbitrary serialization semantics.
pub trait JsInputValue: sealed::Sealed + Clone + 'static {
    #[doc(hidden)]
    fn to_js(&self) -> JsValue;
    #[doc(hidden)]
    fn from_js(value: &JsValue) -> Result<Self, JsValue>;
}
fn mismatch(expected: &str) -> JsValue {
    JsValue::from_str(&format!("fusor: event detail must be {expected}"))
}
macro_rules! primitive {
    ($ty:ty, $method:ident, $label:literal) => {
        impl sealed::Sealed for $ty {}
        impl JsInputValue for $ty {
            fn to_js(&self) -> JsValue {
                JsValue::from(self.clone())
            }
            fn from_js(value: &JsValue) -> Result<Self, JsValue> {
                value.$method().ok_or_else(|| mismatch($label))
            }
        }
    };
}
primitive!(bool, as_bool, "a boolean");
primitive!(String, as_string, "a string");
primitive!(f64, as_f64, "a number");
macro_rules! integer {
    ($ty:ty, $label:literal) => {
        impl sealed::Sealed for $ty {}
        impl JsInputValue for $ty {
            fn to_js(&self) -> JsValue {
                JsValue::from(*self)
            }
            fn from_js(value: &JsValue) -> Result<Self, JsValue> {
                value
                    .as_f64()
                    .filter(|value| {
                        value.is_finite()
                            && value.fract() == 0.0
                            && *value >= <$ty>::MIN as f64
                            && *value <= <$ty>::MAX as f64
                    })
                    .map(|value| value as $ty)
                    .ok_or_else(|| mismatch($label))
            }
        }
    };
}
integer!(i32, "an integer in the i32 range");
integer!(u32, "an integer in the u32 range");
impl sealed::Sealed for JsValue {}
impl JsInputValue for JsValue {
    fn to_js(&self) -> JsValue {
        self.clone()
    }
    fn from_js(value: &JsValue) -> Result<Self, JsValue> {
        Ok(value.clone())
    }
}
impl<T: JsInputValue> sealed::Sealed for Option<T> {}
impl<T: JsInputValue> JsInputValue for Option<T> {
    fn to_js(&self) -> JsValue {
        self.as_ref().map_or(JsValue::NULL, JsInputValue::to_js)
    }
    fn from_js(value: &JsValue) -> Result<Self, JsValue> {
        if value.is_null() {
            Ok(None)
        } else {
            T::from_js(value).map(Some)
        }
    }
}
impl<T: JsInputValue> sealed::Sealed for Vec<T> {}
impl<T: JsInputValue> JsInputValue for Vec<T> {
    fn to_js(&self) -> JsValue {
        self.iter()
            .map(JsInputValue::to_js)
            .collect::<Array>()
            .into()
    }
    fn from_js(value: &JsValue) -> Result<Self, JsValue> {
        if !Array::is_array(value) {
            return Err(mismatch("an array"));
        }
        Array::from(value)
            .iter()
            .map(|item| T::from_js(&item))
            .collect()
    }
}

/// Read a native CustomEvent payload, checking its entire supported value shape.
/// Null is accepted only by Option; integers reject fractional/out-of-range data.
pub fn event_detail<T: JsInputValue>(event: &Event) -> Result<T, JsValue> {
    let event = event
        .dyn_ref::<CustomEvent>()
        .ok_or_else(|| mismatch("a native CustomEvent"))?;
    T::from_js(&event.detail())
}

/// Fields explicitly exposed to the component's JavaScript module. Usually
/// implemented with `#[derive(fusor::JsInputs)]` and `#[js]` field markers.
pub trait JsInputs {
    #[doc(hidden)]
    fn js_inputs(&self) -> Inputs;
}

trait InputGuard {
    fn close(&self);
    fn set_owner(&self, owner: OwnerHandle);
}
struct InputState<T> {
    signal: RefCell<Option<Signal<T>>>,
    owner: RefCell<Option<OwnerHandle>>,
    observer: RefCell<Option<Effect>>,
    javascript: RefCell<Option<JsValue>>,
    pending: Cell<bool>,
    generation: Cell<u64>,
}
impl<T> InputState<T> {
    fn disposed(&self) -> bool {
        self.owner
            .borrow()
            .as_ref()
            .is_some_and(OwnerHandle::is_disposed)
    }
    fn close(&self) {
        let js = self.javascript.take();
        if let Some(js) = js {
            input_dispose(&js);
        }
        self.pending.set(false);
        let observer = self.observer.take();
        drop(observer);
        self.signal.take();
    }
}

impl<T: JsInputValue> InputState<T> {
    fn read(&self) -> Result<JsValue, JsValue> {
        if self.disposed() {
            return Err(JsValue::from_str(
                "fusor: input belongs to a disposed component",
            ));
        }
        let signal = self
            .signal
            .borrow()
            .clone()
            .ok_or_else(|| JsValue::from_str("fusor: input disposed"))?;
        Ok(signal.with_untracked(JsInputValue::to_js))
    }

    fn stop_observing(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.pending.set(false);
        let observer = self.observer.take();
        drop(observer);
    }

    fn observe(self: &Rc<Self>) {
        if self.disposed() {
            self.close();
            return;
        }
        if self.observer.borrow().is_some() {
            return;
        }
        let Some(signal) = self.signal.borrow().clone() else {
            return;
        };
        let weak = Rc::downgrade(self);
        let mut first = true;
        let observer = effect(move || {
            // Track only this field; publication runs after ordinary Rust effects.
            signal.with(|_| ());
            if first {
                first = false;
                return;
            }
            if let Some(state) = weak.upgrade() {
                state.queue_publication();
            }
        });
        *self.observer.borrow_mut() = Some(observer);
    }

    fn queue_publication(self: &Rc<Self>) {
        if self.disposed() {
            self.close();
            return;
        }
        if self.pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        let generation = self.generation.get();
        crate::reactive::after_flush(move || {
            if let Some(state) = weak.upgrade() {
                state.publish(generation);
            }
        });
    }

    fn publish(&self, generation: u64) {
        if self.disposed() {
            self.close();
            return;
        }
        if generation != self.generation.get() {
            return;
        }
        self.pending.set(false);
        let signal = self.signal.borrow().clone();
        let js = self.javascript.borrow().clone();
        let (Some(signal), Some(js)) = (signal, js) else {
            return;
        };
        if self.observer.borrow().is_none() {
            return;
        }
        let value = signal.with_untracked(JsInputValue::to_js);
        untrack(|| input_publish(&js, &value));
    }
}

struct Input<T> {
    state: Rc<InputState<T>>,
    _read: Closure<dyn Fn() -> Result<JsValue, JsValue>>,
    _observe: Closure<dyn Fn(bool)>,
}
impl<T> InputGuard for Input<T> {
    fn close(&self) {
        self.state.close();
    }
    fn set_owner(&self, owner: OwnerHandle) {
        *self.state.owner.borrow_mut() = Some(owner);
    }
}
impl<T> Drop for Input<T> {
    fn drop(&mut self) {
        self.state.close();
    }
}

/// Generated input declarations; constructing these creates no reactive work.
#[doc(hidden)]
pub struct Inputs {
    object: Object,
    guards: Vec<Box<dyn InputGuard>>,
}
impl Default for Inputs {
    fn default() -> Self {
        Self {
            object: inputs_create(),
            guards: Vec::new(),
        }
    }
}
impl Inputs {
    pub fn add<T: JsInputValue>(&mut self, name: &str, signal: Signal<T>) {
        let state = Rc::new(InputState {
            signal: RefCell::new(Some(signal)),
            owner: RefCell::new(None),
            observer: RefCell::new(None),
            javascript: RefCell::new(None),
            pending: Cell::new(false),
            generation: Cell::new(0),
        });
        let weak = Rc::downgrade(&state);
        let read = Closure::wrap(Box::new(move || {
            let state = weak
                .upgrade()
                .ok_or_else(|| JsValue::from_str("fusor: input disposed"))?;
            state.read()
        }) as Box<dyn Fn() -> Result<JsValue, JsValue>>);
        let weak = Rc::downgrade(&state);
        let observe = Closure::wrap(Box::new(move |active: bool| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if active {
                state.observe();
            } else {
                state.stop_observing();
            }
        }) as Box<dyn Fn(bool)>);
        let javascript = input_create(read.as_ref(), observe.as_ref());
        Reflect::set(
            &self.object,
            &JsValue::from_str(name),
            &input_api(&javascript),
        )
        .expect("fresh input object");
        *state.javascript.borrow_mut() = Some(javascript);
        self.guards.push(Box::new(Input {
            state,
            _read: read,
            _observe: observe,
        }));
    }
    fn close(&self) {
        for guard in &self.guards {
            guard.close();
        }
    }
}

/// Autoref fallback lets modules without a JsInputs derive receive empty inputs.
#[doc(hidden)]
pub struct InputSource<'a, T>(pub &'a T);
#[doc(hidden)]
pub trait MaybeInputs {
    fn inputs(self) -> Inputs;
}
impl<T> MaybeInputs for &InputSource<'_, T> {
    fn inputs(self) -> Inputs {
        Inputs::default()
    }
}
impl<T: JsInputs> MaybeInputs for InputSource<'_, T> {
    fn inputs(self) -> Inputs {
        self.0.js_inputs()
    }
}

struct Module {
    inputs: Inputs,
    javascript: JsValue,
    closed: Cell<bool>,
}
impl Module {
    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.inputs.close();
        module_dispose(&self.javascript);
    }
}
impl Drop for Module {
    fn drop(&mut self) {
        self.close();
    }
}

/// Compiler entry point: install once, after bindings but before activation.
#[doc(hidden)]
pub fn mount(scope: &mut Scope, id: &str, inputs: Inputs) -> Result<(), JsValue> {
    if scope.is_coherent() || scope.is_hydrating() {
        return Err(JsValue::from_str(
            "fusor: component JavaScript requires a browser component outside coherent Async and island/server delivery",
        ));
    }
    #[cfg(feature = "islands")]
    if crate::dom::delivery::enabled() {
        return Err(JsValue::from_str(
            "fusor: component JavaScript is unsupported in island delivery",
        ));
    }
    for guard in &inputs.guards {
        guard.set_owner(scope.owner());
    }
    let module = Rc::new(Module {
        javascript: module_create(scope.root()?, &inputs.object, id),
        inputs,
        closed: Cell::new(false),
    });
    let weak = Rc::downgrade(&module);
    let cleanup = scope.owner().on_cleanup(move || {
        if let Some(module) = weak.upgrade() {
            untrack(|| module.close());
        }
    });
    let weak = Rc::downgrade(&module);
    let owner: OwnerHandle = scope.owner();
    let activate = owner.on_activate(move || {
        if let Some(module) = weak.upgrade() {
            if let Err(error) = untrack(|| module_activate(&module.javascript)) {
                module.close();
                web_sys::console::error_1(&error);
            }
        }
    });
    scope.retain((module, cleanup, activate));
    Ok(())
}
