use fusor::{ContextKey, OwnerHandle};
use fusor_async::{
    Resource, ResourceState,
    browser::resource,
    fetch::{FetchError, get_text},
};
use std::rc::Rc;
use wasm_bindgen::JsValue;

/// What the page knows about the person using it.
#[derive(Clone, PartialEq)]
enum SessionStatus {
    Checking,
    SignedOut,
    SignedIn { name: String },
    Unavailable { reason: String },
}

/// Asks the server who is signed in. The page never holds a token: the
/// server keeps the session in an HttpOnly cookie that this code can't read.
#[derive(Clone)]
struct Session {
    me: Resource<(), String, FetchError>,
}

impl Session {
    fn load(owner: &OwnerHandle) -> Self {
        let me = resource(
            owner,
            || Some(()),
            |(), cancel| async move { get_text("/api/me", &cancel).await },
        );
        Self { me }
    }

    fn status(&self) -> SessionStatus {
        self.me.with(|state| match state {
            ResourceState::Ready(data) => SessionStatus::SignedIn {
                name: data.value.to_string(),
            },
            ResourceState::Error { error, .. } => match error.as_ref() {
                FetchError::Status { status: 401, .. } => SessionStatus::SignedOut,
                error => SessionStatus::Unavailable {
                    reason: error.to_string(),
                },
            },
            ResourceState::Idle | ResourceState::Loading { .. } | ResourceState::Disposed => {
                SessionStatus::Checking
            }
        })
    }
}

/// The context key every component uses to find the session.
struct CurrentSession;
impl ContextKey for CurrentSession {
    type Value = Session;
}

struct App {
    session: Session,
}

impl App {
    fn new(owner: OwnerHandle) -> Result<Self, JsValue> {
        let session = Session::load(&owner);
        owner
            .provide::<CurrentSession>(session.clone())
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(Self { session })
    }
}

/// Sits in the header; finds the session through context instead of an input.
struct AccountMenu {
    session: Rc<Session>,
}

impl AccountMenu {
    fn new(owner: OwnerHandle) -> Result<Self, JsValue> {
        let session = owner
            .context::<CurrentSession>()
            .ok_or_else(|| JsValue::from_str("AccountMenu needs a CurrentSession provider"))?;
        Ok(Self { session })
    }
}

fusor::template!("web/index.html");

struct AccountMenuInputs {}
impl fusor::dom::FromInputs for AccountMenu {
    type Error = fusor::dom::JsValue;
    type Inputs = AccountMenuInputs;
    fn from_inputs(
        _inputs: Self::Inputs,
        owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Self::new(owner)
    }
}
