use crate::{app::Store, session::Session};
use fusor::prelude::*;
use std::{cell::Cell, rc::Rc};
use wasm_bindgen::JsValue;

pub struct Away;
pub struct EditorPage {
    store: Rc<Store>,
}
impl EditorPage {
    pub fn new(owner: OwnerHandle) -> Result<Self, JsValue> {
        Ok(Self {
            store: Store::context(&owner)?,
        })
    }
}
pub struct Editor {
    store: Rc<Store>,
    session: Rc<Session>,
    name: &'static str,
    title_id: String,
    title_error: String,
    body_id: String,
    seats_id: String,
    seats_error: String,
}
thread_local! { static VIEW_ID: Cell<u64> = const { Cell::new(0) }; }
impl Editor {
    fn new(store: Rc<Store>, session: Rc<Session>, name: &'static str) -> Self {
        let id = VIEW_ID.with(|next| {
            let id = next.get() + 1;
            next.set(id);
            id
        });
        Self {
            store,
            session,
            name,
            title_id: format!("title-{id}"),
            title_error: format!("title-error-{id}"),
            body_id: format!("body-{id}"),
            seats_id: format!("seats-{id}"),
            seats_error: format!("seats-error-{id}"),
        }
    }
    fn reload(&self) {
        if let Some(data) = self.store.remote.get().data() {
            self.session.reload_reviewed(&data.value);
        }
    }
}
fusor::bindings!(views);

pub struct EditorInputs {
    pub store: Rc<Store>,
    pub name: &'static str,
}
impl fusor::dom::FromInputs for Editor {
    type Inputs = EditorInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        inputs: Self::Inputs,
        _owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        let session = inputs
            .store
            .session
            .get()
            .ok_or_else(|| JsValue::from_str("Editing session is closed"))?;
        Ok(Self::new(inputs.store, session, inputs.name))
    }
}

pub struct EmptyInputs {}
impl fusor::dom::FromInputs for EditorPage {
    type Inputs = EmptyInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        _: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, wasm_bindgen::JsValue> {
        Self::new(owner)
    }
}
impl fusor::dom::FromInputs for Away {
    type Inputs = EmptyInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(_: Self::Inputs, _: fusor::OwnerHandle) -> Result<Self, wasm_bindgen::JsValue> {
        Ok(Self)
    }
}
