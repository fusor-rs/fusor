//! What the router records in browser history: which router owns it, and
//! where each of its entries sits.
use crate::browser::error;
use js_sys::{Object, Reflect};
use std::cell::Cell;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::History;

thread_local! {
    static TAKEN: Cell<bool> = const { Cell::new(false) };
    static SESSION: Cell<u64> = const { Cell::new(0) };
}

/// Browser history has one owner at a time.
pub(super) struct Lease(Cell<bool>);
impl Lease {
    pub(super) fn acquire() -> Result<Self, JsValue> {
        if TAKEN.replace(true) {
            return Err(error("only one fusor router may own browser history"));
        }
        Ok(Self(Cell::new(true)))
    }
    pub(super) fn release(&self) {
        if self.0.replace(false) {
            TAKEN.set(false);
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.release();
    }
}

/// The position of this router's entries in browser history. Each entry's
/// state carries the session, the segment epoch and the entry's index; an
/// entry from another session or segment has no known position.
pub(super) struct Entries {
    session: String,
    epoch: Cell<u32>,
    pub(super) index: Cell<i32>,
}

impl Entries {
    pub(super) fn new() -> Self {
        let session = SESSION.get() + 1;
        SESSION.set(session);
        Self {
            session: format!("{}-{session}", js_sys::Date::now()),
            epoch: Cell::new(0),
            index: Cell::new(0),
        }
    }
    /// Start a new segment at index 0, for entries the router did not create.
    pub(super) fn restart(&self) -> Result<(), JsValue> {
        let epoch = self
            .epoch
            .get()
            .checked_add(1)
            .ok_or_else(|| error("history epoch overflow"))?;
        self.epoch.set(epoch);
        self.index.set(0);
        Ok(())
    }
    /// The state for an entry at `index`, keeping the application's own state.
    pub(super) fn state(&self, history: &History, index: i32) -> Result<JsValue, JsValue> {
        let state = Object::new();
        let previous = history.state()?;
        if previous.is_object() && !previous.is_null() {
            Object::assign(&state, &previous.unchecked_into::<Object>());
        } else if !previous.is_null() && !previous.is_undefined() {
            Reflect::set(&state, &"user".into(), &previous)?;
        }
        let metadata = Object::new();
        Reflect::set(&metadata, &"session".into(), &self.session.as_str().into())?;
        Reflect::set(&metadata, &"index".into(), &index.into())?;
        Reflect::set(&metadata, &"epoch".into(), &self.epoch.get().into())?;
        Reflect::set(&state, &"__fusor".into(), &metadata)?;
        Ok(state.into())
    }
    /// The index of the current entry, if this session and segment made it.
    pub(super) fn current(&self, history: &History) -> Option<i32> {
        let metadata = Reflect::get(&history.state().ok()?, &"__fusor".into()).ok()?;
        let field = |name: &str| Reflect::get(&metadata, &name.into()).ok();
        if field("session")?.as_string()? != self.session
            || field("epoch")?.as_f64()? != f64::from(self.epoch.get())
        {
            return None;
        }
        let index = field("index")?.as_f64()?;
        (index.is_finite() && index.fract() == 0.0 && (0.0..=f64::from(i32::MAX)).contains(&index))
            .then_some(index as i32)
    }
}
