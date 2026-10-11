use fusor::prelude::*;
use fusor::{ContextKey, OwnerHandle};
use std::rc::Rc;
use wasm_bindgen::JsValue;

/// The context key: it names the shared value and fixes its type.
struct CurrentUser;
impl ContextKey for CurrentUser {
    type Value = Signal<String>;
}

struct App {
    user: Signal<String>,
}

impl App {
    fn new(owner: OwnerHandle) -> Result<Self, JsValue> {
        let user = signal("Ada".to_owned());
        owner
            .provide::<CurrentUser>(user.clone())
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(Self { user })
    }
}

/// A layout component. It knows nothing about the user.
#[derive(FromInputs)]
struct Sidebar {}

/// Two levels below App, it reads the user from context.
struct UserBadge {
    user: Rc<Signal<String>>,
}

impl UserBadge {
    fn new(owner: OwnerHandle) -> Result<Self, JsValue> {
        let user = owner
            .context::<CurrentUser>()
            .ok_or_else(|| JsValue::from_str("UserBadge needs a CurrentUser provider"))?;
        Ok(Self { user })
    }
}

fusor::template!("web/index.html");

struct UserBadgeInputs {}
impl fusor::dom::FromInputs for UserBadge {
    type Error = fusor::dom::JsValue;
    type Inputs = UserBadgeInputs;
    fn from_inputs(
        _inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Self::new(owner)
    }
}
